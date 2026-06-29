# Architecture overview

A high-level map of how Kite Ground Control is built. This is the public overview; the detailed
decision record (ADRs) is kept internally.

## Tech stack

| Layer | Technology |
|---|---|
| Shell / packaging | [Tauri 2.0](https://tauri.app/) (Rust) — native window, WebView, installers |
| Backend | **Rust** — serial/transport I/O, protocol decoding, SQLite, file handling |
| Frontend | **Svelte 5** (runes) + SvelteKit + **TypeScript**, rendered in the system WebView |
| 2D map | [Leaflet](https://leafletjs.com/) |
| 3D globe | [CesiumJS](https://cesium.com/platform/cesiumjs/) |
| Database | SQLite via `rusqlite` (bundled) |
| Serial | `serialport` (Rust) |
| i18n | `svelte-i18n` (ICU Message Format) — English, German, French |

The frontend (WebView) and the Rust backend communicate through **Tauri commands** (frontend → backend
calls returning `Result<T, String>`) and **events** (backend → frontend streams). A deliberate rule:
events are emitted with the **same names regardless of the underlying protocol**, so the UI doesn't care
whether telemetry arrived over MSP or MAVLink.

## Backend (Rust)

The backend is organised as one module folder per feature area (`msp/`, `mavlink_proto/`, `flightlog/`,
`mission/`, `transport/`, `scheduler/`, `video/`, …).

- **Transport layer** abstracts the link — USB/serial, Bluetooth (SPP & BLE), TCP and UDP.
- **The scheduler owns the serial connection exclusively** on a dedicated thread. For MSP this means a
  strict **request → response** cycle (one request at a time, wait for the reply before the next), with
  priority polling: Attitude > Status > Analog > GPS > secondaries.
- **Protocol modules** decode MSP (INAV) and MAVLink (ArduPilot / PX4) into a shared telemetry model,
  plus passive listen-only decoders (SmartPort / CRSF / LTM) and a relay that re-encodes telemetry for
  other ground stations.
- **Feature gating** keys off the detected firmware/version and capability flags so the UI only offers
  what the connected aircraft supports (minimum INAV 7.0).
- **Database** uses an incremental `PRAGMA user_version` migration chain (earlier migrations are never
  modified) for the flight log, vehicle and battery libraries.

## Frontend (Svelte 5)

The UI is **runes-only** Svelte 5 (`$state`, `$derived`, `$effect`, `$props`) — no legacy Svelte 4
syntax. Code is split by responsibility:

- `controllers/` — domain logic (no UI)
- `adapters/` — data transforms
- `helpers/` — pure functions
- `stores/` — shared reactive state
- `config/` — data tables (map providers, widget registry)
- `utils/` — generic helpers (geo, units)
- `components/` — self-contained UI via `$props()`

Page components stay thin orchestrators; substantial UI, utilities or heavy CSS are extracted into
components. The theme follows the INAV Configurator dark style.

## Data flow

- **Live:** the backend polls/decodes the link and emits telemetry events → frontend stores → widgets,
  the 2D/3D map, and the recorder.
- **Recording & replay:** flights are recorded to the SQLite log; the same rendering pipeline replays
  them, and importers bring in INAV blackbox, ArduPilot Dataflash, MAVLink `.tlog` and raw-MSP logs.
- **3D:** CesiumJS renders the globe, terrain, the flight track and mission overlay, with an FPV cockpit
  camera; it shares the unified telemetry/mission model with the 2D map so the two stay in sync.

## Where the detail lives

Architecturally significant decisions are captured as **ADRs** in the internal design repository. Code
comments often cite an `ADR-NNN`; those identifiers are stable references to that record. If you need the
reasoning behind a particular decision while working on a change, open an issue and ask.
