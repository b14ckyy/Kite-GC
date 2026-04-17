# Kite Ground Control — Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added — Blackbox Import & Playback (M5b)
- Blackbox import pipeline: `blackbox_decode` binary discovery (app folder first, PATH fallback), invoked with `--merge-gps --datetime --unit-height m --unit-gps-speed mps --stdout`
- Multi-log probing: `flightlog_probe_blackbox_logs` command tries indices 0–31 and returns all found logs with labels
- Import progress events: `flightlog-import-progress` Tauri event emitted at 9 stages (5–100%) during import
- Progress bar UI in Logbook tab shown during active import
- Multi-log selection: if the .TXT contains >1 session, user selects the desired log index before import starts
- CSV parsing performance overhaul: pre-built `HashMap<String, usize>` header index map resolves all column positions once — O(1) access per field per row (vs O(headers²) before)
- Downsampling to 10 Hz: reads `H looptime:` and `H P interval:` from the raw log header, computes effective sample rate, skips rows to keep ≤ 10 Hz in the DB (e.g. 500 Hz → keep 1 in 50 rows)
- Raw CSV lines stored in `blackbox_records.csv_data` (comma-joined) instead of full JSON re-serialization — significantly reduces parsing overhead
- Heading fix: INAV blackbox `heading` column is prioritised over `gps_ground_course`; auto-detects decidegrees (>360 → ÷10)
- `link_quality` field added to `TelemetryRecord` (0–100 %, maps `lq` / `link_quality` / `rxlq` from blackbox CSV; `None` for MSP live recordings)
- DB migration v3: `ALTER TABLE telemetry_records ADD COLUMN link_quality INTEGER`
- Log replay: track loaded into `selectedFlightTrack` on flight selection; orange dashed polyline rendered on map via `playbackTrack` prop
- Playback controls: Play/Pause/Reset buttons + scrubber timeline; timer-based at 120 ms/step
- Playback position marker: amber circle marker moves on map during playback
- `fitBounds` called once on new playback track load
- Wider logbook panel when a flight is selected: CSS `min()` responsive width, `nav-panel-wide` class adds ~560px extra width
- Improved logbook grid proportions (list/detail split)

### Added — Logbook UX Improvements (M5)
- Weather editor: compact read-only weather summary in flight detail + pencil edit icon that opens a weather editor form (temperature/wind steppers, wind direction/conditions dropdowns, save button)
- `flightlog_update_weather` Tauri command + `updateFlightWeather()` frontend store function
- Batch import: file picker allows multi-file selection for Blackbox logs (`.bbl`, `.bfl`, `.csv`, `.txt`)
- Drag & drop import: drop Blackbox files onto the logbook to import (Tauri `dragDropEnabled` + `tauri://drag-drop` listener)
- Logbook minimize/expand: click map → panel minimizes to 280px metadata-only view; click panel → expand back to full detail
- Notes auto-resize: textarea grows with content up to 140px, read-only in minimized mode
- Delete Flight button styled red for danger indication
- Duplicate flight detection dialog on import with force-import option
- Extended flight metadata: Firmware, Total Distance, Max Distance fields in detail panel
- All hardcoded UI strings replaced with i18n keys (duplicate dialog, import progress, weather edit title, status bar connection info)

### Added — Flight Recording & Logbook (M5, core)
- New Rust `flightlog` module: `db.rs`, `recorder.rs`, `raw_logger.rs`, `geocode.rs`, `weather.rs`, `types.rs`
- SQLite storage via `rusqlite` with bundled SQLite (no external SQLite install required)
- Migration system using `PRAGMA user_version` (schema v1)
- New `flights` and `telemetry_records` tables with flight metadata + sampled telemetry points
- Flight recorder integrated into scheduler loop (primary connection only), with ARM/DISARM transition detection from `MSPV2_INAV_STATUS` arming flags
- Automatic flight session start on ARM and finalize on DISARM
- Optional raw text log file output per flight (`raw_logs/*.txt`)
- New handshake step: `MSP_NAME` query; craft name now available in `FcInfo`
- New Tauri commands for logbook operations:
	- `flightlog_list`, `flightlog_get`, `flightlog_get_track`, `flightlog_delete`, `flightlog_update_notes`
	- `flightlog_geocode` (OSM Nominatim), `flightlog_fetch_weather` (Open-Meteo), `flightlog_default_db_path`
- Frontend store `src/lib/stores/flightlog.ts` for typed logbook command wrappers
- Settings integration:
	- `flightLoggingEnabled` (default OFF)
	- `flightLogDbPath` (custom folder or default AppData/portable path)
	- `flightLogRawEnabled` (default OFF)
- Settings UI enhancements:
	- Flight logging enable/disable toggle
	- Raw log toggle
	- Database folder picker using native directory dialog
- New Logbook tab with grouped sort modes:
	- Aircraft -> Location -> Date
	- Location -> Date -> Aircraft
	- Date -> Location -> Aircraft
	- Aircraft -> Date -> Location
- Flight detail panel with metadata, notes editing, and delete action
- English and German i18n keys for flight logging and logbook UI

### Tested — Flight Recording & Logbook
- Rust unit tests for DB schema + CRUD + telemetry batch + cascade delete (5 tests, all passing)
- `cargo check` successful
- `npm run check` successful (0 errors; existing warnings remain)

### Added — Mission Planning (M4)
- Mission module: Rust backend with `mission/types.rs`, `mission/codec.rs`, `mission/store.rs`
- Waypoint data model: all 8 INAV WP action types (Waypoint, PosholdUnlim, PosholdTime, RTH, SetPoi, Jump, SetHead, Land)
- MSP WP codec: `MSP_WP` (118) decode, `MSP_SET_WP` (209) encode, `MSP_WP_GETINFO` (20)
- MSP mission EEPROM: `MSP_WP_MISSION_SAVE` (18), `MSP_WP_MISSION_LOAD` (19)
- 13 Tauri commands: `mission_get`, `mission_clear`, `mission_add_wp`, `mission_update_wp`, `mission_remove_wp`, `mission_insert_wp`, `mission_reorder_wp`, `mission_download`, `mission_upload`, `mission_export_xml`, `mission_import_xml`, `mission_save_file`, `mission_load_file`
- 37 Rust unit tests covering codec, XML serialization, store operations
- Frontend mission store (`mission.ts`): Svelte writable stores, derived stores (`geoWaypoints`, `selectedWpIndex`, `editMode`), invoke wrappers
- MissionLayer.svelte: Leaflet map layer with SVG markers, polyline path, floating editor/labels
- MissionPanel.svelte: sidebar panel with WP table, detail view, FC/EEPROM/file controls
- Type-specific SVG marker icons: blue WP teardrop, orange PosHold circle with orbit ring, purple POI, orange Land teardrop with down-arrow, orange RTH house, grey generic fallback
- Floating editor popup per selected WP: type selector, altitude with REL/AMSL toggle, speed, hold time
- Floating parameter labels on non-selected WPs showing altitude and modifier summary
- Modifier WPs (Jump, RTH, SetHead) grouped into parent geo-WP editor popup
- Add/remove modifiers via dropdown in editor
- Display numbering skips modifier WPs (map markers + sidebar)
- Click-on-polyline to insert WP between existing waypoints
- Map click with editor open deselects WP instead of adding new
- Dashed lines for Jump (purple) and RTH (orange) modifiers on map
- WPs after first LAND/RTH greyed out (35% opacity, dashed grey polyline, non-draggable)
- Greyed WP rows in sidebar list (opacity + grayscale filter)
- FC Download / FC Upload buttons (RAM transfer)
- EEPROM Save / EEPROM Load buttons (save disabled when armed)
- Armed state detection via telemetry `armingFlags` (bit 2)
- File Open / Save via native OS file picker dialog (@tauri-apps/plugin-dialog)
- Drag & drop .mission file import
- MW XML format import/export (interoperable with INAV Configurator, mwp, ezgui)
- Max 120 WP sanity check on map click, polyline insert, and modifier add
- Warning text in modifier dropdown when WP limit reached
- WP count display (n/120) with dirty state badge
- Multi-mission support: dynamic tabs [1][+], up to 9 missions, 120 WP global limit across all missions
- Mission Control settings: Default WP Altitude (1–1000 m, default 50), Default PH Time (1–600 s, default 30), stepper +/− buttons
- Scrollable WP list with fixed (non-scrolling) control buttons at bottom
- Dark-themed scrollbars (custom WebKit styling + `color-scheme: dark`)
- Dark-themed number inputs and selects in editor popup
- Global `color-scheme: dark` on HTML root element

### Added — Installer & Portable Mode
- NSIS installer: install mode `both` — user chooses per-user (%LOCALAPPDATA%) or all-users (Program Files)
- NSIS uninstall hook: asks whether to remove application data (settings, map cache) from AppData
- Portable mode: place a `.portable` file next to the exe → all data stored in `data/` folder beside the binary
- Portable mode works on both Windows (WEBVIEW2_USER_DATA_FOLDER) and Linux (XDG_DATA_HOME/XDG_CONFIG_HOME)

### Added — Internationalization (i18n)
- `svelte-i18n` library with ICU Message Format for interpolation and plurals
- English locale file (`en.json`, ~200 translation keys across 18 namespaces)
- German locale file (`de.json`, complete translation)
- i18n initialization in `+layout.svelte` (blocks rendering until locale loaded)
- All 14 frontend component files converted: `+page.svelte`, `MissionPanel.svelte`, `MissionLayer.svelte`, `Map.svelte`, `DebugPanel.svelte`, 7 widget components
- Language picker in Settings panel (persists selection to localStorage)
- `WP_ACTION_KEYS` map in `mission.ts` for i18n-compatible waypoint action labels
- `labelKey` field in `widgetRegistry.ts` for translated widget names
- `locale` field in `AppSettings` with default `'en'`

### Fixed — Mission Planning (M4)
- Editor popup flicker on value edits: popup now on map (not layerGroup), direct DOM innerHTML update avoids Leaflet layout recalc
- Edit mode auto-disables when switching away from Mission tab or closing navigation panel

## [0.2.0] — 2026-04-15

### Added
- MSP scheduler: dedicated thread with priority-based adaptive polling
- Telemetry groups: Attitude (5 Hz), Status (1 Hz), Analog (1 Hz), GPS (2 Hz), Altitude (rotating)
- Configurable poll rates: Attitude 1–5 Hz, GPS Position 1–5 Hz
- Optional airspeed module (toggleable, rotates with altitude in secondary slot)
- Adaptive degradation: priority-based slot selection degrades low-priority groups first
- Live telemetry strip with real data: ALT, SPD, VARIO, BAT, SATS
- Aircraft position marker on map with heading arrow (SVG, rotates with yaw)
- Battery voltage/current/power display from MSPV2_INAV_ANALOG
- Arming status: pulsing ARMED widget in telemetry strip + status bar indicator
- GPS fix type display (NO GPS, NO FIX, 2D, 3D) + satellite count
- Sensor bar driven by MSP_SENSOR_STATUS (151) hardware health values
- Sensor bar states: green=OK, yellow=warning (GPS no fix), red=unhealthy, gray=none
- UAV platform type detection via MSP2_INAV_MIXER (0x2010) handshake step
- Platform type display in UAV Info panel (Multirotor, Airplane, Helicopter, etc.)
- MSP Debug Monitor panel (dev builds only, toggled via 🔧 Debug button in status bar)
- Debug: per-message LED indicators (yellow=request, green=response, red=timeout)
- Debug: MSG/s and bytes/s throughput counters (TX/RX)
- Debug: target rate vs actual rate per MSP code with throttle highlighting
- Debug: POLL/INIT status badges, request/response/timeout counters
- Debug module: zero-cost in release builds (`#[cfg(debug_assertions)]` + no-op stub)
- Settings: attitude rate, position rate, airspeed toggle (persisted in localStorage)

### Changed
- Floating navigation panel: hamburger menu opens tab rail + content panel
- Tab-based panel navigation: UAV Info, Settings, Mission Control
- Bottom telemetry overlay strip replaces placeholder widgets
- Animated hamburger icon (transforms to X when open)
- Glassmorphism UI elements (backdrop-filter blur, semi-transparent panels)
- Session persistence: panel state, active tab, telemetry rate settings
- Map zoom controls repositioned to top-right
- `MspRc` feature gate (INAV 8.0+), `AuxRc` feature gate (INAV 9.1+)
- Feature gate system: removed `MultiMission` (irrelevant, pre-7.0)

### Fixed
- MSPV2_INAV_ANALOG decode: correct byte offsets (batteryFlags:u8, vbat:u16, amperage:i16, power:u32, ...)
- GPS fix type mapping: added missing case 0 (NO GPS)
- MSPV2_INAV_STATUS decode: correct offsets for sensorStatus, cpuLoad, armingFlags
- Sensor bar: uses actual FC sensor health instead of connection state
- BARO indicator: uses hardware sensor status instead of altitude ≠ 0 heuristic
- Map resize handling on panel toggle transitions

## [0.1.0] — 2026-04-15

### Added
- Initial project setup with Tauri 2.0 + Svelte 5 + TypeScript
- Modular Rust backend structure (msp, transport, commands)
- MSP v1/v2 codec with encode/decode and unit tests
- MSP streaming parser (byte-by-byte state machine)
- Serial transport with cross-platform port listing
- Tauri IPC commands: `list_serial_ports`, `connect`, `disconnect`, `get_app_version`
- MSP handshake: API_VERSION, FC_VARIANT, FC_VERSION, BOARD_INFO
- INAV version parsing with minimum version check (≥ 7.0)
- Version-dependent feature gating (CoreTelemetry, AutolandConfig, Geozones)
- Svelte frontend with dark-themed GCS layout (INAV Configurator color scheme)
- Reactive stores for connection, telemetry, and settings state
- Leaflet map integration with OpenStreetMap tiles
- Connection status display with sensor bar
- GPLv3 license
- Build scripts for Windows and Linux
- Development documentation (DEVLOG, CHANGELOG, ARCHITECTURE, ROADMAP)
