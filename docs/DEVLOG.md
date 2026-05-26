# Kite Ground Control — Development Documentation

## Project Overview

Kite Ground Control is a cross-platform Ground Control Station supporting [INAV](https://github.com/iNavFlight/inav)-based flight controllers (ArduPilot planned). It communicates primarily via MSP (MultiWii Serial Protocol) and aims to provide mission planning, real-time telemetry monitoring, and flight control capabilities.

**Long-term scope reference**: [MWPTools](https://stronnag.grebedoc.dev/mwptools/)

## Technology Stack

| Component | Technology | Purpose |
|---|---|---|
| Application Framework | Tauri 2.0 | Cross-platform desktop + mobile shell |
| Backend | Rust | MSP protocol, serial/BLE communication, state management |
| Frontend | Svelte 5 + TypeScript | User interface, reactive data display |
| Map Library | Leaflet 1.9.4 | Interactive maps for GCS and mission planning |
| File Dialogs | @tauri-apps/plugin-dialog 2.7.0 | Native OS file picker (mission save/load) |
| Build Tool | Vite | Frontend bundling and dev server |
| License | GPL-3.0-only | Open source license |

## Target Platforms

| Platform | Status | Notes |
|---|---|---|
| Windows (x64) | Active Development | Primary development platform |
| Linux (x86_64) | Planned | |
| Linux (ARM64) | Planned | Raspberry Pi, etc. |
| Android | Planned | Via Tauri mobile support |
| macOS | Future | Needs test hardware |
| iOS | Future | Needs test hardware |

## Project Structure

```
Kite Ground Control/
├── src/                          # Svelte Frontend
│   ├── routes/                   # SvelteKit pages/routes
│   │   ├── +page.svelte          # Main application page (CSS Grid zone layout)
│   │   └── +layout.ts            # SvelteKit layout config (SSR disabled)
│   ├── lib/                      # Shared frontend modules
│   │   ├── stores/               # Svelte reactive state stores
│   │   │   ├── connection.ts     # Connection state, FC info, feature set
│   │   │   ├── telemetry.ts      # Telemetry data store (GPS, attitude, battery)
│   │   │   ├── settings.ts       # Session persistence (localStorage)
│   │   │   ├── home.ts           # Home position store (set on arm + GPS fix)
│   │   │   ├── mission.ts        # Mission state: WP types, stores, invoke wrappers, XML I/O
│   │   │   ├── layout.ts        # Layout zone system: profiles, dock visibility, CSS grid overrides
│   │   │   └── flightlog.ts      # Flight log API wrappers, types, grouping/sort helpers
│   │   ├── controllers/          # Domain logic extracted from +page.svelte
│   │   │   ├── connectionController.ts  # Serial port refresh, connect/disconnect, listener mgmt
│   │   │   ├── logbookController.ts     # Flight CRUD, Blackbox import, geocode/weather
│   │   │   ├── playbackController.ts    # Timer-based playback engine (100ms tick, 1×–10× speed)
│   │   │   └── widgetController.ts      # DnD reorder/cross-panel move (pure functions)
│   │   ├── adapters/             # Data format adapters
│   │   │   └── telemetryAdapter.ts      # DB TelemetryRecord → TelemetryData for widgets
│   │   ├── helpers/              # Pure utility functions
│   │   │   ├── telemetry.ts      # isArmed(), hasKnownLocation(), isValidGpsCoordinate()
│   │   │   └── trackColors.ts    # Track color modes, flight mode classification, gradient functions, nav state colors
│   │   ├── components/           # Reusable UI components
│   │   │   ├── Map.svelte        # Leaflet map (trail, home marker, cached tiles, heading-up)
│   │   │   ├── MissionLayer.svelte # Mission map layer (markers, polyline, editor popups)
│   │   │   ├── MissionPanel.svelte # Mission sidebar (WP list, FC/EEPROM/file controls)
│   │   │   ├── WidgetPanel.svelte # Drag-and-drop widget panel container
│   │   │   ├── DebugPanel.svelte # MSP debug monitor (dev builds only)
│   │   │   ├── LogPlayer.svelte  # Playback controls (play/pause/reset, scrubber, speed)
│   │   │   ├── LogbookPanel.svelte # Flight list, detail view, import/weather/notes
│   │   │   ├── SettingsPanel.svelte # All settings sections
│   │   │   ├── Toolbar.svelte    # Logo, sensor bar, port selector, connect button
│   │   │   ├── UavInfoPanel.svelte # FC info, feature gates, craft name
│   │   │   ├── StatusBar.svelte  # Connection status, arming indicator, app title
│   │   │   ├── NavRail.svelte    # Hamburger menu + vertical tab rail
│   │   │   ├── Map3D.svelte      # CesiumJS 3D globe view (optional, alongside Leaflet)
│   │   │   └── widgets/          # HUD widget components
│   │   │       ├── AHI.svelte        # Artificial Horizon Indicator
│   │   │       ├── SpeedWidget.svelte # Ground speed + airspeed
│   │   │       ├── AltWidget.svelte   # Altitude + vario
│   │   │       ├── BatteryWidget.svelte # Voltage, current, mAh
│   │   │       ├── GpsWidget.svelte   # Satellite count + fix type
│   │   │       ├── CompassWidget.svelte # Compass rose + heading
│   │   │       ├── HomeWidget.svelte  # Home direction, distance, bearing
│   │   │       └── RawTelemetryWidget.svelte # Raw telemetry data panel
│   │   ├── cache/                # Map tile cache
│   │   │   ├── tileCache.ts      # IndexedDB backend, LRU eviction
│   │   │   └── CachedTileLayer.ts # Custom Leaflet TileLayer with cache
│   │   ├── config/               # Static configuration
│   │   │   ├── mapProviders.ts   # Map tile provider definitions
│   │   │   └── widgetRegistry.ts # Widget definitions, size constants, classes
│   │   ├── i18n/                 # Internationalization
│   │   │   ├── index.ts          # i18n init, locale registration, SUPPORTED_LOCALES
│   │   │   └── locales/          # Translation files
│   │   │       ├── en.json       # English (default, ~200 keys)
│   │   │       └── de.json       # German (complete)
│   │   ├── utils/                # Utility functions
│   │   │   └── geo.ts            # Haversine distance, bearing, formatting
│   │   └── index.ts              # Library entry point
│   └── app.html                  # HTML entry point
│
├── src-tauri/                    # Rust Backend (Tauri)
│   ├── src/
│   │   ├── main.rs               # Application entry point
│   │   ├── lib.rs                # Tauri app builder and plugin registration
│   │   ├── state.rs              # AppState (ActiveProtocol enum: MSP/MAVLink + FC info)
│   │   ├── commands/             # Tauri IPC commands (frontend-callable)
│   │   │   ├── mod.rs            # Command module registry
│   │   │   ├── connection.rs     # Multi-protocol connect/disconnect (MSP + MAVLink paths)
│   │   │   ├── flightlog.rs      # Flight log commands (list/get/track/delete/notes/geocode/weather/update_weather/import/probe)
│   │   │   ├── mission.rs        # Mission CRUD, FC transfer, XML/file I/O (13 commands)
│   │   │   └── info.rs           # App version and metadata
│   │   ├── flightlog/            # Flight recording + logbook backend
│   │   │   ├── mod.rs            # Module exports
│   │   │   ├── types.rs          # Flight/TelemetryRecord/summary/settings structs
│   │   │   ├── db.rs             # SQLite schema, migrations (v0→v5), CRUD, tests
│   │   │   ├── recorder.rs       # Arm/disarm-driven recording engine (MSP + MAVLink, continuous mode)
│   │   │   ├── raw_logger.rs     # MSP raw text log writer (CSV format)
│   │   │   ├── tlog_logger.rs    # MAVLink tlog binary logger (Mission Planner/QGC compatible)
│   │   │   ├── geocode.rs        # OSM Nominatim reverse geocoding
│   │   │   ├── weather.rs        # Open-Meteo weather fetcher
│   │   │   ├── blackbox.rs       # Blackbox decode pipeline (discovery, invocation, CSV parsing, downsampling)
│   │   │   ├── ardupilot.rs      # ArduPilot DataFlash .bin log import
│   │   │   ├── exchange.rs       # .kflight export/import (self-contained SQLite exchange format)
│   │   │   └── track_export.rs   # KMZ/KML/GPX/CSV track export with RDP simplification
│   │   ├── mission/              # Mission planning module
│   │   │   ├── mod.rs            # Module exports
│   │   │   ├── types.rs          # WpAction enum (8 types), Waypoint, Mission, MissionInfo
│   │   │   ├── codec.rs          # MSP_WP binary codec (encode/decode 21-byte payload)
│   │   │   └── store.rs          # MissionStore (Mutex<Mission>), CRUD, XML serialization
│   │   ├── scheduler/            # MSP scheduler (dedicated thread)
│   │   │   ├── mod.rs            # Scheduler loop, slot management, adaptive polling
│   │   │   ├── telemetry.rs      # Telemetry decoding and configuration
│   │   │   └── debug.rs          # MSP debug stats tracker (dev builds only)
│   │   ├── msp/                  # MSP Protocol implementation
│   │   │   ├── mod.rs            # MSP module exports
│   │   │   ├── types.rs          # Message types, constants, command codes
│   │   │   ├── codec.rs          # MSP v1/v2 frame encode/decode
│   │   │   ├── parser.rs         # Streaming byte-by-byte state machine
│   │   │   ├── transport.rs      # MSP framing layer over ByteTransport
│   │   │   └── features.rs       # Version-dependent feature gating
│   │   ├── mavlink_proto/        # MAVLink Protocol implementation
│   │   │   ├── mod.rs            # Module exports + re-exports
│   │   │   ├── parser.rs         # MAVLink v1/v2 frame parser (byte-level state machine)
│   │   │   ├── codec.rs          # MAVLink v2 frame serialization
│   │   │   ├── handshake.rs      # Connection handshake (HEARTBEAT + AUTOPILOT_VERSION)
│   │   │   └── handler.rs        # Dedicated handler thread (telemetry dispatch + recording)
│   │   └── transport/            # Communication transports
│   │       ├── mod.rs            # ByteTransport trait + transport abstractions
│   │       ├── serial.rs         # Serial port transport (serialport crate)
│   │       ├── tcp.rs            # TCP client transport
│   │       ├── udp.rs            # UDP transport
│   │       └── ble.rs            # Bluetooth Low Energy transport
│   ├── .cargo/config.toml        # Cargo config (target-dir override)
│   ├── Cargo.toml                # Rust dependencies
│   ├── Cargo.lock                # Dependency lock file
│   └── tauri.conf.json           # Tauri configuration
│
├── scripts/                      # Legacy build scripts (still functional)
│   ├── build-windows.bat         # Windows release build (improved)
│   ├── build-linux.sh            # Linux release build (improved)
│   ├── dev.bat                   # Windows dev server (improved)
│   └── dev.sh                    # Linux dev server (improved)
│
├── justfile                      # Primary task runner (recommended way)
│                                 #   just dev / just build / just check
│
├── .github/workflows/ci.yml      # Minimal CI (cargo check + svelte-check)
│
├── docs/                         # Development documentation
│   ├── DEVLOG.md                 # This file — project structure & dev notes
│   ├── CHANGELOG.md              # Version changelog (Keep a Changelog format)
│   ├── ARCHITECTURE.md           # Architecture Decision Records (ADRs)
│   ├── ROADMAP.md                # Feature roadmap by milestone
│   ├── FLIGHTLOG_DATABASE.md     # Flight log database schema documentation
│   ├── DATA_PIPELINE.md          # Data pipeline architecture (live + replay flows)
│   ├── PROTOCOL_REFACTORING.md   # Multi-protocol (MAVLink) integration workstream plan
│   ├── PROTOCOL_FLIGHT_MODES.md  # INAV/ArduPilot flight mode reference
│   ├── COLORED_TRACK_PLAN.md     # Colored flight track design notes
│   ├── ARDUPILOT_IMPORT_PLAN.md  # ArduPilot log import planning
│   └── M5_TEST_CHECKLIST.md      # Manual verification checklist for M5 implementation
│
├── static/                       # Static assets (icons, etc.)
├── .gitignore                    # Git ignore rules
├── LICENSE                       # GPL-3.0 license
├── package.json                  # Node.js project config
└── README.md                     # Project readme
```

## Module Concept

Each feature is self-contained in its own module:

- **Backend (Rust)**: New features get their own subfolder in `src-tauri/src/` with a `mod.rs` entry point. Commands are registered in `commands/mod.rs` and wired in `lib.rs`.
- **Frontend (Svelte)**: State lives in `src/lib/stores/`, domain logic in `src/lib/controllers/`, data adapters in `src/lib/adapters/`, utility functions in `src/lib/helpers/`, UI components in `src/lib/components/`, pages in `src/routes/`.
- **+page.svelte**: Thin orchestrator — imports controllers/adapters/components, wires reactive derivations (`$derived`), routes events. No business logic inline.
- **Adding a new feature**: Create the Rust module → Add commands → Register in `lib.rs` → Create Svelte store → Create controller (if complex logic) → Create UI component → Wire into page.

## Development Setup

### Prerequisites
- Node.js LTS (v24+)
- Rust (via rustup, v1.94+)
- [just](https://github.com/casey/just) (strongly recommended)
- Platform-specific: see [Build Guide](docs/dev/BUILD.md) for details

### Quick Start (recommended)
```bash
npm install
just dev                 # Start development (uses just + Tauri)
```

Alternative (still works):
```bash
npm run tauri dev
```

### Building (recommended)
```bash
just build
just build-windows
just build-linux
```

Alternative (still works):
```bash
npm run tauri build
```

For the complete guide (troubleshooting, CI, common Windows issues, etc.), see **[docs/dev/BUILD.md](../dev/BUILD.md)**.

### Platform Notes

- **Cargo target-dir**: Set to `D:\cargo-target\kite-gc` via `src-tauri/.cargo/config.toml` to avoid issues with OneDrive paths containing spaces.
- **Windows**: Requires Visual Studio Build Tools 2022 (MSVC linker). Node.js v24+ via winget (do NOT use NVM4W — causes PATH conflicts).
- **PATH quirks**: New terminal sessions may need PATH reload: `$env:Path = [System.Environment]::GetEnvironmentVariable("Path","Machine") + ";" + [System.Environment]::GetEnvironmentVariable("Path","User")`

## UI Architecture

The UI uses a **CSS Grid zone layout** — the map fills the entire viewport behind all zones, and UI elements are placed in named grid areas. Floating panels overlay the map within the Panel Zone.

**Grid layout** (4 columns × 4 rows):
```
┌──────────┬──────────────────────┬────────────┬──────────┐
│ TOOLBAR  │      TOOLBAR         │  TOOLBAR   │ TOOLBAR  │
│  (62px)  │       (1fr)          │  (clamp)   │  (54px)  │
├──────────┼──────────────────────┼────────────┼──────────┤
│          │                      │            │          │
│ NAV RAIL │    PANEL ZONE        │ SIDE DOCK  │SIDE DOCK │
│  (62px)  │      (1fr)           │(150-250px) │          │
├──────────┼──────────────────────┼────────────┼──────────┤
│          │                      │            │          │
│ NAV RAIL │   BOTTOM DOCK        │BOTTOM DOCK │ MAP CTRL │
│  (62px)  │  (184-300px tall)    │            │  (54px)  │
├──────────┼──────────────────────┼────────────┼──────────┤
│STATUS BAR│    STATUS BAR        │ STATUS BAR │STATUS BAR│
└──────────┴──────────────────────┴────────────┴──────────┘
```

- **Toolbar** (top, fixed 53px): Logo, sensor status bar, serial port controls, connect button
- **Nav Rail** (left, fixed 62px): Hamburger menu + vertical tab icons
- **Panel Zone** (center, 1fr × 1fr): Floating panels (Settings, UAV Info, Logbook, Mission) — `position: absolute` with grid-variable-derived size limits
- **Bottom Dock** (bottom center, clamp 184–300px): Horizontal widget strip with container-relative sizing
- **Side Dock** (right, clamp 150–250px): Vertical widget strip with container-relative sizing
- **Map Controls** (bottom right, fixed 54px): Zoom, 3D toggle, compass buttons
- **Status Bar** (bottom, fixed 24px): Connection status, arming state, app title
- **Map** (rows 2–3, all columns, z-index 0): Leaflet/CesiumJS map behind all zones

**Layout store** (`src/lib/stores/layout.ts`): Drives grid zone visibility and size overrides via CSS custom properties. Supports layout profiles (`flight`, `mission`, `area-planner`) for future mode switching.

**Widget sizing**: Container-relative px, not viewport-relative vmin. Each dock computes its own `pxPerUnit = crossAxisPx / LARGE_BASE_VMIN` from measured container dimensions, fully decoupling bottom and side dock scaling.

All overlay elements use glassmorphism styling (backdrop-blur, semi-transparent backgrounds) with the INAV Configurator color scheme (#37a8db accent, #2e2e2e panels).

See [ARCHITECTURE.md](ARCHITECTURE.md) ADR-023 for the full rationale.

## MSP Protocol Implementation

### Codec (`msp/codec.rs`)
- MSP v1 encode/decode with XOR checksum
- MSP v2 encode/decode with CRC8 DVB-S2 checksum
- Jumbo frame support (payloads ≥ 255 bytes)

### Parser (`msp/parser.rs`)
- Byte-by-byte streaming state machine (18 decoder states)
- Handles interleaved v1/v2 frames
- Error tracking with packet error counter

### Feature Gates (`msp/features.rs`)
- `InavVersion` with parse, comparison (`is_at_least`), Display
- Version-dependent feature detection:
  - `CoreTelemetry` — always available (≥ 7.0)
  - `AutolandConfig` — INAV 7.1+
  - `Geozones` — INAV 8.0+
  - `MspRc` — INAV 8.0+ (MSP as full RC protocol)
  - `AuxRc` — INAV 9.1+ (auxiliary RC channels via MSP)
- Minimum supported version: **INAV 7.0.0**

### Handshake (`commands/connection.rs`)
Sequence: `MSP_API_VERSION` → `MSP_FC_VARIANT` (must be "INAV") → `MSP_FC_VERSION` (must be ≥ 7.0) → `MSP_BOARD_INFO` → `MSP2_INAV_MIXER` (platform type, mixer preset) → `MSP_NAME` (craft name) → feature gate computation

## Session Persistence

Settings stored in `localStorage` under key `kite-gc-settings`:
- `lastPort` / `lastBaud` — last used serial connection
- `map.center` / `map.zoom` — map viewport state
- `mapProvider` / `mapCacheMaxMB` — tile provider + cache size
- `navPanelOpen` / `activeTab` — floating panel state
- `attitudeRateHz` / `positionRateHz` / `airspeedEnabled` — telemetry poll config
- `flightLoggingEnabled` / `flightRecordingEnabled` / `flightLogDbPath` / `flightLogRawEnabled` — flight logging + recording config
- `defaultWpAltitudeM` / `defaultPhTimeSec` — mission control defaults
- `locale` — UI language (`'en'` or `'de'`)
- `widgetAhi` / `widgetSpeed` / `widgetAltitude` / `widgetBattery` / `widgetGps` / `widgetCompass` / `widgetHome` — per-widget visibility toggles
- `panels` — widget panel layout: `{ bottom: string[], right: string[], positions?: Record<string, 'bottom' | 'right'> }`

Implemented via custom Svelte store with auto-save on every mutation. Schema evolution handled by merging defaults: `{ ...defaults, ...stored }`.

## M5 Test Notes

- Detailed manual test checklist for M5 is in `docs/M5_TEST_CHECKLIST.md`.
- Backend DB tests are in `src-tauri/src/flightlog/db.rs` (`cargo test flightlog --lib`).

## HUD Widget Panel System

The HUD uses a **two-panel drag-and-drop layout** within the CSS Grid zone system:

- **Bottom Dock**: Horizontal strip (grid row 3, col 2–3), height `clamp(184px, 20vh, 300px)`. Edit button + centered widget strip.
- **Side Dock**: Vertical strip (grid row 2, col 3–4), width `clamp(150px, 15vw, 250px)`.

### Widget Classes
- **Large** (22.5 units): AHI, Compass — circular, complex visualizations
- **Small** (13.5 units = 60% of large): All others — square, compact data display

### Container-Relative Sizing
Each dock measures its own cross-axis dimension (`bind:clientWidth/Height`) and computes an independent `pxPerUnit = (crossAxis - padding) / LARGE_BASE_VMIN`. Widget sizes are computed in abstract units by `computeSizes()`, then multiplied by `pxPerUnit` to get CSS `px` values. This fully decouples bottom and side dock scaling — changing viewport width only affects the bottom dock's main axis, not the side dock's widget sizes.

### Drag & Drop
- **Half-position detection**: Cursor position relative to slot midpoint determines before/after insertion
- **Insertion indicator**: Blue line shows exact drop position (vertical for horizontal panel, horizontal for vertical)
- **Cross-panel moves**: Drag from bottom → right or vice versa, with capacity check
- **Tauri interop**: `dragDropEnabled: false` in tauri.conf.json to prevent Tauri from intercepting HTML5 DnD events
- **Edit mode overlay**: Transparent overlay div on each widget captures drag events without blocking widget rendering

### Position Memory
Widget panel assignments are stored in `PanelConfig.positions` (Record<string, 'bottom' | 'right'>). When a widget is toggled OFF, its current panel is saved. When toggled back ON, it restores to its last panel instead of always defaulting to bottom.

## Map View Modes

The map supports two view modes, toggled via a button below the zoom controls:

- **North-Up** (default): Standard map orientation, north at top.
- **Heading-Up**: Map rotates with UAV heading so the aircraft always faces up. CSS `transform: rotate() scale(1.42)` on the map container with `overflow: hidden` on the wrapper. Leaflet controls are counter-rotated. UAV marker icon uses fixed 0° rotation since the map itself rotates.

## Internationalization (i18n)

The app uses `svelte-i18n` for multi-language support with ICU Message Format.

### Architecture
- **Library**: `svelte-i18n` — battle-tested, supports ICU interpolation (`{count}`, `{error}`), plurals, and `$store` auto-subscription in Svelte 5
- **Locale files**: `src/lib/i18n/locales/en.json` (default) and `de.json` — flat namespace structure with ~200 keys across 18 namespaces
- **Init**: `src/lib/i18n/index.ts` registers locales and exports `initI18n(locale?)` + `SUPPORTED_LOCALES`
- **Layout**: `+layout.svelte` reads persisted locale from settings, calls `initI18n()`, and gates rendering on `$isLoading`

### Key Decisions
- **Rust backend errors stay English**: Technical strings with port names, byte counts etc. are not localized. The frontend wraps them in user-facing messages where needed.
- **`$t()` in .svelte files**: Works via Svelte 5's auto-subscription to stores. No `get(t)` needed in template or reactive contexts.
- **`WP_ACTION_KEYS`**: Static `Record<WpAction, string>` mapping action enum values to i18n keys (e.g., `'wpAction.waypoint'`). Used with `$t(WP_ACTION_KEYS[action])` at point of use.
- **Widget labels**: `widgetRegistry.ts` has `labelKey` field alongside the English `label` fallback.
- **MissionLayer HTML**: Uses `$t()` inside plain JS functions within `.svelte` files — Svelte 5 auto-subscribes stores in component scope.

### Adding a New Language
1. Copy `src/lib/i18n/locales/en.json` → `{code}.json`
2. Translate all values
3. Register in `src/lib/i18n/index.ts`: `register('{code}', () => import('./locales/{code}.json'))`
4. Add to `SUPPORTED_LOCALES` array

## Testing

- **37 Rust unit tests** covering MSP codec, parser, feature gates, telemetry decoders, and mission module
- Run: `cd src-tauri && cargo test --target-dir "D:\cargo-target\kite-gc"`
- Frontend type-check: `npx svelte-check --tsconfig ./tsconfig.json`

## MSP Scheduler Architecture

The scheduler owns the `SerialConnection` after the initial handshake and runs in a dedicated `std::thread`. It coordinates all MSP traffic to prevent collisions on the single request-response link.

### Design Principles
1. **Single outstanding request**: MSP is request-response — scheduler sends one request, waits for reply/timeout, then decides what's next
2. **Priority-based adaptive degradation**: When overloaded, highest-priority slots are polled first — lower-priority groups naturally lose bandwidth
3. **No link type detection**: Polls at configured rate as long as the link sustains it. Adaptive degradation handles slow links automatically
4. **Non-blocking commands**: Waypoint uploads/downloads interleave between telemetry polls — bulk items fill gaps, not one-per-cycle

### Scheduler Loop
```
loop {
    1. Find most overdue telemetry slot (by priority, then overdue duration) → poll it
    2. If no slot is due → check command channel (non-blocking)
    3. If no command → try bulk channel (squeeze between polls)
    4. If nothing to do → sleep until next slot is due
}
```

### Telemetry Groups

| Group | MSP Code(s) | Default Rate | Range | Priority | Notes |
|---|---|---|---|---|---|
| Attitude | `MSP_ATTITUDE` (108) | 5 Hz | 1–5 Hz | 5 (highest) | Roll, Pitch, Heading |
| Status | `MSPV2_INAV_STATUS` (0x2000), `MSP_SENSOR_STATUS` (151) | 1 Hz | fixed | 4 | Arming, Flight modes, Sensor health |
| Analog | `MSPV2_INAV_ANALOG` (0x2002) | 1 Hz | fixed | 3 | Voltage, Current, mAh, RSSI |
| Position Primary | `MSP_RAW_GPS` (106) | 2 Hz | 1–5 Hz | 2 | Lat, Lon, Speed, COG, numSat |
| Position Secondary | `MSP_ALTITUDE` (109), `MSPV2_INAV_AIR_SPEED`* (0x2009) | rotates | — | 1 (lowest) | *Airspeed optional |

### Staggered Position Polling
Position Secondary rotates through its codes (one per cycle):
- Default (airspeed off): Only `MSP_ALTITUDE` every cycle
- Airspeed enabled: Alternates ALT → AIRSPEED → ALT → ...
- Future optional modules (wind, etc.) are appended to the rotation array.

### Adaptive Degradation
Instead of detecting link type (USB vs wireless), the scheduler uses **priority-based slot selection**. When multiple slots are overdue simultaneously (i.e. bandwidth is insufficient), the highest-priority slot always wins. This causes lower-priority groups to naturally degrade:

1. **Full bandwidth**: All groups polled at configured rates — no degradation
2. **Moderate overload**: GPS (priority 2) and secondaries (priority 1) lose cycles → effectively lower Hz
3. **Severe overload**: Everything except Attitude degrades → Attitude keeps maximum achievable rate
4. **Extreme overload (very slow link)**: Even Attitude can't sustain configured rate → natural slowdown

This is simpler and more robust than explicit link type detection, since USB devices like SiK radios or STM32-based systems (mLRS) can be "USB-connected but wireless".

### Data Flow
```
connect() → handshake (blocking)
         → SerialConnection moved into scheduler thread
         → scheduler starts telemetry polling
         → Tauri events emitted to frontend (telemetry-attitude, telemetry-gps, ...)
         → commands/bulk sent via mpsc channels
disconnect() → SchedulerCommand::Stop → thread joins → cleanup
```

## Blackbox Integration (M5b)

Blackbox log files from INAV flight controllers contain high-resolution telemetry data in a binary format. Integration is limited to GPS/telemetry archival — **not** a full Blackbox analyzer (no PID/gyro/motor visualization).

### External Binary Approach

Blackbox logs are decoded using the official `blackbox_decode` binary from [iNavFlight/blackbox-tools](https://github.com/iNavFlight/blackbox-tools) (GPL-3.0). The binary is bundled alongside the application, **not** compiled into `kite-gc.exe`.

**Binary discovery** (in order):
1. Application folder (next to executable)
2. System PATH fallback

No settings UI for the path — if the binary is missing, import is disabled with a user-facing message.

**Invocation**:
```
blackbox_decode --merge-gps --datetime --unit-height m --unit-gps-speed mps --stdout <file.TXT>
```
- `--merge-gps`: Interpolates GPS samples into main loop iterations
- `--datetime`: Converts timestamps to absolute date/time using log header
- `--stdout`: Outputs CSV to stdout (captured by Rust `Command`)
- `--unit-height m`: Forces altitude output in metres
- `--unit-gps-speed mps`: Forces speed output in m/s
- `--index N`: Selects specific log from multi-session .TXT files

### Data Pipeline

```
.TXT file (binary Blackbox log)
    │
    ▼
probe_blackbox_logs() — tries --index 0..31, exit-code check per index
    │  returns Vec<BlackboxLogProbe> { index, label }
    │
    ▼
User selects log index (if >1 found)
    │
    ▼
import_blackbox_log_with_progress<F>()
    │  reads H looptime + H P interval from raw header
    │  computes effective log rate (e.g. looptime=500µs × interval=4 = 500 Hz)
    │  computes keep_every = effective_Hz / 10 (downsample to 10 Hz)
    │
    ▼
blackbox_decode (child process, stdout capture)
    │
    ▼
CSV text (dynamic columns, INAV-version-dependent)
    │
    ▼
Rust CSV parser
    ├── pre-builds HashMap<String, usize> header index map (once)
    ├── resolves ColumnIndices from index map (once per file)
    ├── skips (keep_every − 1) rows between kept rows (downsampling)
    └── stores raw comma-joined CSV line (not JSON)
    │
    ▼
telemetry_records → sampled at ≤ 10 Hz (lat, lon, alt, speed, heading, lq, …)
blackbox_records  → same sampled rows, raw CSV text (for future detail analysis)
blackbox_files    → original .TXT archived as BLOB (for re-processing)
```

### Downsampling Design

For a log with `H looptime:500` (500 µs loop) and `H P interval:1/4` (every 4th loop):
- Raw log rate = 500 µs × 4 = 2000 µs = **500 Hz**
- Target = 10 Hz = 100 000 µs interval
- `keep_every` = 100 000 / 2000 = **50** — only 1 in 50 rows stored
- 5-minute flight at 500 Hz: ~150 000 raw rows → ~3 000 DB rows

The raw `.TXT` file is always archived in `blackbox_files` regardless of downsampling.

### TelemetryRecord Fields from Blackbox

| Field | CSV column(s) | Notes |
|---|---|---|
| `timestamp_ms` | `time (us)` ÷ 1000 | |
| `lat` | `GPS_coord[0]` | requires `--merge-gps` |
| `lon` | `GPS_coord[1]` | requires `--merge-gps` |
| `alt_m` | `GPS_altitude` / `altitude` / `baroAlt_cm` | cm values auto-divided by 100 |
| `speed_ms` | `GPS_speed` | in m/s with `--unit-gps-speed mps` |
| `heading` | **`heading`** → `GPS_ground_course` | INAV attitude heading (decidegrees ÷10 auto-detected) |
| `vario_ms` | `gps_velned[2]` → `vario` | NED down cm/s: negated and ÷100 for climb m/s |
| `voltage` | `vbat` | |
| `current_a` | `amperage` | |
| `mah_drawn` | `mahdrawn` | |
| `rssi` | `rssi` | |
| `roll` | `roll` / `attitude[0]` / `attitude_roll` | **always ÷10** (INAV decidegrees → degrees) |
| `pitch` | `pitch` / `attitude[1]` / `attitude_pitch` | **always ÷10** (INAV decidegrees → degrees) |
| `yaw` | `yaw` / `attitude[2]` / `attitude_yaw` | decidegrees auto-detected (>360 → ÷10) |
| `num_sat` | `GPS_numSat` | |
| `link_quality` | `lq` / `link_quality` / `rxlq` | ELRS/CRSF only; `None` if column absent |

### DB Schema (v5)

Current schema version is **5**. Migration path: v0→v1 (initial schema), v1→v2 (blackbox tables + `flights.source`), v2→v3 (link_quality column), v3→v4 (replay telemetry fields), v4→v5 (craft_name column).

```sql
-- v5 migration (2026-04-21):
ALTER TABLE flights ADD COLUMN craft_name TEXT;

-- v4 migration (replay-focused fields):
-- Added baro_alt_m, gps_alt_m, fix_type, num_sat, gps_hdop, active_flight_modes,
-- arming_flags, flight_mode_flags, cpu_load, nav_state, nav_wp_number, wind_speed_ms,
-- wind_direction, rc_channels (JSON), sensors_health to telemetry_records

-- v3 migration (2026-04-17):
ALTER TABLE telemetry_records ADD COLUMN link_quality INTEGER;

-- v2 tables (unchanged):
CREATE TABLE blackbox_records (
    id            INTEGER PRIMARY KEY,
    flight_id     INTEGER NOT NULL REFERENCES flights(id) ON DELETE CASCADE,
    timestamp_us  INTEGER NOT NULL,
    csv_data      TEXT NOT NULL  -- raw comma-joined CSV row (not JSON)
);
CREATE TABLE blackbox_files (
    id                INTEGER PRIMARY KEY,
    flight_id         INTEGER NOT NULL REFERENCES flights(id) ON DELETE CASCADE,
    original_filename TEXT NOT NULL,
    log_index         INTEGER NOT NULL DEFAULT 0,
    file_data         BLOB NOT NULL,
    file_size         INTEGER NOT NULL,
    imported_at       TEXT NOT NULL DEFAULT (datetime('now'))
);
-- flights.source: 'live' | 'blackbox' | 'both'
```
