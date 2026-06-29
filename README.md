<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset=".github/kitegc-banner-dark.png">
    <img alt="Kite Ground Control" src=".github/kitegc-banner-light.png" width="560">
  </picture>
</p>

<p align="center"><b>A modern, cross-platform ground control station for INAV, ArduPilot &amp; PX4 UAV systems.</b></p>

<p align="center">
  <a href="LICENSE"><img alt="License: GPL-3.0-or-later" src="https://img.shields.io/badge/License-GPLv3-blue.svg"></a>
  <a href="https://b14ckyy.github.io/Kite-GC/"><img alt="Documentation" src="https://img.shields.io/badge/docs-online-37a8db"></a>
  <img alt="Platform" src="https://img.shields.io/badge/platform-Windows%20%7C%20Linux-555">
  <img alt="Status" src="https://img.shields.io/badge/status-public%20beta-f5a623">
  <a href="https://paypal.me/b14ckyy"><img alt="Donate via PayPal" src="https://img.shields.io/badge/Donate-PayPal-00457C?logo=paypal&logoColor=white"></a>
</p>

---

**Kite Ground Control (Kite GC)** is a modern, cross-platform ground control station for **INAV**,
**ArduPilot** and **PX4** aircraft — planes, multirotors, VTOL, helicopters, rovers and boats. It
combines everything you expect from a GCS with a fast, intuitive interface and a few things you won't
find anywhere else — like a full 3D flight view, a fleet & battery manager, and live video right next
to the map.

Built with [Tauri 2.0](https://tauri.app/) (Rust backend) and [Svelte 5](https://svelte.dev/)
(TypeScript frontend).

<p align="center">
  <img alt="Kite Ground Control in 3D mode" src="docs/user/assets/main_interface_3d.png" width="820">
</p>

<p align="center">
  <b><a href="https://b14ckyy.github.io/Kite-GC/">📖 Documentation</a></b>
  &nbsp;·&nbsp;
  <b><a href="https://github.com/b14ckyy/Kite-GC/releases">⬇️ Download</a></b>
</p>

## Highlights

- **🧊 Immersive 3D flight view** — a full 3D globe with real terrain, your aircraft and track in 3D, a
  3D mission overlay, an FPV cockpit camera and real-time day/night lighting; switch 2D ⇄ 3D seamlessly.
- **🚁 One GCS for INAV, ArduPilot & PX4** — plan, fly and log across all three autopilots with a
  consistent interface, including passive (listen-only) and relay link modes.
- **🏭 Fleet, Battery & Mission managers** — keep a library of your aircraft and batteries with full
  build sheets and lifetime stats, plus a reusable mission library — all linked to your flight log.
- **⚡ Fast & intuitive** — a performance-oriented interface with dockable widgets and panels that
  remembers your layout, so the focus stays on flying.

## The essentials

Everything you'd expect from a ground station:

- **Live telemetry & HUD** — attitude, altitude, speed (incl. airspeed), a compass with wind and
  ground-track indicators, GPS/sensor health, link quality and flight-mode display.
- **Customisable widget dashboard** — drag-and-drop flight widgets docked to the side and bottom.
- **2D moving map** — aircraft, track, home and mission, with heading-up mode and day/night shading.
- **Mission planning** — create, upload, download and edit missions; undo/redo; a survey-pattern
  generator; terrain-following / AGL waypoints.
- **Vehicle control** — arm/disarm, flight-mode changes, takeoff/RTL/loiter and more (ArduPilot/PX4).
- **Comfort** — a multi-language interface (English, German, French at launch) with persistent window,
  layout and settings between sessions.

## What makes Kite special

- **Full 3D mode** — Cesium 3D globe with real terrain, a unified 3D mission overlay, an FPV cockpit
  camera with a conformal HUD, and live day/night lighting.
- **Terrain awareness** — AGL (above-ground) waypoints, a terrain-profile analysis for your mission,
  and live *terrain radar* / AGL widgets in flight.
- **Flight Logbook** — automatic recording with replay, plus import of INAV blackbox, **ArduPilot
  Dataflash**, MAVLink `.tlog` and **MWPTools-compatible raw-MSP** logs — unified into one searchable
  flight history.
- **Fleet (Vehicle) Manager** — a build sheet per aircraft (airframe, propulsion, FC, sensors, photo)
  with lifetime statistics, auto-linked to your flights; export/import as `.kvehicle`.
- **Battery Manager** — track each pack by serial: cycles, lifetime usage and health, with `.kbatt`
  export/import.
- **Safety suite** — geofences (ArduPilot/PX4), geozones (INAV), safe-home & fixed-wing autoland,
  airspace overlays (airports, controlled airspace, obstacles) and **foreign-vehicle radar** with
  ADS-B proximity & conflict alerts.
- **Live video** — low-latency RTSP video shown alongside (or behind) the map, with one-click
  map ⇄ video swapping.
- **Telemetry relay** — re-encode and forward live telemetry to other ground stations, handsets or an
  antenna tracker.
- **RC control** — fly from the GCS with a gamepad/joystick (HID).
- **RF link analysis** — visualise signal quality to find the best antenna setup.

## Supported setups

- **Autopilots:** INAV (7.0+), ArduPilot, PX4.
- **Aircraft:** fixed-wing, flying-wing, VTOL, multirotor, helicopter, rover, boat.
- **Connections:** USB / serial, Bluetooth (SPP & BLE), TCP, UDP.
- **Link modes:** live control link, **passive** listen-only telemetry, or a **relay** that
  re-broadcasts to other ground stations.
- **Platforms:** Windows (primary), Linux (x86 / ARM). Android is on hold.

## Download

Grab the latest installer for your platform from the
[**Releases**](https://github.com/b14ckyy/Kite-GC/releases) page, or [build from source](#building-from-source)
below.

## Documentation

Full documentation is online at **[b14ckyy.github.io/Kite-GC](https://b14ckyy.github.io/Kite-GC/)**:

- **Getting started:** [Installation](https://b14ckyy.github.io/Kite-GC/getting-started/installation/) ·
  [First connection](https://b14ckyy.github.io/Kite-GC/getting-started/first-connection/) ·
  [Quick tour](https://b14ckyy.github.io/Kite-GC/getting-started/quick-tour/)
- **Guides:** [Missions](https://b14ckyy.github.io/Kite-GC/guides/missions/) ·
  [Telemetry & display](https://b14ckyy.github.io/Kite-GC/guides/telemetry-and-display/) ·
  [Logbook](https://b14ckyy.github.io/Kite-GC/guides/logbook/) · [Safety](https://b14ckyy.github.io/Kite-GC/guides/safety/) ·
  [3D map](https://b14ckyy.github.io/Kite-GC/guides/map-3d/) · [Video](https://b14ckyy.github.io/Kite-GC/guides/video/)
- **Trouble connecting?** [Troubleshooting → Connection](https://b14ckyy.github.io/Kite-GC/troubleshooting/connection/)
- **For developers:** [Overview](https://b14ckyy.github.io/Kite-GC/for-developers/) ·
  [Architecture](https://b14ckyy.github.io/Kite-GC/for-developers/architecture/) ·
  [Building from source](https://b14ckyy.github.io/Kite-GC/for-developers/building/) ·
  [Contributing](https://b14ckyy.github.io/Kite-GC/for-developers/contributing/)

## Support development

Kite GC is free, open-source software built in my spare time. If it's useful to you and you'd like to
support its development, a donation is hugely appreciated — thank you! 💛

<p align="center">
  <a href="https://paypal.me/b14ckyy"><img alt="Donate via PayPal" src="https://img.shields.io/badge/Donate%20via-PayPal-00457C?logo=paypal&logoColor=white&style=for-the-badge"></a>
</p>

## Building from source

### Prerequisites
- [Node.js](https://nodejs.org/) LTS (v20 or v24)
- [Rust](https://rustup.rs/) (via rustup)
- [just](https://github.com/casey/just) — the primary task runner
- Platform toolchain & system dependencies — see the [Build Guide](https://b14ckyy.github.io/Kite-GC/for-developers/building/)

### Develop
```bash
npm install      # one-time
just dev         # start with hot reload  (alt: npm run tauri dev)
```

### Build
```bash
just build           # current platform   (alt: npm run tauri build)
just build-windows   # Windows release
just build-linux     # Linux release (on Linux)
```

> **Tip:** install `just` for the best developer experience (see the `justfile` in the project root).
> More detail — setup, troubleshooting and CI — is in the
> [Building from source](https://b14ckyy.github.io/Kite-GC/for-developers/building/) guide, and the
> [Architecture overview](https://b14ckyy.github.io/Kite-GC/for-developers/architecture/) explains how Kite fits together.

## Contributing

Issues and pull requests are welcome. CI runs automatically on push/PR (`cargo check` +
`svelte-check`). Recommended IDE: [VS Code](https://code.visualstudio.com/) with the
[Svelte](https://marketplace.visualstudio.com/items?itemName=svelte.svelte-vscode),
[Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) and
[rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer) extensions.

## License

[GPL-3.0-or-later](LICENSE) — Copyright © 2026 Marc Hoffmann ([b14ckyy](https://github.com/b14ckyy)).
