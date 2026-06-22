# Kite Ground Control — Development Documentation

## Project Overview

Kite Ground Control is a cross-platform Ground Control Station for **INAV** (via MSP), **ArduPilot & PX4**
(via MAVLink) and **passive telemetry** (listen-only: SmartPort/CRSF/LTM/MAVLink). It provides mission
planning, real-time telemetry + flight logging/replay, GCS **vehicle control** and **RC injection**,
**foreign-vehicle radar**, an **airspace** overlay, and **terrain / RF-link** analysis.

**Long-term scope reference**: [MWPTools](https://stronnag.grebedoc.dev/mwptools/)

### Start here (new collaborators)

Read in this order to get the mental model fast:
1. **This file (DEVLOG)** — stack, project structure, module concept, and the per-subsystem foundations
   (incl. the *Subsystems beyond the telemetry pipeline* section near the end).
2. **[ARCHITECTURE.md](ARCHITECTURE.md)** — the **why**: every cross-cutting decision is an ADR
   (ADR-001…054), with context + consequences. Skim the titles; read the ones touching your area.
3. **[reference/DATA_PIPELINE.md](reference/DATA_PIPELINE.md)** — the **how**: end-to-end data flow
   (three live protocols → unified events → store → widgets/map, plus the parallel networks).
4. Per-feature detail lives in **`docs/active/`** (open plans), **`docs/reference/`** (living refs) and
   **`docs/archive/`** (shipped plans, kept for rationale). `CLAUDE.md` has the working rules.

## Technology Stack

| Component | Technology | Purpose |
|---|---|---|
| Application Framework | Tauri 2.0 | Cross-platform desktop + mobile shell |
| Backend | Rust | MSP protocol, serial/BLE communication, state management |
| Frontend | Svelte 5 + TypeScript | User interface, reactive data display |
| Map Library | Leaflet 1.9.4 | Interactive maps for GCS and mission planning |
| File Dialogs | @tauri-apps/plugin-dialog 2.7.0 | Native OS file picker (mission save/load) |
| Build Tool | Vite | Frontend bundling and dev server |
| License | GPL-3.0-or-later | Open source license |

## Target Platforms

| Platform | Status | Notes |
|---|---|---|
| Windows (x64) | Active Development | Primary development platform |
| Linux (x86_64) | Active Development | Tested in snapshots — co-developed on a Linux machine, verified on backend changes + general functionality |
| Linux (ARM64) | Planned | Raspberry Pi, etc. |
| Android | Future | Separate Android UI — scope/timeline TBD, **not** targeted for 1.0; via Tauri mobile |
| macOS | Future | Needs test hardware |
| iOS | Future | Needs test hardware |

## Project Structure

> Folder-level overview — per-file detail is intentionally omitted here so it does not rot.
> See the module-concept notes below and the per-feature docs in `active/` for specifics.

```
Kite Ground Control/
├── src/                              # Svelte 5 / SvelteKit frontend
│   ├── routes/+page.svelte           # Thin orchestrator (ADR-009): wires stores/controllers + map + widgets
│   └── lib/
│       ├── stores/                   # Reactive state (connection, telemetry, mission, settings, flightlog, video, …)
│       ├── controllers/              # Domain logic extracted from +page (connection, logbook, playback, widget)
│       ├── adapters/                 # DB record → widget data (telemetryAdapter)
│       ├── helpers/                  # Pure utils (trackColors, surveyPatterns, missionIcons/Geometry, missionLibrary, …)
│       ├── components/
│       │   ├── panel/                # Reusable panel framework: PanelShell + Button/Toggle/SegmentedToggle (ADR-029)
│       │   ├── logbook/              # LogbookPanel, FlightDetail, BatteryManager, LogPlayer, WeatherEditor
│       │   ├── mission/              # INAV/Ardu mission panels + layers, MissionManager, survey pattern UI, AutopilotSelect
│       │   ├── terrain/              # Terrain analysis panel + cursor layer
│       │   ├── video/               # Video panel + floating window
│       │   ├── widgets/              # HUD widgets (AHI, Compass, Speed, Alt, Battery, GPS, Home, Terrain, …)
│       │   └── Map.svelte · Map3D.svelte · UavInfoPanel · SettingsPanel · Toolbar · StatusBar · NavRail · dialogs …
│       ├── cache/                    # IndexedDB tile cache (+ CachedTileLayer)
│       ├── config/                   # mapProviders, widgetRegistry
│       ├── i18n/                     # svelte-i18n setup + locales/{en,de,fr}.json
│       └── utils/                    # geo, units
│
├── src-tauri/src/                    # Rust backend (Tauri 2)
│   ├── lib.rs · main.rs · state.rs   # App builder + plugin registration + AppState (ActiveProtocol: MSP/MAVLink/PassiveTelemetry)
│   ├── commands/                     # Tauri IPC (connection, flightlog, mission, info, control, rc, radar, aero, hid, terrain)
│   ├── flightlog/                    # Recording + logbook + SQLite (schema v13) + blackbox/ardupilot/raw import + exchange/exports
│   ├── mission/                      # INAV mission model + MSP_WP codec + store (MAVLink missions live in mavlink_proto)
│   ├── scheduler/                    # MSP scheduler (dedicated thread) + telemetry decode + RC-injection state (rc_tx) + dev debug
│   ├── msp/                          # MSP v1/v2 codec, parser, transport framing, feature gating, RC encoders
│   ├── mavlink_proto/               # MAVLink parser/codec/handshake/handler + mission microprotocol + control + RC stream
│   ├── passive_telemetry/           # Listen-only telemetry: detect + decode SmartPort/CRSF/LTM/MAVLink (+ MSP probe)
│   ├── telemetry_forward/           # Telemetry Relay: re-encode live telemetry → LTM/MAVLink/CRSF/SmartPort out (ADR-051)
│   ├── radar/                        # Foreign-vehicle tracking: ADS-B + FormationFlight sources → radar-vehicles
│   ├── aero/                         # Airspace Manager aeronautical data (OpenAIP, ADR-038)
│   ├── flightmode/                   # Protocol-agnostic flight-mode classification (ADR-044)
│   ├── hid/                          # Native HID/gamepad backend for RC control (WGI / evdev)
│   ├── terrain/                      # Copernicus DEM elevation provider (fetch/decode/cache/sample)
│   └── transport/                    # ByteTransport trait + serial / tcp / udp / ble
│
├── docs/                             # Core dev docs: ARCHITECTURE (ADRs) · ROADMAP · CHANGELOG · DEVLOG · BUILD
│   ├── active/                       # Active feature plans (open work)
│   ├── reference/                    # Living reference docs (DATA_PIPELINE, FLIGHTLOG_DATABASE, PROTOCOL_FLIGHT_MODES)
│   ├── future/                       # Exploratory, not-planned notes
│   └── archive/                      # Completed feature plans (kept for design rationale)
│
├── justfile · scripts/              # Task runner (just dev/build/check) + legacy build scripts
├── .github/workflows/ci.yml          # CI (cargo check + svelte-check)
└── package.json · README.md · LICENSE (GPL-3.0) · static/
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
- Platform-specific: see [Build Guide](docs/BUILD.md) for details

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

For the complete guide (troubleshooting, CI, common Windows issues, etc.), see **[docs/BUILD.md](../dev/BUILD.md)**.

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

- Detailed manual test checklist for M5 is in `docs/archive/M5_TEST_CHECKLIST.md`.
- Backend DB tests are in `src-tauri/src/flightlog/db.rs` (`cargo test flightlog --lib`).

## HUD Widget Panel System

The HUD uses a **two-panel drag-and-drop layout** within the CSS Grid zone system:

- **Bottom Dock**: Horizontal strip (grid row 3, col 2–3), height `clamp(184px, 20vh, 300px)`. Edit button + centered widget strip.
- **Side Dock**: Vertical strip (grid row 2, col 3–4), width `clamp(150px, 15vw, 250px)`.

### Widget Classes
- **Large** (22.5 units): AHI, Compass, Terrain Radar — circular / square complex visualizations
- **Small** (13.5 units = 60% of large): most others — square, compact data display
- **Wide** (2×1 = two large units wide, one tall): Live AGL — a horizontal forward-looking terrain HUD. In the bottom dock it renders `sizePx × 2` wide; in the side dock it falls back to a half-height landscape tile

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

Blackbox logs are decoded using the official `blackbox_decode` binary from [iNavFlight/blackbox-tools](https://github.com/iNavFlight/blackbox-tools) (GPL-3.0). The binary is kept **external** (next to the app, **not** compiled in) on purpose, so it can be updated independently when a new INAV version changes the log format.

**Binary discovery** (`flightlog/blackbox.rs::find_decoder`, in order):
1. Application folder (next to executable)
2. Auto-download install dir (`<AppData>/kite-gc/bin`, portable → `data/bin`)
3. System PATH fallback

**On-demand download** (`flightlog/decoder.rs`, ADR-008-style external dep): if the decoder is missing when an INAV-blackbox import starts, the user is offered a one-click download from the latest GitHub release. Windows pulls the `.zip` (extracted via the `zip` crate); Linux/macOS pull the `.tar.zst` (via `zstd` + `tar`), with the executable bit set. Arch matching uses aliases because the release names the Linux x64 build `x64_64` (not `x86_64`). Android isn't supported (no blackbox import there).

**Version visibility** (`blackbox_decoder_version` → `blackbox_decode --version`, e.g. `9.0.0 INAV 1918a75`): shown read-only in Settings → Flight Logbook with a Download/Update button — works for an externally-placed binary too (the tool carries no Windows file-version resource), so the user can tell whether it needs updating for a new INAV version.

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
| `heading` | `gps_ground_course` / `gps_cog` / `course` | course over ground, **degrees** (kept as-is; f64) |
| `vario_ms` | `gps_velned[2]` → `vario` | NED down cm/s: negated and ÷100 for climb m/s |
| `voltage` | `vbat` | |
| `current_a` | `amperage` | |
| `mah_drawn` | `mahdrawn` | |
| `rssi` | `rssi` | |
| `roll` | `roll` / `attitude[0]` / `attitude_roll` | **always ÷10** (INAV decidegrees → degrees) |
| `pitch` | `pitch` / `attitude[1]` / `attitude_pitch` | **always ÷10** (INAV decidegrees → degrees) |
| `yaw` | `heading` / `attitude[2]` / `attitude_yaw` | FC fused heading, **always ÷10** (INAV decidegrees → degrees; f64) |
| `num_sat` | `GPS_numSat` | |
| `link_quality` | `lq` / `link_quality` / `rxlq` | ELRS/CRSF only; `None` if column absent |

### DB Schema

Schema evolves via `PRAGMA user_version` + a sequential migration chain (`flightlog/db.rs`,
`migrate_vN_to_vN+1`) — **earlier migrations are never modified**. **Current version: v13.** Highlights of
the chain: v2 (blackbox tables + `flights.source`), v3 (link_quality), v4 (replay telemetry fields), v5
(craft_name), v10 (battery DB + serial soft-link), v11 (live-recording temp-store columns), v12
(canonical `mode_primary` / `mode_modifiers` for the unified flight mode), v13.

The full, current table/column layout is the canonical **reference**, not duplicated here — see
[reference/FLIGHTLOG_DATABASE.md](reference/FLIGHTLOG_DATABASE.md). `flights.source`: `'live' | 'blackbox'
| 'both'`; `blackbox_records` stores raw comma-joined CSV rows; `blackbox_files` archives the original
`.TXT` BLOB.

## Survey Pattern Generator

Frontend-only feature (no Rust): users define a geometric area on the map and generate mission waypoints from it. See [ARCHITECTURE.md](ARCHITECTURE.md) ADR-024 (architecture) and ADR-025 (algorithms) for the rationale.

### Files

| File | Role |
|---|---|
| `stores/surveyPattern.svelte.ts` | Rune store: `activeSurveyPattern { config, isActive }`, `switchShape()`, per-family param cache, `update*Params` / `apply*DragUpdate` |
| `helpers/surveyPatterns.ts` | Pure geometry + all six generators (no Svelte, fully unit-testable) |
| `components/mission/SurveyPatternPanel.svelte` | Parameter UI per shape, generate flow, live WP count |
| `components/mission/SurveyPatternLayer.svelte` | Leaflet preview: shape, path, editing markers, drag handling |
| `components/NumberStepper.svelte` | Reusable +/- numeric input used throughout the panel |

### Shape families

Three families, each with its own params type, panel `$state`, and generators — no cross-family code reuse (a bug in one shape can't corrupt another):

- **rect** (`rectangle`, `rectangle-lawnmower`) → `RectanglePatternParams`
- **circle** (`circle`, `spiral`) → `CirclePatternParams` (adds `radius`, `ringPoints`)
- **polygon** (`polygon`, `polygon-lawnmower`) → `PolygonPatternParams` (adds `points`, `stayInsideArea`)

`switchShape()` caches the current family's params and restores the target's (or builds defaults); same-family switches just rename `shape`. The cache lives for the session (reset on app close).

### Generators (`surveyPatterns.ts`)

All geometry runs in a local equirectangular metre frame around the centroid, converting back to lat/lng at the end.

- **`generateRectangleZigzag` / `generateClassicZigzag`** — boustrophedon; track-orientation mode rotates+clips tracks to the shape, otherwise tracks follow shape orientation.
- **`generateRectangleLawnmower`** — concentric rectangles, diagonal layer transitions.
- **`generateCircleStepped`** — concentric rings, `ringPoints`/ring (auto-reduced), centre-point finish.
- **`generateSpiral`** — Archimedean; fixed angular step outer, arc-clamped inner; stops at UAV-turn > 60° or sub-spacing arc; centre finish.
- **`generatePolygonZigzag`** — perpendicular scanline, even-odd pairing; `stayInsideArea` toggles cross-gap serpentine vs. connected-fill DFS; turn-distance only before real (next-line) turns.
- **`generatePolygonLawnmower`** — convex decomposition (`decomposeConvexXY`) → Hertel-Mehlhorn merge (`mergeConvexPiecesXY`) → robust half-plane inward offset (`offsetConvexInwardXY`) → per-zone concentric rings + spine (`spineOfConvexXY`), short edges pruned (`removeShortEdgesXY`).

### Path model

Generators return `SurveyPathSegment[]`; `kind: 'survey'` points become waypoints, `kind: 'turn'` are visual-only connectors. Rings are flown open with diagonal inward steps (one vertex past nearest) to avoid re-flown points. User-action flags are applied in final flight order (after `reverse`). Generation checks the INAV 120 WP limit with a live count in the panel.

### Interaction notes

- Polygon editing: independently draggable corners, midpoint-click insertion (max 50 verts), centroid drag moves the whole shape, right-click / drag-to-delete-zone removes a vertex (min 3). Self-intersection is rejected on drop (vertex reverts); live preview pauses while invalid.
- `Map.svelte` renders `<SurveyPatternLayer>` unconditionally (it self-clears when inactive); `InavMissionLayer` blocks map-click WP placement while pattern mode is active.

## Terrain Elevation & AGL Waypoints

Local terrain elevation for mission planning. See [ARCHITECTURE.md](ARCHITECTURE.md) ADR-026 for the rationale; [TerrainFeatures.md](dev/TerrainFeatures.md) for the feature plan.

### Elevation provider (`src-tauri/src/terrain/`)

- **Source**: Copernicus DEM GLO-30 (AWS Open Data `copernicus-dem-30m`, Cloud Optimized GeoTIFF, 1°×1° tiles, Float32, EGM2008 geoid ≈ MSL, no API key).
- **Flow**: `tile_name(lat,lon)` → HTTPS fetch → disk cache (portable-aware, `<data>/terrain/`) → `tiff`-crate decode (Float32, DEFLATE, floating-point predictor) → in-memory LRU (4 tiles) → bilinear sample. Geo-transform read from `ModelPixelScale`/`ModelTiepoint` tags.
- **Runtime**: CPU decode + 42 MB disk I/O run on `spawn_blocking` (async workers never stalled); tile loads serialized + cache-rechecked via an async lock so concurrent requests coalesce.
- **Commands**: `terrain_elevation(lat,lon) -> Option<f32>`, `terrain_profile(points, spacing_m) -> [{dist_m, lat, lon, elev_m}]`.
- **Why geoid (≈MSL)**: GPS altitude, INAV AMSL waypoints, and GLO-30 are all ≈MSL → directly comparable, no geoid-undulation conversion (unlike Cesium's ellipsoid terrain).
- **Follow-up**: COG partial reads (HTTP range requests + chunk decode) for weak-hardware latency — see TerrainFeatures.md.

### AGL waypoints

- INAV waypoints only encode REL (p3 bit0=0) or AMSL (p3 bit0=1) — there is **no AGL flag**. AGL is a GCS-only authoring mode.
- `Waypoint.alt_mode` (0=REL, 1=AMSL, 2=AGL) added to the backend model; for REL/AMSL it mirrors p3 bit0, decoded from p3 on MSP/XML load.
- **Editor**: the altitude toggle cycles REL→AMSL→AGL and converts the value via terrain + the launch point (so the physical height is preserved): value → absolute MSL → target mode.
- **Survey patterns**: the `ground` altitude option = AGL; generated waypoints carry `alt_mode=AGL`.
- **Export**: `resolve_agl()` converts AGL waypoints → AMSL (`AMSL = terrain(lat,lon) + AGL`, p3 bit0=1) in `mission_save_file` / `mission_export_xml` / `mission_upload` (async, before serialization/upload). Not round-trippable — a loaded/​downloaded mission returns as AMSL.

### Launch / home reference

- `launchPoint` store (`mission.ts`); auto-placed on entering edit mode (FC home → first geo-WP → map center), always-visible draggable map marker, orange dashed connector to the first waypoint.
- Persisted in `.mission` via the mwp-compatible `<mwp home-x="lon" home-y="lat">` meta element (`Mission.home`): written on save/export, parsed on load/import (overrides the current launch point). Other tools (INAV Configurator) ignore the element and read only `<missionitem>`.

### Terrain Analysis panel (elevation profile)

Full-width NavRail overlay (`TerrainAnalysisPanel.svelte`) — a side-view profile of the mission/track vs terrain. No external runtime dependency: hand-rolled **SVG** chart (`TerrainProfileChart.svelte`); data built in `helpers/terrainProfile.ts` from `terrain_profile` + per-WP MSL resolution. Session state in `stores/terrainAnalysis.ts` (in-memory, survives close/reopen; not persisted to disk).

- **Two view modes**: *Waypoint* (planned mission, WP altitudes → absolute MSL via terrain + launch point) and *Track* (flown live temp-log or loaded blackbox — source is whichever track is on the map; mutually exclusive so no selector). Profiles are cached per mode by signature → instant Waypoints↔Track switching.
- **Chart**: terrain fill + flight/track line (MSL), waypoint markers, dashed clearance floor (`terrain + Ground Clearance`), red coloring where clearance < floor. Hover crosshair with readouts (distance / terrain / altitude / clearance). **MSL ↔ AGL** datum toggle (AGL view = clearance curve on a 0 baseline).
- **Zoom/pan**: wheel zooms the X-axis, drag pans, double-click resets (explicit scales, no SVG `viewBox`). **Rendering scales with zoom** — only the visible distance slice is drawn, decimated to ~screen resolution (per-bucket worst-clearance / peak-terrain envelope so peaks + unsafe spots survive); full-res data still drives readouts.
- **Min-clearance trimming**: leading/trailing runs below clearance (take-off climb-out / landing descent on the ground) are ignored — only the en-route portion alerts; a mid-route dip still alerts.
- **Climb angle**: waypoint vertices used as-is; flown tracks low-pass the altitude (~10-sample window, measured per ≥20 m segment) to reject sensor jitter that otherwise spikes slopes toward 90°.
- **Void bridging**: interior null terrain samples (tile-edge / nodata) are linearly interpolated so the line/clearance stay continuous; genuine out-of-coverage at the route ends stays null.
- **Compact mode** (*Show Map* toggle): collapses to a short top-docked strip (animated, like the panel transitions), stopping short of the side widget dock. The chart cursor is mirrored onto the 2D map via `TerrainCursorLayer` — a transient hover dot plus a click-pinned persistent marker (click again to clear). The pin is visual-only and lives in the `terrainCursor` store, so it stays on the map after the panel closes (reference while editing in mission control); it's also mirrored back into the chart as a vertical pin line.
- **Datum advantage**: terrain plotted in MSL (Copernicus EGM2008), consistent with FC GPS MSL + AMSL waypoints — unlike INAV Configurator's WGS84/ellipsoid terrain labeling.

### Terrain Correction (Phase 2)

Pure-function engine (`helpers/terrainCorrection.ts`) over the same `ProfileData` — no new backend calls. Two modes, applied to a WP range (display numbers, default first/last); Land/RTH/Jump/SetHead and out-of-range WPs are **fixed anchors**.

- **Terrain Follow**: set correctable WPs (Waypoint + PosHold) to `ground + Ground Clearance`, then lift legs. **Clearance Check**: raise-only from the original altitudes.
- **Convergence loop** (monotonic raises): WP clearance → leg deficit (raise both endpoints by the max deficit; one anchor → raise only the correctable one to the exact requirement) → optional fixed-wing **climb/descent-angle** pass (raise the *lower* endpoint of any too-steep leg; 2 params, 0 = off). Bounded → converges; iteration cap as a safety net.
- **No auto-insert** (it added too many, unreliably). Instead a **manual *Add WP***: pin a marker on the chart → inserts a waypoint at that lat/lon on the current track (interpolated AMSL), respecting the WP limit; re-run Follow.
- **Clearance warning at 95%** of the target (5% grace) for the readout *and* the red colouring; the dashed floor stays at 100%.
- **Live green preview** (drawn *behind* the path so it never hides it), recomputed as params change; y-scaling includes the preview so a raised line can't clip. **APPLY** updates changed WPs in place (→ **AGL** mode) behind a confirm dialog.

### Jump-loop simulation

`expandRoute()` (in `terrainProfile.ts`) simulates **one** loop per jump (`4J2` → branch `4→2`, cut, resume `4→5`; repeat count ignored), with no duplicate WP dots. Each continuous segment is terrain-sampled separately and stitched with a gap; the cut is a `cut` terrain sample that breaks terrain/path/clearance/preview + a dashed marker. The jump-back leg is coloured like the map (`#b56be0`) and ends in a `↩N` target marker; the resume point shows its WP dot. **Correction stays correct**: the engine keys altitude **per WP index** (one `Cell` shared by all revisits), so the jump-back leg constrains the same WP as its first-pass legs; cut legs are skipped. Jump target resolves as `p1 − 1` (absolute WP index, matching the map layer).

### Live Track mode (Phase 3) — data flow for later debugging

Track mode follows the **live flown track** when an FC is connected. Key pieces (untested in the field at time of writing — documenting the flow so issues are traceable):

- **`stores/liveTrack.ts`** — shared `liveTrack` writable `{lat, lon, alt_m, timestamp_ms}[]`. `appendLivePoint()` mutates the array in place (O(1) append; `update` still notifies) with a 5 m move filter; `clearLiveTrack()` resets. `alt_m` = telemetry **`altMsl`** (GPS MSL, matches blackbox `alt_m`).
- **Accumulator** — in `+page` inside the existing `telemetry.subscribe`. Tracks `prevArmed`; armed = `isArmed(t.armingFlags, t.lastUpdate)`. On **disarmed→armed**: `clearLiveTrack()` + fire-and-forget `terrain_elevation(lat,lon)` to warm the Copernicus tile. While **armed** + valid GPS: `appendLivePoint`. So the track exists from arm onward regardless of whether the panel is open (and independent of the map trail, which is lat/lon only, and of the flight-log DB).
- **`LiveTrackProfiler`** (class in `terrainProfile.ts`) — incremental. Holds accumulated `terrain[]`/`path[]` + `processed` index + `lastLat/lon/dist`. `update(track)`: if `track.length < processed` → `reset()` (new arm); slice the new points, sample terrain only for `[lastPoint, …new]` (prepended last point for continuity; skip its duplicate sample unless terrain is empty), append; recompute the cheap folding via `finishProfile` over the whole accumulation. **Only new points hit the backend.**
- **Panel** — `liveActive = live && viewMode==='track' && open` (a `$derived` boolean; the polling `$effect` only re-runs when it flips, *not* on every param change, so the 5 s interval and accumulation survive clearance/datum tweaks). The main build-effect early-returns for `live && track`. Poll = 5 s `setInterval` calling `profiler.update(get(liveTrack))`.
- **Follow** (`terrainAnalysis.follow`, header toggle, live only) + view model: default window **250 m** (`LIVE_MIN_WINDOW`; ≥ the 30 m terrain resolution). `pinFollow()` keeps the window pinned to the right edge (`viewEnd = max(window, total)` so it builds up left→right before scrolling); `null/null` viewStart/End = full-zoom-out auto-fit (grows on its own, regardless of Follow). Chart: `live` ⇒ min zoom window 250 m; `follow` ⇒ wheel pins the right edge + drag-pan disabled.
- **Edge/known**: ~5 s latency until the first track appears after arming (poll interval); after **disconnect** the panel falls back to the loaded blackbox `track` prop (the `liveTrack` store still holds the data). `live` = *connected* (not armed), so disarm-while-connected leaves the track for review with Follow toggle-able.

## MAVLink telemetry — stream rates, Debug Monitor, mode + home (ArduPilot SITL validation)

Bringing the MAVLink (ArduPilot) path to parity with MSP — data flow for later debugging. Tested against ArduPilot SITL via Mission Planner (SITL listens on TCP 5760/5762/5763; MP holds 5760, Kite connects to **5762**; UDP forwarding via MP is unreliable — the SerialOutput grid throws `NullReferenceException`).

- **Stream rates** (`mavlink_proto/streamrates.rs`, ADR-043). ArduPilot pushes per its `SRn_*` params once it sees a GCS heartbeat — no request needed (the Debug Monitor shows ~30 msg types streaming on a fresh connect). To get MSP-parity + fit slow links we send `MAV_CMD_SET_MESSAGE_INTERVAL` (511) **once on connect, before the handler thread starts** (we still own the byte transport): wanted msgs at the two settings (ATTITUDE=attitude_rate; GPS_RAW_INT/GLOBAL_POSITION_INT/VFR_HUD=position_rate; SYS_STATUS/BATTERY/NAV_CONTROLLER/MISSION_CURRENT=1 Hz; HOME=0.2 Hz), ballast at `-1`. `interval` is **µs**; `-1`=disable, `0`=reset to SRn default. **Sticky on the FC channel until reboot** → "Full" mode is NOT "send nothing", it sends `0` for every managed message to undo a prior reduction. One shared `WANTED`+`BALLAST_IDS` list drives both `apply_stream_rates` and `reset_stream_rates`. `SET_MESSAGE_INTERVAL` is scoped FC-side to the **channel it arrives on**, so we never need to know which FC port (Telem1/2/USB) our link uses. Burst note: ArduPilot has no spreading scheduler — it fires all due messages per loop, so "all 1 Hz" peaks once/sec (low average, spiky peak).
- **Debug Monitor MAVLink tab** (`mavlink_proto/debug.rs`, `#[cfg(debug_assertions)]` like the MSP one). Keyed by `(message_id, is_tx)` — HEARTBEAT shows as both RX (FC) and TX (our GCS HB). `on_rx`/`on_tx` hooks in the handler loop; `maybe_emit` ~60 Hz → `debug-mavlink-stats`. Rate decays to 0 after >2 s stale; names via a small lookup table (rest `MSG_<id>`).
- **Flight mode** — two bugs: handshake set a generic `fc_variant="ArduPilot"` (frontend `/Plane/i` test → wrong table) and the handler mangled raw `custom_mode` into INAV box-flag bits. Fix: vehicle-specific variant from `mav_type` + **forward raw `custom_mode`** ([handler.rs] HEARTBEAT arm). Frontend `classifyArduPilotMode` already keys per-vehicle on the raw number. **Only the MAVLink adapter changed** — MSP/unified untouched.
- **Home** — handler now processes `HOME_POSITION` (242) → emits the protocol-agnostic `home-position` event (adapter-local `HomeEvent` struct; lat/lon degE7→deg, alt mm→m). On connect the frontend GPS-fix fallback shows briefly until the first 0.2 Hz push corrects it (accepted). Future: telemetry-only adapters (CRSF/Smartport) have no home msg → derive from the arm-edge GPS fix **in the adapter** and emit the same event (see ADR-039).

## Subsystems beyond the telemetry pipeline (foundations + pointers)

The sections above predate several subsystems. Rather than duplicate their design here, this maps the
**foundations** and points at the canonical doc for each. The end-to-end data flow (the three inbound
protocols + these parallel networks) is the living reference in
[reference/DATA_PIPELINE.md](reference/DATA_PIPELINE.md).

- **Three live protocols (ADR-010).** MSP (poll), MAVLink (push), **Passive telemetry** (listen-only:
  SmartPort/CRSF/LTM/MAVLink, sub-protocol auto-detected) — `ActiveProtocol::{Msp, Mavlink,
  PassiveTelemetry}`. All decode to the **same** payload structs + `telemetry-*` events; the frontend never
  branches on protocol. Passive path (listen-only, auto-detected) = **ADR-053**, `passive_telemetry/`
  (archived plan `archive/RADIO_TELEMETRY.md`).
- **Unified flight mode (ADR-044, `flightmode/`).** Each input adapter classifies raw mode data into a
  canonical `{ primary, modifiers[] }`; widget/track/recording consume only that. New protocol = new
  adapter + a few registry ids.
- **RC control — outbound uplink (ADR-054)** (`scheduler/rc_tx.rs`, `hid/`, `msp/rc_encode.rs`,
  `mavlink_proto/handler.rs`). HID → `rcEngine`/`rcManual` → `rc_stream_*` commands → shared `RcTxState`
  (independent of `protocol`) → streamed to the FC: MSP `SET_RAW_RC` + `SET_AUX_RC` (INAV), or
  `RC_CHANNELS_OVERRIDE` (ArduPilot) / `MANUAL_CONTROL` (PX4). Engage seeds from the FC's current
  channels; a frontend heartbeat drives a backend deadman. Docs: `archive/MSP_RC_CONTROL.md` +
  `active/MAVLINK_RC_CONTROL.md`.
- **Radar (`radar/`).** Independent foreign-vehicle tracking — ADS-B (online / serial-MAVLink /
  MSP-from-UAV) + FormationFlight (ESP32 mesh). Aggregator thread → `radar-vehicles` / `radar-adsb-status`
  events. Alerts ADR-035, FormationFlight ADR-036; core plan `archive/RADAR_TRACKING_CORE.md`.
- **Live recording — temp store (ADR-040/041/042, `flightlog/recorder.rs`).** Per-session `.ktmp` SQLite
  on arm; on disarm/disconnect the session becomes **pending** in app-state and is committed only via the
  End-Flight dialog or a re-arm grace; orphan recovery on reconnect. Fed by all three protocols.
- **Telemetry Relay (ADR-051, `telemetry_forward/`).** Taps live decoded telemetry, re-encodes to
  LTM/MAVLink/CRSF/SmartPort, sends out Serial/BLE/TCP/UDP. Plan `archive/TELEMETRY_FORWARDING.md`.
- **Vehicle Control (ADR-052, `commands/control.rs`).** MAVLink command panel (mode/arm/takeoff/RTL/
  guided/…) via `COMMAND_LONG`/`COMMAND_INT` + `COMMAND_ACK` correlation. Plan `active/VEHICLE_CONTROL.md`.
- **Airspace Manager (ADR-038, `aero/`).** On-demand OpenAIP aeronautical overlay (airspaces/obstacles/
  airports) — separate from the live telemetry path. Plan `active/AIRSPACE_MANAGER.md`.
- **Safe Home / autoland (INAV, in progress; `commands/safehome.rs`, `stores/safehome.ts`).** On INAV
  connect, downloads the 8 safehome slots (+ radius settings) always, and the fixed-wing autoland
  approaches + `nav_fw_land_*` on ≥7.1, via `MSP2_INAV_SAFEHOME` / `MSP2_INAV_FW_APPROACH` +
  `read_setting` (shared `commands/fc_settings.rs`). The Safe Home Manager (swapped into the INAV mission
  slim panel via a house button) edits a working copy; "Save to FC" writes everything as one batch +
  `MSP_EEPROM_WRITE`. Gated by `FeatureSet.autoland_config` (≥7.1) / `autoland_validated` (≤9.1.x). Map
  overlay (2D + 3D markers/rings/approach geometry) is the next phase. Plan `active/AUTOLAND_SAFEHOME.md`.
- **Diagnostic file logging (ADR-055, `logging/`).** A custom `log::Log` logger installed before the Tauri
  builder writes a rotating TXT file (`<AppData>/kite-gc/kite-gc.log`, portable → `data/`; previous run →
  `*.prev`). Until this landed, **no logger was installed**, so every `log::` call in the codebase was a
  silent no-op — a failed connect left no trace. Level is user-set in Settings → Diagnostics
  (OFF/Error/Warning/Debug; default Warning) via `set_log_level`, applied at runtime through
  `log::set_max_level`. `eprintln!` still goes to the console; this captures the `log` facade. **Diagnosing
  a connection problem**: set level to Debug, reproduce, hand back the log. The in-app Debug Monitor
  (ADR-008) is the live counterpart for inspecting protocol traffic.
- **Runtime debug mode (ADR-056, `debug_mode.rs`).** A shipped **release** build started with `--debug`
  exposes the full Debug Monitor (incl. the MSP/MAVLink stat tabs) + raises the log to Debug. The stat
  trackers are now compiled into every build and gated at runtime on a single `AtomicBool` (default
  on in debug builds, off in release until `--debug`); the frontend's `DEV_MODE` is
  `import.meta.env.DEV || is_debug_mode`, keeping the `DebugPanel` chunk in the release bundle (lazy).
  Costs ~44 kB + one atomic load per tracker call when off. This **amends ADR-008** (compile-out → runtime
  gate).
