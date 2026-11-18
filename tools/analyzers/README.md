# Analyzers

Read what Kite recorded — the raw MAVLink `.tlog` of a flight, or the flight database — and answer
questions the app itself does not: why did the link drop, what did the GCS actually send, how does the
aircraft turn compared with its bank angle. All read-only; Kite may stay open.

Where the inputs live (installed builds; portable mode keeps everything under `<app>/data/`):

| Input | Windows | Linux | macOS |
|-------|---------|-------|-------|
| `.tlog` — with **Settings → Raw flight logs** on (off by default) every MAVLink recording also writes `raw_logs/<local time>_flight_N.tlog` under the raw log path | `Documents\KiteGC\` | `~/Documents/KiteGC/` | `~/Documents/KiteGC/` |
| `flights.db` | `%APPDATA%\kite-gc\` | `~/.local/share/kite-gc/` | `~/Library/Application Support/kite-gc/` |

Mission Planner / QGroundControl `.tlog` files have the same format (8-byte big-endian epoch-µs prefix
+ raw MAVLink 1/2 frame) and work as well. Nothing here needs pymavlink.

---

## `tlog_analyze.py` — link overview

The first pass over a `.tlog`: who sent what, at what rate, and what happened around a link cut.

```sh
python tools/analyzers/tlog_analyze.py flight.tlog        # times in UTC
python tools/analyzers/tlog_analyze.py flight.tlog 2      # shifted to UTC+2, the flight's local time
```

Prints the senders (sysid / compid) with message counts, the per-30 s frame rate with sequence-gap
loss, GCS traffic, every STATUSTEXT, mode and arm changes, battery, RADIO_STATUS, the position track and
the final seconds before the last frame.

Reading it when a link died mid-flight: if **all** air-side components stop in the same second the
break is downstream of where their streams merge (the radio / ground unit), not one FC UART; a loss
figure that **rises** over the last minutes points at range or RF, a cut from **zero straight to
silence** at a service or power death. Pair it with Kite's own log: "read 0 bytes in N calls" means
the far end went quiet, "parser errors" means garbage was still arriving.

## `tlog_detail.py` — command and parameter detail

The second pass, once `tlog_analyze.py` has narrowed the window:

```sh
python tools/analyzers/tlog_detail.py flight.tlog [utc_offset_hours]
```

Decodes every non-heartbeat GCS message (commands with their parameters, parameter reads/writes), the
FC's `COMMAND_ACK`s and `PARAM_VALUE`s, the last seconds of air-side traffic before a cut, whether the
GCS sent bursts, and the FC's `POWER_STATUS` rails.

## `turn_analysis.py` — bank vs. turn rate

Compares, for every telemetry sample of a flight, three turn rates:

| | |
|---|---|
| `w_cog` | d(COG)/dt — the ground-track turn rate from the GPS course, low-passed like the map's turn arc |
| `w_yaw` | d(yaw)/dt — the FC's fused heading rate |
| `w_model` | g · tan(roll) / V — the coordinated-turn model (what Mission Planner draws), V = ground speed |

and reports per flight the regression factor `k` (`w_cog = k · w_model` over turning samples — below 1
the aircraft turns slower than its bank says, i.e. slips; above 1 it skids), the `lag` of the GPS track
behind the bank (plus link latency on live logs), the correlation at that lag, and every sustained
turn (`|w_yaw| ≥ 5°/s` for ≥ 4 s) with mean roll, speed and the three rates.

```sh
python tools/analyzers/turn_analysis.py                 # list the flights that have attitude data
python tools/analyzers/turn_analysis.py 167 160         # analyse these flight ids
python tools/analyzers/turn_analysis.py 167 --turns     # … and print the per-turn table
python tools/analyzers/turn_analysis.py 167 --json out.json   # 5 Hz series + stats, for charts
```

Needs **numpy**. The database defaults to `%APPDATA%\kite-gc\flights.db`; on Linux / macOS pass
`--db <path>` (see the table above).
