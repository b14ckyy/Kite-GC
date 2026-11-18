#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marc Hoffmann (b14ckyy)
"""Detail pass on a .tlog: decodes every non-heartbeat GCS message (commands, param ops),
FC COMMAND_ACKs and PARAM_VALUEs, the last seconds of air-side traffic before a link cut,
GCS burst check, and FC POWER_STATUS rails. Companion to tlog_analyze.py.

Usage: tlog_detail.py <file.tlog> [utc_offset_hours]
"""
import struct
import sys
from collections import Counter

if len(sys.argv) < 2:
    print(__doc__)
    sys.exit(1)
PATH = sys.argv[1]
UTC_OFFSET_H = float(sys.argv[2]) if len(sys.argv) > 2 else 0.0

CMD_NAMES = {176: "DO_SET_MODE", 179: "DO_SET_HOME", 192: "DO_REPOSITION",
             400: "ARM_DISARM", 410: "GET_HOME_POSITION", 511: "SET_MESSAGE_INTERVAL",
             512: "REQUEST_MESSAGE"}
RESULTS = {0: "ACCEPTED", 1: "TEMP_REJECTED", 2: "DENIED", 3: "UNSUPPORTED", 4: "FAILED", 5: "IN_PROGRESS"}


def pad(p, n):
    return p + b"\x00" * (n - len(p))


def fmt_t(us):
    s = int(us // 1_000_000 + UTC_OFFSET_H * 3600)
    ms = us % 1_000_000 // 1000
    return f"{s // 3600 % 24:02d}:{s % 3600 // 60:02d}:{s % 60:02d}.{ms:03d}"


records = []
data = open(PATH, "rb").read()
off = 0
while off + 12 <= len(data):
    ts = struct.unpack_from(">Q", data, off)[0]
    magic = data[off + 8]
    if magic == 0xFD:
        plen = data[off + 9]
        incompat = data[off + 10]
        sysid, compid = data[off + 13], data[off + 14]
        msgid = data[off + 15] | (data[off + 16] << 8) | (data[off + 17] << 16)
        records.append((ts, sysid, compid, msgid, data[off + 18 : off + 18 + plen]))
        off += 8 + 12 + plen + (13 if incompat & 1 else 0)
    elif magic == 0xFE:
        plen = data[off + 9]
        sysid, compid = data[off + 11], data[off + 12]
        records.append((ts, sysid, compid, data[off + 13], data[off + 14 : off + 14 + plen]))
        off += 8 + 8 + plen
    else:
        off += 1

t_last_fc = max((r[0] for r in records if r[1] == 1), default=0)

print("--- every non-HEARTBEAT GCS(255) message ---")
for ts, s, c, m, p in records:
    if s != 255 or m == 0:
        continue
    if m == 75:  # COMMAND_INT
        pp = pad(p, 35)
        p1, p2, p3, p4 = struct.unpack_from("<4f", pp, 0)
        x, y = struct.unpack_from("<2i", pp, 16)
        z = struct.unpack_from("<f", pp, 24)[0]
        cmd = struct.unpack_from("<H", pp, 28)[0]
        print(f"  {fmt_t(ts)}  COMMAND_INT {CMD_NAMES.get(cmd, cmd)} frame={pp[32]} "
              f"p=[{p1:g} {p2:g} {p3:g} {p4:g}] lat={x/1e7:.5f} lon={y/1e7:.5f} z={z:g}")
    elif m == 76:  # COMMAND_LONG
        pp = pad(p, 33)
        params = struct.unpack_from("<7f", pp, 0)
        cmd = struct.unpack_from("<H", pp, 28)[0]
        print(f"  {fmt_t(ts)}  COMMAND_LONG {CMD_NAMES.get(cmd, cmd)} params={[f'{x:g}' for x in params]}")
    elif m == 20:  # PARAM_REQUEST_READ
        pp = pad(p, 20)
        pid = pp[4:20].split(b"\x00")[0].decode("ascii", "replace")
        print(f"  {fmt_t(ts)}  PARAM_REQUEST_READ '{pid}' idx={struct.unpack_from('<h', pp, 2)[0]}")
    elif m == 23:  # PARAM_SET
        pp = pad(p, 23)
        val = struct.unpack_from("<f", pp, 0)[0]
        pid = pp[6:22].split(b"\x00")[0].decode("ascii", "replace")
        print(f"  {fmt_t(ts)}  PARAM_SET '{pid}' = {val:g}")
    else:
        print(f"  {fmt_t(ts)}  msgid={m} len={len(p)}")

print("\n--- FC COMMAND_ACK (77) ---")
for ts, s, c, m, p in records:
    if m == 77 and s == 1:
        pp = pad(p, 3)
        cmd = struct.unpack_from("<H", pp, 0)[0]
        print(f"  {fmt_t(ts)}  ACK {CMD_NAMES.get(cmd, cmd)} -> {RESULTS.get(pp[2], pp[2])}")

print("\n--- PARAM_VALUE (22) from FC ---")
for ts, s, c, m, p in records:
    if m == 22 and s == 1:
        pp = pad(p, 25)
        val = struct.unpack_from("<f", pp, 0)[0]
        pid = pp[8:24].split(b"\x00")[0].decode("ascii", "replace")
        print(f"  {fmt_t(ts)}  PARAM_VALUE '{pid}' = {val:g}")

print("\n--- last 3s of air-side traffic (msgid counts) ---")
tail = [r for r in records if r[1] == 1 and r[0] >= t_last_fc - 3_000_000]
print(f"  window {fmt_t(t_last_fc - 3_000_000)} .. {fmt_t(t_last_fc)}: {len(tail)} msgs")
for (c, m), n in Counter((r[2], r[3]) for r in tail).most_common():
    print(f"    comp={c:3d} msgid={m}: {n}")
print("  very last 10 air-side messages:")
for ts, s, c, m, p in [r for r in records if r[1] == 1][-10:]:
    print(f"    {fmt_t(ts)}  comp={c} msgid={m} len={len(p)}")

per_min = Counter(r[0] // 60_000_000 for r in records if r[1] == 255)
print(f"\n--- GCS burst check: max msgs in any minute: {max(per_min.values()) if per_min else 0} "
      f"(60-62 = pure 1Hz heartbeat) ---")

print("\n--- POWER_STATUS (125) samples (FC board Vcc/Vservo, flags) ---")
ps = [(ts, pad(p, 6)) for ts, s, c, m, p in records if m == 125 and s == 1]
sel = ps[:: max(1, len(ps) // 12)]
if ps and ps[-1] not in sel:
    sel += ps[-3:]
for ts, pp in sel:
    vcc, vs = struct.unpack_from("<HH", pp, 0)
    print(f"  {fmt_t(ts)}  Vcc={vcc}mV Vservo={vs}mV flags={struct.unpack_from('<H', pp, 4)[0]:#06x}")
