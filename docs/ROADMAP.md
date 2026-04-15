# INAV GCS — Feature Roadmap

This document tracks planned features, organized by milestone.

## Legend
- [ ] Not started
- [~] In progress
- [x] Completed

---

## Milestone 1: Foundation (v0.1.x)

- [x] Project setup (Tauri + Svelte + TypeScript)
- [x] Modular code structure (Rust backend + Svelte frontend)
- [x] MSP v1/v2 protocol codec (encode/decode)
- [x] MSP streaming parser (byte-by-byte state machine)
- [x] Serial port listing and selection UI
- [x] Serial port connection/disconnection
- [x] Basic MSP handshake (API_VERSION, FC_VARIANT, FC_VERSION, BOARD_INFO)
- [x] INAV version parsing & minimum version check (≥ 7.0)
- [x] Version-dependent feature gating system
- [x] FC info panel with feature availability display
- [x] Connection status display in toolbar
- [x] Leaflet map integration with OSM tile layer
- [x] Build scripts (Windows, Linux)
- [x] Development documentation
- [x] Session persistence (port, baud rate, map position/zoom, panel state)
- [x] Floating navigation panel with hamburger menu
- [x] Tab-based panel system (UAV Info, Settings, Mission Control)
- [x] Bottom telemetry overlay strip (placeholder widgets)
- [x] Map zoom controls repositioned (top-right)

## Milestone 2: Basic Monitoring (v0.2.x)

- [ ] MSP telemetry polling (STATUS, RAW_GPS, ATTITUDE, ALTITUDE, ANALOG)
- [ ] Telemetry data display panel (HUD-style)
- [ ] Aircraft position on map (GPS marker with heading)
- [ ] Battery voltage/current display
- [ ] Flight mode display
- [ ] Arming status indicator
- [ ] GPS satellite count and fix type

## Milestone 3: Enhanced Monitoring (v0.3.x)

- [ ] Flight path trail on map
- [ ] Multiple map tile providers (OSM, Satellite, Terrain)
- [ ] Compass / heading indicator widget
- [ ] Altitude graph (real-time)
- [ ] Home position marker

## Milestone 4: Flight Recording & Logbook (v0.4.x)

- [ ] Flight recording engine (optional, toggled via settings)
- [ ] User-configurable database storage path (portable)
- [ ] Automatic per-flight telemetry recording (start on arm, stop on disarm)
- [ ] Flight logbook UI — list all recorded flights with metadata
- [ ] Individual flight telemetry replay on map
- [ ] Blackbox log import and attachment to flights
- [ ] Flight statistics & analysis (from Blackbox high-res data)
- [ ] Cloud sync support (optional, mobile-first)
- [ ] Export flight data (CSV, KML, GPX)

## Milestone 5: Mission Planning (v0.5.x)

- [ ] Waypoint placement on map
- [ ] Mission upload to FC (MSP WP commands)
- [ ] Mission download from FC
- [ ] Mission save/load (file)
- [ ] Waypoint editing (altitude, speed, actions)
- [ ] INAV mission extensions (RTH, LAND, etc.)

## Milestone 6: Advanced Features (v0.6.x+)

- [ ] LTM protocol support
- [ ] MAVLink telemetry support
- [ ] CRSF protocol support
- [ ] Bluetooth (BLE) transport
- [ ] TCP/UDP transport
- [ ] OSD font/element preview
- [ ] Safehome editor
- [ ] Log replay (Blackbox, OTX/ETX)
- [ ] HID controller input (gamepad/joystick)
- [ ] Multi-aircraft monitoring (INAV Radar)
- [ ] Audio status alerts (TTS)
- [ ] Survey/area planner
- [ ] Terrain analysis
- [ ] Embedded video stream
- [ ] FW approach / autoland planner
- [ ] Geozone editor
- [ ] CesiumJS 3D map view (terrain, 3D flight paths, log replay in 3D)

---

*Last updated: 2026-04-15*
