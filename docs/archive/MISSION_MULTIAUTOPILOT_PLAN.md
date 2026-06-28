# Multi-Autopilot Mission Planning — Architecture Plan

> ARCHIVED (2026-06-28) — Phases 1–4 complete (ArduPilot + PX4). INAV-over-MAVLink WON'T DO.
> Real-PX4-hardware validation is user-side; not kept open.

**Status:** Phases 1–4 complete for the **ArduPilot/PX4** path (autopilot context + locking,
ArduPilot WP types/UI/conversion, MAVLink mission microprotocol, survey planner). **PX4 path
completed 2026-06-18** — verified command subset, firmware-aware home-slot handling, MAV_TYPE-based
airframe class (see PX4 note below); still untested on real PX4 hardware. **INAV-over-MAVLink: WON'T DO**
(2026-06-21) — only possible from INAV 10 *if* the MSP-over-MAVLink PR merges; INAV uses MSP-WP today, so
this is redundant and zero priority. Stays active only until the **PX4 hardware test** closes it out.  
**Created:** 2026-04-21 · **Last Updated:** 2026-06-21

---

## Context & Decisions

### Decisions Made
1. **File format:** Standard `.waypoints` (QGroundControl-compatible plain text) for ArduPilot/PX4. MW XML (`.mission`) stays INAV-only.
2. **Map layer:** Shared base infrastructure (polylines, popup framework, icon dispatch), per-system rendering logic via thin switcher. Interface stays identical regardless of active system.
3. **System locking when connected:** If a FC is connected, the autopilot system is locked to the connected type. Manual switching is blocked. If a connection to a different system occurs while WPs are loaded, the user is asked: Switch (WPs cleared) or Disconnect.
4. **INAV via MAVLink / ArduPilot via MSP:** Treated as a future phase, not handled now.

---

## State (updated 2026-06-01 — implemented)

| Component | State |
|---|---|
| INAV mission (MSP) | Complete — types, codec, store, UI, file I/O |
| MAVLink telemetry | Complete — HEARTBEAT, attitude, GPS, battery, etc. |
| MAVLink **ArduPilot** mission protocol | ✅ Implemented — `mavlink_proto/mission.rs` upload/download/clear + `ardu_mission_download`/`ardu_mission_upload` commands |
| MAVLink **INAV** mission protocol | ❌ Still `Err("not supported via MAVLink yet")` — deferred (INAV uses MSP WP) |
| `connection.fcInfo.fc_variant` | Reliable: `"INAV"` / `"ArduPilot"` / `"PX4"` / `"Generic"` |
| Autopilot context store | ✅ `stores/autopilotContext.ts` — active system, auto-detect, locking, switch dialog |
| ArduPilot WP types + store | ✅ `stores/missionArdupilot.ts`, converter `helpers/missionConverter.ts`, icons `missionIconsArdupilot.ts` |
| ArduPilot mission UI | ✅ `ArduMissionPanel`/`ArduMissionLayer`, `.waypoints` (QGC WPL) save/load/drop, FC up/download buttons |

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

## Phase 2 — ArduPilot Type System + UI  ✅

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

## Phase 3 — Backend MAVLink Mission Microprotocol (Rust)  ✅

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

## Phase 4 — Survey Planner (FC-agnostic)  ✅

Done — implemented as `SurveyPatternPanel.svelte` / `SurveyPatternLayer.svelte` with all six
shapes (Rectangle/Circle/Spiral/Polygon ZigZag + Lawnmower), AGL support and the 120-WP limit
check (see ROADMAP → Advanced UI & Tools → Survey / area planner). Pattern geometry is pure
math, no FC protocol dependency; output feeds the active autopilot mission.

---

## PX4 path (completed 2026-06-18)

PX4 rides on the ArduPilot foundation — same transport, codec, MISSION_ITEM_INT round-trip, store,
panel, layer, icons. Three firmware-specific differences are handled:

1. **Command catalog (subset).** PX4 implements a smaller, standard MAV_CMD subset than ArduPilot
   (no `JUMP_TAG`/`DO_JUMP_TAG`, no `NAV_LOITER_TURNS`/`SPLINE`/`ALTITUDE_WAIT`/`PAYLOAD_PLACE`, no
   relay/parachute/fence/inverted/gripper/aux/condition commands). `arduCommandCatalog.ts` gains a
   `Firmware` dimension + a verified `PX4_COMMANDS` set; `resolveCatalog(vehicle, firmware)` returns
   the PX4 subset when `firmware === 'px4'`. Source of truth: PX4 docs "Mission Mode" supported list +
   `mavlink_mission.cpp` `parse_mavlink_mission_item`. PX4 rejects unsupported items at upload
   (feasibility checker), so the set stays tight.
2. **No vehicle-type selector.** PX4 has one mission interpreter for all airframes — the full PX4
   catalog (incl. VTOL commands) is always offered; the dropdown is ArduPilot-only. The airframe class
   is read straight from `MAV_TYPE` (PX4 reports it accurately; ArduPilot QuadPlane lies). A connect-time
   soft-warning (`cmdValidForPx4`) flags a VTOL command on a non-VTOL airframe or any PX4-unsupported
   command — never blocking, mirroring the ArduPilot warning.
3. **No home mission-slot.** ArduPilot reserves mission item 0 for home (GCS injects/drops a placeholder);
   **PX4 does not** — item 0 is the first real waypoint. `mission.rs` `upload`/`download` take a
   `reserve_home` flag derived from `fc_variant` (false for PX4), so PX4 missions map straight to seq
   0..len with no placeholder. (Confirmed: QGC `sendHomePositionToVehicle` → true for APM, false for PX4.)

## Open Questions / Future Decisions

- **INAV over MAVLink** — **WON'T DO** (2026-06-21): redundant (INAV uses MSP-WP); only feasible from INAV 10 if the MSP-over-MAVLink PR lands. If ever revisited, the `protocol + fc_variant` combination (not `fc_variant` alone) would determine the system.
- **EEPROM save for ArduPilot**: ArduPilot uses `MAV_CMD_PREFLIGHT_STORAGE` — separate from mission upload
- **Multi-mission for ArduPilot**: ArduPilot doesn't have the INAV multi-mission concept. Single mission only.
