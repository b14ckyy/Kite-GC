# Developer tools

Scripts that help build, test and diagnose Kite Ground Control. Nothing here ships with the app. Each
folder has a README with one section per tool (what it is for, how to start it, what to set on the
Kite side); the tool's own header comment and `--help` are the full reference.

| Folder | Tools |
|--------|-------|
| [`simulators/`](simulators/README.md) | Stand-ins for an aircraft or a video source: `fc_sim.py` (fake flight controller — ArduPilot over MAVLink, INAV over MSP or MAVLink), `ardupilot-sitl-manager.py` and `inav-sitl-manager.py` (run the real firmware's SITL, single vehicle or swarm), `ltm_sim.py` (LTM telemetry link), `rtsp_test_server.py` (RTSP source for the native video client). |
| [`analyzers/`](analyzers/README.md) | Read what Kite recorded: `tlog_analyze.py` + `tlog_detail.py` (MAVLink `.tlog` link diagnosis), `turn_analysis.py` (turn-rate study over the flight database). |
| [`helpers/`](helpers/README.md) | Development loop: `i18n-key-paths.py` (pseudo-locale that shows every i18n key on screen), `phone-emu.ps1` + `phone-devtools.mjs` (Android emulator / device loop with hot reload and page screenshots). |
| [`models/`](models/README.md) | The procedural 3D model pipeline for `static/models/` (own UAV and radar contacts). |

## Conventions

- Run everything from the **repository root** (`python tools/simulators/fc_sim.py …`); the scripts that
  touch repo files resolve their paths from there.
- Python **3.10+**, standard library only unless the tool's section says otherwise (`turn_analysis.py`
  needs numpy, the model pipeline runs under `uv`). The Tk managers use the Tkinter that ships with
  python.org / Homebrew / distro builds.
- Tools that keep state (downloaded SITL binaries, instance EEPROMs, settings) put it under
  `%LOCALAPPDATA%\kite-sitl` (Windows), `~/Library/Application Support/kite-sitl` (macOS) or
  `~/.local/share/kite-sitl` (Linux) — never inside the repo.
- Every file carries the SPDX header with its author's name, like the rest of the code base.
