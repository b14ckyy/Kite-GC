# Waypoint Disable / Enable — plan (not yet implemented)

Status: **planned.** Captures the design so it can be built later. A waypoint can
be *disabled* in a loaded mission without deleting it — frozen in place, excluded
from the flown path and never uploaded to the FC, but kept in the file and on the
map for re-enabling.

## Model

- **A `disabled` flag on the `Waypoint`** (backend struct) that is **never part of
  the MSP / FC encoding**:
  - **FC upload** (`mission_upload`) skips disabled WPs entirely.
  - **Display numbering** (`buildDisplayNumbers`) and the normal flight path skip
    them → active WPs move up. The disabled WP **keeps its frozen number**, so that
    number is visible **twice**: once greyed for the disabled WP and once for the
    active WP that moved into its slot. Map and list keep the two distinguishable.
  - The disabled WP **keeps its position in the UI** (list + map), it is not
    reordered.

- **XML save/load:** on save, a disabled WP is **removed from the main
  `<missionitem>` list** and written into the **meta area** with a `disabled`
  attribute + its **original position** (the index it had when disabled). On load
  it is restored from meta into the list at that position with the flag set. It is
  thus preserved across save/load and **never reaches the FC**. Other tools
  (INAV Configurator) only read `<missionitem>`, so this stays inter-app
  compatible (they simply won't see the disabled WP).

## Rendering

- **Edit mode:** list + map show the disabled WP **grey / transparent**, with a
  **dashed grey connector to its original position**; the active flight path is
  drawn **without** it.
- **Non-edit mode:** only the **grey transparent marker** is rendered (no
  connector lines); it is **not shown in the list**.

## Toggle

- Context-menu entry **"Disable / Enable"** on a waypoint (list row + map marker).
- Backend write: **reuse `mission_update_wp`** (it already carries all WP fields,
  so it can carry `disabled`). A dedicated `mission_set_wp_disabled(index, bool)`
  is **optional sugar** (flips just the flag without resending the whole WP, clear
  intent) — only add it if we want the explicit endpoint.

## Touched areas (when implemented)

- **Rust:** `Waypoint` gains `disabled: bool` (default false, not in MSP encode);
  `mission_upload` skips disabled; XML serializer (save → meta, load → restore);
  `mission_update_wp` carries the flag.
- **Frontend:** `mission.ts` (`Waypoint` type + toggle), `buildDisplayNumbers`
  (skip + frozen-number handling), `InavMissionLayer` (grey marker, dashed
  connector, path excludes disabled), `InavMissionPanel` (list styling + hide in
  non-edit), the waypoint context menu (`buildWaypointMenu`).

## Open points

- Exact meta element/attribute shape in the `.mission` XML (extend the existing
  mwp-style meta).
- How the "frozen original position" is anchored if surrounding WPs are later
  added/removed/moved (store original lat/lon for the connector; index may drift).
