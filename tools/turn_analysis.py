#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marc Hoffmann (b14ckyy)
"""Turn analysis over flights in the Kite log database (flights.db).

For every telemetry sample of a flight:

  w_cog   = d(COG)/dt      ground-track turn rate from the GPS course (`heading` column), using only
                           samples where the value changed and the same low-pass the map arc uses
  w_yaw   = d(yaw)/dt      FC fused heading rate (`yaw` column), same method
  w_model = g*tan(roll)/V  coordinated-turn model (Mission Planner's turn rate), V = ground speed

Reported per flight:

  k       regression w_cog = k * w_model over turning samples (|w_cog| >= 3 deg/s, V >= 8 m/s,
          |roll| <= 60 deg, after the first 20 s) — below 1 the aircraft turns slower than its bank
          says (slip), above 1 faster (skid)
  lag     time shift of the model that maximises the correlation with w_cog — how far the GPS
          track trails the bank (plus link latency on live logs)
  corr    correlation at that lag
  turns   every sustained turn (|w_yaw| >= 5 deg/s for >= 4 s) with mean roll, speed, the three
          rates and the ratio w_cog / w_model

Usage:
  python tools/turn_analysis.py                 # list flights with attitude data
  python tools/turn_analysis.py 167 160         # analyse these flight ids
  python tools/turn_analysis.py 167 --turns     # ... and print the per-turn table
  python tools/turn_analysis.py 167 --json out.json   # dump 5 Hz series + stats (for charts)

Reads the DB read-only; the app may stay open. Requires numpy.
"""
import argparse
import json
import os
import sqlite3

import numpy as np

G = 9.80665
DEFAULT_DB = os.path.join(os.environ.get("APPDATA", ""), "kite-gc", "flights.db")


def wrap(d):
    return (d + 540) % 360 - 180


def rate_fresh(t, v, alpha=0.4, max_gap=3.0):
    """Turn rate of a wrapped angle series, sampled only when the value changes (the map's rule)."""
    out = np.zeros_like(v)
    prev = None
    prev_t = 0.0
    r = 0.0
    for i in range(len(v)):
        if np.isnan(v[i]):
            out[i] = r
            continue
        if prev is None:
            prev, prev_t = v[i], t[i]
        elif v[i] != prev:
            dt = t[i] - prev_t
            if 0 < dt <= max_gap:
                r += (wrap(v[i] - prev) / dt - r) * alpha
            else:
                r = 0.0
            prev, prev_t = v[i], t[i]
        out[i] = r
    return out


def load(c, fid):
    rows = c.execute(
        "select timestamp_ms, heading, yaw, roll, speed_ms, airspeed_ms from telemetry_records "
        "where flight_id=? order by timestamp_ms", (fid,)).fetchall()
    return np.array(rows, dtype=float)


def analyse(c, fid):
    a = load(c, fid)
    if len(a) < 50:
        return None
    meta = c.execute(
        "select craft_name, start_time, fc_version, protocol, duration_sec from flights where id=?", (fid,)).fetchone()
    t = (a[:, 0] - a[0, 0]) / 1000.0
    cog, yaw, roll, gs = a[:, 1], a[:, 2], a[:, 3], a[:, 4]
    hz = len(t) / t[-1] if t[-1] > 0 else 0
    w_cog = rate_fresh(t, cog)
    w_yaw = rate_fresh(t, yaw)
    V = np.nan_to_num(gs)
    roll_c = np.clip(np.nan_to_num(roll), -80, 80)
    with np.errstate(divide="ignore", invalid="ignore"):
        w_model = np.degrees(G * np.tan(np.radians(roll_c)) / V)
        w_model[~(V >= 5)] = 0

    base = (V >= 8) & (np.abs(w_cog) >= 3) & (np.abs(np.nan_to_num(roll)) <= 60) & (t > 20)
    if base.sum() < 20:
        return dict(id=fid, meta=meta, hz=hz, n=len(t), turning=int(base.sum()))
    k0 = float(np.dot(w_model[base], w_cog[base]) / np.dot(w_model[base], w_model[base]))
    best = None
    for lag in range(0, int(2.0 * hz) + 1):
        ms = np.roll(w_model, lag)
        mask = base.copy()
        mask[:lag] = False
        x, y = ms[mask], w_cog[mask]
        k = float(np.dot(x, y) / max(np.dot(x, x), 1e-9))
        corr = float(np.corrcoef(x, y)[0, 1]) if np.std(x) > 0 and np.std(y) > 0 else float("nan")
        if best is None or corr > best[1]:
            best = (lag, corr, k)
    lag, corr, k_lag = best

    # sustained turns: runs of one yaw-rate sign >= 4 s
    sign = np.sign(w_yaw) * (np.abs(w_yaw) >= 5)
    segs = []
    s = 0
    for i in range(1, len(t) + 1):
        if i == len(t) or sign[i] != sign[s]:
            if sign[s] != 0 and t[i - 1] - t[s] >= 4:
                segs.append((s, i))
            s = i
    turns = []
    for s, e in segs:
        sl = slice(s, e)
        mm = float(np.mean(w_model[sl]))
        if abs(mm) < 2:
            continue
        turns.append(dict(
            t0=round(float(t[s]), 1), dur=round(float(t[e - 1] - t[s]), 1),
            roll=round(float(np.mean(roll[sl])), 1), V=round(float(np.mean(V[sl])), 1),
            w_cog=round(float(np.mean(w_cog[sl])), 1), w_yaw=round(float(np.mean(w_yaw[sl])), 1),
            w_model=round(mm, 1), ratio=round(float(np.mean(w_cog[sl])) / mm, 2),
            aerobatic=bool(np.max(np.abs(roll[sl])) > 60)))
    clean = [x["ratio"] for x in turns if not x["aerobatic"]]
    return dict(id=fid, meta=meta, hz=hz, n=len(t), turning=int(base.sum()), k0=k0, lag_s=lag / hz,
                corr=corr, k_lag=k_lag, turns=turns,
                ratio_median=float(np.median(clean)) if clean else float("nan"),
                series=dict(t=t, w_cog=w_cog, w_yaw=w_yaw, w_model=w_model, roll=roll, V=V))


def series_5hz(r):
    step = max(1, int(round(r["hz"] / 5)))
    idx = np.arange(0, r["n"], step)
    f = lambda arr: [None if np.isnan(v) else round(float(v), 1) for v in arr[idx]]
    s = r["series"]
    return {k: f(s[k]) for k in ("t", "w_cog", "w_yaw", "w_model", "roll", "V")}


def list_flights(c):
    q = """select f.id, f.start_time, f.craft_name, f.fc_variant, f.protocol, f.duration_sec,
                  (select count(*) from telemetry_records t where t.flight_id=f.id and t.roll is not null) n
           from flights f order by f.start_time desc"""
    print("id | start | craft | fc | protocol | dur | samples with roll")
    for r in c.execute(q):
        if r[6]:
            print(" | ".join(str(x)[:19] for x in r))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("flights", nargs="*", type=int, help="flight ids (none = list flights)")
    ap.add_argument("--db", default=DEFAULT_DB)
    ap.add_argument("--turns", action="store_true", help="print the per-turn table")
    ap.add_argument("--json", metavar="FILE", help="write 5 Hz series + stats for the given flights")
    args = ap.parse_args()
    c = sqlite3.connect(f"file:{args.db}?mode=ro", uri=True)
    if not args.flights:
        list_flights(c)
        return
    out = []
    print("id | craft | date | fc | Hz | turning samples | k | k at lag | lag | corr | turns | ratio median")
    for fid in args.flights:
        r = analyse(c, fid)
        if r is None:
            print(fid, "| too short")
            continue
        m = r["meta"]
        if "k0" not in r:
            print(f"{fid} | {m[0]} | {m[1][:10]} | {m[2]} | {r['hz']:.1f} | {r['turning']} | not enough turning samples")
            continue
        print(f"{fid} | {m[0]} | {m[1][:10]} | {m[2][:24]} | {r['hz']:.1f} | {r['turning']} | {r['k0']:.2f} | "
              f"{r['k_lag']:.2f} | {r['lag_s']:.2f} s | {r['corr']:.2f} | {len(r['turns'])} | {r['ratio_median']:.2f}")
        if args.turns:
            print("    at | len | roll | V | w_cog | w_yaw | model | ratio")
            for x in r["turns"]:
                flag = "  (aerobatic, excluded)" if x["aerobatic"] else ""
                print(f"    {x['t0']:6.0f}s | {x['dur']:4.0f}s | {x['roll']:+6.1f} | {x['V']:4.1f} | {x['w_cog']:+6.1f} | "
                      f"{x['w_yaw']:+6.1f} | {x['w_model']:+6.1f} | {x['ratio']:.2f}{flag}")
        if args.json:
            out.append(dict(id=fid, craft=m[0], start=m[1][:10], fc=m[2], protocol=m[3], dur=m[4], hz=round(r["hz"], 1),
                            k=round(r["k0"], 2), k_lag=round(r["k_lag"], 2), lag_s=round(r["lag_s"], 2),
                            corr=round(r["corr"], 2), turns=r["turns"], series=series_5hz(r)))
    if args.json:
        with open(args.json, "w", encoding="utf-8") as f:
            json.dump(out, f, separators=(",", ":"))
        print("wrote", args.json)


if __name__ == "__main__":
    main()
