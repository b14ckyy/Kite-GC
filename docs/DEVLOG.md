# INAV GCS — Development Documentation

## Project Overview

INAV GCS is a cross-platform Ground Control Station for [INAV](https://github.com/iNavFlight/inav)-based flight controllers. It communicates primarily via MSP (MultiWii Serial Protocol) and aims to provide mission planning, real-time telemetry monitoring, and flight control capabilities.

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
INAV GCS/
├── src/                          # Svelte Frontend
│   ├── routes/                   # SvelteKit pages/routes
│   │   ├── +page.svelte          # Main application page (floating panel layout)
│   │   └── +layout.ts            # SvelteKit layout config (SSR disabled)
│   ├── lib/                      # Shared frontend modules
│   │   ├── stores/               # Svelte reactive state stores
│   │   │   ├── connection.ts     # Connection state, FC info, feature set
│   │   │   ├── telemetry.ts      # Telemetry data store (GPS, attitude, battery)
│   │   │   ├── settings.ts       # Session persistence (localStorage)
│   │   │   ├── home.ts           # Home position store (set on arm + GPS fix)
│   │   │   └── mission.ts        # Mission state: WP types, stores, invoke wrappers, XML I/O
│   │   ├── components/           # Reusable UI components
│   │   │   ├── Map.svelte        # Leaflet map (trail, home marker, cached tiles, heading-up)
│   │   │   ├── MissionLayer.svelte # Mission map layer (markers, polyline, editor popups)
│   │   │   ├── MissionPanel.svelte # Mission sidebar (WP list, FC/EEPROM/file controls)
│   │   │   ├── WidgetPanel.svelte # Drag-and-drop widget panel container
│   │   │   ├── DebugPanel.svelte # MSP debug monitor (dev builds only)
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
│   │   ├── utils/                # Utility functions
│   │   │   └── geo.ts            # Haversine distance, bearing, formatting
│   │   └── index.ts              # Library entry point
│   └── app.html                  # HTML entry point
│
├── src-tauri/                    # Rust Backend (Tauri)
│   ├── src/
│   │   ├── main.rs               # Application entry point
│   │   ├── lib.rs                # Tauri app builder and plugin registration
│   │   ├── state.rs              # AppState (serial connection + FC info)
│   │   ├── commands/             # Tauri IPC commands (frontend-callable)
│   │   │   ├── mod.rs            # Command module registry
│   │   │   ├── connection.rs     # Serial connect/disconnect + MSP handshake
│   │   │   ├── mission.rs        # Mission CRUD, FC transfer, XML/file I/O (13 commands)
│   │   │   └── info.rs           # App version and metadata
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
│   │   │   └── features.rs       # Version-dependent feature gating
│   │   └── transport/            # Communication transports
│   │       ├── mod.rs            # Transport abstractions
│   │       └── serial.rs         # Serial port transport (serialport crate)
│   ├── .cargo/config.toml        # Cargo config (target-dir override)
│   ├── Cargo.toml                # Rust dependencies
│   ├── Cargo.lock                # Dependency lock file
│   └── tauri.conf.json           # Tauri configuration
│
├── scripts/                      # Build and development scripts
│   ├── build-windows.bat         # Windows release build
│   ├── build-linux.sh            # Linux release build
│   ├── dev.bat                   # Windows dev server
│   └── dev.sh                    # Linux dev server
│
├── docs/                         # Development documentation
│   ├── DEVLOG.md                 # This file — project structure & dev notes
│   ├── CHANGELOG.md              # Version changelog (Keep a Changelog format)
│   ├── ARCHITECTURE.md           # Architecture Decision Records (ADRs)
│   └── ROADMAP.md                # Feature roadmap by milestone
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
- **Frontend (Svelte)**: State lives in `src/lib/stores/`, UI components in `src/lib/components/`, pages in `src/routes/`.
- **Adding a new feature**: Create the Rust module → Add commands → Register in `lib.rs` → Create Svelte store → Create UI component → Wire into page.

## Development Setup

### Prerequisites
- Node.js LTS (v24+)
- Rust (via rustup, v1.94+)
- Platform-specific: see build scripts for required system packages

### Quick Start
```bash
npm install              # Install frontend dependencies
npm run tauri dev        # Start development mode with hot-reload
```

### Building
```bash
npm run tauri build      # Build release for current platform
```

### Platform Notes

- **Cargo target-dir**: Set to `D:\cargo-target\inav-gcs` via `src-tauri/.cargo/config.toml` to avoid issues with OneDrive paths containing spaces.
- **Windows**: Requires Visual Studio Build Tools 2022 (MSVC linker). Node.js v24+ via winget (do NOT use NVM4W — causes PATH conflicts).
- **PATH quirks**: New terminal sessions may need PATH reload: `$env:Path = [System.Environment]::GetEnvironmentVariable("Path","Machine") + ";" + [System.Environment]::GetEnvironmentVariable("Path","User")`

## UI Architecture

The UI follows a **floating overlay** pattern — the map fills the entire viewport and all panels float on top:

- **Toolbar** (top): Logo, sensor status bar, serial port controls, connect button
- **Hamburger Menu** (top-left over map): Opens the navigation rail + floating panel
- **Navigation Rail**: Vertical icon buttons — UAV Info (✈), Settings (⚙), Mission (◎)
- **Floating Panel**: Semi-transparent, backdrop-blur, slides in from left with animation
- **HUD Widget Panels** (bottom + right): Drag-and-drop widget layout, viewport-relative sizing (vmin)
- **Raw Telemetry Panel** (right side): Compact numeric readouts — implemented as a widget in the right panel
- **Status Bar** (bottom): Connection status, arming state, app title

All overlay elements use glassmorphism styling (backdrop-blur, semi-transparent backgrounds) with the INAV Configurator color scheme (#37a8db accent, #2e2e2e panels).

Widget sizes use `vmin` units exclusively (no fixed pixels) to scale with viewport — this ensures consistent sizing on desktop and mobile in landscape mode.

See [ARCHITECTURE.md](ARCHITECTURE.md) ADR-005 for the full rationale.

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
Sequence: `MSP_API_VERSION` → `MSP_FC_VARIANT` (must be "INAV") → `MSP_FC_VERSION` (must be ≥ 7.0) → `MSP_BOARD_INFO` → `MSP2_INAV_MIXER` (platform type, mixer preset) → feature gate computation

## Session Persistence

Settings stored in `localStorage` under key `inav-gcs-settings`:
- `lastPort` / `lastBaud` — last used serial connection
- `map.center` / `map.zoom` — map viewport state
- `mapProvider` / `mapCacheMaxMB` — tile provider + cache size
- `navPanelOpen` / `activeTab` — floating panel state
- `attitudeRateHz` / `positionRateHz` / `airspeedEnabled` — telemetry poll config
- `defaultWpAltitudeM` / `defaultPhTimeSec` — mission control defaults
- `widgetAhi` / `widgetSpeed` / `widgetAltitude` / `widgetBattery` / `widgetGps` / `widgetCompass` / `widgetHome` — per-widget visibility toggles
- `panels` — widget panel layout: `{ bottom: string[], right: string[], positions?: Record<string, 'bottom' | 'right'> }`

Implemented via custom Svelte store with auto-save on every mutation. Schema evolution handled by merging defaults: `{ ...defaults, ...stored }`.

## HUD Widget Panel System

The HUD uses a **two-panel drag-and-drop layout**:

- **Bottom Panel**: Horizontal strip above the status bar, centered. Reserved corner (22.5vmin) at bottom-right for future controls.
- **Right Panel**: Vertical strip on the right edge, centered vertically.

### Widget Classes
- **Large** (22.5vmin): AHI, Compass — circular, complex visualizations
- **Small** (13.5vmin = 60% of large): All others — square, compact data display

### Dynamic Sizing
Panels compute available space from window dimensions minus reserved areas. Widgets render at base size and only shrink (down to 50% minimum) when the total exceeds available space. Window resize is tracked reactively.

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

## Testing

- **37 Rust unit tests** covering MSP codec, parser, feature gates, telemetry decoders, and mission module
- Run: `cd src-tauri && cargo test --target-dir "D:\cargo-target\inav-gcs"`
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
