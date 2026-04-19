# Colored Track & Flight Mode System — Implementation Plan

## Overview

Color-coded flight tracks on the map based on flight mode, altitude, speed, or signal quality. Includes a Flight Mode widget and UAV icon coloring by nav state.

---

## 1. INAV Flight Mode Flags (from `runtime_config.h`)

| Bit | Constant | Value |
|---|---|---|
| 0 | `ANGLE_MODE` | 1 |
| 1 | `HORIZON_MODE` | 2 |
| 2 | `HEADING_MODE` | 4 |
| 3 | `NAV_ALTHOLD_MODE` | 8 |
| 4 | `NAV_RTH_MODE` | 16 |
| 5 | `NAV_POSHOLD_MODE` | 32 |
| 6 | `HEADFREE_MODE` | 64 |
| 7 | `NAV_LAUNCH_MODE` | 128 |
| 8 | `MANUAL_MODE` | 256 |
| 9 | `FAILSAFE_MODE` | 512 |
| 10 | `AUTO_TUNE` | 1024 |
| 11 | `NAV_WP_MODE` | 2048 |
| 12 | `NAV_COURSE_HOLD_MODE` | 4096 |
| 15 | `TURTLE_MODE` | 32768 |
| 16 | `SOARING_MODE` | 65536 |
| 17 | `ANGLEHOLD_MODE` | 131072 |
| 18 | `NAV_FW_AUTOLAND` | 262144 |

## 2. Flight Mode Classification (priority top-to-bottom)

| Prio | Condition | Label | Color | Hex |
|---|---|---|---|---|
| 1 | `FAILSAFE` + `RTH` | Failsafe RTH | Red | `#e60000` |
| 2 | `FAILSAFE` (without RTH) | Failsafe | Red | `#e60000` |
| 3 | `NAV_WP` | Mission | INAV Blue | `#37a8db` |
| 4 | `NAV_RTH` | RTH | Violet | `#9b59b6` |
| 5 | `NAV_LAUNCH` | Launch | Magenta | `#e91e9c` |
| 6 | `NAV_POSHOLD` | PosHold | Cyan | `#00bcd4` |
| 7 | `NAV_COURSE_HOLD` (±`ALTHOLD`) | Cruise | Orange | `#ff8c00` |
| 8 | `ANGLE` | Angle | Green | `#59aa29` |
| 9 | `HORIZON` | Horizon | Green | `#59aa29` |
| 10 | `MANUAL` | Manual | Grey | `#808080` |
| 11 | No relevant flags | Acro | Light grey | `#c0c0c0` |

**Logic**: Highest matching priority wins. Nav modes (PosHold, Cruise, RTH, WP, Launch) include ALTHOLD and ANGLE implicitly. PosHold ranked before Cruise because it also controls position.

**Note**: NAV_ALTHOLD is only a modifier, never a standalone flight mode. It's shown as a modifier tag ("ALT") in the FlightModeWidget, not in the primary mode badge. ANGLE and HORIZON are separate stabilization modes but share the same track color (both are self-leveling).

## 3. Modules & Files

### A. `src/lib/helpers/trackColors.ts` (NEW)
- `TrackColorMode` type: `'flightmode' | 'altitude' | 'speed' | 'signal' | 'none'`
- `FlightModeInfo` type: `{ label: string, color: string, i18nKey: string }`
- `classifyFlightMode(flags: number): FlightModeInfo` — bitmask → mode + color
- `getGradientColor(value: number, min: number, max: number): string` — value → HSL gradient (blue→green→yellow→red)
- `segmentTrackByFlightMode(track): Segment[]` — consecutive points with same mode = one segment
- `segmentTrackByGradient(track, getValue, min, max): Segment[]` — quantized to ~20 steps

### B. `src/lib/components/Map.svelte` (MODIFY)
- New prop: `trackColorMode: TrackColorMode` (from page)
- `updatePlaybackTrack()`: Instead of single polyline → `L.layerGroup()` with colored segment polylines
- Segments are merged (same mode/step = one polyline) → typically 20-100 segments
- Canvas renderer for performance: `renderer: L.canvas()`
- **Live trail** (`trailLine`): Also colored by flight mode (color from current telemetry point)

### C. `src/lib/components/FlightModeWidget.svelte` (NEW)
- Shows current flight mode as colored badge/label
- Input: `activeFlightModeFlags` (from TelemetryData)
- Uses `classifyFlightMode()` from trackColors.ts
- Small widget size (13.5vmin)
- Works for both **live MSP** AND **Blackbox replay**

### D. `src/lib/adapters/telemetryAdapter.ts` (MODIFY)
- Add `activeFlightModeFlags` to `TelemetryData` interface
- Add `navState` to `TelemetryData` (for UAV icon coloring)
- Mapping: `activeFlightModeFlags: r.active_flight_mode_flags ?? 0`

### E. `src/lib/stores/telemetry.ts` (MODIFY)
- `TelemetryData` interface: add `activeFlightModeFlags: number` and `navState: number`

### F. UAV Icon Coloring by Nav State (Map.svelte)
- UAV marker base color changes based on `nav_state`
- Idle = default, WP Enroute = blue, RTH = violet, Landing = orange, Emergency = red, PosHold = cyan
- Uses `MW_NAV_STATE_*` values from INAV `navigation.h`

### G. Settings: "Alerts" Group (NEW)
- New settings group `alerts` in `settings.ts`
- `warnAltitude: number` — default 120m, 0 = disabled (then scaling uses max altitude)
- Used by altitude track coloring as reference max
- UI in SettingsPanel.svelte

## 4. Track Color Modes

| Mode | Data Source | Segmentation | Color Scale |
|---|---|---|---|
| **Flight Mode** | `active_flight_mode_flags` | Segment on mode change (merged) | Discrete colors (see table above) |
| **Altitude** | `nav_alt_m` → `baro_alt_m` → `alt_m` | ~20 steps, quantized | Gradient: blue(0)→green→yellow→red(`warnAltitude` or max) |
| **Speed** | `speed_ms` | ~20 steps, quantized | Gradient: blue(0)→green→yellow→red(top speed) |
| **Signal** | `link_quality` ?? `rssi` | ~20 steps, quantized | Gradient: green(100%)→yellow→red(0%) — **inverted** |
| **None** | — | Single polyline | Orange `#f5a623` (current default) |

## 5. UI Integration

### Replay Mode
- Track color dropdown in **LogPlayer** (replay control panel)
- Options: Flight Mode | Altitude | Speed | Signal | None
- Default: Flight Mode
- For linked flights (`source: both`), replay source can switch between **REC** (live DB track) and **BBX** (linked blackbox track)

### Live Monitoring (MSP/MAVLink)
- Trail always colored by **flight mode** (no selection)
- No dropdown needed

### Legend
- Horizontal strip directly **below** the replay control panel
- Flight Mode: colored badges with labels — **only modes actually used in the current log/flight are shown**
- Gradient modes: color bar with min/max values
- Toggleable in Settings (default: on)
- In monitoring mode: **top map edge** as optional overlay
- Can be a persistently available element for mode tracking

## 6. Implementation Order

| Step | Task | Dependencies | Status |
|---|---|---|---|
| **S1** | `trackColors.ts` — flight mode classification + gradient function | — | ✅ Done |
| **S2** | `telemetryAdapter.ts` + `telemetry.ts` — pass through `activeFlightModeFlags` + `navState` | S1 | ✅ Done |
| **S3** | `FlightModeWidget.svelte` — mode badge widget | S1, S2 | ✅ Done |
| **S4** | `Map.svelte` — multi-segment rendering (flight mode) | S1 | ✅ Done |
| **S5** | Settings "Alerts" + `warnAltitude` | — | ✅ Done |
| **S6** | Gradient track modes (altitude, speed, signal) | S4, S5 | ✅ Done |
| **S7** | UAV icon coloring by nav state | S2 | ✅ Done |
| **S8** | LogPlayer dropdown + legend | S4, S6 | ✅ Done |
| **S9** | i18n for all labels (mode names, UI elements) | S8 | ❌ Dropped for mode labels (mode names stay English) |
| **S10** | Live trail in flight mode colors | S1, S4 | ✅ Done |

## 7. Performance Considerations

- Segments are **merged** (same color = one polyline) → typically 20-100 polylines instead of 10,000
- `L.canvas()` renderer instead of SVG for large tracks
- Gradient quantized to 20 steps → max 20 polylines for gradient modes
- Segment array computed once on track load and re-segmented on mode switch (no re-render per frame)
