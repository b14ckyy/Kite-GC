# Kite Ground Control — Feature Roadmap

This document tracks planned features, organized by milestone.

## Legend
- [ ] Not started
- [~] In progress
- [x] Completed

---

## ✅ Milestone 1: Foundation (v0.1.x)

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

## ✅ Milestone 2: Basic Monitoring (v0.2.x)

### MSP Scheduler & Transport
- [x] MSP scheduler (dedicated thread, owns SerialConnection after handshake)
- [x] Priority-based request queue (telemetry slots + command/bulk channels)
- [x] Request-Response flow control (next request only after reply/timeout)
- [x] Adaptive polling with priority-based degradation (no link type detection needed)

### Telemetry Polling
- [x] Attitude group: `MSP_ATTITUDE` — configurable 1–5 Hz (default 5)
- [x] Analog group: `MSPV2_INAV_ANALOG` — fixed 1 Hz
- [x] Position primary: `MSP_RAW_GPS` — configurable 1–5 Hz (default 2)
- [x] Position secondary: `MSP_ALTITUDE` — staggered rotation at position rate
- [x] Airspeed module: `MSPV2_INAV_AIR_SPEED` — optional, toggleable (default off)
- [x] Status group: `MSPV2_INAV_STATUS` — fixed 1 Hz
- [x] Telemetry data pushed via Tauri events to frontend

### Adaptive Degradation
- [x] Priority-based slot selection: when overloaded, high-priority groups polled first
- [x] Degradation order: PositionSecondary → PositionPrimary → Analog → Status → Attitude
- [x] Natural rate reduction: polls at configured rate as long as link sustains it

### Frontend Display
- [x] Live telemetry strip (ALT, SPD, VARIO, BAT, SATS) with real data
- [x] Aircraft position on map (GPS marker with heading arrow)
- [x] Battery voltage/current display
- [x] Arming status indicator (pulsing ARMED widget + status bar)
- [x] GPS satellite count and fix type (sensor bar + telemetry strip)
- [x] Sensor bar live indicators (GYRO, ACC, MAG, BARO, GPS)

### MSP Data Correctness (Bug Fixes)
- [x] Fix MSPV2_INAV_ANALOG decode (correct byte offsets for voltage, current, power)
- [x] Fix GPS fix type display mapping (0=NO GPS, 1=NO FIX, 2=2D, 3=3D)
- [x] Sensor bar driven by MSP_SENSOR_STATUS (151) hardware health instead of connection state
- [x] BARO status from actual sensor health (not altitude ≠ 0 heuristic)
- [x] GPS sensor warning state (yellow when sensor OK but no fix)
- [x] UAV platform type detection via MSP2_INAV_MIXER (0x2010) during handshake
- [x] Platform type display in UAV Info panel (Multirotor, Airplane, Helicopter, etc.)

### Dev Tools
- [x] MSP Debug Monitor panel (dev builds only, `import.meta.env.DEV` gated)
- [x] Per-message LED indicators (yellow=request, green=response, red=timeout)
- [x] MSG/s and bytes/s throughput counters (TX/RX)
- [x] Target rate vs actual rate per MSP code with throttle highlighting
- [x] POLL/INIT status badges (active polling vs handshake-only)
- [x] Zero-cost release build: `#[cfg(debug_assertions)]` with no-op stub

### Settings
- [x] Attitude poll rate setting (1–5 Hz)
- [x] Position poll rate setting (1–5 Hz)
- [x] Airspeed module toggle (on/off)
- [x] Settings persisted in localStorage, passed to backend on connect

## ✅ Milestone 3: Enhanced Monitoring (v0.3.x)

### Map Providers
- [x] Map provider configuration system (`mapProviders.ts`)
- [x] OSM Standard (default)
- [x] ESRI World Street Map
- [x] ESRI World Imagery (Satellite)
- [x] ESRI Hybrid (Satellite + Boundaries & Places + Transportation overlays)
- [x] OpenTopoMap (contour lines)
- [x] CartoDB Dark Matter (dark theme)
- [x] Provider selection in Settings panel (persisted)
- [x] Live provider switching without page reload

### Tile Cache
- [x] IndexedDB-backed tile cache (`tileCache.ts`)
- [x] Custom Leaflet TileLayer with cache integration (`CachedTileLayer.ts`)
- [x] Configurable cache size (No Cache / 100 / 200 / 500 / 1000 MB)
- [x] LRU eviction strategy (oldest tiles removed first when limit reached)
- [x] Cache fill indicator with progress bar in Settings
- [x] Clear cache button

### HUD Widget System
- [x] Widget bar layout (grid: left–center–right, viewport-relative sizing)
- [x] AHI — Artificial Horizon Indicator (SVG, pitch/roll animation, pitch ladder, roll scale)
- [x] Speed widget (ground speed + optional airspeed)
- [x] Altitude widget (altitude + vario with directional indicator)
- [x] Battery widget (voltage bar, voltage, current, mAh drawn)
- [x] GPS widget (satellite count + fix type with color coding)
- [x] Compass widget (rotating compass rose with heading + cardinal label)
- [x] Home widget (direction arrow, distance, bearing — set on arm + GPS fix)
- [x] Per-widget ON/OFF toggle in Settings (persisted)
- [x] All sizes viewport-relative (vmin units, no fixed pixels)

### Map Features
- [x] Flight path trail (polyline, INAV blue, 1m minimum distance filter)
- [x] Home position marker (green "H" circle, set on arm transition)
- [x] Home position store for cross-component access

### Raw Telemetry Panel
- [x] Right-side floating panel with all numeric telemetry values
- [x] 12 data rows: ALT, SPD, VRT, HDG, ROL, PIT, BAT, CUR, MAH, SAT, RSSI, CPU

### Customizable Widget Panels
- [x] Drag-and-drop widget panel system (bottom + right panels)
- [x] Large (AHI, Compass) and Small (all others) widget classes
- [x] Dynamic panel sizing — adapts to content, shrinks only at screen edge
- [x] Half-position insertion detection with visual insertion indicator
- [x] Cross-panel drag (move widgets between bottom and right panels)
- [x] Widget position memory — toggle OFF/ON restores last panel assignment
- [x] Edit mode toggle button (✎) with visible panel outlines
- [x] Per-widget ON/OFF toggle in Settings with panel label indicator

### Map View Modes
- [x] North-Up mode (default) — map oriented north
- [x] Heading-Up mode — map rotates with UAV heading, auto-centers on UAV
- [x] View mode toggle button on map (compass/heading indicator)

## [~] Milestone 4: Mission Planning (v0.4.x)

### INAV Mission System (MSP WP)
- [x] MSP WP codec — `MSP_WP` (118), `MSP_SET_WP` (209), `MSP_WP_GETINFO` (20)
- [x] MSP mission save/load to EEPROM — `MSP_WP_MISSION_SAVE` (18), `MSP_WP_MISSION_LOAD` (19)
- [x] Waypoint data model — all 8 INAV action types (WAYPOINT, POSHOLD_UNLIM, POSHOLD_TIME, RTH, SET_POI, JUMP, SET_HEAD, LAND)
- [x] Mission download from FC (sequential MSP_WP reads)
- [x] Mission upload to FC (sequential MSP_SET_WP writes + verification)
- [x] End-mission flag management (0xA5 on last WP)
- [x] FlyBy Home waypoint support (flag 0x48)
- [x] Mission clear / new mission

### Map-Based Editing
- [x] Waypoint placement on map (click-to-add)
- [x] Waypoint drag to reposition
- [x] Click-on-polyline to insert WP between existing waypoints
- [x] Floating editor popup per selected WP (type, altitude, speed, hold time)
- [x] Floating parameter labels on non-selected WPs (altitude, modifiers summary)
- [x] P3 bitfield support (altitude mode toggle: REL/AMSL)
- [x] Map click with editor open deselects WP (does not add new)
- [x] Type-specific SVG marker icons:
  - Waypoint: blue teardrop with number
  - PosHold: orange circle with orbit ring, number + hold time
  - POI: purple circle with eye icon + number
  - Land: orange teardrop with down-arrow (no number)
  - RTH: orange house icon
  - Generic fallback: grey circle

### Modifier WP Support
- [x] Modifier WPs (JUMP, RTH, SET_HEAD) grouped into parent geo-WP editor
- [x] Add/remove modifiers via dropdown in editor popup
- [x] Display numbering skips modifier WPs
- [x] Sidebar indents modifiers without numbers
- [x] SET_POI: standalone marker on map, excluded from flight path polyline

### Mission Path Visualization
- [x] Flight path polyline connecting geo-WPs (excludes POI)
- [x] Dashed lines for JUMP modifier (purple, target WP indication)
- [x] Dashed lines for RTH modifier (orange, back to first WP)
- [x] WPs after first LAND/RTH greyed out (35% opacity markers, dashed grey polyline)
- [x] Greyed WPs are non-draggable and have no editor popups

### Sidebar Panel (MissionPanel)
- [x] WP list table (#, Type, Alt, Params) with scrollable body and sticky header
- [x] Read-only detail view for selected WP
- [x] Edit mode toggle
- [x] Waypoint reorder (move up/down buttons in editor)
- [x] FC Download / FC Upload buttons (RAM)
- [x] EEPROM Save / EEPROM Load buttons (save disabled when armed)
- [x] File Open / Save with native file picker dialog (.mission XML format)
- [x] Drag & drop .mission file import
- [x] WP count display (n/120)
- [x] Dirty state indicator
- [x] WPs after LAND/RTH shown greyed in list
- [x] Scrollable WP list with fixed (non-scrolling) control buttons

### Safety & Limits
- [x] Max 120 WP sanity check (map click, polyline insert, modifier add all blocked)
- [x] EEPROM save disabled when FC is armed
- [x] Warning text in modifier dropdown when limit reached

### UI / Theming
- [x] Dark-themed scrollbars (custom WebKit + color-scheme: dark)
- [x] Dark-themed number inputs and select dropdowns in editor popup
- [x] Global `color-scheme: dark` on HTML root element
- [x] Editor popup flicker fix (popup on map, direct DOM innerHTML update)

### Multi-Mission
- [x] Dynamic mission tabs [1][+] with up to 9 missions
- [x] 120 WP global limit across all missions
- [x] Per-mission + total WP count display
- [x] Switch / add / remove missions via tab UI

### Mission Control Settings
- [x] Default WP Altitude (1–1000 m, default 50 m)
- [x] Default PH Time (1–600 s, default 30 s)
- [x] Settings used when placing new WPs and switching type to PosholdTime
- [x] Stepper +/− buttons matching WP editor popup style

### UX
- [x] Edit mode auto-disables when leaving Mission tab or closing panel

### Internationalization (i18n)
- [x] `svelte-i18n` library integration with ICU Message Format
- [x] English locale (default, ~200 translation keys)
- [x] German locale (complete translation)
- [x] Locale initialization in app layout (waits for locale load before rendering)
- [x] All UI strings extracted to locale files (14 component files converted)
- [x] Language picker in Settings panel with persistence
- [x] WP action labels via i18n keys (`WP_ACTION_KEYS` map)
- [x] Widget registry with `labelKey` for translated widget names
- [x] Rust backend errors remain English (technical strings)

### Installer & Distribution
- [x] NSIS installer: `installMode: both` — per-user or all-users choice
- [x] NSIS uninstall hook: optional AppData cleanup dialog
- [x] Portable mode: `.portable` marker → `data/` folder next to exe (Windows + Linux)

### Future: Mission Enhancements
- [ ] Undo/redo for mission edits
- [ ] Abstraction layer for protocol-specific mission systems (ArduPilot/PX4 MAVLink)

## Milestone 5: Flight Recording & Logbook (v0.5.x)

### Flight Recording Engine
- [x] SQLite database via `rusqlite` (bundled, zero user dependencies)
- [x] Schema migration system (`user_version` pragma, sequential migrations)
- [x] User-configurable database storage path (respects portable mode)
- [ ] Protocol-agnostic recording: works with any `TelemetrySource` that provides arming state
- [x] Records ONLY from primary connection (no secondary telemetry sources)
- [x] Automatic flight session creation on arm event
- [x] Automatic session close on disarm event
- [x] Telemetry data recording at configured poll rate (lat, lon, alt, speed, heading, vario, battery, RSSI, timestamp)

### Flight Metadata
- [~] Core: date/time, duration, max altitude, max speed, max distance from home, total distance, battery usage
- [~] Location: start GPS coordinates + reverse-geocoded place name via OSM Nominatim API
- [x] Aircraft: craft name (from FC, queried during handshake), craft type (platform type)
- [~] Source: telemetry protocol (MSP/MAVLink/CRSF/LTM), firmware variant + version
- [~] Weather: temperature, wind speed/direction, conditions at flight start (Open-Meteo, free API)
- [ ] Weather + geocode fetched at ARM time (async spawn, non-blocking) instead of lazy on logbook view

### Handshake Enhancement
- [x] Query craft name from FC during handshake (`MSP_NAME` / `MSP2_COMMON_SETTING` or equivalent per protocol)
- [x] Store craft name in FC info for UI display + flight metadata

### Flight Logbook UI
- [x] Logbook panel/tab with flight list
- [x] Groupable sort modes:
  - Aircraft → Location → Date → Flights by time (model-centric pilots)
  - Location → Date → Aircraft → Flights by time (location-centric pilots)
  - Date → Location → Aircraft → Flights by time (chronological)
  - Aircraft → Date → Location → Flights by time (per-model history)
- [ ] Collapsible group headers with flight count + aggregate stats
- [x] Flight detail view with metadata summary (location, weather, aircraft, source)
- [ ] Flight path replay on map (animated marker playback)
- [ ] Playback controls (play, pause, speed 1x/2x/4x, scrub timeline)
- [x] Delete flight records
- [ ] Search/filter (by aircraft name, location, date range)

### Blackbox Integration
- [ ] External `blackbox_decode` binary discovery (app folder → PATH fallback)
- [ ] Blackbox decode invocation: `blackbox_decode --merge-gps --datetime --unit-height m --stdout <file>`
- [ ] CSV parsing in Rust → JSON blob per row (dynamic fields, INAV-version-independent)
- [ ] Original .TXT file archived as BLOB in `blackbox_files` table (re-downloadable)
- [ ] Standalone Blackbox import: creates new flight with `source: "blackbox"`, metadata from header
- [ ] Attach Blackbox to existing live flight: `source: "both"`, playback toggle MSP vs Blackbox
- [ ] `flights.source` field: `live` | `blackbox` | `both` — Blackbox-only flights marked with icon
- [ ] Multi-log support: single .TXT may contain multiple ARM/DISARM sessions (`--index N`)
- [ ] Blackbox-imported flights use header metadata (FW version, date, GPS start, duration)
- [ ] NOT a full Blackbox analyzer — no PID/gyro/motor visualization (use dedicated tools)

### Export
- [ ] Export flight path as KML (Google Earth)
- [ ] Export flight path as GPX (universal GPS format)
- [ ] Export telemetry as CSV

## Milestone 5b: Blackbox Import (v0.5.x — current focus)

### Blackbox Decode Pipeline
- [ ] Settings: auto-detect `blackbox_decode` in app folder, fallback to PATH
- [ ] Invoke `blackbox_decode --merge-gps --datetime --unit-height m --stdout <file>` as child process
- [ ] Parse CSV stdout in Rust (dynamic columns from header line)
- [ ] Store parsed rows as JSON blobs in `blackbox_records` table
- [ ] Archive original .TXT as BLOB in `blackbox_files` table
- [ ] Clean up intermediate CSV after import

### Standalone Import
- [ ] "Import Blackbox" button in Logbook tab
- [ ] File picker for .TXT / .bbl / .bfl files
- [ ] Extract metadata from Blackbox header: FW type/version, date/time, duration, GPS start
- [ ] Create flight entry with `source: "blackbox"` marker
- [ ] Logbook shows Blackbox-only flights with distinct icon
- [ ] Multi-log .TXT: show picker for which log index to import

### Attach to Existing Flight
- [ ] "Attach Blackbox" button in flight detail view
- [ ] Link Blackbox data to existing live-recorded flight
- [ ] Flight marked as `source: "both"`
- [ ] Playback UI toggle: MSP telemetry vs Blackbox data

### DB Schema Extension (v2 migration)
- [ ] `blackbox_records` table: `flight_id`, `timestamp_us`, `csv_data` (JSON TEXT)
- [ ] `blackbox_files` table: `flight_id`, `original_filename`, `log_index`, `file_data` (BLOB), `file_size`, `imported_at`
- [ ] `flights.source` column: `live` | `blackbox` | `both`
- [ ] `PRAGMA user_version = 2` migration from v1

---

## Milestone 6: Advanced Features (v0.6.x+)

### Multi-Protocol Telemetry Architecture
- [ ] `TelemetrySource` trait — protocol-agnostic abstraction (refactor `poll_slot`)
- [ ] `MspSource` — existing MSP poll/decode extracted into trait impl
- [ ] `LtmSource` — LTM (Lightweight Telemetry) protocol support
- [ ] `MavlinkSource` — MAVLink v1/v2 telemetry + heartbeat (ArduPilot/PX4)
- [ ] `CrsfSource` — CRSF/ELRS telemetry frames
- [ ] `ReplaySource` — playback from recorded flights (Blackbox, OTX/ETX logs)
- [ ] Multi-aircraft support (multiple TelemetrySource instances, per-UAV stores)

### Additional Transports
- [ ] Bluetooth (BLE) transport
- [ ] TCP/UDP transport
- [ ] Wi-Fi Direct transport

### Advanced UI & Tools
- [ ] OSD font/element preview
- [ ] Safehome editor
- [ ] HID controller input (gamepad/joystick)
- [ ] Audio status alerts (TTS)
- [ ] Survey/area planner
- [ ] Terrain analysis
- [ ] Embedded video stream
- [ ] FW approach / autoland planner
- [ ] Geozone editor
- [ ] CesiumJS 3D map view (terrain, 3D flight paths, log replay in 3D)

---

*Last updated: 2026-07-06*
