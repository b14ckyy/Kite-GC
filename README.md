# Kite Ground Control

A cross-platform Ground Control Station for [INAV](https://github.com/iNavFlight/inav)-based flight controllers.

Built with [Tauri 2.0](https://tauri.app/) (Rust backend) and [Svelte 5](https://svelte.dev/) (TypeScript frontend).

## Features (Planned)

- Real-time telemetry monitoring via MSP protocol
- Interactive 2D map with aircraft position (Leaflet)
- CesiumJS 3D globe view with terrain, chase camera, and flight replay
- Mission planning with waypoint editor
- Flight recording & logbook with Blackbox import
- Colored flight tracks (flight mode, altitude, speed, signal)
- Multi-protocol support (MSP, LTM, MAVLink, CRSF)
- Multi-transport support (Serial, BLE, TCP/UDP)
- Log replay (Blackbox, OTX/ETX) in 2D and 3D
- Cross-platform: Windows, Linux (x86/ARM), Android

## Quick Start

### Prerequisites
- [Node.js](https://nodejs.org/) LTS (v24+)
- [Rust](https://rustup.rs/) (v1.94+)
- Platform-specific dependencies (see [DEVLOG](docs/DEVLOG.md))

### Development (recommended)
We use **[just](https://github.com/casey/just)** as the primary task runner.

```bash
# One-time setup
npm install

# Start development (hot reload)
just dev
```

Alternative (still works):
```bash
npm run tauri dev
```

### Build (recommended)
```bash
just build
```

Platform-specific:
```bash
just build-windows
just build-linux
```

Alternative (still works):
```bash
npm run tauri build
# or
./scripts/build-windows.bat
./scripts/build-linux.sh
```

> **Tip**: Install `just` for the best developer experience (see `justfile` in the project root).

## Documentation

- [Development Log](docs/DEVLOG.md) — Project structure and setup
- [Architecture Decisions](docs/ARCHITECTURE.md) — Why we chose what
- [Data Pipeline](docs/DATA_PIPELINE.md) — Telemetry data flow (live + replay)
- [Roadmap](docs/ROADMAP.md) — Feature planning
- [Changelog](docs/CHANGELOG.md) — Version history
- [Flight Modes Protocol](docs/PROTOCOL_FLIGHT_MODES.md) — INAV bitmask reference
- [Build & Development Guide](docs/dev/BUILD.md) — Setup, just commands, troubleshooting, CI

**Build & Contribution**
- Use `just` (see `justfile` in root) for development and builds
- CI runs automatically on push/PR (cargo check + svelte-check)

## Recommended IDE Setup

[VS Code](https://code.visualstudio.com/) + [Svelte](https://marketplace.visualstudio.com/items?itemName=svelte.svelte-vscode) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer).

## License

GPL-3.0-only — See [LICENSE](LICENSE)
