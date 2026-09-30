#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marc Hoffmann (b14ckyy)
"""Analyze a Kite-GC / Mission Planner .tlog (8-byte big-endian epoch-us prefix + raw MAVLink frame).

Overview pass: senders, per-30s rate + seq-gap loss, GCS traffic, STATUSTEXT, mode/arm
changes, battery, RADIO_STATUS, position track, and the final seconds before a link cut.
Needs no pymavlink — parses MAVLink 1/2 framing and hand-decodes the relevant payloads.

Usage: tlog_analyze.py <file.tlog> [utc_offset_hours]
       (offset shifts displayed times from UTC to the flight's local time; default 0)
"""
import math
import struct
import sys
from collections import Counter, defaultdict

if len(sys.argv) < 2:
    print(__doc__)
    sys.exit(1)
PATH = sys.argv[1]
UTC_OFFSET_H = float(sys.argv[2]) if len(sys.argv) > 2 else 0.0

PLANE_MODES = {
    0: "MANUAL", 1: "CIRCLE", 2: "STABILIZE", 3: "TRAINING", 4: "ACRO",
    5: "FBWA", 6: "FBWB", 7: "CRUISE", 8: "AUTOTUNE", 10: "AUTO",
    11: "RTL", 12: "LOITER", 13: "TAKEOFF", 14: "AVOID_ADSB", 15: "GUIDED",
    17: "QSTABILIZE", 18: "QHOVER", 19: "QLOITER", 20: "QLAND", 21: "QRTL",
    22: "QAUTOTUNE", 23: "QACRO", 24: "THERMAL", 25: "LOITER_ALT_QLAND",
}

MSG_NAMES = {
    0: "HEARTBEAT", 1: "SYS_STATUS", 2: "SYSTEM_TIME", 20: "PARAM_REQUEST_READ",
    21: "PARAM_REQUEST_LIST", 22: "PARAM_VALUE", 23: "PARAM_SET", 24: "GPS_RAW_INT",
    27: "RAW_IMU", 29: "SCALED_PRESSURE", 30: "ATTITUDE", 32: "LOCAL_POSITION_NED",
    33: "GLOBAL_POSITION_INT", 35: "RC_CHANNELS_RAW", 36: "SERVO_OUTPUT_RAW",
    39: "MISSION_ITEM", 40: "MISSION_REQUEST", 42: "MISSION_CURRENT",
    43: "MISSION_REQUEST_LIST", 44: "MISSION_COUNT", 47: "MISSION_ACK",
    51: "MISSION_REQUEST_INT", 62: "NAV_CONTROLLER_OUTPUT", 65: "RC_CHANNELS",
    66: "REQUEST_DATA_STREAM", 73: "MISSION_ITEM_INT", 74: "VFR_HUD",
    75: "COMMAND_INT", 76: "COMMAND_LONG", 77: "COMMAND_ACK",
    87: "POSITION_TARGET_GLOBAL_INT", 109: "RADIO_STATUS", 111: "TIMESYNC",
    116: "SCALED_IMU2", 124: "GPS2_RAW", 125: "POWER_STATUS", 129: "SCALED_IMU3",
    136: "TERRAIN_REPORT", 137: "SCALED_PRESSURE2", 147: "BATTERY_STATUS",
    148: "AUTOPILOT_VERSION", 152: "MEMINFO", 163: "AHRS", 165: "HWSTATUS",
    168: "WIND", 173: "RANGEFINDER", 174: "AIRSPEED_AUTOCAL", 178: "AHRS2",
    182: "AHRS3", 193: "EKF_STATUS_REPORT", 195: "PID_TUNING", 226: "RPM",
    241: "VIBRATION", 242: "HOME_POSITION", 245: "EXTENDED_SYS_STATE",
    253: "STATUSTEXT", 285: "GIMBAL_DEVICE_ATTITUDE_STATUS",
    11020: "AOA_SSA", 11039: "MCU_STATUS",
}

CMD_NAMES = {
    176: "DO_SET_MODE", 177: "DO_JUMP", 178: "DO_CHANGE_SPEED", 179: "DO_SET_HOME",
    183: "DO_SET_SERVO", 192: "DO_REPOSITION", 300: "MISSION_START",
    400: "COMPONENT_ARM_DISARM", 410: "GET_HOME_POSITION", 511: "SET_MESSAGE_INTERVAL",
    512: "REQUEST_MESSAGE",
}


def pad(payload: bytes, n: int) -> bytes:
    """MAVLink 2 zero-truncates trailing payload zeros — pad back to nominal length."""
    return payload + b"\x00" * (n - len(payload)) if len(payload) < n else payload


def fmt_t(us: int) -> str:
    s = int(us // 1_000_000 + UTC_OFFSET_H * 3600)
    return f"{s // 3600 % 24:02d}:{s % 3600 // 60:02d}:{s % 60:02d}"


def parse_tlog(path):
    """Yield (ts_us, sysid, compid, msgid, seq, payload) for every frame."""
    records = []
    data = open(path, "rb").read()
    off = 0
    bad = 0
    while off + 12 <= len(data):
        ts = struct.unpack_from(">Q", data, off)[0]
        magic = data[off + 8]
        if magic == 0xFD:  # MAVLink 2
            plen = data[off + 9]
            incompat = data[off + 10]
            sysid, compid = data[off + 13], data[off + 14]
            msgid = data[off + 15] | (data[off + 16] << 8) | (data[off + 17] << 16)
            seq = data[off + 12]
            payload = data[off + 18 : off + 18 + plen]
            records.append((ts, sysid, compid, msgid, seq, payload))
            off += 8 + 12 + plen + (13 if incompat & 1 else 0)
        elif magic == 0xFE:  # MAVLink 1
            plen = data[off + 9]
            sysid, compid = data[off + 11], data[off + 12]
            msgid = data[off + 13]
            seq = data[off + 10]
            payload = data[off + 14 : off + 14 + plen]
            records.append((ts, sysid, compid, msgid, seq, payload))
            off += 8 + 8 + plen
        else:
            bad += 1
            off += 1  # resync byte-wise
    return records, bad


records, bad = parse_tlog(PATH)
if not records:
    print("no records parsed")
    sys.exit(1)

t0, t1 = records[0][0], records[-1][0]
print(f"=== {PATH}")
print(f"records={len(records)} resync_bytes={bad}")
print(f"span: {fmt_t(t0)} -> {fmt_t(t1)}  ({(t1 - t0) / 60e6:.1f} min, UTC{UTC_OFFSET_H:+g}h)")

by_sender = Counter((r[1], r[2]) for r in records)
print("\n--- senders (sysid,compid) ---")
for (s, c), n in by_sender.most_common():
    print(f"  sys={s:3d} comp={c:3d}: {n}")

last_by_sender = {}
for r in records:
    last_by_sender[(r[1], r[2])] = r
print("\n--- last message per sender ---")
for (s, c), r in sorted(last_by_sender.items()):
    print(f"  sys={s:3d} comp={c:3d}: {fmt_t(r[0])}  {MSG_NAMES.get(r[3], r[3])}")

print("\n--- FC(1,1) msgs + seq-gaps per 30s (loss estimate) ---")
bins = defaultdict(lambda: [0, 0])
prev_seq = None
for ts, s, c, m, seq, p in records:
    if (s, c) != (1, 1):
        continue
    b = (ts - t0) // 30_000_000
    bins[b][0] += 1
    if prev_seq is not None:
        gap = (seq - prev_seq - 1) % 256
        if 0 < gap < 128:
            bins[b][1] += gap
    prev_seq = seq
for b in sorted(bins):
    n, g = bins[b]
    lost = g / (n + g) * 100 if (n + g) else 0
    print(f"  {fmt_t(t0 + b * 30_000_000)}  rx={n:5d} lost~{lost:4.1f}%  {'#' * min(60, n // 20)}")

print("\n--- GCS(sys=255) sent messages (grouped) ---")
gcs = [r for r in records if r[1] == 255]
for m, n in Counter(r[3] for r in gcs).most_common():
    print(f"  {MSG_NAMES.get(m, m)}: {n}")

print("\n--- GCS traffic in final 180s ---")
comp = [(ts, m) for ts, s, c, m, seq, p in gcs if ts >= t1 - 180_000_000]
i = 0
while i < len(comp):
    j = i
    while j + 1 < len(comp) and comp[j + 1][1] == comp[i][1]:
        j += 1
    name = MSG_NAMES.get(comp[i][1], str(comp[i][1]))
    suffix = f" x{j - i + 1} (until {fmt_t(comp[j][0])})" if j > i else ""
    print(f"  {fmt_t(comp[i][0])}  {name}{suffix}")
    i = j + 1

SEV = ["EMERG", "ALERT", "CRIT", "ERROR", "WARN", "NOTICE", "INFO", "DEBUG"]
print("\n--- FC STATUSTEXT (all) ---")
for ts, s, c, m, seq, p in records:
    if m == 253 and s == 1:
        pp = pad(p, 51)
        txt = pp[1:51].split(b"\x00")[0].decode("ascii", "replace")
        print(f"  {fmt_t(ts)}  [{SEV[pp[0]] if pp[0] < 8 else pp[0]}] {txt}")

print("\n--- FC mode/arm changes (ArduPlane mode table) ---")
prev = None
for ts, s, c, m, seq, p in records:
    if m == 0 and (s, c) == (1, 1):
        pp = pad(p, 9)
        custom = struct.unpack_from("<I", pp, 0)[0]
        armed = bool(pp[6] & 0x80)
        if (custom, armed) != prev:
            print(f"  {fmt_t(ts)}  mode={PLANE_MODES.get(custom, custom)} armed={armed}")
            prev = (custom, armed)
hb = [r for r in records if r[3] == 0 and (r[1], r[2]) == (1, 1)]
if hb:
    print(f"  last FC heartbeat: {fmt_t(hb[-1][0])}")

print("\n--- SYS_STATUS samples (V / A / % / drop_rate / errors_comm) ---")
sysst = [(ts, pad(p, 31)) for ts, s, c, m, seq, p in records if m == 1 and (s, c) == (1, 1)]
shown = sysst[:: max(1, len(sysst) // 25)]
if sysst and sysst[-1] not in shown:
    shown = shown + sysst[-5:]
for ts, pp in shown:
    v = struct.unpack_from("<H", pp, 14)[0] / 1000
    a = struct.unpack_from("<h", pp, 16)[0] / 100
    drop = struct.unpack_from("<H", pp, 18)[0]
    errs = struct.unpack_from("<H", pp, 20)[0]
    rem = struct.unpack_from("<b", pp, 30)[0]
    print(f"  {fmt_t(ts)}  {v:5.2f}V {a:6.2f}A rem={rem:3d}% drop={drop} errC={errs}")

rs = [(ts, pad(p, 9)) for ts, s, c, m, seq, p in records if m == 109]
print(f"\n--- RADIO_STATUS: {len(rs)} msgs ---")
for ts, pp in rs[:: max(1, len(rs) // 20)] if rs else []:
    rxe, fixed = struct.unpack_from("<HH", pp, 0)
    print(f"  {fmt_t(ts)}  rssi={pp[4]} rem={pp[5]} noise={pp[7]}/{pp[8]} rxerr={rxe} txbuf={pp[6]}")

gpos = [(ts, pad(p, 28)) for ts, s, c, m, seq, p in records if m == 33 and (s, c) == (1, 1)]
if gpos:
    lat0 = struct.unpack_from("<i", gpos[0][1], 4)[0] / 1e7
    lon0 = struct.unpack_from("<i", gpos[0][1], 8)[0] / 1e7
    print(f"\n--- GLOBAL_POSITION_INT: start {lat0:.5f},{lon0:.5f}, dist-from-start ---")
    sel = gpos[:: max(1, len(gpos) // 16)]
    if gpos[-1] not in sel:
        sel = sel + [gpos[-1]]
    for ts, pp in sel:
        lat = struct.unpack_from("<i", pp, 4)[0] / 1e7
        lon = struct.unpack_from("<i", pp, 8)[0] / 1e7
        alt = struct.unpack_from("<i", pp, 12)[0] / 1000
        rel = struct.unpack_from("<i", pp, 16)[0] / 1000
        dx = (lon - lon0) * 111320 * math.cos(math.radians(lat0))
        dy = (lat - lat0) * 110540
        print(f"  {fmt_t(ts)}  d={math.hypot(dx, dy):6.0f}m amsl={alt:6.1f} rel={rel:6.1f}")

print("\n--- final 30s: all messages (runs compressed) ---")
tail = [(ts, s, c, m) for ts, s, c, m, seq, p in records if ts >= t1 - 30_000_000]
i = 0
while i < len(tail):
    j = i
    while j + 1 < len(tail) and tail[j + 1][1:] == tail[i][1:]:
        j += 1
    ts, s, c, m = tail[i]
    suffix = f" x{j - i + 1} (until {fmt_t(tail[j][0])})" if j > i else ""
    print(f"  {fmt_t(ts)}  sys={s} comp={c} {MSG_NAMES.get(m, m)}{suffix}")
    i = j + 1
