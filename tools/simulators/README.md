# Simulators

Everything that stands in for an aircraft or a video source, so Kite can be developed and tested without
hardware. Which one to reach for:

| You want … | Tool |
|------------|------|
| the **real firmware** — ArduPilot's parameters, mission logic, quirks; one vehicle or a swarm | `ardupilot-sitl-manager.py` |
| the real **INAV** firmware — its MSP surface, settings, ports (no flying unless a RealFlight / X-Plane is attached) | `inav-sitl-manager.py` |
| a **flying aircraft in seconds** with nothing to download — ArduPilot over MAVLink, or INAV over MSP / MAVLink, including RC stick control, missions, guided and parameters | `fc_sim.py` |
| a passive **LTM telemetry** link, e.g. for the phone's background-telemetry path | `ltm_sim.py` |
| an **RTSP camera** for the native video client (MJPEG, H.264, H.265) | `rtsp_test_server.py` |

All run from the repository root with a stock Python 3.10+; the two SITL managers need Tkinter (bundled
with python.org, Homebrew and distro Pythons).

---

## `fc_sim.py` — fake flight controller

A flight controller modelled in Python, by Sebastian Kumor (#142). One process, three combinations:

| Command | Kite sees | Kite connection |
|---------|-----------|-----------------|
| `python tools/simulators/fc_sim.py` | ArduPlane / ArduCopter 4.8.0-dev over MAVLink | type **UDP**, host `127.0.0.1`, port `14550` |
| `python tools/simulators/fc_sim.py --firmware inav` | INAV 9.1.0 over MSP | protocol **MSP**, transport **TCP**, host `127.0.0.1`, port `5761` |
| `python tools/simulators/fc_sim.py --firmware inav --protocol mavlink` | INAV's own MAVLink port — deliberately as limited as the firmware's `telemetry/mavlink.c` | type **UDP**, port `14550` |

The airframe is a point mass with turn-rate, climb-rate and acceleration limits taken from the firmware
defaults (ArduPlane `config.h`, INAV `settings.yaml`): a plane cannot hover, so every "hold position"
becomes an orbit; a copter (`--vehicle copter`) stops and hovers. Commands change real vehicle state
instead of being acknowledged and ignored — arm/disarm (refused without a 3D fix or in flight), modes,
takeoff, land, RTL, guided reposition, change speed / heading, set home, pause / continue, missions
(upload, download, clear, then flown incl. loiter turns/time, `DO_JUMP`, `CONDITION_DELAY/YAW`),
parameters or INAV settings, `SET_MESSAGE_INTERVAL` (really re-rates the stream), and RC stick
control. Anything unimplemented answers `MAV_RESULT_UNSUPPORTED` or an MSP error frame, never a
plausible-looking success.

INAV has no arm or mode *command* — as on a real board both come from RC channels, which Kite's RC page
drives through `MSP_SET_RAW_RC`: **CH5** arm (`< 1300` disarmed, `> 1700` armed), **CH6** mode
(`< 1300` MANUAL, `1300–1700` POSHOLD, `> 1700` WAYPOINT MISSION), CH1–4 sticks.

Switches for provoking the UI: `--no-fix` (no GPS, prearm warning), `--disarmed` (parked at home),
`--mode loiter|auto|manual|rtl`, `--chatter` (ArduPilot STATUSTEXT nag, exercises the toast de-dup),
`--no-gcs-nav` (INAV-MAVLink: every guided target comes back DENIED, like a board without the GCS NAV
box), `--lat/--lon`, `--radius`, `--speed`, `--alt`, `--version` (INAV version to report), `--verbose`.

MAVLink layouts, CRC_EXTRA seeds and enums are read at runtime from the dialect XML of the vendored
`mavlink` crate (`~/.cargo/registry/…/ardupilotmega.xml`, present after any `cargo check` of
`src-tauri`); `--defs PATH` points elsewhere. MSP layouts follow INAV's `fc_msp.c` field order — that is
how the 1.0.1 HDOP field-offset bug was found. The INAV-MAVLink model reproduces one firmware
contradiction on purpose (the `DO_REPOSITION` altitude in `MAV_FRAME_GLOBAL` is used as metres above
home, not AMSL — iNavFlight/inav#11884); the docstring explains why, so nobody validates a GCS's
altitude conversion against it.

## `ardupilot-sitl-manager.py` — ArduPilot SITL desk

Runs ArduPilot's own SITL binaries — single vehicle or a swarm of up to 32 — without Mission Planner
(whose Swarm button is broken with current binaries: `-P` options rejected, `SYSID_THISMAV` renamed
`MAV_SYSID`, and an instance dies when a TCP client disconnects).

```sh
python tools/simulators/ardupilot-sitl-manager.py                # the window
python tools/simulators/ardupilot-sitl-manager.py --auto-start   # window, Start pressed for you
python tools/simulators/ardupilot-sitl-manager.py --headless --count 3 --frame quad --layout udp --seconds 600
```

- **Vehicles**: the common ArduPilot frames (plane, quadplane, copter frames, heli, rover, boat, sub)
  with their default parameter files, start position from named presets, swarm formation (line / grid,
  spacing), speedup, sysid per vehicle, extra parameters applied after boot via `PARAM_SET` and confirmed
  by the `PARAM_VALUE` echo (an unknown name is reported, not silently dropped — names change between
  versions: `ARMING_CHECK` → `ARMING_SKIPCHK`, `SYSID_MYGCS` → `MAV_GCS_SYSID` on master).
- **Kite link layouts** — the window's hint line shows exactly what to enter:
  - *TCP per vehicle*: vehicle k on `tcp 127.0.0.1:5760 + 10·k` (5760, 5770, 5780 …)
  - *UDP fan-in*: every vehicle pushes to one UDP port (default 14550; the host is configurable, so a
    Kite on another machine works)
  - *Chain* (Mission Planner style): ArduPilot routing carries every sysid on vehicle 1's `tcp 5760`
- **Live status** per vehicle (mode, armed, altitude, speed, heading, GPS, battery, last STATUSTEXT) over
  the manager's own UDP monitor channel (SERIAL5 of each instance); the manager announces itself as
  sysid 254 so its heartbeats never mask a lost Kite link. A **watchdog** restarts an instance that
  exits (the Windows Cygwin builds die the moment a TCP client disconnects — a Kite disconnect would
  otherwise leave a dead port).
- **Binaries**: Windows uses Mission Planner's `Documents\Mission Planner\sitl` folder or downloads a
  channel (latest / Stable / Beta / PlaneStable / …) from firmware.ardupilot.org; Linux downloads the
  native SITL builds; macOS points at a waf build (`./waf configure --board sitl && ./waf plane`).
  Parameter files come from the ArduPilot GitHub tree. Linux and macOS paths are untested so far.
- **State**: settings + presets, instance EEPROMs / logs, downloads and parameter files under the
  `kite-sitl` data dir (see [../README.md](../README.md)).

## `inav-sitl-manager.py` — INAV SITL desk

The INAV sibling: fetches INAV's SITL binaries, runs one or several instances and shows their state.

```sh
python tools/simulators/inav-sitl-manager.py                                   # the window
python tools/simulators/inav-sitl-manager.py --headless --count 2 --nightly latest --seconds 600
python tools/simulators/inav-sitl-manager.py --headless --configurator latest --wipe
```

- **Binaries without building**: every nightly (`iNavFlight/inav-nightly`) ships `sitl-resources.zip`
  with the SITL for Windows (Cygwin), Linux (x86_64 / arm64) and macOS; every Configurator release zip
  carries the SITL of that version under `resources/sitl/` (pulled out of the 150 MB zip with HTTP
  range reads). Or point at your own build. The table shows the binary's own banner — nightly tags
  carry the wrong version number.
- **Ports**: instance k listens on TCP `5760 + 10·k`. INAV's SITL has eight UARTs (base port + 0…7)
  but only binds those with a function; the default config is MSP on UART1 (Kite) and UART2 (the
  manager). More links are assigned in the Configurator's Ports tab (e.g. MAVLink on UART3 → port 5762
  for a Kite MAVLink link; telemetry functions also need the Telemetry feature). The manager only
  **reads** the serial configuration back and shows it per UART — it never writes the vehicle's
  configuration. `--wipe` / the wipe checkbox deletes the instance's `eeprom.bin` before the start.
- **Kite connection**: protocol **MSP**, transport **TCP**, host `127.0.0.1`, port `5760` (5770, 5780 …
  for the other instances).
- **No physics of its own**: without a simulator the firmware runs "configurator only" — arming stays
  blocked, nothing flies. The first instance can be attached to RealFlight or X-Plane (`--sim`, host,
  port, `--useimu`, `--chanmap`) and to a serial receiver / proxy FC. For a flying INAV without a
  simulator, use `fc_sim.py --firmware inav`.
- **Live status** per instance over the manager's own MSP link on UART2 (firmware version, armed /
  arming blocked, modes, GPS, altitude, speed, heading, battery); watchdog restart on exit. INAV's SITL
  survives client disconnects, so that only matters for crashes.

## `ltm_sim.py` — LTM telemetry link

A fake aircraft for Kite's passive **Telemetry** protocol: LightTelemetry frames (A / G / S at `--rate`,
O at 1 Hz) the way an INAV LTM output sends them. Arms after `--arm-after` seconds, then circles
`--home` at `--radius` m in CRUISE with a slowly sagging battery. Made for the phone's background
telemetry path (foreground service, notification, track backfill) but works anywhere.

```sh
python tools/simulators/ltm_sim.py --udp 192.168.1.87:14551   # send TO the phone; Kite: Telemetry · UDP · port 14551
python tools/simulators/ltm_sim.py --tcp 14551                # Kite connects to us: Telemetry · TCP · 127.0.0.1:14551
```

Over UDP Kite binds the port locally and learns the peer from the first datagram, so the sim sends to
the phone's address. The TCP variant pairs with `adb reverse tcp:14551 tcp:14551` when the phone has no
Wi-Fi. Options: `--rate 5`, `--home lat,lon`, `--radius 300`, `--speed 15`, `--arm-after 3`,
`--seconds N`.

## `rtsp_test_server.py` — RTSP source

A minimal RTSP server for benching the native RTSP client: serves **one** client (OPTIONS / DESCRIBE /
SETUP / PLAY over TCP), then streams RTP over UDP or TCP-interleaved, whichever the client asked for.

```sh
python tools/simulators/rtsp_test_server.py --port 8600                              # synthetic MJPEG, no file needed
python tools/simulators/rtsp_test_server.py --port 8600 --codec h264 --file clip.264 # RFC 6184, single-NAL + FU-A
python tools/simulators/rtsp_test_server.py --port 8600 --codec h265 --file clip.265 # RFC 7798
```

- MJPEG (RFC 2435) is synthetic; every 10th frame has two fragments swapped (UDP only) to exercise the
  client's reorder window.
- H.264 / H.265 need an Annex-B elementary stream **with access-unit delimiters**:
  `ffmpeg -i in.mp4 -c:v libx264 -bf 0 -bsf:v h264_metadata=aud=insert -f h264 clip.264`
  (`hevc_metadata=aud=insert` for H.265). Parameter sets go into the SDP, access units loop at `--fps`.
- Kite side: video source `rtsp://127.0.0.1:8600/test` (the request path is not checked). The ignored
  end-to-end tests in `src-tauri/src/video/` (`rtsp_native.rs`, `win_sink.rs`, `linux_sink.rs`) expect a
  running server: `KITE_RTSP_URL=rtsp://127.0.0.1:8600/test cargo test <name> -- --ignored --nocapture`.
- `--fps 15`, `--seconds 30` (how long it streams after PLAY).
