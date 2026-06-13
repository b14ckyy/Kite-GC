# Kite Ground Control — Architecture Decisions

This document records key architecture decisions and their rationale.

---

## ADR-001: Tauri 2.0 as Application Framework

**Date**: 2026-04-15
**Status**: Accepted

**Context**: Need a cross-platform framework for Windows, Linux (x86/ARM), Android, with future macOS/iOS support. Must support native hardware access (Serial, BLE, HID) and produce standalone executables.

**Considered**: Electron, Flutter, Qt/C++, Kotlin Multiplatform, Tauri 2.0

**Decision**: Tauri 2.0

**Rationale**:
- Covers all target platforms from a single codebase (including mobile)
- ~5-15MB app size vs ~200MB for Electron
- Rust backend provides native hardware access via crates (serialport, btleplug, hidapi)
- No license costs (MIT/Apache, compatible with GPLv3)
- Web-based frontend allows full UI customization (not widget-locked like GTK4/Qt)

---

## ADR-002: Svelte 5 as Frontend Framework

**Date**: 2026-04-15
**Status**: Accepted

**Context**: Need a reactive UI framework for real-time telemetry display that is lightweight, performant, and easy to read (primary developer does not write code directly).

**Considered**: Svelte, React, Vue

**Decision**: Svelte 5

**Rationale**:
- Lowest boilerplate — most readable code for non-coders
- Compiler-based — smallest bundle size, no runtime overhead
- Best suited for real-time telemetry (many frequently updated values)
- Excellent Tauri integration with official template

---

## ADR-003: Leaflet for Maps

**Date**: 2026-04-15
**Status**: Accepted

**Context**: Map display is central to a GCS. Need a performant, lightweight map library that works well on mobile and low-power ARM devices.

**Considered**: Leaflet, MapLibre GL, OpenLayers

**Decision**: Leaflet

**Rationale**:
- Proven performance on low-power Android devices (tested via High-Res Map Generator)
- Lightweight, well-documented, huge plugin ecosystem
- Easy to integrate with Svelte
- Supports custom tile providers and offline tiles

---

## ADR-004: Modular Architecture

**Date**: 2026-04-15
**Status**: Accepted

**Context**: The project will grow significantly over time. Features like mission planning, log replay, radar view, survey planning etc. will be added incrementally.

**Decision**: Each feature is a self-contained module in both backend (Rust) and frontend (Svelte), following a consistent pattern.

**Rationale**:
- New features don't require modifying existing code (open/closed principle)
- Easy to identify where each feature lives in the codebase
- Enables parallel development of features
- Simplifies maintenance and debugging

---

## ADR-005: Floating Panel UI Layout

**Date**: 2026-06-15
**Status**: Superseded by ADR-023

**Context**: A GCS needs maximum map visibility at all times. Traditional sidebar layouts waste horizontal space — especially on smaller screens or when information panels are not actively needed.

**Decision**: All UI panels are floating overlays on top of a full-viewport map. Navigation uses a hamburger menu button that opens a side rail with tab buttons and a floating content panel.

> **Note**: The original floating-panel concept is retained, but the overall layout is now governed by a CSS Grid zone system (ADR-023). Panels float within the Panel Zone rather than using hardcoded viewport-relative positioning.

**Layout**:
```
┌─────────────────────────────────────────┐
│  Toolbar (logo, sensors, port, connect) │
├─────────────────────────────────────────┤
│ ☰ │                                     │
│ ─ │  ┌─────────┐              [+][-]    │
│ ✈ │  │ Panel   │     MAP                │
│ ⚙ │  │ Content │   (fullscreen)         │
│ ◎ │  │         │                        │
│   │  └─────────┘                        │
│   │                                     │
│   │    ┌───┬───┬───┬───┬───┐            │
│   │    │ALT│SPD│DST│BAT│SAT│            │
├───┴────┴───┴───┴───┴───┴───┴────────────┤
│  Status Bar                             │
└─────────────────────────────────────────┘
```

**Key elements**:
- **Nav Rail** (left): Hamburger button + tab icons — only icons visible when closed, labels when open
- **Floating Panel**: Semi-transparent, rounded, backdrop-blur, slides in with animation
- **Telemetry Strip** (bottom center): Horizontal widget bar overlaying the map
- **Map**: Always fills entire viewport between toolbar and statusbar (z-index: 0)
- All overlays use `backdrop-filter: blur()` for glassmorphism effect

**Rationale**:
- Map is always 100% visible — no layout shifts when toggling panels
- Minimal default footprint (just a hamburger button)
- Tab system is extensible — new sections only require adding a tab definition
- Glassmorphism keeps panels visually distinct without fully obscuring the map
- Mobile-friendly — panels can be dismissed with a single tap

---

## ADR-006: Session Persistence via localStorage

**Date**: 2026-06-15
**Status**: Accepted

**Context**: Users expect the app to remember their last-used serial port, baud rate, map position, and panel state between sessions.

**Decision**: Use a Svelte writable store backed by `localStorage` under key `kite-gc-settings`. The store auto-saves on every mutation via `set()`, `update()`, or `patch()`.

**Persisted state**: `lastPort`, `lastBaud`, `map.center`, `map.zoom`, `navPanelOpen`, `activeTab`

> **Note**: Native *window geometry* (size / position / maximized) is **not** stored here — it
> is persisted separately by `tauri-plugin-window-state` (see ADR-030), since it's owned by the
> OS window rather than the WebView.

**Rationale**:
- localStorage is synchronous and available in all Tauri WebView contexts
- No backend/database needed for simple preferences
- `patch()` helper allows updating individual fields without replacing the whole object
- Defaults are merged on load (`{ ...defaults, ...stored }`) to handle schema evolution

---

## ADR-007: MSP Scheduler with Dedicated Thread

**Date**: 2026-04-15
**Status**: Accepted

**Context**: MSP is a strict request-response protocol — only one request can be outstanding at a time. The GCS needs to poll multiple telemetry groups at different rates while also supporting on-demand operations (waypoint upload/download, configuration reads). Over wireless links (SiK, Bluetooth, ELRS backpack) bandwidth is severely limited.

**Considered**: async tasks with tokio, main-thread polling, dedicated thread

**Decision**: Dedicated `std::thread` that owns the `SerialConnection` exclusively after the initial handshake. Communication with the rest of the app via `mpsc` channels.

**Architecture**:
```
connect() handshake (blocking, main thread)
    │
    ▼
SerialConnection moved → Scheduler Thread
    │
    ├── Telemetry Slots (time-based, configurable rates)
    ├── Command Channel (mpsc, oneshot requests e.g. read config)
    ├── Bulk Channel (mpsc, batch operations e.g. waypoints)
    │
    ▼
Tauri Events → Frontend (telemetry-attitude, telemetry-gps, ...)
```

**Scheduling algorithm**:
1. Find most overdue telemetry slot **(priority first, then overdue duration)** → poll it
2. Nothing overdue → drain one command from command channel
3. No commands → try one bulk item (waypoint upload fills gaps between polls)
4. Nothing to do → sleep until next slot due

**Adaptive degradation** (replaces static link profiles):
- Each telemetry slot has a priority: Attitude (5) > Status (4) > Analog (3) > GPS (2) > Secondaries (1)
- When multiple slots are overdue, the highest-priority slot is polled first
- Lower-priority groups naturally lose bandwidth — GPS degrades before Attitude
- No link type detection needed: USB-connected wireless devices (SiK, mLRS, ELRS backpack) are handled correctly
- Next poll scheduled at `last_reply_time + interval` — if the link is too slow, all groups slow down proportionally, with priority determining who slows first

**Mode decoding detail (INAV)**:
- Scheduler queries `MSP_BOXIDS` (119) once at startup.
- `MSPV2_INAV_STATUS.activeModes` bits are decoded with this index→box-id map.
- This avoids false mode detection caused by treating activeModes bits as permanent box IDs.

**Optional modules**: Airspeed polling is toggleable (`airspeed_enabled`). Future optional modules follow the same pattern — disabled by default, enabled via settings.

**Rationale**:
- Thread ownership eliminates all concurrency issues on the serial port
- No async runtime needed (serialport crate is blocking anyway)
- Channel-based design cleanly separates telemetry polling from UI-triggered commands
- Staggered secondary polls (ALT, AIRSPEED rotate) minimize per-cycle message count
- The scheduler can be stopped cleanly by sending a Stop command and joining the thread

---

## ADR-008: Dev-Only Debug Module with Zero-Cost Release Build

**Date**: 2026-04-15
**Status**: Accepted

**Context**: Debugging MSP communication (timing, throughput, throttling, timeouts) requires visibility into the scheduler's internal state. Shipping debug instrumentation in release builds adds overhead and attack surface.

**Decision**: A `scheduler/debug.rs` module compiled only under `#[cfg(debug_assertions)]`. In release builds, a zero-sized no-op stub with `#[inline(always)]` methods is substituted — the compiler eliminates all tracking calls completely. The frontend uses `import.meta.env.DEV` to gate the debug UI — Vite tree-shakes the entire `DebugPanel.svelte` component from production bundles.

**Debug Monitor features**:
- Per-MSP-code LED indicators (yellow=request, green=response, red=timeout, gray=idle)
- Target rate vs actual rate per code (throttle detection highlighted in orange)
- MSG/s and bytes/s throughput counters (TX/RX, 1-second sliding window)
- POLL/INIT status badges distinguishing active polling from handshake-only codes
- Request, response, and timeout counters per code
- Updates emitted via `debug-msp-stats` Tauri event at ~4 Hz (250ms interval)

**Rationale**:
- `#[cfg(debug_assertions)]` is the idiomatic Rust pattern for dev-only code
- Zero-sized type with inline no-ops has zero runtime cost — verified by compiler optimization
- Vite's `import.meta.env.DEV` is statically replaced at build time, enabling dead code elimination
- Dynamic import (`import()`) ensures the component is not even bundled in production
- No dev dependencies leak into release builds on either backend or frontend

---

## ADR-009: Frontend Modularization — Thin Orchestrator + Controllers/Adapters

**Date**: 2026-04-18
**Status**: Accepted

**Context**: `+page.svelte` had grown to ~2846 lines — mixing connection logic, logbook operations, playback state, widget management, and UI rendering. This made the file difficult to navigate and modify.

**Decision**: Extract domain logic into 4 controllers, 1 adapter, 1 helper module, and multiple extracted UI components (including shared dialog). `+page.svelte` is reduced to a thin orchestrator (~1100 lines) that imports and wires these modules.

**Extracted modules**:

| Module | Type | Responsibility |
|---|---|---|
| `controllers/connectionController.ts` | Controller | Serial port refresh, connect/disconnect, telemetry listener management |
| `controllers/logbookController.ts` | Controller | Flight list/CRUD, Blackbox import, geocode/weather enrichment |
| `controllers/playbackController.ts` | Controller | Timer-based playback engine (100ms tick, 1×/2×/4×/10× speed, seek) |
| `controllers/widgetController.ts` | Controller | Drag-and-drop reorder/cross-panel move (pure functions) |
| `adapters/telemetryAdapter.ts` | Adapter | `toTelemetryData()` — maps DB `TelemetryRecord` → widget-consumable `TelemetryData` |
| `helpers/telemetry.ts` | Helper | `isArmed()`, `hasKnownLocation()`, `isValidGpsCoordinate()` |
| `LogPlayer.svelte` | Component | Playback controls UI (play/pause/reset, scrubber, speed selector) |
| `LogbookPanel.svelte` | Component | Flight list, detail view, import/weather/notes UI |
| `ConfirmDialog.svelte` | Component | Promise-based in-app dialog used for confirm/info workflows |
| `SettingsPanel.svelte` | Component | All settings sections |
| `Toolbar.svelte` | Component | Logo, sensor bar, port selector, connect button |
| `UavInfoPanel.svelte` | Component | FC info, feature gates, craft name |
| `StatusBar.svelte` | Component | Connection status, arming indicator, app title |
| `NavRail.svelte` | Component | Hamburger menu + vertical tab rail |

**Rationale**:
- Controllers contain logic that was previously inline in `+page.svelte` — enables unit testing
- Adapter pattern (`toTelemetryData`) cleanly separates DB format from widget expectations
- Components are self-contained with `$props()` — no direct store access except where needed
- `+page.svelte` is now a wiring layer: imports, reactive derivations, event routing

---

## ADR-010: Multi-Protocol Architecture — ByteTransport + Separate Schedulers

**Date**: 2026-04-16
**Status**: Accepted (implementation starting)
**Supersedes**: Original TelemetrySource trait plan

**Context**: The GCS needs to support MSP (request/response polling) and MAVLink (push-based streams) — two fundamentally different communication models. A single `TelemetrySource::poll()` trait cannot cleanly represent both paradigms. The current `Transport` trait (`fn msp_request()`) is MSP-specific and cannot serve MAVLink's continuous byte-stream needs.

**Decision**: Two-layer architecture with protocol-specific schedulers sharing a common byte-level transport.

**Layer 1 — ByteTransport trait** (protocol-agnostic):
```rust
pub trait ByteTransport: Send {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, TransportError>;
    fn write(&mut self, data: &[u8]) -> Result<usize, TransportError>;
    fn description(&self) -> String;
    fn close(&mut self) -> Result<(), TransportError>;
}
```
All transports (Serial, TCP, UDP, BLE) implement `ByteTransport`. New transports automatically work with all protocols.

**Layer 2 — Protocol-specific handlers** (separate modules):
- **MspScheduler**: Existing priority-based poll loop, uses `MspTransport` wrapper (MSP framing over `ByteTransport`)
- **MavlinkHandler**: Reader thread (continuous parse + dispatch) + Heartbeat writer (1 Hz) + Command channel (COMMAND_LONG/ACK)

**Protocol selection**: Explicit UI dropdown (no auto-detect). User selects MSP or MAVLink before connecting. Simplifies handshake logic and avoids issues on slow wireless links.

**Unified data output**: Both handlers emit the **same** Tauri events (`telemetry-attitude`, `telemetry-gps`, etc.) with identical payload shapes. Frontend never knows which protocol is active.

**MAVLink specifics**:
- Firmware scope: ArduPilot + PX4 + INAV MAVLink (common dialect)
- MAVLink v1 + v2 (backwards-compatible)
- `mavlink` Rust crate for generated message definitions
- GCS IDs: sysid=255, compid=190 (industry standard)
- 10 receive messages + HOME_POSITION covering all widget fields

**Key differences from original TelemetrySource plan**:
- No single trait for all protocols — MSP polling and MAVLink push are too different
- ByteTransport extracted at a lower level — enables transport reuse across protocols
- Protocol auto-detection deferred — explicit selection is safer and simpler
- Each protocol handler is a self-contained module (`msp/`, `mavlink/`)

Full implementation plan: `docs/archive/PROTOCOL_REFACTORING.md`

**Rationale**:
- Separate handlers avoid forcing push-based protocols into a poll-based abstraction
- ByteTransport reuse eliminates duplicate transport code across protocols
- Modular structure: adding a new protocol means adding a new handler module, not modifying existing ones
- Frontend stays completely unchanged — same events, same stores, same widgets

---

## ADR-011: Drag-and-Drop Widget Panel System

**Date**: 2026-04-16
**Status**: Accepted (sizing updated by ADR-023)

**Context**: The GCS needs a flexible HUD with multiple instrument widgets that users can arrange to their preference. Fixed layouts don't work well across different screen sizes and use cases (e.g. FPV flying prioritizes AHI, long-range prioritizes GPS/battery).

**Decision**: Two drag-and-drop widget panels (bottom horizontal, right vertical) with edit-mode toggling. Widgets are classified as Large (22.5 units), Small (13.5 units = 60% of large), or Wide (2:1 landscape — two large units wide in the horizontal dock, half-height in the side dock; used by the Live AGL HUD). Sizing is container-relative (px computed from cross-axis), not viewport-relative.

> **Note**: Originally sized in `vmin` CSS units. ADR-023 replaced this with container-relative px sizing — each dock independently computes its own `pxPerUnit = crossAxisPx / LARGE_BASE_VMIN`, fully decoupling bottom and side dock scaling.

**Key design choices**:
- **Snap positions only** — no free-form positioning. Widgets snap into ordered slots within panels.
- **Half-position insertion** — drag cursor position relative to slot midpoint determines before/after placement. Visual insertion indicator (blue line) shows exact drop position.
- **Dynamic sizing** — panels compute available space from window dimensions minus reserved areas. Widgets render at base size and shrink uniformly (min 50%) only when total exceeds available space.
- **Edit mode overlay** — transparent overlay div captures drag events without interfering with widget rendering. Solves the SVG/canvas event interception problem.
- **Tauri DnD disabled** — `dragDropEnabled: false` in tauri.conf.json prevents Tauri's native file-drop handler from intercepting HTML5 drag events.
- **Position memory** — `PanelConfig.positions` records last panel per widget. Toggle OFF/ON restores to last panel, not always bottom.

**Layout**:
```
┌─────────────────────────────────────────────┐
│  Toolbar                                    │
├─────────────────────────────────────────────┤
│                                    │ Right  │
│              MAP                   │ Panel  │
│           (fullscreen)             │(vert)  │
│                                    │        │
│  ┌──────────────────────────┐  ┌───┤        │
│  │    Bottom Panel (horiz)  │  │Rsv│        │
├──┴──────────────────────────┴──┴───┴────────┤
│  Status Bar                                 │
└─────────────────────────────────────────────┘
```

**Rationale**:
- Two panels cover the most common layouts (instruments at bottom, data sidebar on right)
- Snap positions eliminate alignment/overlap issues — clean look without manual tweaking
- vmin sizing ensures consistent proportions on any screen size or aspect ratio
- Edit mode is explicit (toggle button) — no accidental drags during flight monitoring
- Position memory reduces friction when temporarily hiding widgets

---

## ADR-012: Map Heading-Up Mode via CSS Transform

**Date**: 2026-04-16
**Status**: Accepted

**Context**: Pilots prefer heading-up map orientation during flight — the direction of travel is always "up" on screen, matching the natural view from the cockpit. Leaflet does not natively support map rotation.

**Considered**: leaflet-rotate plugin, custom Leaflet fork, CSS transform on container

**Decision**: CSS `transform: rotate() scale(1.42)` on the Leaflet map container element, with `overflow: hidden` on a wrapper div.

**How it works**:
- `rotate(var(--map-rotation))` — CSS variable updated on each telemetry tick with `-yaw` degrees
- `scale(1.42)` (√2) — ensures the rotated rectangle always fills the viewport corners
- Wrapper div with `overflow: hidden` clips the extended corners
- Leaflet controls (zoom, attribution) are counter-rotated and scaled back (0.707) to stay readable
- UAV marker icon uses 0° rotation in heading-up mode since the map itself provides the rotation
- Map auto-centers on UAV position in heading-up mode

**Trade-offs**:
- Map interaction (panning) works but direction feels rotated — acceptable for a monitoring view
- 42% visual zoom increase from scale factor — tiles load for the larger visible area
- No dependency on external plugins or Leaflet forks

**Rationale**:
- Zero dependencies — pure CSS, no additional JavaScript libraries
- GPU-accelerated transforms — smooth rotation even at 5 Hz telemetry rate
- Simple toggle — just add/remove CSS class + update CSS variable
- Can be upgraded to leaflet-rotate or CesiumJS 3D in future milestones if needed

---

## ADR-013: Mission Planning — Backend-Owned State with Frontend Mirror

**Date**: 2026-04-16
**Status**: Accepted

**Context**: Mission planning requires persistent waypoint state that can be transferred to/from the flight controller via MSP. The mission state is modified from both the map (click-to-add, drag) and the sidebar panel (reorder, edit). All modifications must be consistently reflected in both views.

**Decision**: The mission state lives in the Rust backend as `MissionStore` (a `Mutex<Mission>`) and is mirrored to the frontend via Tauri `invoke()` calls. Every mutation goes through the backend.

**Architecture**:
```
Frontend (Svelte)                    Backend (Rust)
┌──────────────┐                    ┌───────────────────┐
│ mission.ts   │──invoke()──────────│ commands/mission.rs│
│ (writable    │◄──return Mission──│   │                │
│  store)      │                    │   ▼                │
│              │                    │ mission/store.rs   │
│ MissionLayer │                    │ (Mutex<Mission>)   │
│ MissionPanel │                    │   │                │
└──────────────┘                    │   ├── codec.rs     │
                                    │   └── types.rs     │
                                    │                    │
                                    │ MSP Transfer:      │
                                    │ download → FC→Store │
                                    │ upload   → Store→FC │
                                    └───────────────────┘
```

**Key design choices**:
- **Backend-owned state**: All CRUD operations (`add_wp`, `update_wp`, `remove_wp`, `insert_wp`, `reorder_wp`) are Rust functions that return the updated `Mission`
- **Frontend mirror**: `mission.ts` writable store is updated after each invoke call returns
- **FC transfer via scheduler**: Upload/download use the existing scheduler bulk channel to avoid concurrent serial access
- **XML serialization**: `mission_to_xml` / `mission_from_xml` in Rust for MW XML format (interoperable with INAV Configurator, mwp, ezgui)
- **File I/O in Rust**: `mission_save_file` / `mission_load_file` use Rust's filesystem APIs — frontend passes file path from native dialog
- **Dirty flag**: Tracks whether mission has been modified since last FC transfer
- **Max 120 WPs**: INAV firmware limit enforced in frontend (map click, polyline insert, modifier add)

**Modifier WP handling**:
- Modifier WPs (Jump, RTH, SetHead) are stored in the flat waypoint array at their natural index
- The frontend groups modifiers with their preceding geo-WP for display (editor popup, sidebar indent)
- Display numbering skips modifiers — only geo-WPs get visible numbers on map markers
- SET_POI has coordinates but craft does NOT fly to it — shown as standalone marker, excluded from flight path polyline

**Mission termination**:
- LAND and RTH are mission-terminating actions — the flight controller stops execution after them
- All WPs after the first LAND/RTH are greyed out (35% opacity on markers, dashed grey polyline)
- Greyed WPs are non-draggable and have no editor popups to prevent accidental editing of unreachable WPs

**Rationale**:
- Rust ownership prevents data races between concurrent map/panel edits
- Backend state is always authoritative — no frontend-only state divergence possible
- Invoke pattern is consistent with existing connection/telemetry architecture
- MW XML format ensures file interoperability with the INAV ecosystem

---

## ADR-014: Internationalization via svelte-i18n

**Date**: 2026-04-16
**Status**: Accepted

**Context**: The UI had ~200 hardcoded English strings across 14 component files. Multi-language support was needed before the codebase grew further. Needed a solution that works with Svelte 5's runes mode and `$store` auto-subscription.

**Considered**: `svelte-i18n`, `paraglide-js` (Inlang), custom i18n solution

**Decision**: `svelte-i18n` with JSON locale files and ICU Message Format

**Architecture**:
```
src/lib/i18n/
├── index.ts              # register() + initI18n() + SUPPORTED_LOCALES
└── locales/
    ├── en.json           # English (default, ~200 keys, 18 namespaces)
    └── de.json           # German (complete translation)

+layout.svelte            # Reads saved locale, calls initI18n(), gates render on $isLoading
+page.svelte              # $t('key') in templates, $locale for current locale
settings.ts               # locale field persisted in localStorage
```

**Key design choices**:
- **JSON locale files**: Simple, toolable, easy for community contributors to translate
- **ICU Message Format**: Supports interpolation (`{count} WPs`), plurals, and select — no custom template syntax
- **Lazy loading**: Locales loaded via dynamic `import()` — only the active locale is bundled
- **Rust stays English**: Backend errors are technical (port names, byte offsets, protocol errors). Frontend wraps in user-facing `$t()` messages where appropriate
- **`WP_ACTION_KEYS` pattern**: Enum-to-i18n-key map in `mission.ts` enables localized labels without making the store reactive on locale changes
- **`labelKey` in widget registry**: Parallel to existing `label` field, allows gradual migration

**Rationale**:
- `svelte-i18n` is battle-tested (900K+ weekly npm downloads), works with Svelte 5 `$store` syntax
- `paraglide-js` was considered but is overkill for an SPA with 2 locales — adds build complexity
- Custom solution would reinvent ICU message formatting, locale loading, and store integration
- Early adoption (before M5) avoids a large "string extraction" refactor later

---

## ADR-015: Flight Recording Integrated Into Scheduler (M5)

**Date**: 2026-04-16
**Status**: Accepted

**Context**: M5 required automatic flight recording (ARM to DISARM), flight metadata, optional raw logs, and a user-visible logbook. Recording must not interfere with MSP scheduling and must work with the existing single-primary-connection architecture.

**Decision**: Integrate a `FlightRecorder` directly into the backend scheduler poll loop. The scheduler continues to emit telemetry events to frontend as before, and additionally forwards decoded samples to the recorder handle when enabled.

**Architecture**:
```
connect() command
    ├─ MSP handshake (+ MSP_NAME)
    ├─ build FlightLogSettings (enabled/path/raw)
    ├─ initialize FlightRecorder (optional)
    └─ start scheduler(serial, telemetryConfig, appHandle, recorder)

scheduler poll loop
    ├─ poll MSP slot
    ├─ decode payload
    ├─ feed recorder (if enabled)
    └─ emit telemetry event to frontend

FlightRecorder
    ├─ detect ARM/DISARM from MSPV2_INAV_STATUS arming_flags bit 2
    ├─ start/finish flight session rows in SQLite
    ├─ batch-write telemetry samples
    └─ optional raw text log writer
```

**Data storage**:
- SQLite via `rusqlite` with `bundled` feature (no external SQLite dependency)
- Schema migration via `PRAGMA user_version` (v1 currently)
- DB path policy:
    - Custom folder selected by user -> `<folder>/flights.db`
    - Portable mode -> `<exe>/data/flights.db`
    - Default install -> `%APPDATA%/kite-gc/flights.db` (Windows)

**Metadata enrichment**:
- Reverse geocoding: OSM Nominatim (`flightlog_geocode`)
- Weather: Open-Meteo (`flightlog_fetch_weather`)
- Enrichment is currently lazy/on-demand from logbook UI to keep recorder path non-blocking

**Rationale**:
- Feeding recorder in scheduler avoids duplicate event parsing and keeps timing consistent with polling
- ARM/DISARM transition logic in backend guarantees recording even if frontend is not focused
- Bundled SQLite simplifies distribution and installer behavior
- On-demand geocode/weather avoids network latency in real-time telemetry thread

---

## ADR-016: Blackbox Integration via External Binary + Raw CSV Storage

**Date**: 2026-04-17
**Status**: Accepted

**Context**: INAV Blackbox logs contain high-resolution flight telemetry in a compact binary format. Users want to import these logs into the Kite GCS logbook for GPS track replay and telemetry archival. The Blackbox binary format is complex, version-dependent, and already decoded by the official `blackbox_decode` tool from iNavFlight/blackbox-tools.

**Considered**:
1. Reimplement Blackbox decoder in Rust
2. Compile blackbox_decode C source into kite-gc via `cc` crate
3. Bundle blackbox_decode as external binary, invoke as child process

**Decision**: Option 3 — bundle `blackbox_decode` as an external binary, invoke via `std::process::Command`.

**Binary discovery** (in order):
1. Application folder (next to the kite-gc executable)
2. System PATH fallback
3. If not found → Blackbox import disabled with user-facing message

No settings UI for the binary path — the discovery is automatic.

**Invocation**:
```
blackbox_decode --merge-gps --datetime --unit-height m --unit-gps-speed mps --stdout <file>
```
- `--stdout`: CSV output captured via `Command::output()` (no temp files)
- `--index N`: Selects a specific log from multi-session .TXT files

**Performance design**:
- **Pre-built header index map**: `HashMap<String, usize>` built once per CSV file; all column lookups are O(1) per row
- **`ColumnIndices` struct**: All relevant column positions resolved once, before iterating rows
- **Downsampling**: `H looptime` + `H P interval` headers read from raw file to compute effective log Hz; rows skipped to achieve ≤ 10 Hz in the DB (e.g. 500 Hz → keep 1 in 50 rows)
- **Raw CSV line storage**: Comma-joined raw CSV stored in `blackbox_records.csv_data` — no JSON re-serialization overhead

**Heading handling**: INAV blackbox `heading` column is in decidegrees (0–3600). Parser auto-detects: if value > 360 → divide by 10. Same for `yaw`.

**Data storage**:
- **Parsed telemetry**: `telemetry_records` table — downsampled at ≤ 10 Hz, same schema as live MSP recordings
- **Blackbox archive rows**: `blackbox_records` table — same downsampled rows, raw CSV text for future detail analysis
- **Original file**: Raw .TXT archived as BLOB in `blackbox_files.file_data` for re-download/re-processing
- **Intermediate CSV**: Not persisted — parsed in-memory from stdout, discarded after import

Detailed replay-oriented DB field selection is documented in `docs/FLIGHTLOG_DATABASE.md`.

**DB schema** (migrations v1→v2→v3→v4→v5):
```sql
-- v2: blackbox tables + flights.source
ALTER TABLE flights ADD COLUMN source TEXT NOT NULL DEFAULT 'live';
CREATE TABLE blackbox_records (flight_id, timestamp_us, csv_data TEXT);
CREATE TABLE blackbox_files (flight_id, original_filename, log_index, file_data BLOB, ...);

-- v3: link quality in telemetry
ALTER TABLE telemetry_records ADD COLUMN link_quality INTEGER;

-- v4: replay-focused telemetry fields
ALTER TABLE telemetry_records ADD COLUMN baro_alt_m REAL;
ALTER TABLE telemetry_records ADD COLUMN gps_hdop REAL;
-- ... (18 columns total: nav state, flight modes, wind, RC arrays, sensor health)

-- v5: INAV navigation filter data
ALTER TABLE telemetry_records ADD COLUMN nav_lat REAL;   -- always NULL
ALTER TABLE telemetry_records ADD COLUMN nav_lon REAL;   -- always NULL
ALTER TABLE telemetry_records ADD COLUMN nav_alt_m REAL;  -- navPos[2]/100, fused altitude
```

**Use cases**:
1. **Standalone import**: .TXT → new flight entry with `source: "blackbox"`, metadata from BB header
2. **Multi-log**: Single .TXT with multiple ARM/DISARM sessions → probe first, user picks `--index`
3. **Attach to flight** (planned): Link BB to existing live-recorded flight → `source: "both"`, UI toggle between MSP/BB data

**Rationale**:
- External binary avoids maintaining a parallel Blackbox decoder as INAV evolves
- Pre-built index map + ColumnIndices struct gives O(1) per-row field access vs O(headers²) naive approach
- Downsampling keeps DB size manageable (5-min 500 Hz flight: 150K raw rows → ~3K stored)
- BLOB archive preserves original data for future re-processing with newer decoder versions
- ~50-160KB per platform — negligible size impact on distribution
- Child process isolation: decoder crash cannot bring down the GCS

---

## ADR-017: Weather Fetch at ARM Time

**Date**: 2026-04-16
**Status**: Accepted

**Context**: Weather and reverse geocoding were fetched lazily when the user opens a flight in the logbook UI. This fails for flights viewed offline or in areas without internet at viewing time. The data should be captured at recording time when GPS coordinates are first available.

**Decision**: Weather + geocode fetching runs at ARM time. When a flight starts (ARM transition detected in `recorder.rs`), `tauri::async_runtime::spawn` fires an async task that:
1. Opens a fresh SQLite connection (avoids contention with recorder's batch writes)
2. Reads the flight's `start_lat`/`start_lon` (from the INSERT)
3. Calls OSM Nominatim for reverse geocoding
4. Calls Open-Meteo for weather conditions
5. Writes results to the flight record

The existing `flightlog_geocode` and `flightlog_fetch_weather` Tauri commands remain as manual fallback for flights recorded before this change or where the network request failed at ARM time.

**Rationale**:
- Captures weather conditions at the actual time of flight, not at viewing time (which could be days/weeks later)
- Async spawn keeps the scheduler thread non-blocking
- Separate DB connection avoids contention with the recorder's batch writes
- Fallback commands preserve backward compatibility

---

## ADR-018: MSP Link Quality — MSP2_INAV_GET_LINK_STATS (Planned, INAV 9.x)

**Date**: 2026-04-17
**Status**: Planned

**Context**: Current RSSI data comes from `MSPV2_INAV_ANALOG`, which only provides a single RSSI value. For CRSF/ELRS setups, Link Quality (LQ) and SNR are more meaningful metrics. INAV PR [#11496](https://github.com/iNavFlight/inav/pull/11496) (targeting `maintenance-9.x`) introduces a new MSP2 message with dedicated link statistics.

**New message**: `MSP2_INAV_GET_LINK_STATS` — code `0x2103` (decimal 8451)

**Reply payload** (3 bytes):

| Field | Type | Unit | Description |
|---|---|---|---|
| `uplinkRSSI_dBm` | uint8_t | -dBm | Uplink RSSI, positive magnitude (70 = -70 dBm) |
| `uplinkLQ` | uint8_t | % | Uplink Link Quality (`rxLinkStatistics.uplinkLQ`) |
| `uplinkSNR` | int8_t | dB | Uplink SNR (`rxLinkStatistics.uplinkSNR`) |

**Decision**: Add `LinkStats` feature gate at `InavVersion >= 9.1` (exact version TBD once PR merges). When available:
- Add `MSP2_INAV_GET_LINK_STATS` to the Analog telemetry group (or a dedicated Link slot)
- Populate `link_quality` in `TelemetryRecord` (field already present since schema v3)
- Store `uplinkRSSI_dBm` (already available via `rssi` field) and `uplinkSNR` (new field, future schema v4)
- Fall back to `MSPV2_INAV_ANALOG` RSSI for firmware < 9.1

**Current state**: `link_quality` field is `None` for all MSP live recordings. Populated from `lq` column in Blackbox imports (ELRS/CRSF setups log it via `blackbox_decode --merge-gps`).

**Rationale**:
- Feature-gated: no code executed on older firmware, zero overhead
- `link_quality` column already in DB (schema v3) — no future migration needed for that field
- Consistent with existing adaptive degradation design: the new slot gets a priority just like the current Analog slot

---

## ADR-019: .kflight Flight Data Exchange Format

**Date**: 2026-04-18
**Status**: Accepted

**Context**: Users need to share flight records between KiteGC installations (e.g. club members, support requests). The internal SQLite database is not suitable for selective sharing — it contains all flights, is open/locked during use, and has no metadata envelope.

**Decision**: A custom `.kflight` file format for flight data exchange. Each file is a self-contained SQLite database with the same schema as the main DB, plus a metadata table.

**Format**:
- File extension: `.kflight`
- Internal format: SQLite database
- Schema: identical `flights`, `telemetry_records`, `blackbox_records`, `blackbox_files` tables
- Additional `_kflight_meta` table: `schema_version`, `app_id` ("KiteGC"), `exported_at`, `flight_count`
- `VACUUM` applied after export for minimal file size

**Export flow**:
1. User selects one or more flights (Ctrl+click multi-select or single selection)
2. Clicks "Export .kflight" → native Save dialog with `.kflight` filter
3. `exchange::export_flights()` creates a fresh SQLite file, copies selected flights with all associated data:
   - Flight metadata (`flights` row)
   - All `telemetry_records` for each flight
   - All `blackbox_records` for each flight
   - All `blackbox_files` BLOBs (original raw Blackbox binary) for each flight
4. `_kflight_meta` table written, database VACUUMed

**Import flow**:
1. User clicks "Import .kflight" → native Open dialog, or drag & drop `.kflight` file
2. `exchange::import_flights()` opens the `.kflight` file, validates `_kflight_meta`
3. Duplicate detection: matches `craft_name` + `start_time` within ±10 seconds
4. Non-duplicate flights copied into main DB with all associated data
5. Result dialog shows imported/skipped counts

**Blackbox raw file export**:
- Separate "Export Blackbox" button extracts the original raw binary file from `blackbox_files.file_data` BLOB
- Only available when `flight.source` is `"blackbox"` or `"both"` (greyed out otherwise)
- Writes to user-selected path via native Save dialog

**Logbook flight source indicators**:
- `◈` (open diamond) prefix = `source: "blackbox"` — imported from Blackbox file
- `◉` (filled circle) prefix = `source: "both"` — live recording with attached Blackbox
- No prefix = `source: "live"` — pure live MSP recording

**Multi-select**:
- Ctrl+click (or Cmd+click on macOS) toggles flights into a multi-selection set
- Normal click clears multi-selection and selects a single flight
- Export uses multi-selection if active, otherwise the currently selected flight

**Offline replay** (planned):
- `list_flights_in_file()`, `get_flight_from_file()`, `get_track_from_file()` — read directly from `.kflight` without importing into main DB
- Enables opening `.kflight` files as standalone viewers

**Key design choices**:
- SQLite-based: no custom binary format to maintain, standard tooling can inspect files
- Same schema: no format conversion needed during import/export
- BLOBs included: raw Blackbox files travel with the flight for re-processing
- Duplicate detection: prevents accidental double-imports

**Rationale**:
- Self-contained files are easy to share (email, USB, cloud)
- SQLite is universally supported and can be inspected with standard DB tools
- Including raw Blackbox BLOBs preserves the ability to re-decode with newer `blackbox_decode` versions
- The metadata table enables future format versioning

---

## ADR-020: Track Export — Raw GPS Position + Fused Nav Altitude

**Date**: 2026-04-18
**Status**: Accepted

**Context**: Users need to export flight tracks for visualization in Google Earth, planning tools, or post-flight analysis. INAV Blackbox logs contain both raw GPS coordinates (`GPS_coord[0/1]`) and navigation-fused position estimates (`navPos[0/1/2]`). Initial implementation tried to convert `navPos[0,1]` (local-frame NE offsets in cm relative to home) to geographic coordinates using the GPS home position. This conversion introduced visible track offset compared to reference tools like `flightlog2kml`.

**Key finding**: `navPos[0,1]` are **local North-East-Up centimeter offsets**, not geographic coordinates. Converting them to lat/lon using `home + offset / 111320` produces inaccurate results due to the local tangent plane approximation and accumulated EKF drift. The reference tool `flightlog2kml` uses raw GPS for position and only `navPos[2]` for altitude.

**Decision**: Always use raw GPS (`r.lat` / `r.lon`) for geographic position in all exports and map displays. Use `navPos[2] / 100` (fused altitude relative to home) for altitude only, with baro fallback.

**Export module** (`track_export.rs`):

| Format | Content |
|---|---|
| **KMZ** | Zipped KML (via `zip` crate v2, deflate compression) |
| **KML** | `<LineString>` with `<altitudeMode>relativeToGround</altitudeMode>`, red track line |
| **GPX** | Standard `<trk>/<trkseg>/<trkpt>` with `<ele>` and `<time>` |
| **CSV** | All telemetry columns including raw nav fields for analysis |

**Track processing pipeline**:
1. **Filter valid GPS**: Remove records without lat/lon or with invalid (0,0) coordinates
2. **Spike filter**: Remove points with impossible speed (> 150 m/s via haversine from previous point)
3. **Douglas-Peucker 3D simplification**: ε = 0.5 m, reduces hover jitter while preserving path shape
4. **Altitude selection**: `nav_alt_m` → `baro_alt_m` → `0.0` (fused preferred, baro fallback)

**Altitude source hierarchy** (`best_alt_relative()`):
```
nav_alt_m     ← navPos[2] / 100  (INAV EKF fused, relative to home, smooth)
baro_alt_m    ← BaroAlt           (barometric, relative to home)
0.0           ← fallback
```

**Position source** (all exports and statistics):
```
r.lat / r.lon ← raw GPS coordinates (always, no nav fallback)
```

**UI integration**:
- "Export Track" button in LogbookPanel detail view, visible for all flights with telemetry
- Unified native Save dialog with format filter (KMZ/KML/GPX/CSV)
- Format auto-detected from file extension

**What nav_lat/nav_lon are NOT used for**:
- Track export position (KML/KMZ/GPX/CSV coordinates)
- Map polyline display (Map.svelte uses `point.lat` / `point.lon`)
- Flight statistics (start position, total distance, max distance from home)
- telemetryAdapter position output

**DB columns**: `nav_lat` and `nav_lon` remain in schema v5 but are always `NULL` (the local→geographic conversion was removed). `nav_alt_m` is actively populated and used.

**Rationale**:
- Raw GPS matches reference tools (flightlog2kml) — proven correct on real flight data
- navPos[0,1] local frame conversion introduces systematic offset (verified in Google Earth comparison)
- navPos[2] altitude is genuinely useful — smoother than raw GPS altitude (which has 1m integer steps)
- Keeping nav columns in schema allows future use if a better conversion method is found
- Douglas-Peucker + spike filter produce clean tracks without requiring fused position data

---

## ADR-021: CesiumJS 3D Globe View

**Date**: 2026-04-18
**Status**: Accepted

**Context**: A 2D Leaflet map is insufficient for understanding 3D flight paths, terrain clearance, and altitude relationships. Users want a 3D "follower view" for monitoring and log replay — not a Google Earth replacement, but a functional UAV monitoring perspective.

**Considered**: MapLibre GL (2.5D), Three.js + custom globe, CesiumJS, deck.gl

**Decision**: CesiumJS (Apache 2.0 license) as an optional 3D view alongside the existing Leaflet 2D map.

**Architecture**:
```
+page.svelte
├── mapViewMode: '2d' | '3d'   (toggle button)
├── Map.svelte                   (Leaflet, existing)
└── Map3D.svelte                 (CesiumJS, new)
    ├── createCachedImageryProvider()   — shared tile cache
    ├── updatePlaybackTrack3D()         — geoid-corrected track
    ├── updateChaseCamera()             — lerp-smoothed follow cam
    └── waitForTerrain()                — async terrain readiness
```

**Vite integration**: Custom `cesiumPlugin()` in `vite.config.js`:
- **Dev**: sirv middleware serves `/cesium/*` from `node_modules/cesium/Build/Cesium/`
- **Build**: `writeBundle` hook copies Cesium assets via `fs.cpSync`
- Replaces `vite-plugin-static-copy` (assets "collected" but not served) and `vite-plugin-cesium` (URL-encoding bug with spaces in workspace path)

**Tile caching**: Same `IndexedDB` cache shared with 2D Leaflet view:
- `requestImage` overridden on `UrlTemplateImageryProvider`
- Routes through `getCachedTile()` → `fetchAndCacheImage()` → `putCachedTile()`
- `errorEvent.addEventListener(() => {})` silences tile errors — parent tiles remain visible
- Per-provider `cesiumMaxZoom` limits (e.g. ESRI: 17) prevent "No tiles available" placeholder images

**Altitude correction** (geoid undulation):
- GPS `alt_m` is MSL (Mean Sea Level). CesiumJS expects WGS84 ellipsoid height.
- Difference ≈ geoid undulation (e.g. ~40m in Central Europe, ~25m in Scandinavia).
- Fix: `sampleTerrainMostDetailed()` at first track point → `geoidOffset = terrainHeight - groundMsl`
- Applied to all positions (track, markers, live UAV).
- Must wait for `Cesium.Terrain.fromWorldTerrain()` to finish loading (async) — `waitForTerrain()` listens for `terrainProviderChanged` event.

**Chase camera smoothing**:
- Direct `camera.lookAt()` from telemetry produces jerky 1° heading snaps.
- Solution: `requestAnimationFrame` loop with exponential lerp (`CHASE_SMOOTHING = 0.07`).
- Position: `lerp(current, target, 0.07)` per frame for lat, lon, alt.
- Heading: `lerpAngle()` with shortest-path wrap (handles 359°→1° via `((diff + 540) % 360) - 180`).
- First update snaps immediately (no lerp from 0,0).

**Performance optimizations**:
- `requestRenderMode: true` — only re-renders when scene changes (reduces idle GPU)
- `scene3DOnly: true` — disables 2D/Columbus View mode switching overhead
- `fog.density: 2.5e-4` — hides distant terrain, reduces tile loading
- `tileCacheSize: 100` — limits RAM usage from terrain/imagery tiles
- MSAA 2× — balanced quality vs performance

**Rationale**:
- CesiumJS is the only production-grade open-source 3D globe with terrain support
- Apache 2.0 license is compatible with GPLv3
- Cesium Ion free tier provides World Terrain (15m resolution) — sufficient for UAV monitoring
- Shared tile cache means 3D view benefits from previously cached 2D tiles
- Optional view: 2D map remains the primary interface; 3D is for visualization/monitoring
- Custom Vite plugin avoids all dependency issues we hit with third-party plugins

---

## ADR-022: Separate Flight Recording from Flight Logbook

**Date**: 2026-04-21
**Status**: Accepted

**Context**: The original "Flight Logging" toggle (`flightLogEnabled`) conflated two distinct features:
1. **Flight Recording** — raw telemetry stream capture to protocol-native files (future: MWP .raw for MSP, .tlog for MAVLink)
2. **Flight Logbook** — structured SQLite database entries (flights table, telemetry_records, metadata) for logbook browsing, search, and replay

Users may want recording without DB overhead, or DB logging without raw file capture. The upcoming multi-protocol architecture (ADR-010) requires protocol-native raw logs that are independent of the DB pipeline.

**Decision**: Split into two independent toggles in Settings:

| Setting | Store field | Rust field | Purpose |
|---|---|---|---|
| **Flight Recording** | `flightRecordingEnabled` | `raw_enabled` | Raw stream capture to protocol-native files |
| **Flight Logbook** | `flightLoggingEnabled` | `db_enabled` | SQLite database entries for logbook |

**Frontend changes**:
- `SettingsPanel.svelte`: Two separate toggles with clear labels
- `settings.ts` store: `flightRecordingEnabled` (default: true) + `flightLoggingEnabled` (default: true)
- Logbook tab in NavRail hidden when `flightLoggingEnabled` is false
- `+page.svelte`: Passes both flags independently to Tauri backend

**Backend changes**:
- `FlightLogSettings` struct: `db_enabled: bool` + `raw_enabled: bool` (future)
- DB schema v4→v5: `flights.craft_name` column for user-editable craft names
- Craft name editing: `flightlog_update_craft_name` Tauri command

**Recording pipeline** (future, see `docs/archive/PROTOCOL_REFACTORING.md`):
- Raw-first: recording starts on ARM/connect, writes protocol-native bytes
- DB import happens after DISARM/disconnect (post-processing)
- Crash-safe: raw file survives even if app crashes during flight

**Rationale**:
- Clean separation of concerns: raw capture ≠ structured database
- Enables raw-only mode for minimal overhead during flight
- Aligns with multi-protocol architecture: each protocol has its own raw format
- Logbook UI makes no sense when DB logging is disabled → hide the tab

---

## ADR-023: CSS Grid Zone Layout System

**Date**: 2026-04-20
**Status**: Accepted
**Supersedes**: ADR-005 (layout structure only; floating panel concept is retained)
**Updates**: ADR-011 (widget sizing changed from `vmin` to container-relative `px`)

**Context**: The original layout used `position: absolute` for all overlay elements (panels, widgets, status bar) with hardcoded viewport-based calculations. This caused several problems:
1. Widget sizing used `vmin` (= `min(viewport-width, viewport-height)`) — both dock panels scaled together when either viewport dimension changed
2. Bottom widgets shrank below dock height when viewport height decreased (because `vmin` tracked the shorter dimension)
3. Panel width/height limits used hardcoded pixel offsets (`calc(100vw - 86px)`) that didn't account for dynamic dock sizes
4. No formal zone boundaries — elements overlapped unpredictably at extreme aspect ratios

**Decision**: Replace ad-hoc absolute positioning with a CSS Grid layout on the `.app` container. The grid defines 7 named zones with fixed and flexible sizing. Widget sizing is converted from viewport-relative `vmin` to container-relative `px`.

**Grid structure** (4 columns × 4 rows):

```
┌───────────┬───────────────────────────┬───────────────┬──────────┐
│           │                           │               │          │
│  TOOLBAR  │         TOOLBAR           │   TOOLBAR     │ TOOLBAR  │
│  (62px)   │         (1fr)             │  (clamp)      │  (54px)  │
├───────────┼───────────────────────────┼───────────────┤──────────┤
│           │                           │               │          │
│ NAV RAIL  │      PANEL ZONE           │  SIDE DOCK    │SIDE DOCK │
│  (62px)   │        (1fr)              │(150-250px)    │          │
│           │                           │               │          │
├───────────┼───────────────────────────┤───────────────┼──────────┤
│           │                           │               │          │
│ NAV RAIL  │     BOTTOM DOCK           │ BOTTOM DOCK   │MAP CTRL  │
│  (62px)   │   (184-300px tall)        │               │  (54px)  │
│           │                           │               │          │
├───────────┼───────────────────────────┼───────────────┼──────────┤
│           │                           │               │          │
│STATUS BAR │      STATUS BAR           │  STATUS BAR   │STATUS BAR│
│  (24px)   │                           │               │          │
└───────────┴───────────────────────────┴───────────────┴──────────┘
```

**CSS Grid definition**:
```css
grid-template-rows:    53px 1fr var(--grid-bottom-height) 24px;
grid-template-columns: 62px 1fr var(--grid-side-width)    54px;
grid-template-areas:
  "toolbar      toolbar      toolbar      toolbar"
  "nav-rail     panel        side-dock    side-dock"
  "nav-rail     bottom-dock  bottom-dock  map-controls"
  "status-bar   status-bar   status-bar   status-bar";
```

**Zone descriptions**:

| Zone | Grid Area | Size | Purpose |
|------|-----------|------|---------|
| **Toolbar** | Row 1, Col 1–4 | 53px fixed | Logo, sensor bar, port selector, connect button |
| **Nav Rail** | Row 2–3, Col 1 | 62px fixed | Hamburger menu + vertical tab icons |
| **Panel Zone** | Row 2, Col 2 | 1fr × 1fr | Floating panels (Settings, UAV Info, Logbook, Mission) |
| **Side Dock** | Row 2, Col 3–4 | clamp(150px, 15vw, 250px) | Vertical widget strip |
| **Bottom Dock** | Row 3, Col 2–3 | clamp(184px, 20vh, 300px) | Horizontal widget strip |
| **Map Controls** | Row 3, Col 4 | 54px fixed | Zoom, 3D toggle, compass buttons |
| **Status Bar** | Row 4, Col 1–4 | 24px fixed | Connection status, arming state |
| **Map** | Row 2–3, Col 1–4 | Full area (z-index 0) | Leaflet/CesiumJS map behind all zones |

**Layout store** (`src/lib/stores/layout.ts`):
- `LayoutProfile` type: `'flight' | 'mission' | 'area-planner'`
- `ZoneDock` interface: `{ visible: boolean; sizeOverride: string | null }`
- `GRID_DEFAULTS` constants for default clamp values
- Methods: `setProfile()`, `setBottomDockVisible()`, `setSideDockVisible()`, `setBottomDockHeight()`, `setSideDockWidth()`
- CSS custom properties `--grid-bottom-height` and `--grid-side-width` driven by store

**Widget sizing — container-relative px** (replaces `vmin`):

The old `vmin` approach meant `1vmin = min(viewport-width, viewport-height) / 100`, which coupled both docks to the same dimension. The new approach:

```
Container cross-axis (bind:clientWidth/Height)
    ÷ LARGE_BASE_VMIN (22.5)
    = pxPerUnit (unique per dock)

Widget size (abstract units from computeSizes())
    × pxPerUnit
    = sizePx (CSS: --ws: {sizePx}px)
```

- **Bottom dock**: `pxPerUnit = (dockHeight - padding) / 22.5` → large widget fills dock height exactly
- **Side dock**: `pxPerUnit = (dockWidth - padding) / 22.5` → large widget fills dock width exactly
- Docks scale independently — changing viewport width doesn't affect bottom dock widget sizes
- Zone padding (6px per side) provides gap between widgets and edges

**Floating panels constrained to Panel Zone**:

Panels remain `position: absolute` (floating overlay pattern) but their `max-height` and `width` are now derived from grid zone variables:
```css
max-height: calc(100vh - 53px - var(--grid-bottom-height) - 24px - 12px);
width: min(360px, calc(100vw - 62px - var(--grid-side-width) - 54px - 12px));
```
This ensures panels never overflow into the bottom dock, side dock, or map controls — regardless of dock size configuration.

**Rationale**:
- CSS Grid provides declarative, maintainable zone boundaries without manual pixel math
- Named grid areas make the layout self-documenting and easy to reconfigure per profile
- Container-relative widget sizing eliminates all viewport coupling between docks
- `clamp()` on dock sizes provides responsive behavior with hard min/max limits
- Zone padding gives widgets breathing room without additional wrapper elements
- Layout store enables future profile switching (e.g. hide side dock in mission mode)
- Map always fills the full content area behind all zones (z-index 0) — no wasted space

---

## ADR-024: Survey Pattern Generator — Frontend-Only Pure Functions + Rune Store

**Date**: 2026-04-22
**Status**: Accepted
**Extended by**: ADR-025 (per-shape geometry algorithms — all six shapes now functional)

**Context**: The survey pattern generator needs to let users define geometric patterns (rectangle, polygon, circle, etc.) on a map, generate waypoints from those patterns, and append them to the active mission. Unlike the existing mission system (which relies on the Rust backend for state management and MSP communication), the pattern generator is a pure frontend feature — no FC communication is needed during pattern editing.

Key questions:
1. Where should pattern state live? (Backend? Frontend? Both?)
2. How should the pattern be rendered on the map?
3. How should generated waypoints be added to the mission?

**Options considered**:

1. **Backend-owned state** (like mission module): Pattern params stored in Rust `Mutex`, frontend reads via Tauri events/commands. Adds IPC overhead for every parameter change (slider drag, marker drag). Real-time map preview would require state sync on every mouse move.

2. **Frontend-only rune store**: Pattern state lives in a Svelte 5 `.svelte.ts` rune module. Geometry computation and waypoint generation are pure TypeScript functions. Only the final "append to mission" step uses the existing mission backend commands.

3. **Both**: Backend stores the config for persistence, frontend has a working copy for interactive editing. Adds sync complexity.

**Decision**: **Option 2 — Frontend-only rune store + pure functions.**

**Rationale**:

- **No IPC latency**: Parameter changes during map interaction (dragging a corner marker updates length/width/center 30–60 times/second) must be instantaneous. IPC round-trips would introduce visible lag.
- **No backend logic needed**: Pattern geometry is pure math (trigonometry, coordinate transforms) — no MSP commands, no protocol dependency. Rust has no advantage here.
- **Only two backend touchpoints**: (a) `missionAddWp()` via existing `mission.ts` store for appending generated WPs, (b) future `saveSurveyPattern()`/`loadSurveyPattern()` for persistence.
- **PatternGenerator.md already defined this**: The phased plan always specified a frontend-only helper file.

**Implementation**:

1. **`surveyPattern.svelte.ts`** (rune store): `$state` for `activeSurveyPattern { config, isActive }`. Functions: `enterPatternMode()`, `exitPatternMode()`, `updateRectangleParams()`, `applyRectangleDragUpdate()`. Config persists between mode toggles (cleared on app close).

2. **`surveyPatterns.ts`** (pure helpers): `LngLat`, `SurveyWaypoint`, `SurveyPathSegment`, `RectangleCorners`, `generateRectangleZigzag()`, `generateClassicZigzag()`, `computeRectangleCorners()`, `updateRectangleFromDraggedCorner()`, `updateRectangleFromDraggedCenter()`.

3. **`SurveyPatternLayer.svelte`** (map component): Renders shape polygon (gray semi-transparent), path preview (blue survey lines + turn connections), draggable corner/center markers. Uses Leaflet `L.polygon`, `L.polyline`, `L.circleMarker`. Reads directly from `activeSurveyPattern` rune store for reactivity.

4. **`SurveyPatternPanel.svelte`** (UI component): Parameter inputs using `NumberStepper`. Altitude type dropdown, user action trigger checkboxes. Shape selector dropdown — all six shapes (rectangle, rectangle-lawnmower, polygon, polygon-lawnmower, circle, spiral) are functional (see ADR-025). Generate button with 120 WP limit check via `ConfirmDialog`.

5. **Deduplication**: Turn connections duplicate survey endpoints. During generation, consecutive identical lat/lng points are skipped. User action flags from the survey end point are preserved.

6. **P3 encoding**: `altMode` → bit 0 (0=REL, 1=AMSL), `userActionFlags` → bits 1–4 (shifted: `(flags & 0x0F) << 1`), matching INAV's `P3_USER_ACTION_1..4` bit positions in `mission/types.rs`.

**Key design choices**:

- **SurveyPathSegment kind**: `'survey'` vs `'turn'` — survey segments are the actual flight lines (with start/end flags). Turn segments are visual-only connectors (no waypoints generated from them).
- **Track orientation**: When enabled, tracks are rotated independently from the shape. Tracks are clipped to the shape boundary (intersection math). When disabled, tracks follow `shapeOrientation`.
- **Reverse**: Swaps start/end of the flight path without changing the track direction.
- **Turn Distance**: Extends the outbound leg beyond the shape boundary. Only affects generated waypoints, not the shape polygon.
- **No re-editing after generation**: Once waypoints are appended, pattern config is preserved (for re-entry with same params) but there is no link back to the original pattern. Users can edit individual waypoints freely.

**Map layer integration**:

- `Map.svelte` includes `<SurveyPatternLayer {map} />` unconditionally (it clears itself when not active).
- `InavMissionLayer` blocks new WP placement via `if (activeSurveyPattern.isActive) return;`.
- `InavMissionPanel` conditionally renders `SurveyPatternPanel` instead of the WP table when `showPatternPanel` is true.
- FC upload/download/save/load buttons are hidden while pattern mode is active (`{#if !showPatternPanel}`).

## ADR-025: Survey Pattern Generator — Per-Shape Geometry Algorithms

**Date**: 2026-05-29
**Status**: Accepted
**Extends**: ADR-024

**Context**: ADR-024 established the frontend-only architecture (rune store + pure functions) with rectangle + rectangle-lawnmower. The remaining four shapes (circle, spiral, polygon, polygon-lawnmower) each need a distinct generation algorithm. A first attempt that shared parameters and config-building logic across shape families caused state corruption on shape switching (e.g. a circle inheriting rectangle params with no `radius`). This ADR records the clean per-shape separation and the geometry algorithms.

**Decision**: Strict separation by shape family, with one pure generator per shape and no cross-shape code reuse. All geometry runs in a local equirectangular metre frame around the shape centroid (so angles/distances aren't distorted by longitude compression), converting back to lat/lng only at the end.

### Shape families & state

| Family | Shapes | Params type | Generators |
|---|---|---|---|
| rect | `rectangle`, `rectangle-lawnmower` | `RectanglePatternParams` | `generateRectangleZigzag`, `generateClassicZigzag`, `generateRectangleLawnmower` |
| circle | `circle`, `spiral` | `CirclePatternParams` | `generateCircleStepped`, `generateSpiral` |
| polygon | `polygon`, `polygon-lawnmower` | `PolygonPatternParams` | `generatePolygonZigzag`, `generatePolygonLawnmower` |

- **`switchShape()`** caches the current family's params before switching and restores the target family's cached params (or builds defaults). Same-family switches only rename `shape`, preserving all params.
- **Panel state** is per-family: `rectangleParams` / `circleParams` / `polygonParams` are independent `$state` objects, each with its own sync `$effect` and change handler. No shared `Record<string, any>`.
- **Layer reactivity** is a single `$effect` (avoids the double-render of two effects sharing a `$state` flag); `prevShape` is a plain `let`.

### Circle (Stepped) — `generateCircleStepped`

Concentric rings from `radius` inward, spaced by `targetLineSpacing`. Each ring uses up to `ringPoints` evenly-distributed waypoints, auto-reduced when the arc between points would fall below `targetLineSpacing`. When even the minimum (3 points) is too dense, a single centre point closes the path. `trackOrientation` = bearing of the first waypoint; `clockwise` = orbit direction; `reverse` = inside↔outside.

### Spiral — `generateSpiral`

Archimedean spiral: radius decreases linearly with total angle turned. **Outer phase** uses a fixed angular step (360°/`ringPoints`); **inner phase** widens the step so each arc stays ≥ `targetLineSpacing`. Two stop conditions: (a) the interior angle at the previous waypoint drops below 120° (UAV turn > 60° — impractical), (b) the next arc would be shorter than `targetLineSpacing`. Always terminates with a waypoint at the exact centre.

### Polygon ZigZag — `generatePolygonZigzag`

Scanline sweep perpendicular to `trackOrientation`. For each scan line, all polygon-edge intersections are found, sorted, and paired (entry, exit) by the even-odd rule — concave shapes naturally yield multiple segments per line. Two concave modes (`stayInsideArea`):
- **false (cross-gap)**: serpentine — fly every segment of each scan line in order, crossing intra-polygon gaps. Turn-distance extension is applied only to the last segment of each scan line (the real turn to the next line), never to collinear cross-gap segments.
- **true (connected-fill)**: DFS over segments connected across adjacent scan lines (Y-overlap), staying within connected sub-regions like 3D-printer infill (U-shape → left arm → bottom → right arm).

Waypoint-frame note: track-frame coordinates `(perp, along)` are rotated back to ENU before lat/lng conversion, so `trackOrientation` rotates the *scan lines*, not the polygon.

### Polygon Lawnmower — `generatePolygonLawnmower`

Contour-offset coverage of an arbitrary (concave) polygon. The pinch-off/island problem is avoided by **convex pre-decomposition**:

1. **`decomposeConvexXY`** — recursive reflex-cut: at each reflex vertex find a valid internal diagonal (no edge crossing, midpoint inside), preferring a diagonal to another reflex vertex; split and recurse until all pieces are convex. Convex pieces only ever shrink under offset — they never pinch off.
2. **`mergeConvexPiecesXY`** — Hertel-Mehlhorn: merge adjacent pieces sharing an edge whenever their union stays convex (re-combines two triangles into a quad → fewer pieces, cleaner paths).
3. **`offsetConvexInwardXY`** — inward offset by half-plane intersection (Sutherland-Hodgman clipping against each edge shifted inward). Robust by construction: clipping a convex polygon can never self-intersect, and collapsed edges drop out automatically. Returns `null` on collapse. This replaced an earlier miter-intersection offset that overshot on sharp vertices.
4. **Per zone (= per convex piece)**: offset to collapse → concentric rings; `removeShortEdgesXY` drops waypoints that would create sub-`lineSpacing` tracks (tiny inner rings disappear); `spineOfConvexXY` adds a final medial-axis line for elongated remainders (via binary-search to near-collapse, then the two farthest residual points). Rings are flown **open** and each inner ring is entered one vertex past the nearest point → diagonal inward steps, no re-flown waypoints. A zone is one continuous survey segment; **turn (transfer) legs occur only between zones**.

### Shared conventions

- **User-action flags** are assigned in final flight order (after any `reverse`), so start/end flags land on the correct waypoints. Lawnmower/circle/spiral/polygon-lawnmower use Start/Track/End; zigzag (rectangle/polygon) uses Line-Start/Line-End.
- **`SurveyPathSegment.kind`**: `'survey'` points become mission waypoints; `'turn'` points are visual-only connectors.
- **120 WP limit** is checked at generation with a live, reactive count in the panel (red over limit).

**Rationale**:
- Per-shape generators keep each algorithm independent and debuggable — a change to one shape can't corrupt another.
- Convex pre-decomposition sidesteps general polygon-offset topology handling (islands, holes) — each piece is a simple convex shrink, which is trivially robust.
- Half-plane clipping is the minimal robust offset primitive; no external geometry library (e.g. Clipper) was needed for the supported scale (≤ 50 vertices).

---

## ADR-026: Terrain Elevation Provider & AGL Waypoints

**Date**: 2026-05-30
**Status**: Accepted

**Context**: Mission planning needs terrain elevation for four features: AGL waypoint planning, terrain clearance validation, a live AGL widget, and LOS analysis. The 3D map already uses Cesium World Terrain, but that is ellipsoid-referenced (needs a geoid-undulation hack), online/token-gated, and visual-only. The 2D/planning path needs an accurate, offline-capable, MSL-referenced source and a clean sampling abstraction.

### Part A — Elevation source & provider

**Decision**: **Copernicus DEM GLO-30** via AWS Open Data (`copernicus-dem-30m`, Cloud Optimized GeoTIFF, 1°×1° tiles, no API key), sampled by a **Rust backend** module (`src-tauri/src/terrain/`).

**Rationale**:
- Chosen over SRTM / Mapzen-Terrarium: ~±4 m vs ~±9 m RMSE, global coverage **including > 60°N** (SRTM/Terrarium have none), and **geoid-referenced (EGM2008 ≈ MSL)**.
- Geoid ≈ MSL means GLO-30 elevation, GPS altitude, and INAV AMSL waypoints are directly comparable — **no geoid conversion** (the key simplification vs Cesium's ellipsoid terrain).
- Backend (not frontend): GeoTIFF parsing + tile cache fit Rust; one provider serves multiple frontend features without IPC duplication.

**Implementation**: `tile_name(lat,lon)` → HTTPS fetch → disk cache (portable-aware) → `tiff`-crate decode (Float32, DEFLATE, floating-point predictor; geo-transform from `ModelPixelScale`/`ModelTiepoint`) → in-memory LRU (4 tiles) → bilinear sample. **CPU decode + 42 MB disk I/O run on `spawn_blocking`** so the async runtime is never stalled (critical on weak hardware); tile loads are serialized + cache-rechecked via an async lock to coalesce concurrent requests. Commands: `terrain_elevation`, `terrain_profile`, `terrain_fan` (polar grid — see Part E).

**Follow-up**: full-tile decode is multi-second on weak hardware. The planned optimization is **COG partial reads** — HTTP range requests + per-chunk decode of only the blocks covering the queried points — turning multi-second decodes into sub-100 ms.

### Part B — AGL waypoints

**Context**: INAV waypoints encode only REL (p3 bit0=0) or AMSL (p3 bit0=1) — there is **no AGL flag** (verified against INAV docs and mwp). AGL must therefore be a GCS-only authoring concept.

**Decision**: Add `alt_mode` (0=REL, 1=AMSL, 2=AGL) to the backend `Waypoint`; **resolve AGL → AMSL at export** using the terrain provider.

- `alt_mode` is authoritative for the GCS; for REL/AMSL it mirrors p3 bit0 (derived from p3 on MSP/XML decode). AGL holds an above-ground value in `altitude`.
- **Export resolution** (`resolve_agl`, async): for each AGL waypoint, `AMSL = terrain(lat,lon) + AGL`, set p3 bit0=1. Applied in `mission_save_file` / `mission_export_xml` / `mission_upload` before serialization/upload. **Not round-trippable** — a loaded/downloaded mission returns as AMSL.
- **Editor**: the alt-mode toggle cycles REL→AMSL→AGL and converts the value (via terrain + the launch point) so the physical height is preserved. Survey patterns expose AGL via the `ground` option.
- **Why export-time, backend-side**: terrain is async and lives in the backend; keeping the conversion at the export boundary avoids per-edit terrain calls in the serializer and keeps the stored mission in the user's chosen mode.

**Scope — INAV only (ArduPilot/PX4 AGL is TBD)**: AGL support currently covers the **INAV mission path** (MSP WP / MW XML). The ArduPilot/PX4 mission path (separate MAVLink WP implementation, `missionArdupilot.ts` + `ArduMission*`) is still rudimentary due to lack of test hardware, and **AGL compatibility there is not implemented**. Note that ArduPilot/PX4 have a *native* terrain-follow altitude frame (`MAV_FRAME_GLOBAL_TERRAIN_ALT`), so a future implementation would likely map AGL to that frame directly rather than resolving to AMSL — to be decided once test hardware is available.

### Part C — Launch / home reference

A planning-time launch point (frontend `launchPoint` store) is the home-altitude reference for REL↔AGL conversion and (future) REL-mission clearance. Auto-placed on entering edit mode (FC home → first geo-WP → map center), shown as an always-visible draggable marker with a connector to the first waypoint. **Persisted in the `.mission` file** via the mwp-compatible `<mwp home-x="lon" home-y="lat">` meta element (`Mission.home`): written on save/export, parsed on load/import (overriding the current launch point). Other tools (INAV Configurator) ignore the element and read only `<missionitem>`, so this stays inter-app compatible.

**Validation**: an AGL survey pattern exported to `.mission`, loaded into INAV Configurator, showed consistent terrain-relative altitude across all waypoints in its terrain analysis.

### Part D — Terrain Analysis panel (elevation profile)

**Context**: Mission planners need to *see* terrain clearance, like INAV Configurator / mwp. mwp shells out to an external tool for the graph; we require **no external runtime dependency**.

**Decision**: A **full-width, viewport-centered overlay** opened from the NavRail (not a narrow side panel — a profile is wide/short by nature), rendering a **hand-rolled SVG** side-view. Behaves like a floating panel (the nav rail stays open; mutually exclusive with the nav panel content; the X hides all). Built entirely on the existing `terrain_profile` command + the frontend altitude pipeline.

- **Data** (`helpers/terrainProfile.ts`): one `terrain_profile` call per route at 30 m spacing; waypoint altitudes resolved to absolute MSL via terrain + the launch point. Two builders — Waypoint (planned mission) and Track (flown live temp-log / loaded blackbox). All MSL (Copernicus EGM2008), consistent with FC GPS + AMSL waypoints.
- **State** (`stores/terrainAnalysis.ts`): in-memory session store (survives close/reopen, not persisted to disk). Profiles cached per mode by signature → instant Waypoints↔Track switching.
- **Chart** (`TerrainProfileChart.svelte`): explicit pixel scales (no SVG `viewBox`) so axis labels stay crisp; wheel-zoom / drag-pan on the X domain. **Rendering scales with zoom** — only the visible distance slice is drawn, decimated to ~screen resolution via a per-bucket worst-clearance / peak-terrain envelope (peaks + unsafe spots survive); full-resolution data still drives the readouts.
- **Analysis nuances**: min-clearance trims leading/trailing below-clearance runs (take-off/landing on the ground) so they don't false-alert; track climb angle is low-pass filtered against sensor jitter; interior void terrain samples are bridged by interpolation.
- **Chart ↔ map link** (Compact mode): a `terrainCursor` store + `TerrainCursorLayer` mirror the chart cursor onto the 2D map — a transient hover dot plus a click-pinned persistent marker that **persists when the panel is closed** (reference while editing in mission control). Visual-only; 2D Leaflet for now (3D follows the later Cesium rework).
- **Why frontend/SVG**: the profile is presentation built on an existing backend command; an SVG component matches the widget stack, is themeable and natively interactive, and avoids any charting dependency.

**Terrain Correction (Phase 2)**: a pure-function engine (`helpers/terrainCorrection.ts`) over the same `ProfileData`. *Terrain Follow* sets correctable WPs (Waypoint + PosHold in the range) to a target AGL; *Clearance Check* only raises. A monotonic convergence loop raises WP/leg clearance and (optional fixed-wing) the lower endpoint of any too-steep climb/descent leg. Land/RTH/Jump/SetHead and out-of-range WPs are fixed anchors. Insertion is **manual** (*Add WP* at a pinned chart marker, on the track). A live green preview (drawn *behind* the path) precedes an APPLY confirm dialog; corrected WPs are written in **AGL** mode.

**Jump simulation**: `expandRoute()` simulates one loop per jump (`4J2` → branch `4→2`, cut, resume `4→5`); revisits carry no extra WP dots. The cut is a break in terrain/path/clearance + a marker; the jump-back leg is coloured like the map and ends in a target marker. Correction keys altitude **per WP index** (one shared `Cell` across revisits), so the jump-back leg constrains the same WP as its first-pass legs; cut legs are skipped. Jump target resolves as `p1 − 1` (matching the map).

### Part E — Live terrain widgets (Live AGL + Terrain Radar)

Two HUD widgets reuse the provider for in-flight terrain awareness. Both are **driven by the telemetry frame**, self-throttled (a frame is dropped while a backend sample is in flight), re-sample only on meaningful change, and share the speed-driven forward-distance scale (300/900/1800/3600 m with boundary hysteresis).

- **Live AGL** (`liveAgl`, 2×1 `wide`): a side-view forward-looking HUD on the existing `terrain_profile` command. History is accumulated **internally from the telemetry stream** (not the armed-only `liveTrack` store) so it works on a live link **and** during replay. Forward terrain is one heading-projected `terrain_profile` segment; the dashed flight line uses the averaged FC vario.
- **Terrain Radar** (`terrainRadar`, 1×1 `large`): a top-down, track-up EGPWS-style 120° fan. Needs 2-D coverage, so it adds **one new command — `terrain_fan(lat, lon, heading, half_angle, range, ang_cells, rad_cells)`** — server-side polar sampling over the tile cache (one IPC call per refresh, vs N radial `terrain_profile` calls). Clearance is coloured on a continuous ramp against either current MSL or a sink-angle prediction; an SVG turbulence/displacement filter gives the heatmap texture. Its clearance colour scale is a **dedicated setting** (60/120/250 m), intentionally separate from the planning `groundClearance`.

**Why a new command for the radar but not the AGL widget**: the AGL widget samples a 1-D polyline (`terrain_profile` fits); the radar samples a 2-D fan, where doing it as N frontend ray-calls would be N× the IPC + redundant tile locking — one backend command that walks the polar grid against the already-resident tile is the clean fit.

---

## ADR-027: Mission Undo/Redo — Frontend Snapshot History

**Date**: 2026-05-31
**Status**: Accepted

**Context**: The mission editor mutates state from many entry points — list/map single edits, marker drag, multi-select batch delete, the Batch Edit popup (N waypoint updates), *Move to mission* (cross-mission), survey-pattern append (N adds), terrain correction (N updates), and multi-mission add/remove. Users need undo/redo, and the batch features were deliberately built around a **single APPLY** (no live-apply) to make this tractable. Two questions drove the design: *what* to snapshot, and *where* the boundary of "one undo step" sits.

**Decision**: A **frontend, snapshot-based history** (`stores/mission.ts`) over a **two-stack** model (`undoStack` / `redoStack`, limit 50). Mission state lives in the backend (ADR-013), but undo orchestration is a frontend concern, so the history lives with the frontend mirror.

### What a snapshot covers — all missions

A snapshot captures the **entire multi-mission state**: `activeMissionIndex`, `missionCount`, and a deep copy of **every** mission's waypoints (the active one read live from the `mission` store, the rest from the `missionSlots` cache). This is what makes **cross-mission *Move to mission* undoable** — a narrower "active mission only" snapshot couldn't restore waypoints that moved to another slot. The **launch point is excluded**: it's a planning reference, touched rarely, and not part of what gets uploaded to the FC (the guiding scope — "undo only what reaches the FC").

### One step = one user action — the suspend-group pattern

The granularity problem: a batch of N waypoint updates must be **one** undo step, not N. The mechanism:

- The **primitive** store mutators (`missionAddWp` / `InsertWp` / `RemoveWp` / `UpdateWp` / `ReorderWp` / `missionClear`) call `pushUndo()` at entry, which snapshots the *pre-mutation* state and clears the redo stack.
- A module-level **`undoSuspend` counter** gates `pushUndo()` — it's a no-op while `> 0`.
- Multi-step actions wrap their primitives in **`beginUndoGroup()`** (records one snapshot, then `undoSuspend++`) and **`endUndoGroup()`** (`undoSuspend--`). All inner primitive `pushUndo()` calls are suppressed, so the whole action collapses to the single snapshot taken at group start.

This keeps the recording logic in one place: a primitive called standalone records itself; the same primitive called inside a group doesn't. Grouped callers: batch delete, Batch Edit apply + alt-mode toggle, single/batch *Move to mission*, `removeMission`, survey-pattern append, terrain correction, and the map editor's "delete WP + its modifiers". (Function declarations are hoisted, so primitives defined above the undo block can call `pushUndo` defined below it.)

### Restoring — one atomic backend command

Undo/redo restore the active mission to the snapshot via a **new `mission_set(waypoints)` backend command** that replaces the whole WP list in **one** IPC call, **preserving every field including `alt_mode`**. This is chosen over the existing clear-then-re-add loop (used by `switchMission`), which costs N IPC calls **and drops `alt_mode`** (a pre-existing limitation of that path). Restore also rebuilds `missionSlots`, sets `missionCount` / `activeMissionIndex`, and runs under `undoSuspend++` so it can't record itself.

### History lifecycle

History is **cleared on load / download / import / reset** (`missionLoadFile`, `missionImportXml`, `missionDownload`, `resetMultiMission`, `missionResetMemory`) — a loaded mission is a fresh baseline, not an undoable edit. It **persists across edit-mode toggles** (the stack survives; only the UI affordances hide). Tab-switching is pure navigation and is **not** recorded (and doesn't clear history) — the all-missions snapshot already makes switches irrelevant to correctness.

### UI

Flat `↶` / `↷` buttons sit right of the Edit button, **edit-mode only** and hidden in Pattern mode (no undo target there; a pattern *append* is itself one undoable WP action afterwards). Keyboard **Ctrl+Z / Ctrl+Y / Ctrl+Shift+Z**, suppressed while a text field is focused so native input-undo keeps working. `canUndo` / `canRedo` stores drive button enablement. Mission clear gained a **confirm dialog** (shared `ConfirmDialog`) so the now-undoable-but-destructive action isn't a one-tap accident.

**Consequences**: O(total-WP) memory per step (deep copy of all missions) — negligible at the 120-WP cap × 50 steps. Selection is not part of the snapshot (cleared on restore). A grouped action that ends up changing nothing can still push a no-op step; the batch callers guard the common cases (`updates.size > 0`). The single-APPLY batch model (ADR — context-menu/batch work) is what keeps each batch a clean one-step boundary.

---

## ADR-028: Map Over-Zoom Placeholder Detection & Parent Fallback

**Date**: 2026-05-31
**Status**: Accepted

**Context**: ESRI World Imagery advertises zoom 1–20, but many areas only carry real satellite tiles to z17–19. Above the available level the ArcGIS server does **not** return a 404 — it replies **HTTP 200 with a fixed "Map data not yet available" blank tile**. That is tolerable on the 2D map (the user zooms out), but unacceptable in the **3D follow camera**, which can descend to UAV altitude and demand z19/z20, leaving a blank ground. The previous mitigation was a blunt global `cesiumMaxZoom: 17` cap that sacrificed detail everywhere. We want the real detail where it exists and a graceful fallback where it doesn't — with no per-area coverage table (ESRI exposes only a single global max LOD, not regional availability).

### Detection — self-calibrating content hash

The blank is bit-for-bit identical everywhere, so a **content hash** (FNV-1a over the tile bytes, stride-sampled, length-folded) that recurs across **two different tile URLs** is, with practical certainty, the placeholder — real imagery tiles are never byte-identical. This is preferred over (a) a **byte-size** check (real JPEGs collide on length → false positives) and (b) a **hardcoded signature** (brittle if the provider changes its blank). No seed is shipped; the first session self-calibrates (one blank may flash before the second copy confirms the hash — accepted). Detection only runs at **z ≥ 19**, so normal browsing is zero-cost.

Per coarse region (the z14 ancestor ≈ a town) we learn two facts: the **lowest zoom confirmed to be a placeholder** (what to skip) and the **highest zoom confirmed to hold real imagery** (what to fall back to). State is **in-memory per session** — ESRI adds imagery over time, so re-learning each run is safer than persisting a cap that could hide newly available detail; the cost is a few placeholder fetches per sparse region per session.

### Fallback — single detector, engine-appropriate action

Both maps fetch every tile as an ArrayBuffer through one choke point (2D `CachedTileLayer._fetchAndCache`, 3D `Map3D.fetchAndCacheImage`), so detection lives there once and is shared. Placeholders are **never cached**. The fallback differs by engine because their native behaviour differs:

- **3D (Cesium)**: reject the tile request. Cesium's imagery LOD then marks it FAILED and **keeps the parent (z-1) tile visible** — native upsampling, exactly the desired effect. `cesiumMaxZoom` for ESRI satellite/hybrid was raised **17 → 20**; the detection covers the gaps.
- **2D (Leaflet)**: Leaflet does **not** upsample errored tiles, so we build the fallback ourselves — a clipping `<div overflow:hidden>` holding a scaled, quadrant-offset child `<img>` of the real ancestor tile, resolved **through the IndexedDB cache first** (already-cached lower-zoom tiles are reused) then network. We separate "lowest placeholder zoom" from "verified real zoom" so the fallback **walks down** to the actual coverage level (the immediate parent may itself be a placeholder where coverage stops several zooms lower), then a coalesced `redraw()` repaints the layer at the verified level. CSS `background-image` on an `<img>` was tried first and rendered unreliably in WebView2 — the clipping-`<div>` is engine-robust.

### Pan smoothness

The clipped fallback tiles get their **own GPU layer** (`will-change: transform` on the div, `translateZ(0)` on the child img) so the tile pane's pan transform just composites a cached texture instead of re-rasterising the clip edge each frame (which caused a flickering seam grid + tearing). A **1px bleed** on the child img removes sub-pixel hairline gaps, and the learned-cap **redraw is deferred to gesture-idle** (`moveend`) so it never flashes the grid mid-pan.

**Consequences**: cross-session re-learning means the first over-zoom into a sparse region costs a few placeholder fetches (and a one-tile blank flash before the hash confirms). The 2D fallback is visibly a touch different from native tiles (upscaled ancestor imagery) — accepted. The region cap is z14-granular, so a view spanning a coverage boundary uses one cap for the region; fine in practice. Overlay layers (boundaries/labels) are excluded from detection (they legitimately return sparse/transparent tiles).

---

## ADR-029: Reusable Panel Framework + Control Library

**Context**: The app grew to 6 nav-rail panels in a handful of recurring formats, but each
panel rolled its own markup, sizing and buttons — no single source of truth. Symptoms: ~116
button usages across 25 files each re-defining its own `.btn-*` classes; `.nav-panel` widths
hard-coded per panel (360 / 414 / 430 / 920 / 280 px); every layout edit hand-replicated, and
panels still drifted apart despite being the "same type". Full plan: `docs/active/PANEL_FRAMEWORK.md`.

**Decision**: Build a reusable **`PanelShell`** plus a small **control library**, and migrate
panels onto them.

- **`PanelShell`** — one component, a `variant` prop with **5 formats**: `info` (content-sized,
  capped), `compact` (fixed width, fills the panel area: header / thin-framed scrolling field /
  footer), `advanced` (1:2 split, right region with its own header/toolbar/field/footer),
  `wide-compact` and `fullscreen` (floating overlays, terrain-analyzer style — *almost*
  full-screen, not edge-to-edge). Content goes into snippet slots
  (`headerActions`/`toolbar`/`body`/`footer`/`detail*`/`params`). The shell owns the frame,
  positioning, the scroll/vertical-bounding, and the transitions. **All variants are
  left-anchored and sized by width/height/top** (no `right`) so the shell morphs between any
  two; the **instance persists across rail switches** (no `{#key}` remount) so switching panels
  animates; `info`'s intrinsic size animates via `interpolate-size: allow-keywords`.
- **Control library** (`src/lib/components/panel/`): `Button` (6 variants: standard / mode /
  data / danger / warning / compact; fixed height, dynamic width, `full`; a flat-SVG icon
  registry via `currentColor`, shared through its `module` script), `SegmentedToggle`
  (one-element multi-position slide switch) and `Toggle` (on/off, centralised from the settings
  markup).
- **Lives in `.ui-scale`** (the UI-scaling layer), so everything scales with the global
  UI-scale for free.
- **Migration = strangler / parallel run**: a duplicate (bottom) nav-rail group opens the new
  framework panels next to the still-working old panels; build empty shells first, then rebuild
  each panel on the shell (reusing the existing controllers/stores), cut over panel-by-panel,
  then delete the old panel + its rail button. Pre-release + single developer, so the
  scaffolding is not hidden behind a flag.

**Consequences**: panels become "content placed onto the shell" — consistent by construction,
drift-proof, edited in one place; the control set guarantees identical buttons everywhere. Cost:
Svelte's one-component-per-file rule means each control is its own small file (the `panel/`
folder is the de-facto control library); the migration carries temporary duplication (old +
new panels) until each cutover; throwaway scaffolding (`PanelPlayground`) is removed at the end.
Cross-variant morphing relies on `interpolate-size` (Chromium 129+ / WebView2) for the `info`
case and degrades gracefully on older engines (info snaps, the rest still animates).

---

## ADR-030: Window Geometry Persistence via tauri-plugin-window-state

**Date**: 2026-06-04
**Status**: Accepted
**Related**: ADR-006 (session persistence via localStorage — for *app* preferences)

**Context**: The app always reopened at the configured default size (1280×800); the window's
size, position and maximized state were never remembered between launches. ADR-006's
localStorage store handles app-level preferences, but the **native window geometry** is owned by
the OS window, not the WebView, so it can't be set cleanly from frontend JS without a visible
resize/reposition flash after the window is already shown, and JS has no robust multi-monitor /
maximized / off-screen-clamping logic.

**Decision**: Use the official **`tauri-plugin-window-state`** (Rust side). Registered in
`lib.rs` as `.plugin(tauri_plugin_window_state::Builder::default().build())`. The plugin saves
the window state on close and restores it on the next launch, before the window is presented —
so there is no flash. No frontend code, JS package, or capability permission is needed (it runs
through the Rust window lifecycle, not via IPC commands).

**Rationale**:
- Native, flash-free restore (applied at window creation, not after show).
- Handles position/size/maximized + off-screen clamping and multi-monitor out of the box.
- Keeps *window geometry* (a native concern) separate from *app preferences* (ADR-006's
  localStorage store), each persisted by the mechanism that owns it.

**Portable-mode caveat (2026-06-08)**: the plugin writes its `.window-state.json` to Tauri's
**app-config dir**. On **Linux** that follows portable mode because `setup_portable_mode()` sets
`XDG_CONFIG_HOME` → `data/`. On **Windows** the app-config dir comes from the Known-Folder API
(`%APPDATA%\com.kitegc.app\`) and is **not** redirectable by env vars — so it would leak outside
the `data/` folder, breaking the portable guarantee ("nothing on system paths"). Therefore the
plugin is **only registered when `!is_portable()`** (`lib.rs`). Portable builds intentionally do
**not** persist window geometry; everything else (localStorage/IndexedDB via the WebView2 data
folder, SQLite DBs, raw logs, terrain cache) already lives under `data/`.

**Consequences**: one more Tauri plugin dependency; the persisted geometry lives in a plugin
JSON file (not the `kite-gc-settings` localStorage blob), so the two persistence layers must be
kept conceptually distinct. First launch after adoption still uses the config default until a
state file exists. Portable builds skip the plugin (no geometry persistence — by design).

---

## ADR-031: 2D↔3D View Continuity — Cesium Viewer Kept in RAM + Camera Hand-off

**Date**: 2026-06-05
**Status**: Accepted
**Related**: ADR-021 (CesiumJS 3D globe), ADR-003 (Leaflet 2D)

**Context**: The 2D (Leaflet) and 3D (Cesium) maps were mounted via `{#if mapViewMode === '2d'}…
{:else}…`, so each toggle **destroyed and recreated** the Cesium viewer (full terrain/imagery
re-init, ~seconds) and started it at a hardcoded camera far from the content, then a 1.2 s fly-to
swept to it. Switching also lost the user's place: the 3D camera reset every time, and the 3D view
did not hand its location back to 2D. Separately, the 2D map re-`fitBounds`-ed the replay trail on
every (re)mount because its "already framed" flag was instance state.

**Decision**:
- **Keep the Cesium viewer in RAM** once 3D is first opened: lazily mount `Map3D`, then keep it
  mounted but hidden (`visibility:hidden`, so the canvas keeps a real size) while 2D is shown. An
  `active` prop pauses its render loop (`useDefaultRenderLoop = false`) when hidden — zero GPU cost,
  but entities/telemetry keep updating from the stores, so re-show is instant and current.
- **Geographic hand-off, independent zoom.** On 3D→2D the 2D map re-centres on the ground point the
  3D camera looks at (`getCamFocus` picks the globe at screen centre). On 2D→3D the camera targets
  the 2D centre. **Zoom is never transferred** (each view keeps its own — cross-mapping zoom over
  mountainous terrain was unreliable): the 3D camera re-uses its **own** saved range/heading/pitch.
- **Drift-free restore.** A free-mode camera **snapshot** (full matrix + target + range) is captured
  when leaving 3D. If the 2D map wasn't panned, the exact matrix is replayed (`setView`); re-deriving
  it from a ground pick drifted the zoom one step per round-trip (pick hits terrain height > 0, a
  `lookAt` targets the ellipsoid at 0). Follow/orbit re-anchor onto the UAV instead.
- Module-scope the 2D map's "already framed this track" key so `fitBounds` only runs on the first DB
  load, not on every remount.

**Rationale**: instant, deterministic switching that preserves each view's own state; no fly-to
sweep (camera is correct synchronously on show); the snapshot replay makes round-trips loss-less.

**Consequences**: both map components live in the DOM after the first 3D open (more memory, the
intended trade); state that must survive a 2D remount has to be module-scoped or in a store.

---

## ADR-032: Live Terrain-Sampling Widgets — Discontinuity Reset + Bounded History

**Date**: 2026-06-06
**Status**: Accepted
**Related**: ADR-026 (Terrain Elevation Provider & AGL Waypoints)

**Context**: Telemetry-driven terrain HUDs (the **Live-AGL** forward profile, the Terrain Radar fan)
accumulate the flown path from the unified `telem` stream so they work live AND during replay. Two
non-obvious hazards surfaced — both invisible in normal single-flight use, both catastrophic for
replay:

1. **Cross-site bridging.** The Live-AGL profiler appends each new fix and samples the terrain of the
   *segment from the previous fix to the new one* (`terrain_profile`, 30 m spacing). Loading a
   **different log while the player stays open** feeds a fix thousands of km away — so the next
   segment spans two continents and asks the backend for a profile at 30 m over that distance
   (hundreds of thousands of samples → thousands of 42 MB DEM tiles). The backend thread parks on the
   serialized tile loads (low CPU, no extra load — it is *waiting*), the IPC responses dry up, and the
   webview's main thread — starved of the tile-load callbacks it interleaves with — stutters and stops
   painting map imagery (which Cesium/Leaflet load over native `fetch`, NOT through Rust). Terrain
   kept meshing because that decodes in worker threads; imagery froze because its callbacks are on the
   main thread. The tell: stutter ONLY while the replay plays (positions advancing), gone on pause.

2. **Unbounded accumulation.** Even within ONE flight, the retained terrain/path arrays grow for the
   whole replay, and `finishProfile` re-folds them O(n) every tick → the per-tick cost climbs with
   replay length (a slow, progressive version of the same stall).

**Decision**:
- **Reset on a discontinuity.** A telemetry-fed accumulator resets (history + derived buffers +
  profiler) when time runs backwards (scrub / new flight) OR the position jumps more than a sane
  per-fix maximum (**> 1000 m** in Live-AGL) — i.e. a log switch or a large seek. This is the primary
  fix: it stops the cross-site bridge before it can issue a continent-spanning terrain request.
- **Bound the retained history to a sliding window.** The HUD only shows a recent window, so the
  profiler keeps just that: accumulate to a wide trigger (5 km) then trim back to the keep window
  (1.5 km). The wide trigger→keep gap makes the O(n) compaction run only every few km of travel
  (≈ once per several minutes), so the per-tick fold cost stays flat with **near-zero** amortised cost
  and the arrays never grow without bound. The full-track Terrain-Analysis panel leaves it unbounded.
- **Bound every backend DEM fetch** with a connect/total timeout, so a stalled tile download can never
  hang indefinitely while holding the terrain provider's `load_lock` (which serializes all loads).

**Rationale**: the reset is the correct, minimal fix (an earlier attempt that only window-trimmed the
arrays was both insufficient — it didn't stop the continent-spanning request — and buggy, since a
huge cross-region jump trimmed every point and left `path < 2`). The window is best-practice insurance
against long single replays. The timeout is defence-in-depth for the backend.

**Consequences**: any FUTURE widget/overlay that samples backend terrain (or any per-distance data)
from the live/replay telemetry stream MUST reset on the same discontinuities and bound its retained
data — otherwise a log switch will re-introduce the continent-spanning request. This is a property of
the *shared telemetry stream* (live ↔ replay ↔ source-switch), not of any one widget.

---

## ADR-033: Real-Time Lighting + Day/Night Dimming (2D & 3D)

**Date**: 2026-06-06
**Status**: Accepted
**Related**: ADR-031 (2D↔3D View Continuity)

**Context**: Cesium already renders a physically-correct sun/moon (`Simon1994PlanetaryPositions`) and
a globe day/night terminator (`globe.enableLighting`), but nothing drove the clock or surfaced it.
We wanted (a) real lighting on the 3D globe optionally tied to the replayed flight's time, and (b) a
"Night Mode" that darkens the **map imagery** (2D Leaflet + 3D globe) like Cesium's night side — for
readability and as a nice touch (it also makes the app correct for a future space-vehicle use).

**Decision**:
- **Sky clock source priority** (3D): a DEV-only time-of-day slider → the replay log's timestamp
  (when *Log Replay Time* is on) → real wall-clock `now`. Telemetry `timestamp_ms` is **flight-relative**,
  so the absolute instant for the sun is `flight.start_time` (RFC3339, DB) `+ timestamp_ms`. *Log
  Replay Time* is gated on *Real Daytime and Lighting* (no effect without it). The LogPlayer shows the
  reconstructed wall-clock time-of-day, formatted in **UTC** (the DB stores the log's reported time
  verbatim in the UTC fields — INAV's local clock round-trips correctly; ArduPilot TZ is a known TODO).
- **Night dimming = the darker of two continuous brightness curves, never stacked.** Both 2D and 3D
  share `cesiumLikeBrightness(α) = clamp(max(sin α,0)·0.9 + 0.3, 0.3, 1.0)` — a re-derivation of the
  GlobeFS night floor (`vertexShadowDarkness 0.3`, `lambertDiffuseMultiplier 0.9` → night = ×0.3).
  - **3D imagery** is dimmed via `ImageryLayer.brightness` (NOT a CSS/canvas filter), so entities,
    sky and sun stay bright — exactly like the real-lighting night side. Night Mode **OFF** → 1.0;
    **ON** → forces `enableLighting = false` (uniform flat 0.3 ground; sky/sun still real) + imagery
    0.3; **AUTO** → `imageryBrightness = min(cesiumFactor, nightFactor) / cesiumFactor`, where
    `cesiumFactor` is the viewed location at the clock time (1.0 if lighting off) and `nightFactor` is
    the **user's physical location** at system time. The ratio means the extra dim engages *only*
    where Night Mode is darker than Cesium's own shading — no double-dark, and the soft terminator is
    preserved across the day↔night transition (the earlier binary test produced a "too dark, then pops
    to 0.3" artefact in the twilight band).
  - **2D Leaflet** has no lighting, so it applies the same `nightFactor` directly as a `brightness()`
    filter on the imagery tile-layer containers only (telemetry/markers untouched), with a CSS fade.
- **User location (Night-Mode AUTO)** is decoupled from the camera/view entirely (orbiting the globe
  must not flip night). It is persisted (`settings.userLocation`) and refreshed from, in priority: an
  OS geolocation check (app start + a manual button) → the first valid UAV GPS fix per connection
  (`fixType ≥ 3`, HDOP < 10 — coarse is fine). `resolveUserLocation()` falls back stored → home →
  persisted map centre, never the live camera.

**Rationale**: imagery-brightness (vs a canvas filter) is the only way to keep entities/sky bright in
3D and matches the 2D "dim imagery only" behaviour. The min-of-curves ratio is what makes "Night Mode
only takes over when Cesium is brighter than it" literally true and removes the twilight artefact.
Persisting the location + UAV-GPS refresh means AUTO works offline in the field without a per-frame
location lookup.

**Consequences**: a render bug class to remember — with `requestRenderMode: true`, any state change
that affects appearance but not geometry (e.g. toggling `enableLighting`) MUST call
`scene.requestRender()` or it only shows on the next camera move. The sun is tied to the clock, not
the flight date unless *Log Replay Time* is on. ArduPilot log timezone handling is unverified (INAV is
correct as-is) and tracked as a TODO.

---

## ADR-034: 3D FPV (Cockpit) Camera View + Conformal HUD

**Date**: 2026-06-06
**Status**: Accepted
**Related**: ADR-031 (2D↔3D View Continuity), ADR-033 (Real-Time Lighting)

**Context**: The 3D map had Free / Follow / Orbit camera modes — all *external* views of the UAV model.
With real lighting in place a cockpit / first-person view became compelling. We wanted a fourth camera
mode that puts the eye on the aircraft, plus a minimal projected-style HUD.

**Decision**:
- **FPV is a fourth mode in the cycle** (Free → Follow → Orbit → **FPV** → Free). In FPV the model is
  hidden and the Cesium camera is placed at the track point (raised `FPV_EYE_HEIGHT_M` = 0.5 m),
  oriented from the **same body quaternion that drives the model**: nose = rotation matrix column 0
  (camera direction), body-up = column 2 (so bank tilts the view). Cesium input is disabled
  (`enableInputs = false`); the camera is fully data-driven, no extra smoothing. The flight track is
  dimmed to 40% alpha so it doesn't fill the view. "Zoom" maps to the **lens FOV** (30–120°, default
  60°) via a wheel handler, not a dolly.
- **Camera-mode persistence across 2D↔3D.** 3D keeps its mode naturally (viewer kept in RAM, ADR-031)
  for Free/Follow/Orbit. **FPV needs explicit suspend/resume**: leaving 3D must undo FPV's viewer
  changes (input, lens, model/track, wheel) or the hidden viewer's frozen input blocks the map. On
  deactivate we `restoreFromFpv()` but keep `cameraMode = 'fpv'`; on reactivate we `enterFpv()`. The
  **2D map remounts** on every toggle, so its follow state is lifted to the page (`bind:viewMode`) and
  re-applied on mount.
- The activate/deactivate `$effect` reads `cameraMode` (which `enterFpv`/`exitFpv` write); its body is
  wrapped in **`untrack()`** so a mode write can't self-trigger the effect (the read-then-write freeze
  class from the telemetry-widget work). It now depends only on `active`.
- **HUD = two SVG layers.** A **conformal** artificial horizon (horizon line + pitch ladder) spanning
  the full viewport, plus a compact (≤50%) instrument cluster (boresight, bank scale, heading tape,
  speed, altitude). The AHI is true perspective: `y = CY − f·tan(θ)`, with focal length `f` derived
  from the **vertical FOV** (from the horizontal FOV at the viewBox aspect). This keeps the artificial
  horizon aligned with the real terrain horizon **as the FOV/zoom changes**. The conformal mapping
  needs the full vertical FOV ↔ full height, so the AHI fills the viewport (a ≤50% box would clip the
  ladder to ±9° at 60° FOV); only the instrument cluster stays compact. Readouts use the user's display
  units; everything carries a dark drop-shadow for legibility against a bright sky.

**Rationale**: reusing the model's quaternion guarantees the cockpit view matches the rendered attitude
exactly. Conformal (tan-based, FOV-aware) scaling is the only way the AHI lines up with the real horizon
at every zoom — a fixed px/deg would drift. Splitting the AHI (full-screen, conformal) from the
instruments (compact) resolves the tension between "the HUD must be small" and "the AHI must match
reality".

**Consequences**: FPV is intentionally raw (no smoothing) — jittery on noisy logs, accepted for now.
The conformal AHI assumes the map panel is ≥16:9 wide (so `meet` height-fills); a taller panel scales
the AHI slightly small. Suspend/resume of FPV (rather than a hard exit on hide) is required by the
kept-in-RAM viewer model.

---

## ADR-035: ADS-B Conflict Alerts — Two-Stage Proximity + 3D CPA

**Date**: 2026-06-07
**Status**: Accepted
**Related**: ADR-009 (frontend architecture — controllers/stores), ADR-031 (2D↔3D continuity), the radar
plan docs (`docs/active/RADAR_TRACKING_*`, `RADAR_ALERTS.md`)

**Context**: The radar subsystem surfaces foreign vehicles (ADS-B first; FormationFlight / Radio later) on
the 2D + 3D maps. Beyond "there is traffic", we wanted *smart* conflict alerts for the connected UAV —
distinguishing a genuine threat from a distant airliner — with map, audio and banner outputs. The math
had to stay simple (the user is not a mathematician) yet account for both aircraft moving in 3D.

**Decision**:
- **Protected point = the connected UAV only** (valid GPS fix). No fix ⇒ no alerts (no GCS/area alerting).
- **ADS-B traffic only.** FormationFlight / Radio-telemetry contacts are monitoring / pilot-to-pilot
  awareness and never enter the alert pipeline (they share the map + `TrackedVehicle` model, not alerts).
- **Two stages**, both computed in a local **ENU frame centred on the UAV** (plain arithmetic, one
  quadratic minimum — no matrices):
  - **Stage 1 — caution (proximity):** contact inside `R_warn` (5 km) and the ±`H_warn` (2000 m) band
    **and closing** (horizontal range-rate `ḋ < −ḋ_min`). `ḋ` is recomputed only on a *new* contact
    position (`lastSeenMs`) and **held** between bursty ADS-B updates (else it reads "not closing" every
    other frame and flickers). Stage 1 **stays only while still approaching** — once it recedes past
    `recedeDeadband` it clears, even inside the radius.
  - **Stage 2 — collision warning (CPA):** `t_cpa = −(r·v_rel)/(v_rel·v_rel)` over both tracks' full 3D
    velocity (course + climb/sink of contact *and* UAV vario); a cylinder test (`R_cpa` 1000 m / `H_cpa`
    250 m) at the clamped CPA time within a 45 s look-ahead. Banner shows an **evade heading** ⟂ to the
    contact's track, on the side away from the CPA point.
- **UAV course-stability gate:** the UAV's horizontal course is trusted in the CPA only if it stayed
  within ±20° over the last 10 s (steady cruise) and ground speed ≥ 5 m/s; otherwise the UAV is a
  non-translating point (vario still used). Avoids phantom CPAs while loitering/manoeuvring.
- **Jitter control = persistence + hysteresis, not bigger thresholds:** Stage 2 confirms over ~3 s before
  raising and clears at ×1.3 separation + a hold. `H_warn` doubles as the global vertical relevance
  cutoff (one number gates both stages). Threshold rationale (250 kt < 10 000 ft ⇒ 45 s ≈ 5.7 km max
  closure) lives in `RADAR_ALERTS.md §4`.
- **Pure frontend (ADR-009):** `controllers/radarAlerts.ts` (the evaluator + `AlertConfig` single source
  of truth, override-ready for future user tuning) produces a `radarAlerts` store + a derived
  `radarAlertLevels` map. `controllers/alertAudio.ts` drives the audio. No Rust involved — all inputs are
  already in the browser.
- **Outputs:** a top-of-map **banner** (stacked warning/caution, all affected contacts); **audio** (Web
  Audio tone + Web Speech callout, localised with English fallback; Stage 1 once-per-entry, Stage 2 looped
  and suppressing Stage 1); **map highlight** — the 3D **ground circle is coupled to `R_cpa`** and pulses
  red/yellow via a `CallbackProperty` material, with `requestRenderMode` flipped to continuous **only
  while an alert is active**; 2D uses a pulsing CSS ring. The alert level is part of the 3D `groundSig`
  so geometry/material rebuild only on a real state change (the flicker-safe pattern from the radar 3D
  work).
- **Dev tooling:** the in-app `DebugPanel` became **multi-tab** (MSP | Alerts) with a per-contact maths
  readout, plus a global **GPS-inject** row (override the UAV position from the map centre) so alerts can
  be exercised over busy airspace from the desk.

**Rationale**: computing CPA in 3D (vertical rates included) but testing a horizontal/vertical *cylinder*
matches how a pilot thinks ("how close, how high") and keeps the threshold tunable per axis. Holding the
range-rate across bursty ADS-B updates was the key to Stage 1 not flickering. Coupling the ground circle
to `R_cpa` makes the existing 1 km projection *mean* something — it is literally the collision zone.
Toggling `requestRenderMode` only during alerts buys smooth pulsing without paying continuous-render cost
the rest of the time. Keeping every numeric threshold in one `AlertConfig` object leaves a clean path to
user-tunable parameters without touching the logic.

**Consequences**: alerts are **ADS-B-only by design** — FormationFlight/Radio stay monitoring-only and
never alert. The spoken callout depends on an installed system voice — on Linux WebKitGTK
it may be silent and degrades to tone-only (pre-recorded audio is a later option). The CPA is a linear
(constant-velocity) extrapolation — correct for a 45 s window, not a turning/accelerating predictor.
Numeric thresholds are code-fixed until C3 exposes them; the override merge is already in place.

---

## ADR-036: FormationFlight (INAV-Radar / ESP32) — Kite as an MSP-Slave FC Emulator

**Date**: 2026-06-07
**Status**: Accepted
**Related**: ADR-035 (conflict alerts — FF never alerts), the radar plan docs
(`docs/active/RADAR_FORMATION_FLIGHT.md`, `RADAR_TRACKING_*`)

**Context**: INAV-Radar / FormationFlight is an ESP32 + radio (LoRa / ESP-NOW) that shares aircraft
positions within a formation. Each module talks **MSP to a flight controller**: it polls the FC for its
own telemetry, broadcasts it, and relays received peers back into the FC (for the OSD) via
`MSP2_COMMON_SET_RADAR_POS`. We wanted Kite to join the formation as a **ground node** and show the peers,
for monitoring / pilot-to-pilot awareness (never as a conflict alert — that's ADS-B only, ADR-035).

**Decision**:
- **Kite emulates an INAV FC (MSP slave).** The module is the MSP *master*; Kite answers its requests on a
  dedicated serial port (`radar/sources/formation_flight.rs`), reporting `FC_VARIANT="INAV"` + a synthetic
  3D GPS fix at the **GCS location** + a name. Reporting `"INAV"` (not `"GCS"`) is what makes the module
  **transmit** (its TX is gated on the host type, not on armed) — and sidesteps the historically-buggy
  `"GCS"` listen-only path. The low-level MSP v1/v2 frame codec is reused (`msp/codec.rs`, with new
  `encode_*_response` helpers); the slave loop is written fresh (the main pipeline is master-role).
- **Fully isolated** from the main scheduler/FC link — a FormationFlight fault must never disturb the
  safety-critical connection. It's just another `RadarSource` feeding the shared aggregator with
  `system='formationFlight'`.
- **The serial source is persistent.** Reopening the port toggles DTR/RTS and **resets the ESP32**, so the
  FF source is kept open across radar reconfigures and restarted ONLY on a port/baud change
  (`reconcile_ff`); the node name + GCS position are **live** (shared `Arc`s, `radar_set_node_pos`) so
  changing them never cycles the port. The source never asserts DTR/RTS.
- **Data model:** peers carry only position / altitude / heading / speed / link-quality / armed state — **no
  name over MSP**, so the slot id maps to a **letter A..F** (matches the OSD). `lost` is a **timeout we
  apply** (no update for >12 s → red, retained 5 min at the last position), not a firmware state.
- **Rendering is state-based, not altitude-based** (FF opts out of the ADS-B altitude scale): armed = dark
  blue, disarmed = grey, lost = red; a slim paper-plane SVG (2D) / `ff-uav.glb` (3D) with just the model +
  a thin solid drop-line; link quality as pips in the list.

**Rationale**: a real INAV FC is exactly what the module expects to talk to, so emulating one (rather than
the buggy GCS mode) is the robust path and needs no firmware change. Keeping the port open is essential
because the ESP's USB auto-reset makes any reopen a reboot. Deriving `lost` from our own timeout (instead
of trusting the state byte) keeps the display correct regardless of how the firmware uses `state`.

**Consequences**: the `MSP_NAME` we report is clamped to ≤15 printable-ASCII bytes — the module stores it
in a `char name[16]` C-string and a 16-byte name (no NUL) overflows and crashes it. Peers are identified
only by letter (no real name available). FormationFlight requires the contact altitudes to be geoid-
corrected even with no UAV connected, which surfaced (and fixed) the radar-only geoid-offset gap.
`SET_RADAR_ITD` (a module status string) is received but not yet surfaced; the `"GCS"` listen-only mode and
a proper paper-plane model are later work.

---

## ADR-037: Live Connection Discovery — Polled Serial + Event-Streamed BLE

**Date**: 2026-06-08
**Status**: Accepted
**Related**: ADR-006 (settings/localStorage — `lastPort` restore)

**Context**: The connection toolbar required a manual ⟳ to refresh the serial-port list and to scan for
BLE devices. Desktop configurators (e.g. INAV Configurator) discover devices live — a freshly plugged
adapter appears and is selected on its own, and BLE devices populate in real time during a scan. We route
serial through the Rust `serialport` crate and BLE through `btleplug`, **not** the WebView's Web Serial /
Web Bluetooth APIs, so we get no OS hotplug/scan events for free.

**Decision**: Auto-discover while disconnected, with a mechanism matched to each transport:
- **Serial — polling.** `serialport` only enumerates on demand (no hotplug events), but enumeration is
  cheap, so a 1 s `setInterval` (in `+page`, only while serial is selected and not connecting/connected)
  re-lists ports. `refreshSerialPorts` diffs against the previous list: a newly appeared port is
  auto-selected, a vanished selected port is deselected, and the first population keeps the restored
  `lastPort` (no hijack on launch).
- **BLE — event streaming.** A backend scan session (`ble_scan_start`/`ble_scan_stop`, `run_scan_session`
  in `ble.rs`) consumes btleplug's `adapter.events()` and emits a `ble-device` Tauri event per
  discovered/updated device; the frontend upserts them into a store, so the list (and live RSSI) populate
  in real time. The scan is stopped before connecting (an adapter can't scan and open a GATT link at once).
- True OS device-change events (Windows `WM_DEVICECHANGE` / Linux udev / macOS IOKit) were considered but
  rejected for now: per-platform Rust code, more complexity, and 1 s polling already feels instant.

The manual ⟳ buttons were removed (obsolete). **Other COM-port pickers (Radar / FormationFlight) stay on
manual refresh on purpose** — reopening the FF port resets the ESP32 (ADR-036), and the radar port is a
deliberate configuration, not a hot-swap pick.

**Consequences**: a 1 s serial poll runs whenever the toolbar shows a disconnected serial transport
(negligible cost); a continuous BLE scan runs while BLE is selected and disconnected (radio on, by design).
The BLE list is still a native `<select>` that re-renders on live RSSI updates — acceptable until the
connection bar is reworked into the panel/control framework. The old one-shot `scan_ble_devices` command
remains registered but unused.

---

## ADR-038: Airspace Manager — Aeronautical Data Subsystem (Single Pluggable Provider, OpenAIP-First, User Key)

**Date**: 2026-06-08
**Status**: Accepted
**Related**: ADR-028 (tile cache), ADR-033 (Radar subsystem — the moving-vehicle analogue), the Cesium Ion
token setting (user-supplied key pattern)
**Plan**: `docs/active/AIRSPACE_MANAGER.md`

**Context**: OpenAIP is a full aeronautical database, not just airspace — it also has **obstacles** (wind
turbines, masts, towers), **airports**, **RC/model airfields**, **navaids**, hotspots and reporting points.
That is hugely valuable for a UAV GCS (obstacles for low flight, airspace to avoid, RC fields), so the
feature is scoped as an **Airspace Manager**: a dedicated nav-rail panel (under Radar) + a backend
aeronautical-data subsystem with toggleable map (2D) and globe (3D) layers — the **static counterpart to the
Radar subsystem**. There is no single free, global, commercial-use-clean source: **OpenAIP** has the best
global community coverage but its free tier is non-commercial; **FAA** is public domain but US-only; others
are fragmented or paid. We also offer Kite under a commercial-support model (GPLv3 + paid support), so the
data licence matters.

**Decision**:
- **Dedicated "Airspace Manager" panel** (nav rail, under Radar, `PanelShell` **`advanced`** variant): left =
  controls (per-layer **2D/3D visibility** toggles + filters + cache readout/reset), right = a **per-layer
  grouped nearby-feature list** (Obstacles · Airspaces · Airfields, distance-sorted, count-capped, click →
  centre + info). The `advanced` variant is chosen precisely for that grouped right column — the same shape
  as the Radar panel's three system groups (multi-format: can later also present as an `info` card).
  **Four layers, scope-locked:** airspaces, obstacles, RC/model airfields, airports (navaids / hotspots /
  reporting points are out of scope). Airspaces render **all** in 2D but only the **UAV-relevant** ones in 3D
  (boundary ≤ 500 m above / ≤ 5000 m lateral, or inside) to avoid clutter + cost.
- **One active provider at a time**, behind a backend `AeroProvider` trait, **selected in the Data
  settings**. No multi-provider compositing — it keeps the UI lean and avoids overlapping-geometry display
  problems. Start with **OpenAIP** (`api.core.openaip.net`, one endpoint per data type); FAA / openFlightMaps
  / national open-data are future impls behind the same trait.
- **User-supplied API key** (Data settings, persisted, shown only when the provider needs one) — same model
  as the Cesium Ion token and map-tile providers. The data-licence obligation sits with the end user, who
  obtains and accepts the provider's key; Kite is a client. This keeps the commercial-support model clean
  and leaves a path to commercial-clean sources via the pluggable trait.
- **Settings split:** the **main Data settings** keep only the **global feature toggle** (enables the
  subsystem + shows the panel, like `radar.enabled` / `flightLoggingEnabled`), the **provider dropdown**, and
  the **API key**. Everything else — **per-layer 2D/3D visibility**, filters, cache readout/reset, and (later)
  per-type **alerts** — lives **in the panel**. Disabling the feature (or all layers) stops the API download,
  not just the rendering.
- Backend fetches a large region (~500 km radius around the reference) and **caches it in RAM** (aero data
  is static → long TTL; far less than tiles, so no size cap — just a usage readout + clear button). Normalize
  each provider's response into shared models (`Airspace` polygon + `PointFeature` for obstacles/airports/…,
  altitude units/datum → metres) so the 2D and 3D renderers + legend agree. 2D = Leaflet polygons + category
  markers; 3D = extruded airspace volumes + obstacle columns + billboards.

**Rationale**: A single-provider trait is the smallest design that covers global airspace now (OpenAIP) and
stays open to commercial-clean sources later, without the UI/visual complexity of compositing multiple
overlapping datasets. The user-key model mirrors what we already do for Cesium and shifts the licence
obligation to the user. Normalizing in the backend keeps the two renderers and the legend in agreement.

**Consequences**: provider switching requires a re-fetch + cache clear (cheap). OpenAIP's free tier is
non-commercial — documented as the user's responsibility via their key. Altitude datum (GND/MSL/FL-STD) is
normalized pragmatically (FL treated as MSL for display) until precise pressure-altitude handling is needed.
The exact OpenAIP pagination + enum encoding is confirmed in Phase 1 against a live key (the schema is 403
without one).

---

## ADR-039: Home / Launch Reference Point — One Source-Tagged Marker + FC-Home Recovery

**Date**: 2026-06-13
**Status**: Accepted
**Related**: ADR-013 (mission planning state), ADR-026 (terrain / AGL — launch as the REL altitude reference)

**Context**: There were **two** independent on-map points for one concept. The orange **"L" launch
marker** (`launchPoint` store, user-draggable) is the planning reference for REL↔AGL altitude conversions.
The green **"H" home marker** (`homePosition` store) was set **only on the disarmed→armed transition**, and
inconsistently rendered (2D drew it imperatively on arm; 3D drew it from the store). Connecting **mid-flight**
or after an **app restart** left no home at all (no arm transition was witnessed). The two markers could both
be visible at once, and `homePosition` carried no notion of *where it came from*.

**Decision**: collapse them into **one reference marker per map**, driven by a `source` tag on `homePosition`
(`'fc' | 'manual' | 'replay'`):

- **FC home (`source: 'fc'`)** → a **locked green "H"** pinned to the real home, non-draggable; `launchPoint`
  is forced to it. The draggable "L" is hidden (the "H" represents the same point).
- **No FC home (`'manual'`)** → the **draggable orange "L"** *is* the home reference. A `launchPoint` →
  `homePosition` **mirror** keeps them equal so the Home widget points at the "L".
- **Replay (`'replay'`)** → the flight's start position shown as "H".
- Two derived stores express the policy: `homeLocked` (`set && source==='fc'`) hides the "L"; `homeMarkerShown`
  (`set && source∈{fc,replay}`) gates the green "H". The old arm-driven 2D home marker is gone — both maps'
  "H" now follows `homePosition`.

**FC-home recovery** is a one-shot **`MSP_WP` #0** at connect (INAV's RTH home). It is parsed **raw** (21-byte
WP payload), *not* via `decode_wp` and *not* via a box-id table, so it is independent of any version-specific
numbering; `lat==lon==0` ⇒ no home set yet ⇒ skipped. It emits a **protocol-agnostic `home-position` event**
(so a MAVLink `HOME_POSITION` path can emit the same later). The live arm transition also sets `source:'fc'`.

**Arm-edge baseline**: home/launch moves **only on a genuine disarmed→armed edge observed live**. The first
valid telemetry frame of each connection seeds `prevArmed`, so a **reconnect while already armed** is not seen
as an edge — the marker stays put (it does not jump to the in-flight position). `resetTelemetry()` on
disconnect makes the baseline reliable.

**3D "L" visibility**: the orange launch billboard shows **only during an active primary connection** (the
live manual-reference case) — offline planning and replay show no "L" (replay shows the start "H"). The "H"
home entity is mission-independent (always shown for FC/replay home).

Coordination is centralized in `+page`: the mirror, the disconnect **`fc`→`manual` downgrade** (keeps the
position as a manual reference so planning continues), and `applyMissionLaunchDefault` **keeping** an FC home
over a mission file's embedded home.

**Consequences**: one coherent reference instead of two; mid-flight connect / restart recovers home; a manual
operator reference exists when there is no FC home (forward-looking for telemetry-only tracking). The 3D
manual "L" billboard still lives inside the mission render, so a manual reference **without a loaded mission**
is not drawn in 3D (the FC-home "H" is, since it is mission-independent) — accepted; the common HITL case is
FC home. Box-id flight-mode decoding is unrelated (see the CHANGELOG fix: MSP_BOXIDS returns the **`permanentId`**
field, not the `boxId_e` ordinal).

---

## ADR-040: Live Recording — Temp Session Store (Commit-on-Disarm) + Capture Completeness

**Date**: 2026-06-13
**Status**: Accepted
**Related**: ADR-039 (arm-edge baseline / reconnect-while-armed), DATA_PIPELINE.md, FLIGHTLOG_DATABASE.md,
[LIVE_RECORDING_TEMP_STORE.md](active/LIVE_RECORDING_TEMP_STORE.md)

**Context**: A live flight was recorded **straight into the main `flights.db`** — the `flights` row was
inserted on the disarmed→armed edge and `telemetry_records` batch-flushed every 50 samples during flight.
A crash therefore left a **half-written, non-finalized flight** in the production DB (`end_time`/stats
`NULL`) and the session **could not be resumed**. Separately, the unified `TelemetryRecord` (the
protocol-agnostic unit both MSP and MAVLink converge on) was captured **incompletely**: `active_wp_number`,
`nav_state`, `gps_hdop`, and `hw_health_status` were hard-coded `None` even though those MSP messages are
**already polled and emitted** — so a live-recorded mission showed **no active-WP tracking on replay** while
live tracking worked.

**Decision**: record each armed session into a **separate per-session SQLite file**
(`<db_dir>/sessions/active_<ts>.ktmp`, same `telemetry_records` DDL + a self-describing `session_meta`
table), and **commit it into the main DB atomically on disarm** (insert the finalized `flights` row →
`ATTACH` the temp file → copy `telemetry_records` rewriting `flight_id` → `COMMIT` → `DETACH` → delete the
temp file). The temp file is the durable buffer (not memory); the main DB never sees a half-flight. This
**defers `flight_id` to disarm**: `flight-recording-started` becomes an id-less signal, mission save+link
moves to the `flight-recording-ended` event, and weather/geocode enrichment runs at disarm against the new
id. The temp store sits at the `TelemetryRecord` level, so it is inherently **protocol-independent**.

Capture completeness is fixed in the same rework (schema untouched — columns exist since `v4`): add
`on_nav_status` / `on_gps_stats` / `on_sensor_status` to the recorder, the matching
`MSP_NAV_STATUS`/`MSP_GPSSTATISTICS`/`MSP_SENSOR_STATUS` branches to `feed_recorder()`, and pack
`hw_health_status` from the per-sensor status — so the temp store carries the **complete** unified record.

**Deferred (next phase)**: crash **recovery/resume** (startup scan of `sessions/*.ktmp`, finalize from
`session_meta`), **reconnect during an active flight** (continue the same `.ktmp` rather than start a new
flight), and **save-trigger tuning** (batch size / fsync cadence). The self-describing temp file is designed
to enable these without further schema change. **Raw recording** (`raw_logger`/tlog) is unchanged — it
remains the parallel write-only backup, out of scope here.

**Consequences**: the production DB stays clean during flight and a crash leaves a recoverable session file
instead of a corrupt half-flight; replay of a live recording regains active-WP / nav context. Cost: a
recorder lifecycle change (insert-on-disarm) that ripples into the two lifecycle events, enrichment timing,
and the DB-disabled/raw-only path; the copy-on-commit reuses the `.kflight` flight-copy machinery in
`exchange.rs`. Fields that are genuinely **Blackbox-sourced** (`wind_*`, `rc_*`, `gps_eph/epv`,
`baro_temperature`, `state_flags`) stay `NULL` for live MSP — whether any merit a dedicated MSP poll is a
separate, version-safe investigation, not part of this rework.

---

## ADR-041: Deferred Commit — End-Flight Dialog as the Commit Gate + 5 s Re-Arm Grace

**Date**: 2026-06-13
**Status**: Accepted (supersedes the commit-on-disarm timing of ADR-040; the temp-store + capture
parts of ADR-040 stand)
**Related**: ADR-040 (temp session store), ADR-039 (arm-edge baseline),
[LIVE_RECORDING_TEMP_STORE.md](active/LIVE_RECORDING_TEMP_STORE.md)

**Context**: ADR-040 committed the temp session into the main DB **immediately on disarm**, then
showed the End-Flight summary as a post-save editor. That made **Discard** mean "delete an
already-saved flight", couldn't honour an INAV-style **disarm→re-arm grace** (an accidental disarm
re-armed within seconds is *one* flight, not two), and wrote to the production DB before the operator
had decided the flight's fate.

**Decision**: **defer the commit.** On disarm the recorder flushes + closes the temp `.ktmp`, builds
the finalized `Flight`, and stores it as a **pending session** in app-state (a shared
`Arc<Mutex<Option<PendingSession>>>`, *not* in the connection-scoped recorder — it must outlive a
disconnect while the dialog is open). The `flight-recording-ended` event now carries the **stats**
(no `flight_id` exists yet). The pending session is resolved by exactly one path, each `take()`ing it
under the mutex so the command thread and the recorder can never both commit it:

- **Save** (`flightlog_commit_pending_session` command) → commit → main DB, return `flight_id`, enrich,
  delete temp.
- **Re-arm after grace (≥ 5 s)** → commit the previous flight, enrich, delete temp, emit
  `flight-recording-committed{flight_id}`, then start a fresh session.
- **Re-arm within grace (< 5 s)** → reopen the **same** `.ktmp` and continue the flight (timestamps
  resume from `max(timestamp_ms)`); emit `flight-recording-resumed`; no commit.
- **Discard** (`flightlog_discard_pending_session`, confirmed in-dialog) → delete temp; no commit.

The commit is one atomic main-DB transaction (insert finalized `flights` row → ATTACH temp → copy
`telemetry_records` rewriting `flight_id` → COMMIT → DETACH → delete temp). `flight_id` is therefore
born at commit, not arm: `flight-recording-started` is an id-less signal, and the **FC-synced flown
mission is captured at disarm** (waypoints + hash, while FC-sync holds) and **linked at commit**.
The End-Flight dialog becomes the commit gate: **modal** (no backdrop/Escape dismiss so a stray click
can't lose the flight), **Save** / **Discard (+confirm)**, **Skip** removed; a re-arm force-closes it.

**Disconnect with an active flight** (interim): `shutdown()` finalizes the active flight as a pending
session + emits `flight-recording-ended` (dialog appears) instead of committing outright — a
reconnect+arm within grace continues the same `.ktmp`.

**Consequences**: the production DB is written only on an explicit Save or a genuine next-flight arm;
Discard is a temp delete; an accidental disarm re-armed within 5 s stays one continuous log. Cost: the
pending session is shared mutable state across the recorder, the Tauri command layer, and (next phase)
startup recovery; the dialog can be force-closed by backend events, so the frontend must treat a
`null` dialog result as "backend already handled it". The **recovery prompt on start/reconnect**
(orphan `.ktmp` after a kill, the explicit "continue on reconnect" wait-state) and the **single-temp
startup sweep** remain the next phase.

---

## ADR-042: Live-Recording Recovery — Orphan Scan + Continue-on-Reconnect

**Date**: 2026-06-13
**Status**: Accepted
**Related**: ADR-041 (deferred commit + pending session), ADR-040 (temp store), ADR-039 (arm-edge),
[LIVE_RECORDING_TEMP_STORE.md](active/LIVE_RECORDING_TEMP_STORE.md)

**Context**: With deferred commit (ADR-041) a flight lives in a temp `.ktmp` until committed. A hard
crash/close — or a disconnect that left the recorder with no chance to commit — leaves an **orphan**
`.ktmp`. The recording must be recoverable, the operator must be able to **continue** a flight that is
still airborne after a reconnect, and there must **never be more than one** temp file lingering.

**Decision**: on startup, `flightlog_scan_orphan_sessions` scans `<db_dir>/sessions/*.ktmp`, deletes
empty ones, and returns the newest non-empty one. The frontend shows a **modal 3-option prompt**:

- **Discard** (confirm) → delete the temp file.
- **Save Incomplete** → `summarize_temp_session` reconstructs the flight from the temp DB
  (`session_meta` + one pass over the telemetry to recompute stats; `end_time` = last sample) and
  commits it via the same `commit_pending_session` path.
- **Continue on Reconnect** → load the reconstructed session into a shared `resume_pending` slot in
  app-state. The next connection's recorder consults it on its **first polled status** (the
  trustworthy point, past any handshake residual flags): **armed → resume the same `.ktmp`** (reopen,
  timestamps continue from `max(timestamp_ms)`); **disarmed → finalize into the `pending_session` +
  End-Flight dialog**. Because resume sits at the protocol-agnostic `TelemetryRecord` level, the
  reconnect may use a **different protocol** than the original (e.g. start MSP, continue via MAVLink);
  fields the new link does not provide simply stay `NULL`.

A continue-on-reconnect leaves the `.ktmp` on disk while the in-memory `resume_pending` waits, so if
the app is closed again before reconnecting it is re-offered on the next launch (no data loss). The
single-temp invariant holds: commit/discard remove the file and the startup scan clears empty
stragglers.

**Disconnect while armed** is handled by origin:

- **Manual (button):** the frontend **confirms first** (`DisconnectArmedDialog`: Stay Connected /
  Discard / Save Incomplete / Continue on Reconnect) — we do not disconnect until the user chooses,
  since the flight may not be over (COM-port change, switch to a telemetry link). On confirm,
  `disconnectFC` tears down — the recorder's `shutdown()` stashes the flight as the pending session
  **silently** (no event) — and the frontend applies the choice via the
  `flightlog_commit/discard/continue_pending_session` commands.
- **Device gone (USB unplug / BLE drop):** the MSP transport sets `is_connection_lost` on a **fatal**
  error — a failed write, or a `Disconnected`/IO read error, as opposed to a response **timeout**. The
  scheduler tears down, calls the recorder's `shutdown_lost()` (→ `flight-recording-interrupted` → the
  recovery prompt, since no pre-confirm was possible) and emits `connection-lost` so the frontend
  cleans up the connection (the UI shows disconnected and a reconnect works without a manual
  disconnect first — previously a dead serial handle kept the scheduler polling forever).
- **Telemetry/OTA stall** (link down, device still attached): a timeout is **not** fatal, so the
  scheduler keeps polling and the log just gains time gaps — no teardown, no prompt.

MAVLink auto-drop tears down silently (the orphan is offered on the next launch); MSP serial is the
focus here.

**Consequences**: a crash no longer loses the flight and an accidental disconnect mid-air can be
stitched back into one log across a reconnect — even across protocols. Cost: a second shared slot
(`resume_pending`) and a first-status gate in the recorder; the orphan scan trusts the **newest**
non-empty file and defers a (anomalous) non-empty straggler to a later launch rather than auto-deleting
it. Save-trigger tuning (batch size / fsync cadence — a hard crash can still lose the last unflushed
batch) remains open.

---

*End of Architecture Decision Records*

