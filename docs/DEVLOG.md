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
| Map Library | Leaflet | Interactive maps for GCS and mission planning |
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
│   │   │   └── settings.ts      # Session persistence (localStorage)
│   │   ├── components/           # Reusable UI components
│   │   │   └── Map.svelte        # Leaflet map with position persistence
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
│   │   │   └── info.rs           # App version and metadata
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
- **Telemetry Strip** (bottom center): Horizontal widget bar overlay (ALT, SPD, DIST, BAT, SATS)
- **Status Bar** (bottom): Connection status, app title

All overlay elements use glassmorphism styling (backdrop-blur, semi-transparent backgrounds) with the INAV Configurator color scheme (#37a8db accent, #2e2e2e panels).

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
Sequence: `MSP_API_VERSION` → `MSP_FC_VARIANT` (must be "INAV") → `MSP_FC_VERSION` (must be ≥ 7.0) → `MSP_BOARD_INFO` → feature gate computation

## Session Persistence

Settings stored in `localStorage` under key `inav-gcs-settings`:
- `lastPort` / `lastBaud` — last used serial connection
- `map.center` / `map.zoom` — map viewport state
- `navPanelOpen` / `activeTab` — floating panel state

Implemented via custom Svelte store with auto-save on every mutation. Schema evolution handled by merging defaults: `{ ...defaults, ...stored }`.

## Testing

- **16 Rust unit tests** covering MSP codec, parser, and feature gates
- Run: `cd src-tauri && cargo test --target-dir "D:\cargo-target\inav-gcs"`
- Frontend type-check: `npx svelte-check --threshold error`
