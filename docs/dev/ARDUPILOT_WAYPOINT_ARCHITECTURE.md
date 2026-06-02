# ArduPilot Waypoint Architecture — vehicle-class & VTOL aware mission planning

**Status:** Planning (research done; not yet implemented)
**Created:** 2026-06-02

ArduPilot's available mission commands depend on the **vehicle class** (Copter / Plane /
Rover / Boat / Sub) and, for QuadPlanes, on a **VTOL vs fixed-wing phase** — and several
commands / behaviours are **firmware-version dependent**. Today our ArduPilot mission path
(`stores/missionArdupilot.ts`, `ArduMissionPanel/Layer`) offers one flat, vehicle-agnostic WP
list. This doc plans a vehicle-class- and version-aware model so we can fully support ArduPilot.

This extends the multi-autopilot work in `MISSION_MULTIAUTOPILOT_PLAN.md` (Phases 1–4 done:
autopilot context, ArduPilot WP types/UI/conversion, MAVLink mission microprotocol).

---

## Decisions (locked with the user)

| # | Decision | Value |
|---|----------|-------|
| 1 | First-class vehicle scope | **Copter + Plane + QuadPlane** (Rover / Boat / Sub later) |
| 2 | Validation strictness | **Soft-warning** — flag invalid commands, never hard-block |
| 3 | Command scope, step 1 | **NAV_* commands first**, per class; DO_* / CONDITION_* later |
| 4 | This planning doc | new `docs/dev/ARDUPILOT_WAYPOINT_ARCHITECTURE.md` |
| 5 | Catalog design | **Model it after QGroundControl** — a layered, "collapsing" command tree (a common base + per-firmware-type and per-vehicle-type overrides), not flat per-vehicle duplicates |
| 6 | Firmware-family axis | Keep a `firmwareType` axis (`ardupilot` now, `px4` later) in the structure even though only ArduPilot is implemented — so PX4 slots in without a redesign (QGC covers both this way) |
| 7 | Firmware **version** gating | **Not a core axis.** Neither QGC nor Mission Planner gate command availability by firmware version — only by firmware/vehicle *type*. Version stays an **optional soft-hint** for a few documented behaviour cases (e.g. VTOL-land pre/post-4.1) |

---

## Background (researched against the ArduPilot source + wiki)

- A mission item is a `MAV_CMD` with up to 7 params (`param1..4`, `x`=lat, `y`=lon, `z`=alt) and
  a `MAV_FRAME` (0 = global/AMSL, 3 = relative-alt, 10 = terrain). Which `MAV_CMD`s the FC will
  actually execute is defined **per vehicle** by its `start_command()` / `verify_command()`
  switch (the ground truth — see Sources).
- **Vehicle classes expose different command sets** (e.g. only Plane has `NAV_LAND` /
  `NAV_LOITER_TO_ALT` / `NAV_ALTITUDE_WAIT`; only Copter has `NAV_SPLINE_WAYPOINT` /
  `NAV_PAYLOAD_PLACE` / `NAV_ATTITUDE_TIME`; Rover has no takeoff/land/altitude at all).
- **QuadPlane = Plane firmware + VTOL.** It adds `NAV_VTOL_TAKEOFF` (84), `NAV_VTOL_LAND` (85)
  and `DO_VTOL_TRANSITION` (param 3 = VTOL, 4 = fixed-wing). Mode is driven by mission commands,
  not a hard per-phase command filter.
- **Firmware-version dependence is real:** e.g. `NAV_VTOL_LAND` transitions *immediately* to
  VTOL pre-4.1, but in **4.1+** stays fixed-wing and only transitions near the landing point
  (airbraking; ~60–80 m approach), tunable via `Q_OPTIONS` / `Q_RTL_MODE` / `Q_GUIDED_MODE`.
  Newer NAV commands (`NAV_ATTITUDE_TIME`, `NAV_PAYLOAD_PLACE`, …) were added over time.

---

## Current state (what exists)

- `stores/autopilotContext.ts` — active system `inav | ardupilot | px4`, auto-detected from
  `fc_variant`, lock-when-connected, switch dialog. **No vehicle *class* model yet.**
- `stores/missionArdupilot.ts` — `ArduWaypoint { command, frame, param1..4, lat, lon, alt,
  autocontinue }`, flat store. **No per-class palette, no param schemas, no version gating.**
- `helpers/missionConverter.ts` — INAV ↔ ArduPilot WP conversion (coordinate-preserving).
- `helpers/missionIconsArdupilot.ts` — loiter circles, takeoff icon, etc.
- `ArduMissionPanel.svelte` / `ArduMissionLayer.svelte` — single flat list, `.waypoints`
  (QGC WPL) file I/O, FC up/download via the MAVLink mission microprotocol
  (`mavlink_proto/mission.rs`).

---

## Prior art — how QGroundControl and Mission Planner do it

Both established GCSs use **static tables keyed by *type*, not read from the vehicle, and not
gated by firmware version** (there is no MAVLink query for "which mission commands do you
support"). They differ in structure:

| | QGroundControl | Mission Planner |
|---|---|---|
| Source | static JSON (`MavCmdInfo*.json`) | static `mavcmd.xml` |
| Axes | **2 axes**: firmware-type (APM/PX4) **×** vehicle-type (MR/FW/**VTOL**/Rover/Sub) | **1 axis**: firmware bucket (`<AC2>` / `<APM>` / `<APRover>`) |
| Build | **hierarchical "collapsing"**: common (MAVLink spec) → vehicle-type → firmware-type → firmware+vehicle overrides; plus `supportedMissionCommands()` per firmware plugin | **flat**: one command list per bucket, entries carry only param labels (P1–P4, X/Y/Z) |
| VTOL | **own vehicle type** (dedicated VTOL JSON) | folded into `<APM>` (Plane) |
| Granularity | fine (per firmware×vehicle param overrides, e.g. ArduCopter `NAV_WAYPOINT` drops p2/p3/p4) | coarse (param labels per bucket) |
| Firmware version | no role | no role |

**Conclusion (drives this design):** a static catalog is the right call (matches both). We model
it on **QGC** — layered, two-axis (firmware-type × vehicle-type), with a common base + overrides
— because it is more modern/efficient, treats VTOL as its own type, and **already accounts for
PX4**. Even though we ship ArduPilot only first, we keep the `firmwareType` axis so PX4 drops in
later. We **do not** gate by firmware version (neither reference GCS does).

Sources: [QGC MissionCommandTree.cc](https://github.com/mavlink/qgroundcontrol/blob/master/src/MissionManager/MissionCommandTree.cc) ·
[QGC Mission Command Tree guide](https://docs.qgroundcontrol.com/master/en/qgc-dev-guide/plan/mission_command_tree.html) ·
[MP mavcmd.xml](https://github.com/ArduPilot/MissionPlanner/blob/master/mavcmd.xml)

## Vehicle-class model

```
ArduVehicleClass = 'copter' | 'plane' | 'quadplane' | 'rover' | 'boat' | 'sub'
```

Detected from the MAVLink `HEARTBEAT.type` (`MAV_TYPE`):

| MAV_TYPE | value(s) | → class |
|---|---|---|
| FIXED_WING | 1 | plane |
| QUADROTOR / HEXA / OCTO / TRI / HELI | 2, 13, 14, 15, 4 | copter |
| VTOL_* (tailsitter/duo/quad/tilt/fixedrotor…) | 19–25 | **quadplane** |
| GROUND_ROVER | 10 | rover |
| SURFACE_BOAT | 11 | boat |
| SUBMARINE | 12 | sub |

- **Online:** class is derived from the HEARTBEAT and **locked** (like the existing
  system-lock). QuadPlane is the VTOL_* range; if a QuadPlane reports plain FIXED_WING on some
  setups, fall back to "plane" (the VTOL commands then just soft-warn) — _verify on hardware._
- **Offline (planning):** the user picks the class in the ArduPilot mission panel (defaults to
  the last used). Drives the palette + soft-warnings.

---

## NAV command catalog — Copter / Plane / QuadPlane (step 1)

Ground truth: `ArduCopter/mode_auto.cpp` and `ArduPlane/commands_logic.cpp` `start_command()` /
`verify_command()` switches (see Sources). `MAV_FRAME` column = altitude frames the command
meaningfully uses. ⚠ = number/availability to re-verify against MAVLink `common.xml`.

### Common NAV (all three classes)

| MAV_CMD | # | params (ArduPilot semantics) | notes |
|---|---|---|---|
| NAV_WAYPOINT | 16 | p1 delay(s) · p2 accept-radius(m) · p3 pass-by · p4 yaw · x/y/z | core |
| NAV_LOITER_UNLIM | 17 | p3 radius(m) · p4 yaw · x/y/z | loiter forever |
| NAV_LOITER_TURNS | 18 | p1 turns · p3 radius(m) · x/y/z | |
| NAV_LOITER_TIME | 19 | p1 time(s) · p3 radius(m) · x/y/z | |
| NAV_RETURN_TO_LAUNCH | 20 | — | |

### Copter-specific NAV

| MAV_CMD | # | params | notes |
|---|---|---|---|
| NAV_TAKEOFF | 22 | p7/z alt | multirotor climb |
| NAV_LAND | 21 | x/y/z (0,0 = land in place) | |
| NAV_SPLINE_WAYPOINT | 82 | p1 delay · x/y/z | smooth curve |
| NAV_LOITER_TO_ALT | 31 | p2 radius · x/y/z | climb/descend while looping |
| NAV_GUIDED_ENABLE | 92 | p1 on/off | hand control to companion |
| NAV_DELAY | 93 | p1 sec · p2/3/4 hh/mm/ss | |
| NAV_PAYLOAD_PLACE | 94 | p1 max-descent(m) · x/y/z | gripper place |
| NAV_ATTITUDE_TIME | ⚠ verify | roll/pitch/yaw + climb-rate + time | GPS-free, newer FW |
| NAV_ARC_WAYPOINT | 36 ⚠ | x/y/z | seen in switch; verify exposure |

### Plane-specific NAV

| MAV_CMD | # | params | notes |
|---|---|---|---|
| NAV_TAKEOFF | 22 | p1 min-pitch · z alt | fixed-wing takeoff |
| NAV_LAND | 21 | p1 abort-alt · p4 yaw · x/y/z | |
| NAV_LOITER_TO_ALT | 31 | p2 radius · x/y/z | |
| NAV_CONTINUE_AND_CHANGE_ALT | 30 | p1 climb/descend · z alt | |
| NAV_ALTITUDE_WAIT | 83 | p1 alt · p2 descent-rate · p3 wiggle | balloon/launch wait |

### QuadPlane = Plane set **+** VTOL

| MAV_CMD | # | params | notes |
|---|---|---|---|
| NAV_VTOL_TAKEOFF | 84 | z alt (lat/lon ignored) | climbs at `Q_WP_SPD_UP`, then transitions to FW |
| NAV_VTOL_LAND | 85 | x/y/z | FW approach + airbrake → VTOL (4.1+); immediate VTOL pre-4.1 |
| DO_VTOL_TRANSITION | (DO) | p1: 3 = VTOL, 4 = fixed-wing | explicit mid-mission switch |
| NAV_PAYLOAD_PLACE | 94 | as Copter | QuadPlane also handles it |

> The full **DO_\*** and **CONDITION_\*** catalog (CHANGE_SPEED, SET_ROI, SET_SERVO/RELAY,
> DIGICAM, JUMP, SET_HOME, MOUNT_CONTROL, LAND_START, CONDITION_DELAY/DISTANCE/YAW, …) is
> **step 2** — modelled the same declarative way.

---

## VTOL phase model (QuadPlane)

ArduPilot does not hard-forbid commands by phase, so we **soft-model** it for guidance/visuals:

- Track an implied **VTOL ↔ fixed-wing phase** along the mission: `NAV_VTOL_TAKEOFF` ⇒ start
  VTOL→FW; `DO_VTOL_TRANSITION(3/4)` flips it; `NAV_VTOL_LAND` ⇒ FW-approach then VTOL.
- **Visualise transition points** on the map/list (e.g. a VTOL↔FW badge) and the
  **VTOL-land approach** (the ~60–80 m airbrake leg, FW-version dependent).
- **Soft-warn** (not block) when a command is unusual for the current phase, or when VTOL
  commands appear on a non-QuadPlane class.

---

## Firmware-version awareness — optional soft-hint only (NOT a gating axis)

Neither QGC nor Mission Planner gate command availability by firmware version, so **we don't
either**. The command catalog is keyed by firmware-*type* × vehicle-*type* only.

The version is used only for the occasional **behavioural note** (not availability): a few
commands behave differently across versions — most notably `NAV_VTOL_LAND` (immediate VTOL
pre-4.1 vs. fixed-wing airbrake approach 4.1+). For those, a `CmdOverride.note` can surface a
**soft hint** referencing the FC's reported version (`AUTOPILOT_VERSION.flight_sw_version`, read
on connect; offline = none). No `minFw/maxFw` availability filtering, no rewriting, no blocking.
Low priority — defer until the catalog + classes are in.

---

## Architecture — layered command tree (QGC-style)

Push the complexity into **data, layered by specificity**, and "collapse" it to the active
`(firmwareType, vehicleClass)` — exactly QGC's Mission Command Tree, scoped to our needs.

```ts
type FirmwareType  = 'ardupilot' | 'px4';            // px4 = future; layers exist but empty now
type VehicleClass  = 'copter' | 'plane' | 'quadplane' | 'rover' | 'boat' | 'sub';

// Full definition of one MAV_CMD (the common/base layer mirrors the MAVLink spec).
interface MissionCmdSpec {
  cmd: number;                       // MAV_CMD
  key: string;                       // i18n + icon key
  category: 'nav' | 'do' | 'condition';
  hasLocation: boolean;              // positioned marker vs modifier-style (no coords)
  params: ParamSpec[];               // P1..P4 / X / Y / Z: label, unit, range, frame-awareness
}

// An override layer keyed by an optional firmwareType and/or vehicleClass. More-specific
// layers win, in QGC's order: base → vehicleClass → firmwareType → (firmwareType+vehicleClass).
interface CmdOverride {
  firmwareType?: FirmwareType;
  vehicleClass?: VehicleClass;
  cmd: number;
  available?: boolean;               // add (true) or remove (false) the command for this layer
  params?: Partial<ParamSpec>[];     // tweak/remove fields (e.g. ArduCopter WP drops p2/p3/p4)
  note?: string;                     // optional soft-hint (e.g. "VTOL-land behaviour changed in 4.1")
}

// Collapse to the set actually shown/edited for the active vehicle.
function resolveCatalog(fw: FirmwareType, cls: VehicleClass): MissionCmdSpec[];
```

- **Single source of truth = the layered tree.** `resolveCatalog()` collapses it once per
  `(firmwareType, vehicleClass)`; everything derives from the result:
  - **Palette** = the collapsed command set for the active vehicle.
  - **WP editor fields** = render each command's resolved `params[]` (mirrors how the INAV editor
    builds its rows today).
  - **Soft-warnings** = command not in the collapsed set for the current class → warn (don't
    block); plus optional version `note`s and VTOL-phase checks.
  - **Map rendering** = `hasLocation` (positioned marker) vs modifier-style, reusing the INAV
    approach (`missionIconsArdupilot`, line styles).
- **Why layered, not flat:** common NAV commands are defined **once** in the base; classes only
  declare their *additions* and *param tweaks* (QGC-style), instead of Mission Planner's full
  per-bucket duplication. Adding **PX4** later = adding `firmwareType: 'px4'` override layers, no
  structural change.
- **Integration points:** `missionArdupilot.ts` (store + the layered tree + `resolveCatalog`),
  `ArduMissionPanel` (palette + class selector + per-command editor), `ArduMissionLayer`
  (markers/lines + VTOL-transition badges), `missionConverter.ts` (INAV ↔ ArduPilot per command),
  and the MAVLink codec param mapping in `mavlink_proto/mission.rs` (param1..4 / x / y / z).

---

## Phased plan

1. **Vehicle-class model + detection** — `VehicleClass` + `firmwareType` axis, `MAV_TYPE`
   mapping, online lock / offline selector. Class selector in `ArduMissionPanel`.
2. **Layered command tree + NAV catalog (step-1 scope)** — the `MissionCmdSpec` base +
   `CmdOverride` layers + `resolveCatalog()`; fill the **NAV** specs for Copter / Plane /
   QuadPlane (common base + per-class overrides). Palette + per-command param editor + positioned
   vs modifier rendering all derive from the collapsed set. (`firmwareType` axis present, only the
   `ardupilot` layers populated.)
3. **Soft-warnings** — command not in the collapsed set for the active class + VTOL-phase warnings
   (non-blocking markers in list + map).
4. **VTOL phase model + visualisation** — phase tracking, transition badges, VTOL-land approach
   hint.
5. **DO_\* / CONDITION_\* catalog** — extend the same layered tree.
6. **(optional, low prio)** Version soft-hints via `CmdOverride.note` (e.g. VTOL-land 4.1).
7. **Later:** Rover / Boat / Sub vehicle layers; **PX4** = add `firmwareType: 'px4'` override
   layers (structure already accounts for it; ≈80 % shared with the ArduPilot mission protocol).

---

## Open questions / to verify on hardware + against `common.xml`

- Exact `MAV_CMD` numbers for `NAV_ATTITUDE_TIME` and whether `NAV_ARC_WAYPOINT` (36) is really
  exposed to GCS missions.
- QuadPlane `MAV_TYPE` reporting in practice (VTOL_* vs FIXED_WING) → reliable class detection.
- Per-command **param semantics + frames** (REL/AMSL/TERRAIN) for each NAV command — fill the
  `params[]`/frame columns precisely from `common.xml` + the wiki command pages.
- How `missionConverter` maps these to/from INAV WP types (many have no INAV equivalent →
  best-fit + soft-warn on conversion loss).
- EEPROM/storage save for ArduPilot (`MAV_CMD_PREFLIGHT_STORAGE`) — separate from upload.

---

## Sources

- ArduCopter command handlers — `ArduCopter/mode_auto.cpp` (`start_command`/`verify_command`)
- ArduPlane / QuadPlane command handlers — `ArduPlane/commands_logic.cpp`
- Mission command reference — `ardupilot.org/.../common-mavlink-mission-command-messages-mav_cmd.html`
- QuadPlane AUTO missions — `ardupilot.org/plane/docs/quadplane-auto-mode.html`
