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
**Status**: Accepted

**Context**: A GCS needs maximum map visibility at all times. Traditional sidebar layouts waste horizontal space — especially on smaller screens or when information panels are not actively needed.

**Decision**: All UI panels are floating overlays on top of a full-viewport map. Navigation uses a hamburger menu button that opens a side rail with tab buttons and a floating content panel.

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

## ADR-009: Multi-Protocol Telemetry via TelemetrySource Trait (Planned)

**Date**: 2026-04-16
**Status**: Planned (M6)

**Context**: The GCS will need to support multiple telemetry protocols (MSP, LTM, MAVLink/ArduPilot, CRSF) and multiple simultaneous aircraft. The current telemetry pipeline is already mostly protocol-agnostic — the payload structs (`AttitudeData`, `GpsData`, etc.) and Tauri event names (`telemetry-attitude`, etc.) describe domain concepts, not protocol specifics. The only coupling point is `poll_slot()` in `scheduler/mod.rs`, which directly calls `serial.msp_request()` and MSP-specific decode functions.

**Decision**: Introduce a `TelemetrySource` trait when the second protocol is implemented. The trait abstracts protocol-specific polling and decoding behind a single interface:

```rust
trait TelemetrySource: Send {
    /// Poll for new telemetry data. Returns (event_name, payload) pairs.
    fn poll(&mut self) -> Vec<(String, TelemetryPayload)>;
    /// Stop the source gracefully.
    fn stop(&mut self);
}
```

**Planned implementations**:
- `MspSource` — extracted from current `poll_slot()` (MSP request/response + decode)
- `LtmSource` — LTM frame parser (passive, no request needed)
- `MavlinkSource` — MAVLink v1/v2 heartbeat + telemetry messages (ArduPilot, PX4)
- `CrsfSource` — CRSF/ELRS telemetry frames
- `ReplaySource` — playback from recorded flights at original timing

**Key properties**:
- All sources emit the **same** `TelemetryPayload` variants → frontend code is never changed
- The scheduler thread owns a `Box<dyn TelemetrySource>` instead of calling MSP directly
- Multi-aircraft: multiple `TelemetrySource` instances with per-UAV ID, routed to per-UAV stores
- Protocol auto-detection possible: try MSP handshake → fall back to MAVLink heartbeat sniffing

**What stays unchanged**: The frontend stores (`telemetry.ts`), all widgets, the Tauri event interface, the scheduler loop structure (priority slots, command/bulk channels).

**Rationale**:
- Current payload structs are already protocol-agnostic — no rework needed there
- Single insertion point (`poll_slot`) means low refactoring risk
- Defer until second protocol implementation to avoid premature abstraction
- The trait boundary is a natural extension of the existing module structure

---

## ADR-010: Drag-and-Drop Widget Panel System

**Date**: 2026-04-16
**Status**: Accepted

**Context**: The GCS needs a flexible HUD with multiple instrument widgets that users can arrange to their preference. Fixed layouts don't work well across different screen sizes and use cases (e.g. FPV flying prioritizes AHI, long-range prioritizes GPS/battery).

**Decision**: Two drag-and-drop widget panels (bottom horizontal, right vertical) with edit-mode toggling. Widgets are classified as Large (22.5vmin) or Small (13.5vmin = 60% of large), all sized in `vmin` units.

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

## ADR-011: Map Heading-Up Mode via CSS Transform

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

## ADR-012: Mission Planning — Backend-Owned State with Frontend Mirror

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

## ADR-013: Internationalization via svelte-i18n

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

