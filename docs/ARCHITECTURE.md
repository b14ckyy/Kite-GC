# INAV GCS — Architecture Decisions

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

**Decision**: Use a Svelte writable store backed by `localStorage` under key `inav-gcs-settings`. The store auto-saves on every mutation via `set()`, `update()`, or `patch()`.

**Persisted state**: `lastPort`, `lastBaud`, `map.center`, `map.zoom`, `navPanelOpen`, `activeTab`

**Rationale**:
- localStorage is synchronous and available in all Tauri WebView contexts
- No backend/database needed for simple preferences
- `patch()` helper allows updating individual fields without replacing the whole object
- Defaults are merged on load (`{ ...defaults, ...stored }`) to handle schema evolution
