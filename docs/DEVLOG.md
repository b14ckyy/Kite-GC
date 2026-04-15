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
│   │   └── +page.svelte          # Main application page
│   ├── lib/                      # Shared frontend modules
│   │   ├── stores/               # Svelte reactive state stores
│   │   │   ├── connection.ts     # Connection state management
│   │   │   └── telemetry.ts      # Telemetry data store
│   │   └── components/           # Reusable UI components (future)
│   └── app.html                  # HTML entry point
│
├── src-tauri/                    # Rust Backend (Tauri)
│   ├── src/
│   │   ├── main.rs               # Application entry point
│   │   ├── lib.rs                # Tauri app builder and plugin registration
│   │   ├── commands/             # Tauri IPC commands (frontend-callable)
│   │   │   ├── mod.rs            # Command module registry
│   │   │   ├── connection.rs     # Serial port listing, connect/disconnect
│   │   │   └── info.rs           # App version and metadata
│   │   ├── msp/                  # MSP Protocol implementation
│   │   │   ├── mod.rs            # MSP module exports
│   │   │   ├── types.rs          # Message types, constants, command codes
│   │   │   └── codec.rs          # MSP v1/v2 frame encode/decode
│   │   └── transport/            # Communication transports
│   │       ├── mod.rs            # Transport abstractions
│   │       └── serial.rs         # Serial port transport
│   ├── Cargo.toml                # Rust dependencies
│   └── tauri.conf.json           # Tauri configuration
│
├── scripts/                      # Build and development scripts
│   ├── build-windows.bat         # Windows release build
│   ├── build-linux.sh            # Linux release build
│   ├── dev.bat                   # Windows dev server
│   └── dev.sh                    # Linux dev server
│
├── docs/                         # Development documentation
│   ├── DEVLOG.md                 # This file
│   ├── CHANGELOG.md              # Version changelog
│   ├── ARCHITECTURE.md           # Architecture decisions
│   └── ROADMAP.md                # Feature roadmap and planning
│
├── static/                       # Static assets (icons, etc.)
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
