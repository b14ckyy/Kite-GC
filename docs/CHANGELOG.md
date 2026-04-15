# INAV GCS — Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Floating navigation panel with hamburger menu (replaces fixed sidebar)
- Tab-based panel navigation: UAV Info, Settings, Mission Control
- Bottom telemetry overlay strip (ALT, SPD, DIST, BAT, SATS placeholders)
- Animated hamburger icon (transforms to X when open)
- Glassmorphism UI elements (backdrop-filter blur, semi-transparent panels)
- Slide-in panel animation
- `MspRc` feature gate (INAV 8.0+) — MSP as full RC protocol
- `AuxRc` feature gate (INAV 9.1+) — renamed from MspChannelControl
- Session persistence: last serial port, last baud rate, map position/zoom, panel state, active tab
- `AppSettings` store with localStorage persistence

### Changed
- Map now fills the entire viewport behind all UI overlays
- Map zoom controls moved to top-right (avoid panel collision)
- Side panel replaced with free-floating overlay panel system
- Settings store: `sidePanelOpen` replaced with `navPanelOpen` + `activeTab`
- Feature gate system: removed `MultiMission` (irrelevant, pre-7.0)

### Fixed
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
