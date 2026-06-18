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
- [x] Session persistence (port, baud rate, map position/zoom, panel state, window size/position via `tauri-plugin-window-state`)
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
- [x] **Over-zoom placeholder detection** — ESRI returns a fixed blank tile (not a 404) above the available zoom; detected by content hash (self-calibrating) and the tile filled with the scaled real-ancestor imagery instead of a blank. 3D uses Cesium's native parent fallback; 2D uses a clipping-`<div>` ancestor tile. ESRI satellite `cesiumMaxZoom` raised 17 → 20. See ADR-028

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
- [x] Large (AHI, Compass), Small (most), and Wide (2×1, Live AGL) widget classes
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

### Mission Editor Advancements
- [x] **Mission stats — extend** (INAV editor footer): total leg distance, climb/descent totals and an estimated flight time (`computeMissionStats()` in `missionLibrary.ts` — carry-forward per-WP cruise speed + hold times, counts only the active part up to the first Land/RTH; `~` when an assumed cruise speed is used, `≥` when a PosHold-∞ makes it unbounded). _ArduPilot panel (different WP struct) still pending._
- [x] **Custom context menus** — reusable in-app right-click / long-press menus (list + map markers; native WebView menu suppressed except text fields); waypoint menu offers *Move to mission* (multi-mission) + *Batch Edit*
- [x] **Multi-select waypoints + batch editing** — list (Ctrl / Shift / number-circle tap) + map (tap-toggle), edit-mode only; batch **delete** (✕) and **Batch Edit popup**: altitude (absolute + relative-change), speed, hold time, user-action bits across the selection, one APPLY (undo/redo-friendly), unit-aware, `---` for differing values, alt-mode toggle + auto-convert when modes differ. _Pending: set-parameter beyond these fields if needed_
- [x] **Undo/redo** for mission edits — snapshot-based history covering **all** missions (so cross-mission *Move to mission* is undoable); one snapshot = one user action, with multi-step actions (batch edit/delete, move, pattern append, terrain correction, WP+modifiers delete) grouped into a single step. Toolbar buttons (edit-mode only) + Ctrl+Z / Ctrl+Y / Ctrl+Shift+Z; history cleared on load/download/import. Launch point excluded (not part of the FC upload). See ADR-027
- [ ] **Waypoint label editing** — rename/label in edit mode, shown on mouseover
- [ ] **Waypoint parameter view** — refreshed per-WP parameter panel in the editor
- [x] **Active-waypoint marker on the map** — `MSP_NAV_STATUS` (live) + blackbox/ArduPilot `active_wp_number` (replay) highlight the current target WP with a pulsing green glow on the icon (0.5 Hz, FBH included), gated on NAV_WP mode **and** mission trust (see provenance below). INAV done; **Flight-Mode-widget readout (`WP N/X`, `WP-RTH`) done**; ArduPilot layer pending
- [~] **Mission provenance + flown-vs-loaded validation** _(flag model + gating in `docs/active/MISSION_TRACKING_AND_PROVENANCE.md`)_:
  - [x] **3-flag model (FC/FILE/DB) + trust gating** — per-slot content-snapshot flags (auto edit/undo), highlight trust gates, one-time "track?" popups (replay/flight), connect prompt (Download/Upload/Nothing), flag labels in the panel. INAV done.
  - [x] **Mission library + record the active mission with the flight** — _done (see `docs/archive/MISSION_LIBRARY_AND_DB.md` + `MISSION_LIBRARY_UI.md`); awaiting simulator testing_: first-class **mission library** (`missions` table, content-hash dedup, geometry + reverse-geocoded-location metadata, self-healing schema); recorded flights link the flown mission (**arm-save / disarm-link**, FC-synced only, in-flight-upload update prompt); Blackbox `H waypoints:N` → `flights.logged_wp_count` replay `WP N/X` fallback. **UI:** **Mission Manager** (logbook-style grouped browser — metadata/notes, preview mini-map, linked flights → jump to flight, Load-to-Map / Export / Import + drag&drop / Delete), editor **Save-to-library** (NEW/OVERWRITE), logbook **Link/Unlink**. _Deferred:_ list search.
  - [x] **ArduPilot/PX4 mission library — parity with INAV** (ADR-050, `docs/active/ARDUPILOT_MISSION_LIBRARY.md`): ArduPilot editor Save-to-library + Mission Manager; format-aware Manager (per-mission format tag, present-formats filter next to Import, AP preview, `.waypoints` import); cross-format load auto-switches the editor (keep/discard, FC-locked block); system-aware flight↔mission link (arm/disarm capture, manual link, replay-load). Built on the format-agnostic mission DB (no backend change); **PX4 rides the same path** (appears automatically when a PX4 mission is saved). _Phase 2 deferred:_ fc/file/db provenance flags for the AP store (sync indicators).
  - [ ] **Validate flown vs. loaded** — compare the loaded mission against the flown track / the FC's active-WP sequence and flag divergence.
  - [ ] **Verify mission-in-log support** — ArduPilot `.bin` embeds the mission (CMD messages); INAV blackbox only in later FW versions — confirm exact version + parse it on import.
- [ ] **Disable/enable waypoint** — deactivate a WP in a loaded mission without deleting it (frozen in place, excluded from path + FC upload, kept in the file's meta area, greyed on the map). Design captured in `docs/active/WaypointDisable.md`
- [x] **Fly-by-Home waypoint handling** — FBH is INAV's `NAV_WP_FLAG_HOME` (0x48) flag on a real numbered WAYPOINT/POSHOLD_TIME/LAND (executes at the arming home), not a separate type. Added as a **modifier** in the WP editor (creates the FBH WP at the home/launch point) with a nested sub-type + altitude/params section; map shows an orange house on the inbound leg with dashed inbound/outbound legs through a protective home ring; orange numbered row in the WP list. `renumber()` now preserves 0x48 on a last WP. 2D done; 3D overlay pending
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
- [x] Core: date/time, duration, max altitude, max speed, max distance from home, total distance, battery usage
- [x] Location: start GPS coordinates + reverse-geocoded place name via OSM Nominatim API
- [x] Aircraft: craft name (from FC, queried during handshake), craft type (platform type)
- [x] Source: telemetry protocol (MSP/MAVLink/CRSF/LTM), firmware variant + version
- [x] Weather: temperature, wind speed/direction, conditions — user-editable via weather editor + auto-fetch from Open-Meteo
- [x] Weather + geocode fetched at ARM time (async spawn, non-blocking), with lazy fallback on logbook view

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
- [x] Collapsible group headers with flight count (two-level tree, ▾/▸ toggle)
- [x] Aggregate stats per group (Σ flight time + distance shown in both tree-header levels, unit-aware) — computed in `buildFlightTree()`
- [x] Flight detail view with metadata summary (location, weather, aircraft, source)
- [x] Weather editor: compact read-only display + pencil icon → editor form with stepper buttons
- [x] `flightlog_update_weather` command + `updateFlightWeather()` store function
- [x] Logbook minimize/expand: click map → minimize (280px metadata), click panel → expand
- [x] Notes auto-resize textarea (up to 140px, read-only in minimized mode)
- [x] Batch import: multi-file selection in file picker
- [x] Drag & drop import of Blackbox files into logbook tab
- [x] Duplicate flight detection dialog on import
- [x] Delete flight button styled as danger (red)
- [x] Flight path replay on map (animated marker playback)
- [x] Playback controls (play, pause, reset, scrub, speed 1×/2×/4×/10×)
- [x] Type-specific UAV symbols on map during replay (per platform type) — per-platform silhouettes (`uavShapeForPlatform()`: multirotor/airplane/helicopter) on the **2D** map (live + playback); replay uses the flight's `platform_type`. 3D now uses per-platform glTF models with full attitude (see Milestone 7). Platform type is **editable in the flight detail** (dropdown under Craft Name, persisted via `flightlog_update_platform_type`) — fixes existing entries in place. Import sets a best-effort default: ArduPilot from the MSG vehicle banner; INAV Blackbox heuristically from the logged motor/servo field set (no explicit header).
- [x] Flight path replay through HUD widgets (all widgets receive telemetry during playback)
- [x] Delete flight records
- [x] Search/filter by aircraft name, location, date, notes (frontend-only text filter)
- [x] Ctrl+click multi-select for bulk operations

### Blackbox Integration
- [x] External `blackbox_decode` binary discovery (app folder → PATH fallback)
- [x] Blackbox decode invocation: `blackbox_decode --merge-gps --datetime --unit-height m --stdout <file>`
- [x] CSV parsing in Rust → raw CSV storage per row (dynamic fields, INAV-version-independent)
- [x] Original .TXT file archived as BLOB in `blackbox_files` table (re-downloadable)
- [x] Standalone Blackbox import: creates new flight with `source: "blackbox"`, metadata from header
- [x] Attach Blackbox to existing live flight: `source: "both"`, playback toggle MSP vs Blackbox (see Milestone 5b)
- [x] `flights.source` field: `live` | `blackbox` | `both` — Blackbox-only flights marked with icon
- [x] Multi-log support: single .TXT may contain multiple ARM/DISARM sessions (`--index N`)
- [x] Blackbox-imported flights use header metadata (FW version, date, GPS start, duration)
- [x] NOT a full Blackbox analyzer — no PID/gyro/motor visualization (use dedicated tools)

### Export & Data Exchange
- [x] `.kflight` export: self-contained SQLite file with flights + telemetry + blackbox data
- [x] `.kflight` import: drag & drop or file picker, duplicate detection, bulk copy
- [x] Multi-select (Ctrl+click) for multi-flight `.kflight` export
- [x] Export raw Blackbox binary from `blackbox_files` BLOB (original .TXT / .bbl / .bfl)
- [x] `_kflight_meta` table in export files: schema version, app ID, export timestamp, flight count
- [x] Flight source indicators in logbook list: ◈ blackbox, ◉ both, no prefix = live
- [x] Export flight path as KMZ/KML (Google Earth) — see ADR-020, `track_export.rs`
- [x] Export flight path as GPX (universal GPS format)
- [x] Export telemetry as CSV

## Milestone 5b: Blackbox Import (v0.5.x — COMPLETE)

### Blackbox Decode Pipeline
- [x] Settings: auto-detect `blackbox_decode` in app folder, fallback to PATH
- [x] Invoke `blackbox_decode --merge-gps --datetime --unit-height m --unit-gps-speed mps --stdout <file>` as child process
- [x] Parse CSV stdout in Rust — O(1) field access via pre-built `HashMap<String, usize>` index + `ColumnIndices` struct
- [x] Downsample to ≤ 10 Hz using time-based sampling (100ms interval)
- [x] Store parsed rows as raw comma-joined CSV in `blackbox_records` table (no JSON overhead)
- [x] Archive original .TXT as BLOB in `blackbox_files` table
- [x] FC heading (`yaw` ← `heading`/`attitude[2]`) always ÷10 (decidegrees → degrees); COG (`heading` ← `gps_ground_course`) kept in degrees; both stored as f64 (0.1°)
- [x] Link Quality from `lq` / `link_quality` / `rxlq` column → `link_quality` in `TelemetryRecord`
- [x] Roll/pitch column resolution: `roll`/`attitude[0]`/`attitude_roll` — unconditional ÷10 (INAV decidegrees)
- [x] Vario from `gps_velned[2]`: NED down cm/s → negated ÷100 → climb m/s

### Standalone Import
- [x] "Import Blackbox" button in Logbook tab
- [x] File picker for .TXT / .bbl / .bfl files
- [x] Extract metadata from Blackbox header: FW type/version, date/time, duration, GPS start
- [x] Create flight entry with `source: "blackbox"` marker
- [x] Logbook shows Blackbox-only flights with distinct icon (wider panel, new source column)
- [x] Multi-log .TXT: probe all indices (0–31), show picker for which log to import
- [x] Real-time progress events during import (`flightlog_import_progress` Tauri event)

### Telemetry Replay Pipeline
- [x] `telemetryAdapter.ts`: `toTelemetryData(TelemetryRecord → TelemetryData)` — DB rows → widget format
- [x] Automatic live/replay switch via `$derived(telem)` in +page.svelte
- [x] Home position set from `flight.start_lat/lon` during replay, cleared on close
- [x] All widgets confirmed working: AHI, Compass, Vario, Speed, Battery, GPS, Home Distance
- [x] Compass uses GPS COG (heading) for replay, fallback to attitude yaw

### Frontend Modularization
- [x] Thin orchestrator pattern implemented (`+page.svelte` as wiring layer only)
- [x] 4 controllers: connection, logbook, playback, widget
- [x] 1 adapter: telemetryAdapter (DB → widget data mapping)
- [x] 1 helper: telemetry (isArmed, hasKnownLocation, isValidGpsCoordinate)
- [x] 7 extracted components: LogPlayer, LogbookPanel, SettingsPanel, Toolbar, UavInfoPanel, StatusBar, NavRail

### Attach to Existing Flight
- [x] Link Blackbox import to matching live-recorded flight (link dialog)
- [x] Flight marked as `source: "both"`
- [x] Playback UI source toggle: REC telemetry vs linked BBX track
- [x] `.kflight` export auto-includes linked partner flights
- [x] `.kflight` import restores linked relationships (including mixed import/duplicate-skip cases)

### Colored Flight Tracks & Mode Visualization
- [x] `trackColors.ts` helper: `TrackColorMode`, `FlightModeInfo`, `classifyFlightMode()`, gradient functions
- [x] Track color modes: Flight Mode, Altitude, Speed, Signal, None
- [x] Multi-segment rendering: `L.layerGroup()` with merged polylines per color
- [x] Flight mode classification: 11 priority levels (Failsafe RTH → Acro)
- [x] Altitude gradient: blue→red, `warnAltitude` reference from alerts settings
- [x] Speed gradient: blue→red, scaled to max ground speed
- [x] Signal gradient: green→red inverted, prefers LQ over RSSI
- [x] LogPlayer dropdown with 5 color modes + dynamic legend (badges or gradient bar)
- [x] Legend shows only modes actually used in current flight
- [x] UAV icon coloring by `nav_state` (INAV `MW_NAV_STATE_*` → color)
- [x] Live trail colored by flight mode (multi-segment polylines)
- [x] Alerts settings group with `warnAltitude` (default 120 m)
- [x] Protocol reference doc: `PROTOCOL_FLIGHT_MODES.md` (INAV vs ArduPilot)
- [x] Live MSP: parse `flight_mode_flags` from `MSPV2_INAV_STATUS` payload
- [x] Live MSP mode decode uses `MSP_BOXIDS` index→box-id mapping
- [x] Live MSP mode decode mirrors INAV implicit ANGLE behavior for nav modes

### DB Schema (current: v10)
- [x] `blackbox_records` table: `flight_id`, `timestamp_us`, `csv_data` (raw CSV TEXT) — schema v2
- [x] `blackbox_files` table: `flight_id`, `original_filename`, `log_index`, `file_data` (BLOB), `file_size`, `imported_at` — schema v2
- [x] `flights.source` column: `live` | `blackbox` | `both` — schema v2
- [x] `telemetry_records.link_quality` column: INTEGER (0–100%) — schema v3
- [x] Migration v1 → v2, v2 → v3 (incremental, backward compatible)
- [x] Schema v4: replay-focused telemetry fields (`baro_alt_m`, GPS quality, active flight modes, state flags, nav state, wind, RC arrays, sensor health)
- [x] Schema v5: `flights.craft_name` column (user-editable, separate from FC-reported name)
- [x] Schema v6: `flights.linked_flight_id` for live↔blackbox pairing
- [x] Schema v7/v8: mission library (`missions` table + `flights.mission_id` + `flights.logged_wp_count`)
- [x] Schema v9: `flights.pilot_name` + `flights.pilot_id` (manually editable)
- [x] Schema v10: `battery_packs` table + soft `flights.battery_serial` link (serial-resolved)
- [ ] Milestone 4: decode Blackbox header `features` into a human-readable feature decode

### Settings & UI Enhancements
- [x] Separate Flight Recording toggle (`flightRecordingEnabled`) from Flight Logbook toggle (`flightLoggingEnabled`) — see ADR-022
- [x] Logbook tab hidden in NavRail when `flightLoggingEnabled` is false
- [x] Craft name inline editing in LogbookPanel (✎ button, Enter/Escape/blur to confirm)
- [x] `flightlog_update_craft_name` Tauri command for craft name persistence
- [x] Blackbox import filter memory (last-used INAV/ArduPilot order persisted in localStorage)
- [x] i18n: "Flight Logging" split into separate "Flight Logbook" + "Flight Recording" labels (de + en)

---

## Milestone 6: Advanced Features (v0.6.x+)

### MSP Link Statistics (INAV 9.x+)
- [x] Feature gate: `Feature::LinkStats` (`InavVersion >= 9.1`)
- [x] `MSP2_INAV_GET_LINK_STATS` (`0x2103`): `uplinkRSSI_dBm` (u8, negated), `uplinkLQ` (u8, %), `uplinkSNR` (i8, dB) — own 1 Hz poll slot, feeds the RC Link widget (live)
- [x] Fall back to `MSPV2_INAV_ANALOG` RSSI for firmware < 9.1 (the ANALOG-derived RSSI-only link is suppressed once `0x2103` is polled so the two don't clobber each other)
- [x] Record the live link to the DB (schema **v14**): `link_quality` (LQ) now populated live (was Blackbox-only) + new `link_snr` / `link_rssi_dbm` columns, fed from the unified link-stats pipeline (CRSF / INAV 9.1) so the RC Link widget replays the real values.

### Multi-Protocol Architecture (see `docs/archive/PROTOCOL_REFACTORING.md`)

**Phase 1 — ByteTransport trait extraction**:
- [x] `ByteTransport` trait (protocol-agnostic byte-level I/O: read/write/close)
- [x] `SerialByteTransport` — existing serial refactored to ByteTransport
- [x] `TcpByteTransport` — TCP client/server transport
- [x] `UdpByteTransport` — UDP transport (connectionless)
- [x] `MspTransport` — MSP framing layer over ByteTransport (replaces current `Transport` trait)
- [x] Refactor existing `MspScheduler` to use `MspTransport<ByteTransport>`

**Phase 2 — MAVLink integration**:
- [x] `mavlink` Rust crate with `common` + `ardupilotmega` dialects
- [x] `MavlinkHandler` — reader thread (continuous parse + dispatch) + heartbeat writer (1 Hz)
- [x] MAVLink → normalized payloads mapping (10 receive messages → same Tauri events)
- [x] GCS IDs: sysid=255, compid=190
- [x] ArduPilot + PX4 + INAV MAVLink firmware support
- [x] **MAVLink telemetry stream rates — GCS-requested via `SET_MESSAGE_INTERVAL`, MSP-parity** (ADR-043): same two knobs (Attitude/GPS), ballast disabled, fits real RC links (~500–650 B/s); **Full MAVLink Telemetry** toggle leaves it to the FC's `SRn_*` params (reset-to-default)
- [x] **MAVLink Debug Monitor tab** — per-message ID counts/rate/last-seen + RX/TX direction (push-side counterpart to the MSP poll view)
- [x] **ArduPilot per-vehicle flight-mode mapping** — vehicle-specific `fc_variant` (ArduPlane/Copter/Rover/Sub) + raw `custom_mode` forwarded to the per-vehicle mode table (live + replay)
- [x] **PX4 flight-mode classification** — PX4 packs `main_mode`/`sub_mode` into `custom_mode` differently from ArduPilot's flat table; `classify_px4()` + a `classify_mavlink()` dispatcher route PX4 vs ArduPilot by `fc_variant` (live + tlog import), QGC mode names
- [x] **MAVLink home from `HOME_POSITION`** — authoritative FC home on the protocol-agnostic `home-position` event (consistent across reconnects)
- [ ] **Unify home-on-arm into the adapter layer** — for telemetry-only inputs (CRSF/Smartport passthrough) home derives from the GPS fix at the arm edge; move today's frontend fallback into the backend adapter so all home sourcing flows through `home-position` (see ADR-039)

**Phase 3 — Protocol Selection & Connection Handling**:
- [x] Protocol selection: explicit UI dropdown (MSP / MAVLink), no auto-detect
- [x] Transport selection: Serial / TCP / UDP per protocol
- [x] Dual-protocol connection lifecycle in `commands/connection.rs` + `ActiveProtocol` state (no separate ConnectionManager module)

**Phase 4 — Raw Recording**:
- [x] MAVLink raw log: standard tlog format (`.tlog`) — full recording during flight + continuous mode
- [x] MSP: Pre-parsed telemetry text log (current `RawLogger`) written in parallel to DB as backup. Not the originally planned MWP v2 binary format.
- [x] Raw recording per protocol (different formats for MSP vs. MAVLink)
- [x] Crash-safe: raw files survive app crash during flight
- [x] Continuous raw logging: optional always-on recording from connect (pre-arm data included)
- [ ] True raw byte-level serial logger (`SerialLogger` / `RAWLogger`) for link debugging and secondary devices (ADSB, radar, etc.) — planned for later

**Phase 5 — ArduPilot Log Import**:
- [x] Native ArduPilot DataFlash `.bin` parser (self-describing FMT records)
- [x] Functional import of `.bin` files into the logbook (progress events, duplicate detection, armed segment logic, writes to `telemetry_records`)
- [x] Imported ArduPilot flights can be replayed normally through widgets + map
- [ ] ArduPilot `.tlog` MAVLink log import (recording works, import missing)
- [x] ArduPilot mission system (MAVLink WP protocol) — complete: `mavlink_proto/mission.rs` upload/download/clear microprotocol + `ardu_mission_download`/`ardu_mission_upload`, wired to the ArduPilot mission panel/layer; `.waypoints` (QGC WPL) file save/load/drop + INAV↔ArduPilot WP conversion. See `docs/active/MISSION_MULTIAUTOPILOT_PLAN.md`
- [x] **PX4 mission planning** — rides the ArduPilot MAVLink mission pipeline; firmware-aware command catalog (verified PX4 subset, no `JUMP_TAG`/extra-loiter/relay/condition cmds), **no vehicle-type selector** (one PX4 interpreter for all airframes; class read from `MAV_TYPE`; connect-time soft-warning for VTOL-on-non-VTOL / unsupported cmds), and **firmware-aware home-slot** (PX4 has no home item 0, unlike ArduPilot — upload/download no longer inject/drop a placeholder). Untested on real PX4 hardware. See `docs/active/MISSION_MULTIAUTOPILOT_PLAN.md`
- [~] **ArduPilot WP types per vehicle class** — the WP-type palette + validation now adapt to the active vehicle (Copter/Plane/QuadPlane first-class; Rover/Boat/Sub get the base set): **vehicle-type selector** (3-way firmware toggle + dropdown, persisted, locked when connected), **QuadPlane auto-detection** via `Q_ENABLE` (a QuadPlane reports FIXED_WING — issue #7137), **catalog-driven icons** (VTOL/spline/payload badges, ROI/Home), and **soft-warnings** for commands invalid for the class. Jump/Jump-Tag map representation (line + ↺N badge) done. _Open_: the **VTOL-phase model** (transition badges / VTOL-land approach cue) and filling Rover/Boat/Sub-specific command data. Full plan in `docs/active/ARDUPILOT_WAYPOINT_ARCHITECTURE.md`.

### Passive Radio Telemetry (listen-only ground-side, ADR/plan: `docs/archive/RADIO_TELEMETRY.md`)
A third connection mode ("Telemetry") that listens to telemetry forwarded by the transmitter
(EdgeTX/ETHOS) / ELRS backpack / DIY bridge, auto-detecting the protocol. Dev-only for now.
- [x] Phase A/B: listen-only handler, protocol detector, raw capture-to-file, Debug Monitor tab, BLE
  GATT-explorer auto-discovery (validated on ETHOS X20RS, service `0xFFF0` / char `0xFFF6`)
- [x] Phase C: **FrSkyX / S.Port decoder → unified telemetry events** (INAV 7/8/9, dispatch by appID);
  AHI/compass/GPS/speed/altitude/vario/battery/RSSI/airspeed live. MODES (flight mode + armed) + GNSS
  (sats + fix) decoded → flight-mode widget + ARMED + GPS fix. Link quality (`0xF010`/"VFR" = `100 − loss`)
  + RSSI (`0xF101`) now feed the **RC Link widget** (see Advanced UI). _Remaining: ArduPilot
  FrSky-passthrough (0x5000) decoder_
- [~] Phase D: DB recording for telemetry-mode flights — recorder wired (arm/disarm from FrSky MODES,
  fed the decoded telemetry). _Built; pending verification on a real armed flight._
- [ ] `CrsfSource` — CRSF/ELRS telemetry frames (decode by frame/sub-type; INAV reworked these across versions)
- [ ] `LtmSource` — LTM (Lightweight Telemetry) passive frame parser
- [ ] MAVLink-passive decoder (reuse the MAVLink parser, TX disabled)

### Telemetry Relay — forwarding & conversion (ADR-051, plan: `docs/archive/TELEMETRY_FORWARDING.md`)
Re-encode the live inbound telemetry into a chosen wire protocol and send it out a second link (antenna
trackers / monitoring apps / other GCS). Backend transcoder fed by a self-event tap; the inverse of the
passive decoders. Dropdown panel under the connection bar; persisted, auto-connecting relays.
- [x] LTM / MAVLink / CRSF / SmartPort encoders (each the inverse of the matching passive decoder)
- [x] Serial / BLE / TCP-server / UDP outputs; combined Serial+BLE device picker; multiple relays with
  unique-port guard; basic LINK diagnostic (RX/TX B/s, msgs/s, protocol)
- [x] TCP + LTM verified live against mwptools from another PC
- [ ] Validate MAVLink vs Mission Planner / QGC, CRSF vs handset, SmartPort vs OpenTX/Ethos sensor screen
- [ ] MAVLink HEARTBEAT real vehicle type + named flight mode (currently generic + armed flag)
- [ ] Root-cause: passive decoders re-emit only on a fresh frame (not the fixed 10 Hz republish) so
  passive-sourced relays emit at the true data rate; also de-bloats widgets/recorder/logs

### Future Protocols
- [ ] Multi-aircraft support (multiple protocol handler instances, per-UAV stores)

### Additional Transports
- [x] Bluetooth (BLE) transport via ByteTransport (CC2541, Nordic NRF/NUS, SpeedyBee Type 1/2)
- [ ] Wi-Fi Direct transport

### Map Overlays
- [~] **Airspace Manager** — a dedicated nav-rail panel (under Radar) + aeronautical-data subsystem over
  **OpenAIP**: **obstacles** (wind turbines/masts), **airspaces**, **RC/model airfields**, **airports**.
  Single **pluggable provider** (OpenAIP first via a user-supplied key; FAA / open-data future) chosen in
  Data settings; per-layer 2D/3D visibility + cache + grouped nearby list in the panel; region cached in RAM.
  Zoom-density management. **P1 shipped (2D + panel + density)**; **P2 open:** 3D rendering (volumes/columns),
  centre fallback (GCS/map without UAV), density tuning, legend, alerts. The static counterpart to the Radar
  subsystem. See ADR-038 + `docs/active/AIRSPACE_MANAGER.md`
- [ ] Aviation charts (OpenAIP tile layer — airports, navaids, airspace symbology)

### Battery Management
- [x] **Battery library + manager** (Phase A + B) — **complete**; see `docs/archive/BATTERY_MANAGEMENT.md`. `battery_packs`
  DB (identity = serial), soft `flights.battery_serial` link, a **Battery Manager** view-toggle in the
  Flight Logbook (grouped/flat list, status special groups), editable pack identity with computed
  voltage/energy, **lifetime = persistent baseline + Σ(linked flights)**, additive manual usage editor,
  serial link/unlink in the flight detail. Pilot name/ID fields shipped alongside (schema v9).
- [x] **Battery export/import** (`.kbatt`, consolidate/base + import preview with serial-conflict
  resolution) and **cross-jump navigation** (flight ↔ linked mission/battery chips).
- [x] **End-Flight dialog** on disarm — read-only summary always; when DB-recorded also captures the
  battery serial (no autofill), a note, and a mission-link confirmation (FC-synced auto-links; non-FC is
  opt-in). Log-import linking stays manual via the flight detail.
- [x] **Flight-deletion consolidation** — opt-in checkbox in the delete dialog to fold a linked flight's
  battery usage into the pack's lifetime totals before deletion.
- ~~Battery management — Phase C: per-flight telemetry wear metrics (Wh, sag, internal resistance, SoH)~~
  — **cut from scope:** FC telemetry too slow/laggy, Blackbox voltages too hardware-dependent (cabling,
  connectors, wire gauge) for reliable health figures. Possible future if precise log analysis is tackled.
- [ ] **Multi-battery per flight** (future) — ArduPilot-only; needs telemetry / DataFlash research into
  how multiple packs are reported. A `flight_batteries` join table when tackled.
- [ ] **Battery estimation (remaining capacity / time)** — protocol-specific source:
  - **INAV**: derive from the FC's battery configuration (capacity, cell count, voltage thresholds — the battery-estimation-relevant settings) read at connect, combined with live consumed-mAh / voltage telemetry.
  - **ArduPilot**: the firmware lacks INAV's built-in estimation, so compute it **locally** in the GCS from telemetry (consumed mAh, current draw, voltage sag) against the pack's rated capacity (ties into the battery logbook).
  - Surface as a widget: remaining %, estimated time/distance, and a return-home-feasibility hint. Single protocol-agnostic widget, two estimators behind it.

### Advanced UI & Tools
- [x] Survey / area planner — all six shapes complete:
  - [x] Rectangle shape definition (center, length, width, orientation) via UI + draggable map markers
  - [x] Map visualization of shape area (gray semi-transparent polygon)
  - [x] Live path preview (survey legs + turn connections)
  - [x] Interactive corner + center dragging on map (two-way parameter binding)
  - [x] Parameter panel with NumberStepper controls, hidden when edit mode disabled
  - [x] Turn Distance extension for fixed-wing turn zone
  - [x] Reverse direction toggle
  - [x] Track Orientation — independent track angle within shape (clipped to shape boundary)
  - [x] Altitude Type selector (Relative / AMSL / Ground)
  - [x] User Action Trigger flags per line (start + end, bits 1–4 in p3)
  - [x] 120 WP limit check with truncation dialog
  - [x] Deduplication of survey/turn boundary points
  - [x] Altitude, speed, userActionFlags encoded in p3 bitfield per INAV spec
  - [x] Pattern params persist between mode toggles (cleared on app close)
  - [x] Rectangle Lawnmower pattern generation (concentric contour-offset)
  - [x] Circle (Stepped) + Spiral (Archimedean) patterns
  - [x] Polygon (ZigZag) — concave-capable scanline, cross-gap / connected-fill modes, interactive vertex editing
  - [x] Polygon Lawnmower — convex decomposition + contour-offset, diagonal ring transitions
  - [ ] **Keep legs inside concave boundaries (don't cross gaps)** — on concave polygons the scan/connector legs can currently bridge straight across a concavity, sending the aircraft outside the intended area. Route connectors along the boundary or split the area into separate scan regions so no leg leaves the polygon.
  - [ ] Load/save pattern templates
- [ ] OSD font/element preview
- [ ] Safehome editor
- [ ] HID controller input (gamepad/joystick)
- [ ] **Stick / gimbal overlay** — animated RC transmitter sticks (two gimbals) driven by recorded RC-channel data, à la Blackbox Explorer. **Replay only for now** (from `RC_CHANNELS` / blackbox `rcCommand`); live later. Configurable channel map (AETR/TAER) + stick mode 1–4. As a widget or a corner overlay on the map/replay.
- [x] **RC Link widget** — protocol-agnostic, adaptive HUD widget that shows whatever the active link reports and hides the rest (LQ big when present, else RSSI %; RSSI dBm + SNR as meta). Backed by a unified `LinkStatsData` (`rssi_percent` normalized at the source + optional `rssi_dbm` / `lq` / `snr_db`) on a new `telemetry-linkstats` event. Sources: **CRSF** LinkStatistics `0x14` (uplink RSSI dBm + LQ + SNR), **SmartPort** (RSSI `0xF101` + LQ from `0xF010`/"VFR" = `100 − loss`; RSSI-0 from the FC's unconfigured channel ignored), **MAVLink** `RC_CHANNELS` RSSI, **LTM** S-frame RSSI, **INAV** `MSPV2_INAV_ANALOG` RSSI and (9.1+) `MSP2_INAV_GET_LINK_STATS` for real dBm/LQ/SNR. Replay maps the DB `link_quality`/`rssi`. _Open:_ record live LQ/SNR to the DB (see Link Statistics above).
- [ ] Audio status alerts (TTS)
- [x] Terrain analysis — _elevation profile + clearance + correction (Terrain Follow / Clearance Check) + jump simulation done; see Terrain Elevation section_
- [x] **Heading / course / crab cues** — compass COG track-bug + amber readout next to heading; 2D map **HDG / COG nose lines** + **predicted turn-radius arc** at the aircraft (velocity-vector length, arc capped at 180°); unified FC-heading-vs-COG pipeline across MSP / MAVLink / Blackbox and 2D+3D, live+replay; **Direction indicators** settings toggle. Wind-arrow / flight-path-marker **parked** on INAV `MSP2_INAV_WIND` (unmerged, likely v10); 3D map markers **not planned** (revisit only on request). See `docs/archive/WIND_CRAB_INDICATOR.md`
- [x] **Flight-path marker (velocity vector) on the AHI + 3D FPV HUD** — shows where the aircraft is actually *going* vs where the nose points. **No wind estimate** (we have the track directly): horizontal = **COG − heading** (crab), vertical = **flight-path angle** `atan2(vertical speed, ground speed)` vs pitch. Body-frame (pilot's view): AHI marker in yellow, FPV conformal in green (FOV-scaled), both gated ≥ 1.5 m/s and smoothed with the shared exponential ease. Live + replay. Shared geometry in `utils/flightPath.ts`. Superseded the wind-gated FPM from the Wind/Crab feature.
- [~] Embedded video — _built: source router + webcam/USB-capture (`getUserMedia`, cross-platform), NavRail panel (live preview, 60 fps MJPEG fix), 2×1 dock widget, snap/drag floating window, double-click map⇄video swap, native Picture-in-Picture, persistence + auto-start. **Pending (v2):** network streams (RTSP/UDP), native `nokhwa` capture, snapshot/record; see `docs/active/VideoFeature.md`_
- [ ] FW approach / autoland planner
- [ ] Geozone editor
- [ ] MAVLink signing (passphrase-based packet authentication)
- [ ] AI-assisted flight log analysis (potential third-party collaboration)
- [ ] _(deferred)_ Widget layout profiles (save/load named panel arrangements) — not enough widgets to justify yet; revisit when the widget set grows

### Platform & UX Foundations

Cross-cutting groundwork items. Each carries non-trivial architectural or design weight — major considerations noted inline.

- [x] **Local terrain elevation provider (2D map & planning)** — implemented; see dedicated subsection below
  - _Decision_: **Copernicus DEM GLO-30** (free, no API key, AWS Open Data GeoTIFF). Chosen over SRTM/Terrarium: ~±4 m RMSE vs ~±9 m, global coverage incl. > 60°N (SRTM/Terrarium have none), and geoid-referenced (EGM2008 ≈ MSL) — matches our MSL waypoint/altitude pipeline with no geoid model needed. Terrarium PNG tiles were the elegant runner-up but are SRTM-grade globally.
  - _Major considerations_: point-sampling pipeline (fetch tile → decode → bilinear interpolate), region download for offline use, shared elevation abstraction for all four use cases below. Used **locally only** — for visualization/validation, independent of any onboard FC terrain (INAV 10 will add onboard SRTM1, irrelevant to us).
- [ ] **3D map (Cesium) stays as-is for now** — keep Cesium World Terrain; tile-resolution quirks and other 3D refinements tracked separately under Milestone 7.
- [ ] **Better map tile handling**
  - _Major considerations_: current IndexedDB LRU cache works but is reactive only. Consider region prefetch / offline area download, smarter eviction, retry/backoff on tile errors, optional vector tiles, and unified handling shared between Leaflet (2D) and Cesium (3D). Attribution + provider rate-limit compliance.
- [x] **Global UI scale setting** — shipped; see `docs/archive/UI_SCALING.md`. CSS `zoom` on the chrome
  layer (toolbar / panels / docks / widgets / dialogs) at **100 / 125 / 150 %** (Settings → Language),
  persisted as `uiScale`. The **map stays at native resolution** (hoisted into an unzoomed `.layer-map`);
  map overlays are scaled individually — WP markers, param labels, the WP editor popup, Leaflet tooltips,
  and the right-click context menu. Chosen over a `rem` refactor (258 px font-sizes, 0 rem → too invasive).
- [~] **Reusable panel framework + control library** — `PanelShell` (5 variants: info / compact
  / advanced / wide-compact / fullscreen) + shared controls (`Button` 6 types with a flat-SVG
  icon registry, `SegmentedToggle`, `Toggle`) so panels are consistent by construction instead of
  each rolling its own markup/sizing/buttons. Phase 0 (shell + controls, empty-shell review) done;
  panels migrate one at a time via a parallel duplicate rail group (strangler). See ADR-029 +
  `docs/active/PANEL_FRAMEWORK.md`.
- [ ] **Custom tooltip / in-app assistance system** — native `title=` tooltips are rendered by
  WebView2 *outside the DOM*, so they can't be themed or UI-scaled. Replace them with a `use:tooltip`
  action + a themed singleton overlay that scales with the global UI scale (same pattern as the context
  menu). Doubles as an **assistance layer**: short descriptions explaining controls/features for easier
  use, **toggleable in Settings**. ~58 `title=` sites across 17 files to convert.
- [ ] **Improved in-app icons, graphics & app logo**
  - _Major considerations_: replace ad-hoc emoji glyphs with a consistent SVG icon set; proper app/installer/taskbar icons; light/dark variants; coherent branding. Bundle-size and licensing of any icon set.
- [ ] **Theming by connected UAV control system**
  - _Major considerations_: drive the accent color (currently fixed `#37a8db`) as a variable parameter from the detected FC/protocol (INAV / ArduPilot / PX4 / …). Needs the accent fully tokenized as a CSS custom property across all components, a mapping table per firmware, and persistence. Ties into the existing protocol-detection on connect.
- [ ] **Multi serial connections in background (aux devices)**
  - _Major considerations_: the MSP scheduler currently owns a single serial connection exclusively (ADR-007). An aux-device manager would run additional independent background connections (ADSB receiver, ESP-Radar, telemetry monitor, …) without disturbing the primary FC link — each with its own parser/handler, surfaced via an "Aux Devices" submenu. Data fusion onto the map (ADSB traffic, radar contacts). Builds on the ByteTransport abstraction (ADR-010) and the planned raw serial logger. Significant architecture item.

### Radar / foreign-vehicle tracking
Subsystem detailed in `docs/active/RADAR_TRACKING_*`, `RADAR_ALERTS.md`, `RADAR_FORMATION_FLIGHT.md` (ADR-033/035/036). Largely shipped (ADS-B online + receivers + via-MSP, conflict alerts, FormationFlight). Open items:
- [x] **GCS location marker (2D + 3D)** — a satellite-dish marker for the ground-station position (also the FormationFlight node + radar distance reference). Settings dropdown **Off / Manual / Continuous**: Manual places it once via OS geolocation and lets you drag it / right-click "Set GCS here" (Reset snaps back); Continuous follows the OS location live (>20 m anti-jitter). It's a *view* of "Your Location" (no second detector); an on-select accuracy circle. **Open:** in manual it follows the UAV's first fix on connect (snaps to launch) — decouple if it proves annoying; arming-location fallback.
- ~~**FormationFlight follow-ups (F3)**~~ — _dropped: the upstream FormationFlight project is stalled (the GCS-mode bug is unfixed with no timeline); revisit only if FF revives, possibly as its own project. Was: `SET_RADAR_ITD` status string, `"GCS"` listen-only mode, ADS-B↔FF dedup, a proper paper-plane `ff-uav.glb`._
- [ ] **Conflict-alert tuning (C3)** — expose the numeric thresholds as user settings; pre-recorded callout audio for engines without TTS (Linux WebKitGTK).

### Terrain Elevation (local DEM — 2D map & planning)

Source: **Copernicus DEM GLO-30** (geoid/EGM2008 ≈ MSL, no API key, offline-capable). Local use only — no dependency on onboard FC terrain. Shared elevation-sampling abstraction feeds all four features:

- [x] Elevation provider: Rust backend module, tile fetch → disk cache → GeoTIFF decode → bilinear sampling; `terrain_elevation` / `terrain_profile` commands (decode + I/O on `spawn_blocking`, loads coalesced)
  - [x] _follow-up cancelled:_ COG partial reads / region pre-download — not pursued; decoded tiles stay cached in RAM, so sampling latency is a non-issue in practice. Revisit only if offline area download becomes a goal.
- [x] **AGL waypoint planning** — `alt_mode` REL/AMSL/AGL on waypoints; editor toggle converts altitude via terrain + launch point; survey patterns support AGL; AGL resolved to AMSL on export (MSP/​.mission) — validated against INAV Configurator terrain analysis
- [x] **Launch/home reference** — auto-placed, draggable map marker + connector to first WP; persisted in `.mission` via mwp-compatible `<mwp home-x/home-y>` meta (round-trips, inter-app compatible)
- [x] **Terrain clearance validation (Terrain Analysis panel)** — full-width NavRail overlay, hand-rolled SVG side-view (no runtime dep); Waypoint + Track (live/blackbox) modes; terrain + flight-path with red below-clearance coloring; min-clearance (take-off/landing trimmed) + max-climb (track-jitter low-pass); zoom/pan with zoom-scaled resolution; MSL ↔ AGL view; Compact mode mirrors the chart cursor + a pinned marker onto the map
- [x] **Terrain Correction** — Terrain Follow / Clearance Check over a WP range, fixed-wing climb/descent-angle limit, manual *Add WP*, live green preview → APPLY (writes corrected WPs in AGL); pure-function engine, shared-altitude per WP index
- [x] **Jump-loop simulation** — one loop per jump: branch `→target` + cut + resume, distinct colour + target marker, no duplicate WP dots; correction stays correct across revisits
- [x] **Live Track mode** — Track mode follows the live flown track when connected: shared in-RAM `liveTrack` store (accumulates while armed), terrain pre-fetch on arm, incremental 5 s sampling, Follow toggle (pin-right zoom-only / free pan), 250 m default window. _(built; field-test pending)_
- [x] **Live AGL widget** — forward-looking terrain HUD (new 2×1 `wide` widget class): flown terrain/history left, UAV marker tracking altitude, **estimated terrain ahead along heading** right; history accumulated from the telemetry stream so it works live **and** in replay; projected flight line from (averaged) FC vario; speed-based scale (300/900/1800/3600 m) with boundary hysteresis; relative-altitude + distance axes. _(built; field-tested via blackbox replay)_
- [x] **Terrain Radar widget** — 1×1 top-down, track-up EGPWS-style 120° fan coloured by terrain clearance; new `terrain_fan` backend (server-side polar sampling); speed-driven forward distance (shared 300/900/1800/3600 m scale) + own clearance colour scale (60/120/250 m); continuous heatmap ramp with REL/PRED (sink-angle) reference; turbulence-texture filter. _(built; field-tested via blackbox replay)_
- [x] **LOS (line-of-sight) / RF link analysis** for waypoint missions + flown tracks — shipped as the **RF Link / Radio-Shadow** analysis inside the Terrain Analyzer (LOS occlusion + Fresnel/diffraction + two-ray ground reflection, rendered as a background loss field) **plus a 2D-map ray-triangle overlay** of the radio-shadow geometry. See `docs/active/RF_LINK_ANALYSIS.md`. _Phase 2 (absolute link budget / range) deferred._

### Code Health & Maintainability
- [ ] Rust module reorganization when `flightlog/` exceeds 20 files (parsers/, exporters/, models/) — _currently 12/20, not yet needed_
- [ ] `tauri-specta` for auto-generated TypeScript types from Rust structs (Rust↔TS type safety)

## [~] Milestone 7: CesiumJS 3D Map View (v0.7.x)

### Core 3D Infrastructure
- [x] CesiumJS integration (Apache 2.0) with custom Vite plugin
- [x] Custom Vite plugin (`cesiumPlugin()`): sirv middleware (dev) + fs.cpSync (build) for Cesium assets
- [x] 2D/3D view toggle button on map (persisted view mode)
- [x] Cesium Ion token setting for World Terrain data
- [x] Map provider sync: 3D view uses same tile provider as 2D (live switching)
- [x] IndexedDB tile cache integration (shared cache between 2D and 3D views)
- [x] Per-provider `cesiumMaxZoom` limits for 3D view (prevents gray placeholder tiles)
- [x] Tile error handling: failed tile requests silently handled, parent tiles remain visible
- [x] `requestRenderMode: true` for reduced GPU idle load

### Terrain & Altitude
- [x] Cesium World Terrain (requires Ion token, degrades to flat ellipsoid without)
- [x] Geoid undulation correction: GPS MSL → WGS84 ellipsoid height via `sampleTerrainMostDetailed`
- [x] Async terrain provider readiness: `waitForTerrain()` waits for World Terrain load via `terrainProviderChanged` event
- [x] Depth testing against terrain (`depthTestAgainstTerrain: true`)

### UAV Entity & Visualization
- [x] UAV entity: colored point + SVG arrow billboard + label (colored by flight mode)
- [x] Home position marker: green "H", `CLAMP_TO_GROUND` height reference
- [x] Live trail: `CallbackProperty` polyline with 1m minimum distance filter
- [x] Playback track: static polyline from `TelemetryRecord[]`
- [x] Playback marker: point + arrow billboard following scrubber position

### Chase Camera (Follow Mode)
- [x] Toggle button: "🎥 Follow" / "👁 Free" (z-index 10000, always visible)
- [x] Smooth camera interpolation via `requestAnimationFrame` lerp loop
- [x] Exponential smoothing for position (lat, lon, alt) and heading
- [x] Shortest-path angle interpolation (handles 359°→1° wrap correctly)
- [x] Configurable range slider (50–2000 m) and pitch slider (-90° to -5°)
- [x] Works with both live telemetry and playback marker
- [x] Initial snap (no lerp from origin), smooth transitions thereafter
- [x] **Heading-follow jitter fixed** — Cesium's own rotate disabled in follow (it fought the per-frame heading lock); pitch driven by a custom vertical-drag handler; start pitch lowered to 20° (view from behind, horizon visible)

### Performance
- [x] Fog enabled (`density: 2.5e-4`) to hide distant terrain
- [x] Tile cache size limit (`tileCacheSize: 100`)
- [x] `scene3DOnly: true` (no 2D/Columbus mode overhead)
- [x] MSAA 2× anti-aliasing

### Planned (3D Map)
- [x] **3D glTF UAV models with full attitude** (heading/pitch/roll) — per-platform procedural
  low-poly models (quad / tricopter Y-frame / fixed-wing / VTOL quadplane + generic arrow) replace
  the position point in live + replay. Attitude from the unified AHI `TelemetryData` source, built
  from explicit body axes (correct at all attitudes incl. inverted/high-bank). Adaptive position +
  attitude motion smoothing (median-interval, gap-aware) drives the model + follow/orbit camera;
  camera zoom-drift fixed. New manual **VTOL** platform type. Generators in `scripts/gen-uav-*.mjs`,
  assets in `static/models/`
- [x] Flight mode coloring of 3D track segments (Map3D — see Milestone 5b colored tracks)
- [x] **Altitude curtain + ground shadow** under the 3D track (vertical wall to ground + terrain-draped shadow; flight-mode coloured; Settings→Map toggle; in replay built progressively behind the UAV, chunked for scale + reverse-scrub debounce). See `docs/active/Map3DRework.md`
- [x] **Mission overlay in 3D** — mirrors the 2D map (same marker SVGs as billboards + same line styles, always-visible overlay) + per-WP drop-lines; "Show Mission" replay toggle (2D + 3D); planning/live always shown
- [ ] **Live-trail curtain** (same treatment as the replay track; deferred to simulator long-flight tests)
- [x] **Clean terrain-derived geoid offset** — `cesiumGround_ellipsoid − Copernicus MSL` undulation (GPS-independent), replacing the single-point GPS-snap that mis-placed tower starts; track uses the fused arming-relative altitude (`nav_alt_m`) anchored at the first GPS fix → also smooths the stair-stepped vertical track. Live UAV derives its own geoid at the first live fix
- [x] **Source-switch map clearing** — log↔log / replay→live wipe track+trail+markers; live connect clears only when disarmed (armed reconnect keeps track); disconnect never clears; mission kept + re-placed
- [x] **Live trail armed-only + black pre-arm trail** (disarmed GPS movement, 2D + 3D)
- [x] **Recenter camera on every 2D→3D switch** (deferred until the canvas is laid out)
- [x] **Over-zoom placeholders replaced without a manual zoom** — re-request visible tiles when a new blank region is detected
- [x] **Progressive shadow/curtain no longer spans a log switch** (clearDeco cancels pending timers + async-load guard)
- [x] ~~Smoothed flight track (polyline simplification / spline interpolation)~~ — obviated: the track now uses INAV's fused EKF altitude (`nav_alt_m`), which is already smooth; horizontal GPS spacing is fine at the colored-segment resolution. No resample needed.
- [ ] UI button refinements and responsive layout
- [ ] _(polish)_ **2D map: stepless zoom scaling for the UAV model marker** — the canvas marker is fixed-pixel, so at very close zoom its size only updates at zoom-end (Leaflet repositions/resets markers there). Migrate the model from an `L.Marker` to a geographically-anchored, zoom-animated canvas layer so it scales continuously with the map (like the tiles / WP vectors). Only noticeable at the closest zoom range.
- [ ] Altitude exaggeration toggle for low-altitude flights
- [ ] Auto-enable chase camera on live flight start
- [ ] Imagery tile resolution / LOD quirks (blurry or low-detail tiles at certain zooms)
- [ ] _(low priority follow-up)_ Geoid-undulation robustness — N is sampled at a **single point** (track start / first live fix) and applied flight-wide; negligible for local flights, only matters over very large areas. Bundle with terrain-load race hardening + provider-quality review.
- [ ] Evaluate whether Cesium World Terrain provider quality is sufficient or needs an alternative

---

*Last updated: 2026-06-16 (LOS/RF shipped; 3D direction markers dropped; passive radio telemetry + RC Link widget + flight-path marker added)*
