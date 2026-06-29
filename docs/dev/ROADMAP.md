# Roadmap

A high-level view of where Kite Ground Control is and where it's going. Kite is currently in **public
beta**, heading toward a **1.0** release. (The detailed internal roadmap and per-feature plans are kept
privately; new public plans are tracked in [`active/`](active/).)

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
- A customisable, dockable **widget dashboard** with a persistent layout.

**Maps & 3D**

- 2D moving map (Leaflet) with track, home, mission, heading-up mode, day/night shading, multiple tile
  providers and an offline tile cache.
- Full **3D mode** (CesiumJS): real terrain, 3D track + mission overlay, an FPV cockpit camera and live
  day/night lighting — seamless 2D ⇄ 3D.

**Missions**

- INAV (MSP-WP) and ArduPilot / PX4 (MAVLink) mission planning, map-based editing, modifier waypoints,
  multi-mission (INAV), a **survey-pattern generator**, undo/redo, AGL/terrain-following waypoints, and
  flown-vs-loaded provenance tracking.

**Flight logbook & libraries**

- Automatic flight recording with replay; import of INAV blackbox, **ArduPilot Dataflash**, MAVLink
  `.tlog` and raw-MSP logs into one searchable history.
- **Vehicle**, **Battery** and **Mission** managers, all linked to the flight log; `.kflight` exchange.

**Safety & awareness**

- Geozones (INAV), geofence (ArduPilot/PX4), safe-home & fixed-wing autoland, airspace overlays
  (airports, controlled airspace, obstacles), and **foreign-vehicle radar** with ADS-B proximity &
  conflict alerts plus an in-flight breach toast.

**Control & misc.**

- GCS **vehicle control** (MAVLink: arm/disarm, modes, takeoff/RTL/loiter, guided "fly here", …),
  **RC control** via HID gamepad/joystick, low-latency **RTSP video**, and **RF link analysis**.

**Platform**

- Windows and Linux (x86 / ARM), a multi-language UI (English, German, French), global UI scaling, and
  persistent layout/settings.

## In progress / planned (toward 1.0)

- **ArduPilot waypoints:** the VTOL-phase model (transition / VTOL-land cues) and Rover/Boat/Sub-specific
  command data; broader MAVLink mission command coverage.
- **Airspace:** in-flight alerts and polish on the Airspace Manager.
- **RF link analysis:** a map representation of link quality, and a link-budget / range phase.
- **PX4:** on-hardware validation of the MAVLink mission, control and RC paths (implemented, beta-tested
  in SITL).
- **Alerts:** distinct MAVLink alert tones by severity.
- **UX:** a custom in-app tooltip / assistance system; widget layout profiles (when the widget set grows).
- **Maps:** tile-handling improvements and 3D terrain refinements.

## Future / exploratory

Ideas under consideration, not yet scheduled (often gated on something external):

- A scriptable **external API** for third-party integrations.
- **Multi-operator** shared/central flight archive.
- **Radio-source radar** and worldwide **UAV no-fly / NFZ** maps from external providers.
- **AI-assisted flight-log analysis.**
- Background **multi-serial** connections for auxiliary devices (ADS-B, radar, …).
- MAVLink **packet signing**, and `tauri-specta` for Rust↔TypeScript type safety.

---

This page is intentionally high-level. To propose or track a specific piece of work, see the
[planning workflow](README.md) and add a plan in [`active/`](active/).
