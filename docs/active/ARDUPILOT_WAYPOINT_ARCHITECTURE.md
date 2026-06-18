# ArduPilot Waypoint Architecture — vehicle-class & VTOL aware mission planning

**Status:** Implemented — ADR-046. Catalog + UI + framework; **vehicle classes Copter/Plane/QuadPlane
first-class** (offline selector + online detection incl. QuadPlane via `Q_ENABLE`); **catalog-driven
icons**, **soft-warnings**, and jump/jump-tag map representation done. The VTOL-phase model (Phase 4) and
the deferred items below remain open.
**Created:** 2026-06-02

> **Update (2026-06-18):** vehicle-class selector shipped — the firmware picker is now a 3-way
> `SegmentedToggle` (INAV/ArduPilot/PX4) and the ArduPilot toolbar carries a **vehicle-type dropdown**
> (Plane/Copter/QuadPlane/Rover/Boat/Sub), both locked while connected, the class persisted offline.
> **QuadPlane detection:** a QuadPlane reports `MAV_TYPE_FIXED_WING` (ArduPilot issue #7137), so MAV_TYPE
> alone can't tell it apart — we read **`Q_ENABLE`** on connect (one-shot PARAM_REQUEST, like
> `AHRS_EKF_TYPE`) and upgrade the class to quadplane; the MAV_TYPE VTOL-range check is kept for PX4 VTOL
> and a manually-set ArduPilot `MAV_TYPE`. **Icons** are now **catalog-flag-driven** (`missionIconsArdupilot`
> reads isTakeoff/isLand/isLoiter/specifiesCoordinate/standaloneCoordinate) — VTOL takeoff/land carry a "V"
> badge, spline "S"/payload "P", ROI eye, Set-Home house; the whole catalog is covered (no more "?").
> **Soft-warnings** flag mission items whose command isn't valid for the active class (⚠ in the list +
> footer count, never blocking). **Jump representation:** the connector line + a readable repeat-count
> badge (↺N) now cover both `DO_JUMP` and `DO_JUMP_TAG`; a `JUMP_TAG` is inserted **before** the edited
> waypoint and groups with the **following** waypoint (its jump target), matching FC resume behaviour.
> **INAV parity:** the same ↺N jump badge + a **User-Action 4-slot indicator** (UA1–4 from `p3` bits 1–4)
> on the waypoint teardrop.

> **Implemented (2026-06-14, ADR-046):** the catalog-driven editor is live. `arduCommandCatalog.ts` is a
> declarative QGC-style command catalog (friendly + canonical names, params 1..7, units/ranges/enums,
> `(ⓘ)` tooltips, an **Advanced** expander, vehicle filter); it drives the categorized picker, the
> generic param editor, and the INAV-style grouped list (numbered indented modifiers). The popup
> scaffolding is the shared `missionEditorPopup.ts` framework (lifecycle + **redraw guard** fixing the
> dropdown-close-on-redraw bug + HTML/event primitives) — ArduPilot uses it fully, INAV uses the redraw
> guard. Curation (modern set in the picker, legacy round-trips raw, runtime/camera-protocol commands
> excluded) is captured command-by-command in **`ARDUPILOT_COMMAND_COVERAGE.md`**.
>
> **Foundation (earlier 2026-06-14):** round-trip-faithful codec (`FromPrimitive`), home-slot (item 0)
> handling, shared mission-icon primitives (ADR-045), takeoff anchored on home.
>
> **Upload fixed:** the FC's `MAV_MISSION_OPERATION_CANCELLED` was **not** a data problem — (1) the
> handler-loop read timeout was 1 s (now 50 ms) so item responses lagged, and decisively (2) we only
> answered `MISSION_REQUEST_INT` while ArduPilot (SITL) requests items with the deprecated
> `MISSION_REQUEST` (float) variant — we now answer both. Upload completes in < 1 s.
>
> **Still open:** vehicle classes beyond Plane (data fill); INAV's full adoption of the shared popup
> primitives; raw-param display for legacy/unknown commands; the ⏳/⚠ items in the coverage doc; the
> VTOL-phase model + soft-warnings (below). Naming/curation/param decisions are recorded under
> "Naming, curation & param model" below.

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
| 4 | This planning doc | new `docs/active/ARDUPILOT_WAYPOINT_ARCHITECTURE.md` |
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
  autocontinue }`, flat store. **No per-class palette, no param schemas, no version gating.** A flat
  ~11-command label/short/`COMMANDS_WITH_LOCATION` set; the **target of this plan** is to replace it
  with the layered catalog.
- `helpers/missionConverter.ts` — INAV ↔ ArduPilot WP conversion (coordinate-preserving; LOITER_TURNS
  → unlimited hold, unsupported commands dropped not phantom-placed).
- `helpers/missionIconsArdupilot.ts` — **now a thin `MavCmd → primitive` mapper** over the shared
  `missionIconPrimitives.ts` (ADR-045); takeoff anchored on home (dashed leg + jump line), loiter ring.
- `ArduMissionPanel.svelte` / `ArduMissionLayer.svelte` — single flat list + hand-written per-command
  editor (`buildEditorHtml` if-chain — the thing the catalog replaces), `.waypoints` (QGC WPL) file
  I/O, FC up/download via the MAVLink mission microprotocol (`mavlink_proto/mission.rs`).
- `mavlink_proto/mission.rs` — **round-trip-faithful** codec (FromPrimitive command/frame), home-slot
  handling. `param1..4 / x / y / z` map straight through, so the catalog only drives the *UI*, not the
  wire format.

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

// Full definition of one MAV_CMD (the common/base layer mirrors the MAVLink spec). Field names +
// semantics deliberately mirror QGC's MavCmdInfo*.json so we can port its tables near-verbatim.
interface MissionCmdSpec {
  cmd: number;                       // MAV_CMD id
  key: string;                       // i18n + icon key
  friendlyName: string;              // list + editor header (e.g. "Loiter (time)")
  short: string;                     // compact list badge (e.g. "LTRT")
  semantic: 'nav' | 'do' | 'condition';  // structural bucket (implicit in the id range)
  uiCategory: UiCategory;            // QGC-style picker grouping (see below)
  // Coordinate role:
  specifiesCoordinate?: boolean;     // positioned marker + in the flight path (= today's hasLocation)
  standaloneCoordinate?: boolean;    // has coords but NOT a flight-path node (e.g. ROI_LOCATION)
  // — neither set ⇒ a modifier-style "action" item (no map position), exactly like an INAV User Action.
  isLoiter?: boolean; isLand?: boolean; isTakeoff?: boolean;  // drive icon + special handling
  params: Partial<Record<1|2|3|4|5|6|7, ParamSpec>>;  // ONLY the relevant params (rest hidden)
}

// QGC picker categories (drive the grouped command list). ArduPilot-relevant subset:
type UiCategory = 'Basic' | 'Loiter' | 'Flight control' | 'Conditionals'
                | 'Camera' | 'Safety' | 'VTOL' | 'Advanced';

// One editable parameter — mirrors QGC's MissionCmdParamInfo. A `number` field unless enum* is set
// (→ dropdown). `default: null` ⇒ NaN-on-the-wire / "leave unchanged".
interface ParamSpec {
  label: string;                     // i18n key
  units?: string;                    // "secs", "m", "deg", "%"
  default: number | null;            // null = NaN (nanUnchanged)
  decimals?: number;                 // display precision
  min?: number; max?: number;        // hard constraints
  userMin?: number; userMax?: number;// softer UI hints
  enumStrings?: string[];            // dropdown labels …
  enumValues?: number[];             // … paired with the wire values
  advanced?: boolean;                // hide unless "advanced" editing is on
}

// An override layer keyed by an optional firmwareType and/or vehicleClass. More-specific
// layers win, in QGC's order: base → vehicleClass → firmwareType → (firmwareType+vehicleClass).
interface CmdOverride {
  firmwareType?: FirmwareType;
  vehicleClass?: VehicleClass;
  cmd: number;
  available?: boolean;               // add (true) or remove (false) the command for this layer
  paramRemove?: (1|2|3|4|5|6|7)[];   // hide params for this layer (QGC's "paramRemove": "4")
  params?: Partial<Record<1|2|3|4|5|6|7, Partial<ParamSpec>>>;  // tweak label/units/default per layer
  uiCategory?: UiCategory;           // re-group per firmware/vehicle if needed
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
  per-bucket duplication.

> **Implemented (2026-06-18) — simpler than the layered-tree sketch above.** The shipped catalog
> (`arduCommandCatalog.ts`) is a single flat `ARDU_CATALOG` with a per-command `vehicles[]` field, not
> a base+override tree. **PX4 was added** as a `Firmware` dimension on `resolveCatalog(vehicle, firmware)`
> plus a verified `PX4_COMMANDS` set (the MAV_CMDs PX4 accepts in a mission). For `firmware === 'px4'`,
> `resolveCatalog` returns that subset **ignoring the vehicle class** (PX4 has one mission interpreter
> for all airframes), so PX4 needs no vehicle-type selector. Soft-warnings use `cmdValidForPx4()`
> (VTOL-on-non-VTOL or PX4-unsupported command). The override-layer mechanism (`CmdOverride`) was not
> needed and remains a future option if per-firmware param tweaks become necessary.
- **Integration points:** `missionArdupilot.ts` (store + the layered tree + `resolveCatalog`),
  `ArduMissionPanel` (palette + class selector + per-command editor), `ArduMissionLayer`
  (markers/lines + VTOL-transition badges), `missionConverter.ts` (INAV ↔ ArduPilot per command),
  and the MAVLink codec param mapping in `mavlink_proto/mission.rs` (param1..4 / x / y / z).

---

## UI integration (driven entirely by the collapsed catalog)

The collapsed catalog feeds three surfaces — no per-command UI code, mirroring how QGC and the INAV
editor work:

- **Command picker (add / change type)** — replace the flat `CMD_OPTIONS` with a **grouped, vehicle-
  filtered** list: group by `uiCategory` (`Basic / Loiter / Flight control / Conditionals / Camera /
  Safety / VTOL / Advanced`), show only commands in the collapsed set for the active class. This is the
  QGC pattern the user liked. Commands without `specifiesCoordinate` are the "function/action" items —
  the same mental model as INAV's modifier/User-Action waypoints.
- **Edit popout (INAV-style, generic + modifier list)** — structurally identical to today's INAV WP
  popup (a NAV command at the top, its non-location commands listed below, `+ Add modifier…` at the
  bottom):
  - **Primary (NAV) command** at the top: render its resolved `params` in a loop — a **number field**
    (label + `units`, `decimals`, `min/max`) or, when `enumStrings/enumValues` is set, a **dropdown**;
    `null` default ⇒ an "unchanged/NaN" affordance; `advanced` params hidden unless toggled. Coordinates
    shown iff `specifiesCoordinate` (takeoff stays the home-anchored special-case).
  - **Modifier list** below: the trailing non-location `DO_*`/`CONDITION_*` items, each a removable
    sub-section with the same generic param rendering — exactly like the INAV popup's Jump / FBH blocks.
  - **`+ Add modifier…`**: the categorized, vehicle-filtered picker, inserting the chosen command into
    the flat sequence right after the NAV waypoint.
  This deletes the ~150-line `buildEditorHtml` if-chain and gives **exactly the relevant params, nothing
  irrelevant** — for free, per type. (INAV's `Actions` UA1–UA4 row is an INAV-only per-waypoint
  *parameter* set — ArduPilot has no equivalent, so it simply doesn't appear for ArduPilot.)
- **List view (INAV-style, with numbers)** — NAV waypoints are the primary rows; their trailing
  non-location commands render as **indented, smaller sub-rows**, like INAV modifier waypoints — but
  **with their sequence number** (ArduPilot numbers *every* mission item; INAV leaves modifiers
  unnumbered in OSD/map). `friendlyName`/`short` + a generated one-line param summary. Optionally flag
  whether a command applies to the **previous** (attached) waypoint or gates the **next** one
  (`DO_*` execute on arrival → previous; `CONDITION_*` gate the next nav command).

**INAV vs ArduPilot — corrected mapping:** ArduPilot's explicit `DO_*`/`CONDITION_*` items correspond to
**INAV modifier waypoints** (Jump, FBH, SetHead, …) — the items listed *below* the main WP in the editor
— and we present them the same way. INAV **User Actions (UA1–UA4)** are a different thing: standard
per-waypoint *parameters*, with **no ArduPilot equivalent** → ignored for ArduPilot. The shared UI is the
modifier list, not the User-Action toggles. On the wire ArduPilot is a flat numbered sequence (the
grouping is presentation); INAV attaches modifiers to a geo-waypoint and omits their numbers.

---

## Naming, curation & param model (refined 2026-06-14)

The first catalog pass used invented friendly names + simplified params, which diverged from QGC/MP and
would confuse ArduPilot users. Corrected approach (locked with the user):

- **Source of truth = ArduPilot itself** (its mission-command reference + source: per-vehicle support,
  param semantics). QGC's `MavCmdInfo*.json` is the reference for **friendly labels + descriptions**
  (current, Ardu-aware); Mission Planner only as a cross-check. We are not "QGC vs MP" — we anchor on
  ArduPilot and use both GCS for presentation.
- **Friendly Kite name primary + canonical MAV_CMD name as a subtle secondary label** (smaller, grey,
  e.g. `Take Photo  DO_DIGICAM_CONTROL`) so MP/QGC users recognise the command and newcomers read the
  clear name.
- **Only the params a command actually uses are shown** (rarely-needed ones hidden behind an
  advanced/expand affordance) — keep the editor uncluttered.
- **Per-param descriptions as tooltips** (ported from QGC's `description` fields) — turns the cryptic
  fields (Focus Lock, Session, Shutter Command …) into something navigable; clearer than QGC.
- **Curated *modern* picker; legacy commands are round-trip-only.** ArduPilot does **not** formally
  deprecate commands (legacy + modern coexist for back-compat), so "obsolete" is **our curatorial
  choice**: *a command is legacy when a newer preferred method exists for the same job.* Legacy commands
  are **not offered in the picker** but still **download / load from `.waypoints` and round-trip**,
  rendered **raw** (labelled params, no friendly editor). "Want the legacy stuff? Use Mission Planner."
- **Param model extends to 1..7** (5/6/7 = x/y/z). A command uses 5/6/7 **either** as a real coordinate
  (`specifiesCoordinate` → map marker) **or** as labelled data fields (e.g. `DO_DIGICAM_CONTROL` puts
  Shutter/Cmd-ID/Shot in x/y/z and is **not** a coordinate) — so we model the coordinate-repurposing
  honestly and, unlike MP, never show a meaningless map coordinate for them.

> **Full command-by-command coverage table** (supported / round-trip-only / excluded + reasons) lives
> in `ARDUPILOT_COMMAND_COVERAGE.md` — the raw reference destined for user docs.

### Curation calls (research-backed — see Sources)

- **Gimbal pointing:** offer `DO_GIMBAL_MANAGER_PITCHYAW` (Gimbal v2, ArduPilot ≥4.3, preferred);
  `DO_MOUNT_CONTROL` → legacy (round-trip only).
- **Camera trigger:** keep `DO_DIGICAM_CONTROL` (single shot) + `DO_SET_CAM_TRIGG_DIST` (auto every N m,
  param1 = distance, 0 = off) — no mission-level replacement exists. `DO_DIGICAM_CONFIGURE` → legacy.
- **ROI:** `DO_SET_ROI_LOCATION` / `DO_SET_ROI_NONE` (current); old `DO_SET_ROI` form → legacy.
- **Not mission items — exclude:** `SET_CAMERA_ZOOM/FOCUS/SOURCE`, `IMAGE_START_CAPTURE` (runtime camera-
  protocol commands; ArduPilot does not execute them in missions even though MP lists them).
- **Add (modern, were missing):** `JUMP_TAG` / `DO_JUMP_TAG`, `DO_AUX_FUNCTION`,
  `DO_SET_RESUME_REPEAT_DIST`, `DO_SET_ROI_NONE`.

> The current hand-written `arduCommandCatalog.ts` is the **first pass** — it is to be **rebuilt** from
> QGC's tables + ArduPilot docs per the above (friendly+canonical names, descriptions, 1..7 params,
> curated modern set). Final param ranges still want ArduPilot-operator review.

---

## Phased plan

> Implementation note (2026-06-14): the catalog ended up a single flat `arduCommandCatalog.ts` with a
> per-command `vehicles[]` filter rather than the fully layered `MissionCmdSpec` + `CmdOverride` tree
> below — simpler and sufficient for ArduPilot-only, Plane-first. The `firmwareType` axis / per-vehicle
> `paramRemove` overrides remain the documented extension path for PX4 + finer per-class tuning.

0. ✅ **Upload-reject fixed.** Not a data problem — read-timeout latency (1 s → 50 ms) + we now answer
   both `MISSION_REQUEST_INT` and the deprecated `MISSION_REQUEST` (ArduPilot SITL uses the latter).
1. ✅ **Vehicle-class detection + selector** — `arduVehicleClass` from `fc_variant` + `MAV_TYPE` + the
   `Q_ENABLE` param (QuadPlane); **offline selector** in the ArduPilot toolbar (3-way firmware
   `SegmentedToggle` + vehicle-type dropdown), persisted, locked while connected.
2. ✅ **Command catalog + editor** — `arduCommandCatalog.ts` (curated modern set, params 1..7, tooltips,
   Advanced tier) drives the categorized picker, generic param editor, and grouped list. Curation +
   coverage in `ARDUPILOT_COMMAND_COVERAGE.md`.
3. ✅ **Soft-warnings** — command not valid for the active class flagged (⚠ list + footer count, never
   blocking). _VTOL-phase warnings fold into Phase 4._
4. ⏳ **VTOL phase model + visualisation** — phase tracking, transition badges, VTOL-land approach hint.
5. ✅ **DO_\* / CONDITION_\* catalog** — included in the curated set (modifiers).
6. ⏳ **(optional, low prio)** Version soft-hints (e.g. VTOL-land 4.1 behaviour note).
7. ⏳ **Later:** fill Copter / QuadPlane / Rover / Boat / Sub command data; INAV's full adoption of the
   shared popup primitives; raw-param display for legacy/unknown commands; **PX4** = a `firmwareType`
   layer (structure accounts for it; ≈80 % shared with the ArduPilot mission protocol).

---

## Open questions / to verify on hardware + against `common.xml`

- Exact `MAV_CMD` numbers for `NAV_ATTITUDE_TIME` and whether `NAV_ARC_WAYPOINT` (36) is really
  exposed to GCS missions.
- ✅ QuadPlane `MAV_TYPE` reporting: confirmed it reports **FIXED_WING** by default (issue #7137), so we
  detect QuadPlane from the **`Q_ENABLE`** param instead (MAV_TYPE VTOL-range kept as a secondary signal).
- Per-command **param semantics + frames** (REL/AMSL/TERRAIN) for each NAV command — fill the
  `params[]`/frame columns precisely from `common.xml` + the wiki command pages.
- How `missionConverter` maps these to/from INAV WP types (many have no INAV equivalent →
  best-fit + soft-warn on conversion loss).
- EEPROM/storage save for ArduPilot (`MAV_CMD_PREFLIGHT_STORAGE`) — separate from upload.

---

## Planned extensions (noted 2026-06-14, not started)

Future ArduPilot mission work — each reuses the **shared** helpers we already have for INAV where the
job is the same (the established pattern: shared icon primitives ADR-045, shared popup framework ADR-046):

1. ✅ **Active-waypoint tracking** (the pulsing glow INAV has) — shipped. `MISSION_CURRENT` (id 42) →
   `telemetry-nav-status` → `activeWpNumber`; `iconForArduWp` takes an `active` flag (shared
   `mission-wp-active` glow CSS); the widget shows `WP N/X` (total from `arduMission` when ArduPilot).
2. **3D mission rendering** — the **2D** side is already shared (icon primitives + popup framework give
   both planners one visual schema); **only 3D is missing**. Map3D renders INAV missions only today
   (`wpIconSpec` + `WpAction`), yet both mappers already return a 3D-capable `WpIconSpec` (ADR-045). So
   the work is: a **shared 3D mission renderer** consuming a normalised render model (positions + icon
   spec + connector/modifier lines), with INAV and ArduPilot each just producing that model — extending
   the existing shared-helper pattern to 3D so the two stay visually identical there too.
3. **Mission library DB + flight-recording link for ArduPilot.** The mission DB stores INAV `Waypoint[]`
   today; ArduPilot needs `ArduWaypoint[]` (or a normalised form) persisted + linked to MAVLink flight
   recordings (the flightlog already carries `mode_primary`, etc.). Data-layer feature, its own step.
4. **Visual refinement** (also wanted for INAV): extra markers / sub-markers on the WP teardrops.

---

## Sources

- ArduCopter command handlers — `ArduCopter/mode_auto.cpp` (`start_command`/`verify_command`)
- ArduPlane / QuadPlane command handlers — `ArduPlane/commands_logic.cpp`
- Mission command reference — `ardupilot.org/.../common-mavlink-mission-command-messages-mav_cmd.html`
- QuadPlane AUTO missions — `ardupilot.org/plane/docs/quadplane-auto-mode.html`
