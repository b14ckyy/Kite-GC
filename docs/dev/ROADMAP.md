# Roadmap

A high-level view of where Kite Ground Control is and where it's going. Kite is currently in the
**release-candidate** phase, heading toward a **1.0** release. (The detailed internal roadmap and
per-feature plans are kept privately; new public plans are tracked in [`active/`](active/).)

## Shipped

Kite is already a full-featured GCS. Broadly, what works today:

**Connectivity**

- INAV over **MSP**, ArduPilot & PX4 over **MAVLink**, and **passive** listen-only telemetry
  (SmartPort / CRSF / LTM / MAVLink) with auto sub-protocol detection.
- Transports: USB/serial, Bluetooth (SPP & BLE), TCP and UDP.
- A **Telemetry Relay** that re-encodes and forwards live telemetry to other ground stations / handsets.

**Telemetry & display**

- Live HUD (attitude, altitude, speed incl. airspeed, vario, battery), sensor & EKF health, link
  statistics, and a compass with wind / ground-track cues.
- A customisable, dockable **widget dashboard** with layout profiles and persistent panel state.
- Flight-controller status messages with severity-tiered alert tones.

**Maps & 3D**

- 2D moving map (Leaflet) with track, home, mission, heading-up mode, day/night shading, multiple tile
  providers and an offline tile cache.
- Full **3D mode** (CesiumJS): real terrain, 3D track + mission overlay, an FPV cockpit camera and live
  day/night lighting — seamless 2D ⇄ 3D.

**Missions**

- INAV (MSP-WP) and ArduPilot / PX4 (MAVLink) mission planning with broad MAVLink mission-command
  coverage, map-based editing, modifier waypoints, multi-mission (INAV), a **survey-pattern generator**
  (rectangle, circle, spiral), undo/redo, AGL/terrain-following waypoints, and flown-vs-loaded
  provenance tracking.

**Flight logbook & libraries**

- Automatic flight recording with replay; import of INAV blackbox, **ArduPilot Dataflash**, MAVLink
  `.tlog` and raw-MSP logs into one searchable history.
- **Vehicle**, **Battery** and **Mission** managers, all linked to the flight log; `.kflight` exchange.

**Safety & awareness**

- Geozones (INAV), geofence (ArduPilot/PX4), safe-home & fixed-wing autoland, airspace overlays and
  in-flight airspace alerts (airports, controlled airspace, obstacles), and **foreign-vehicle radar**
  with ADS-B proximity & conflict alerts plus an in-flight breach toast.

**Control & misc.**

- GCS **vehicle control** (MAVLink: arm/disarm, modes, takeoff/RTL/loiter, guided "fly here", …),
  **RC control** via HID gamepad/joystick, low-latency **RTSP video**, and **RF link analysis**.

**Platform**

- Windows, macOS and Linux (x86 / ARM), a multi-language UI (English, German, French, Chinese), global
  UI scaling, and persistent layout/settings.

## Toward 1.0

The 1.0 feature scope is **complete**. Kite is in the release-candidate phase: features are frozen and
the remaining work is stabilisation — acting on field reports, validating across platforms and hardware,
and fixing bugs. New feature work is tracked under *Post-1.0* below.

## Post-1.0

Tracked work for after the 1.0 release. **Severity** is priority, not risk — *High*: wanted for the next
release or closes a gap users actually hit; *Med*: valuable, scheduled when there's capacity; *Low*: nice
to have, no pressure. **Status** runs Idea → Planned → In progress → Implemented; an implemented entry
moves up into *Shipped* once it's in a release.

| Feature | Severity | Status | Notes |
|---|---|---|---|
| **Data migration framework** | High | Planned | Before 1.0, schema and format changes were hard switches that orphaned old development data. From 1.0 on, every such change needs a real migration path for data users already have. |
| **H.265 / HEVC video** | Med | Idea | Cameras that only emit HEVC (SIYI gimbals, most IP-cam main streams) currently show no picture at all. No browser engine accepts HEVC over WebRTC, so the choice is an ffmpeg transcode (compatibility, but doubly lossy and heavy on ARM) or native decode via MSE (lossless, added latency). Whether it's worth building at all is still open — settle it with an MSE feasibility test first. |
| **PX4 ULog (`.ulg`) import** | Med | Idea | PX4's onboard flight log — the counterpart to ArduPilot Dataflash `.bin`, which the logbook already imports. PX4 is currently the only supported platform with no full-rate log import, only MAVLink `.tlog`. The work is mostly mapping uORB topic fields onto the flight-record schema. |
| **Linux Bluetooth SPP** | Med | Planned | Serial-Port-Profile support on Linux alongside BLE, for older Bluetooth hardware that offers no BLE. |
| **Auxiliary sources over TCP / UDP** | Med | Idea | Background connections to auxiliary devices already work over serial (ADS-B receivers, INAV-Radar); the source config carries a `transport` field that currently only accepts `serial`. Adding TCP/UDP would let network-attached devices — Kite-Link nodes in particular — feed the same pipeline. |
| **Custom tooltip / assistance system** | Low | Idea | Hints are browser-native `title` attributes today (~200 of them) plus Leaflet tooltips on map objects. A custom system would allow styling, richer content and delay control — cosmetic, so it slipped out of the 1.0 scope. |
| **RF link budget (phase 2)** | Low | Idea | The RF analyser currently models relative obstacle loss only, with the map ray layer shipped. A budget phase would add RF power, antenna gain and a real range estimate. |
| **SVG asset extraction** | Low | Idea | Move hardcoded inline SVG icons (the `Button.svelte` icon registry, map marker icons) into editable files under `static/`. Icons that inherit `currentColor` have to stay inline or be inlined at build time. |

## Future / exploratory

Ideas under consideration, not yet scheduled (often gated on something external):

- A scriptable **external API** for third-party integrations.
- **Multi-operator** shared/central flight archive.
- **Radio-source radar** and worldwide **UAV no-fly / NFZ** maps from external providers.
- **AI-assisted flight-log analysis.**
- MAVLink **packet signing**, and `tauri-specta` for Rust↔TypeScript type safety.

---

This page is intentionally high-level. To propose or track a specific piece of work, see the
[planning workflow](README.md) and add a plan in [`active/`](active/).
