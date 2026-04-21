# Multi-Autopilot Mission Planning — Architecture Plan

**Status:** Phase 1 in progress  
**Created:** 2026-04-21

---

## Context & Decisions

### Decisions Made
1. **File format:** Standard `.waypoints` (QGroundControl-compatible plain text) for ArduPilot/PX4. MW XML (`.mission`) stays INAV-only.
2. **Map layer:** Shared base infrastructure (polylines, popup framework, icon dispatch), per-system rendering logic via thin switcher. Interface stays identical regardless of active system.
3. **System locking when connected:** If a FC is connected, the autopilot system is locked to the connected type. Manual switching is blocked. If a connection to a different system occurs while WPs are loaded, the user is asked: Switch (WPs cleared) or Disconnect.
4. **INAV via MAVLink / ArduPilot via MSP:** Treated as a future phase, not handled now.

---

## Current State (Pre-Implementation)

| Component | State |
|---|---|
| INAV mission (MSP) | Complete — types, codec, store, UI, file I/O |
| MAVLink telemetry | Complete — HEARTBEAT, attitude, GPS, battery, etc. |
| MAVLink mission protocol | **Not implemented** — explicit `Err("not supported")` stub |
| `connection.fcInfo.fc_variant` | Reliable: `"INAV"` / `"ArduPilot"` / `"PX4"` / `"Generic"` |
| `telemetry.fcVariant` | Dead stub — always `"INAV"`, never updated |
| Autopilot context store | **Does not exist** |
| ArduPilot WP types | **Does not exist** |

---

## Target Architecture

```
src/lib/
  stores/
    autopilotContext.ts     ← Phase 1: active system + auto-detection + locking
    mission.ts              ← unchanged (INAV)
    missionArdupilot.ts     ← Phase 2: ArduPilot WP types + store
  helpers/
    missionIcons.ts         ← unchanged (INAV)
    missionIconsArdupilot.ts ← Phase 2: loiter circles, takeoff icon, etc.
    missionConverter.ts     ← Phase 2: coordinate-preserving WP conversion
  components/mission/
    MissionPanel.svelte     ← thin switcher (reads autopilotSystem)
    MissionLayer.svelte     ← thin switcher (reads autopilotSystem)
    InavMissionPanel.svelte ← Phase 2: current panel content, renamed
    InavMissionLayer.svelte ← Phase 2: current layer content, renamed
    ArduMissionPanel.svelte ← Phase 2: ArduPilot-specific WP list + controls
    ArduMissionLayer.svelte ← Phase 2: ArduPilot-specific map rendering
```

---

## Phase 1 — Autopilot Context Foundation ✅

**Scope:** System selection, auto-detection, locking, switch-confirmation dialog.

### New: `autopilotContext.ts`
- `AutopilotSystem` type: `'inav' | 'ardupilot' | 'px4'`
- `autopilotSystem: Readable<AutopilotSystem>` — current active system
- `autopilotLocked: Readable<boolean>` — true when FC connected (derived from `connection`)
- `pendingSystemSwitch: Readable<SystemSwitchRequest | null>` — triggers dialog when set
- `setAutopilotSystem(system)` — blocked when locked; shows dialog if WPs present
- `confirmSystemSwitch()` — clears WPs + applies switch (async)
- `cancelSystemSwitch()` — dismisses dialog (component handles disconnect if needed)
- Auto-detects system from `connection.fcInfo.fc_variant` on connect
- Persists to `settings.lastAutopilotSystem` on every change

### Updated: `settings.ts`
- Add `lastAutopilotSystem: string` (default: `'inav'`)

### Updated: `MissionPanel.svelte`
- System selector dropdown (visible when not locked)
- Locked badge with system name (visible when locked)
- Switch-confirmation dialog overlay (when `pendingSystemSwitch` is set)
  - "Switch & Clear" → `confirmSystemSwitch()`
  - "Cancel" (manual) or "Disconnect" (connection-triggered) → `cancelSystemSwitch()` + optional disconnect

---

## Phase 2 — ArduPilot Type System + UI

### ArduPilot WP Types (`missionArdupilot.ts`)

Relevant MAV_CMD values:

| MAV_CMD | Value | INAV Equivalent | Key Params |
|---|---|---|---|
| `NAV_WAYPOINT` | 16 | Waypoint | p1=hold(s), p2=acceptance_r(m), p4=yaw |
| `NAV_LOITER_UNLIM` | 17 | PosholdUnlim | p3=radius(m) |
| `NAV_LOITER_TURNS` | 18 | — | p1=turns, p3=radius(m) |
| `NAV_LOITER_TIME` | 19 | PosholdTime | p1=time(s), p3=radius(m) |
| `NAV_RETURN_TO_LAUNCH` | 20 | Rth | — |
| `NAV_LAND` | 21 | Land | p1=abort_alt, p4=yaw |
| `NAV_TAKEOFF` | 22 | — | p1=min_pitch, z=alt |
| `DO_JUMP` | 177 | Jump | p1=wp_num, p2=repeat |
| `DO_CHANGE_SPEED` | 178 | — | p1=type, p2=speed(m/s) |
| `DO_SET_ROI` | 201 | SetPoi | lat/lon/alt |
| `CONDITION_DELAY` | 112 | — | p1=delay(s) |

### ArduWaypoint Interface
```typescript
interface ArduWaypoint {
  command: MavCmd;       // MAV_CMD enum value
  frame: MavFrame;       // 0=ABS, 3=REL_ALT, 10=TERRAIN
  param1: number;        // command-specific (float)
  param2: number;
  param3: number;
  param4: number;
  lat: number;           // degrees * 1e7
  lon: number;
  alt: number;           // metres (float — unlike INAV centimetres)
  autocontinue: boolean;
}
```

### WP Conversion (coordinate-preserving)
```
INAV → ArduPilot:
  Waypoint     → NAV_WAYPOINT      (lat/lon/alt preserved, params → defaults)
  PosholdUnlim → NAV_LOITER_UNLIM  (lat/lon/alt preserved)
  PosholdTime  → NAV_LOITER_TIME   (lat/lon/alt preserved, p1=hold_sec)
  Land         → NAV_LAND          (lat/lon/alt preserved)
  Rth          → NAV_RETURN_TO_LAUNCH (no position)
  SetPoi       → DO_SET_ROI        (lat/lon/alt preserved)
  Jump         → DO_JUMP           (p1=target, p2=repeat)
  SetHead      → (skip — no equivalent)

ArduPilot → INAV:
  NAV_WAYPOINT      → Waypoint
  NAV_LOITER_UNLIM  → PosholdUnlim
  NAV_LOITER_TIME   → PosholdTime
  NAV_LAND          → Land
  NAV_RETURN_TO_LAUNCH → Rth
  DO_SET_ROI        → SetPoi
  DO_JUMP           → Jump
  NAV_LOITER_TURNS  → PosholdTime (best fit)
  NAV_TAKEOFF       → Waypoint    (best fit, no INAV takeoff WP type)
  Others            → Waypoint    (preserve position)
```

### File Format
ArduPilot uses **QGroundControl `.waypoints`** (plain text, tab-separated):
```
QGC WPL 110
0	1	0	16	0	0	0	0	lat	lon	alt	1
1	0	3	16	0	5	0	0	lat	lon	alt	1
```
Columns: index, current, frame, command, param1-4, lat, lon, alt, autocontinue

---

## Phase 3 — Backend MAVLink Mission Microprotocol (Rust)

### New: `src-tauri/src/mavlink_proto/mission.rs`

Download sequence:
```
GCS → FC:  MISSION_REQUEST_LIST
FC  → GCS: MISSION_COUNT (n)
for i in 0..n:
  GCS → FC:  MISSION_REQUEST_INT (seq=i)
  FC  → GCS: MISSION_ITEM_INT (seq=i)
GCS → FC:  MISSION_ACK (MAV_MISSION_ACCEPTED)
```

Upload sequence (reverse):
```
GCS → FC:  MISSION_COUNT (n)
for each request:
  FC  → GCS: MISSION_REQUEST_INT (seq=i)
  GCS → FC:  MISSION_ITEM_INT (seq=i)
FC  → GCS: MISSION_ACK
```

### Updates: `commands/mission.rs`
- `mission_download` dispatches to MAVLink arm via `mission_download_mavlink()`
- `mission_upload` dispatches to MAVLink arm via `mission_upload_mavlink()`
- Progress events via Tauri emit (same pattern as blackbox import)

---

## Phase 4 — Survey Planner (Future, FC-agnostic)

- Separate panel `SurveyPlanner.svelte` in `components/mission/`
- Generates grid/lawnmower/perimeter patterns from a drawn polygon
- Output: `SurveyPoint[]` = `{ lat, lon, alt, speed? }`
- "Send to Mission" button → converts via `missionConverter` to active system format
- No FC protocol dependency; works offline with any active autopilot system

---

## Open Questions / Future Decisions

- **INAV over MAVLink**: when this is supported, the `protocol + fc_variant` combination determines system, not just `fc_variant` alone
- **PX4**: very similar to ArduPilot (same MAVLink mission protocol), mainly different MAV_CMD parameter semantics. Phase 2 ArduPilot foundation should cover ~80% of PX4.
- **EEPROM save for ArduPilot**: ArduPilot uses `MAV_CMD_PREFLIGHT_STORAGE` — separate from mission upload
- **Multi-mission for ArduPilot**: ArduPilot doesn't have the INAV multi-mission concept. Single mission only.
