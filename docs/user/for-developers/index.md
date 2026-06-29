# For developers

Kite Ground Control is free, open-source software (GPL-3.0-or-later) and contributions are welcome.
This section is for people who want to **build Kite from source**, understand how it's put together, or
**contribute** code, translations or documentation.

- **[Architecture overview](architecture.md)** — the tech stack and how the backend, frontend and
  protocol layers fit together.
- **[Building from source](building.md)** — prerequisites and the build/run workflow on Windows and
  Linux.
- **[Contributing](contributing.md)** — coding conventions, the i18n and licensing rules, and how to
  submit changes.

## At a glance

- **Desktop app** built with **[Tauri 2.0](https://tauri.app/)** — a **Rust** backend with a
  **[Svelte 5](https://svelte.dev/) / SvelteKit / TypeScript** frontend rendered in the system WebView.
- **Maps:** [Leaflet](https://leafletjs.com/) (2D) and [CesiumJS](https://cesium.com/platform/cesiumjs/)
  (3D globe).
- **Autopilots:** INAV over **MSP**, ArduPilot & PX4 over **MAVLink**, plus passive (listen-only)
  telemetry decoding.
- **Storage:** a local SQLite database (via `rusqlite`) for the flight log, vehicle and battery
  libraries.
- **Platforms:** Windows (primary) and Linux (x86 / ARM).

## Source code

The project lives on GitHub: **[b14ckyy/Kite-GC](https://github.com/b14ckyy/Kite-GC)**. Issues and
pull requests are the best way to get involved.

!!! note "Internal design notes"
    The detailed internal design history — the full architecture record, ADRs, the roadmap and the
    per-feature implementation plans — is kept in a separate private repository. The pages here are the
    curated, public-facing subset aimed at building and contributing. If you're working on a change and
    need deeper context, open an issue and ask.
