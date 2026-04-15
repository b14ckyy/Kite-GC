# INAV GCS

A cross-platform Ground Control Station for [INAV](https://github.com/iNavFlight/inav)-based flight controllers.

Built with [Tauri 2.0](https://tauri.app/) (Rust backend) and [Svelte 5](https://svelte.dev/) (TypeScript frontend).

## Features (Planned)

- Real-time telemetry monitoring via MSP protocol
- Interactive map with aircraft position (Leaflet)
- Mission planning with waypoint editor
- Multi-protocol support (MSP, LTM, MAVLink, CRSF)
- Multi-transport support (Serial, BLE, TCP/UDP)
- Log replay (Blackbox, OTX/ETX)
- Cross-platform: Windows, Linux (x86/ARM), Android

## Quick Start

### Prerequisites
- [Node.js](https://nodejs.org/) LTS (v24+)
- [Rust](https://rustup.rs/) (v1.94+)
- Platform-specific dependencies (see [DEVLOG](docs/DEVLOG.md))

### Development
```bash
npm install
npm run tauri dev
```

### Build
```bash
npm run tauri build
```

Or use the build scripts in `scripts/`.

## Documentation

- [Development Log](docs/DEVLOG.md) — Project structure and setup
- [Architecture Decisions](docs/ARCHITECTURE.md) — Why we chose what
- [Roadmap](docs/ROADMAP.md) — Feature planning
- [Changelog](docs/CHANGELOG.md) — Version history

## Recommended IDE Setup

[VS Code](https://code.visualstudio.com/) + [Svelte](https://marketplace.visualstudio.com/items?itemName=svelte.svelte-vscode) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer).

## License

GPL-3.0-only — See [LICENSE](LICENSE)
