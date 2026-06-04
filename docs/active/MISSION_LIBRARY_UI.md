# Mission Library — UI design (Phase 1b + 2)

**Status:** Implemented (2026-06-03), **awaiting hardware/simulator testing**. Builds on the
Phase 1 backend/logic in [`MISSION_LIBRARY_AND_DB.md`](MISSION_LIBRARY_AND_DB.md). This doc
defines the user-facing surface: the **Mission Manager** (an alternate view of the Mission
Planner panel), the **Mission Editor** save/export actions, and the **Logbook** linking
controls.

> Built to match the Flight-Logbook design language (same panel chrome + width: ~430 px list,
> ~920 px when a mission is selected, via `+page`'s `.nav-panel` classes). Manager view state
> (open + selected mission) lives in `stores/missionManager.ts` so it survives close/reopen and
> drives the panel width. The detail's mission preview is a non-interactive Leaflet mini-map
> (`MissionPreviewMap.svelte`) on the current provider, fixed aspect-ratio = bbox (portrait
> capped to a square height). Linked-flight rows jump to the flight in the Logbook
> (`requestOpenFlightId`). **Deferred:** list search/filter; ArduPilot export (with the
> ArduPilot mission layer).

---

## A. Mission Manager (alternate view of the Mission Planner panel)

Modeled on the Logbook Manager. Lives in the same panel as the mission editor; it is a *view
transform*, not a separate tab.

### Entry / exit
- **Trigger:** only when **not in edit mode**, a **"Mission Manager"** button sits to the right
  of the **Edit** button. Clicking it transforms the mission panel into the manager.
- **Back:** a **Back** button at the **top-left** (left of Import) returns to the mission editor
  view. (Edit mode is entered from the editor view as before; the Manager button is hidden in
  edit mode.)

### Layout
- **Left — grouped, collapsible list** of missions, grouped by **location** (geocoded
  `location_name`). Missions without a location (geocode pending/failed, or no geo waypoints)
  go in an **"Unknown location"** group. On opening the Manager, (re)trigger `mission_db_geocode`
  for any mission lacking a `location_name`.
- **Selecting a mission** expands the panel to the **right** (like the Logbook) showing:
  - **Metadata:** waypoint count, total distance, altitude diff (max−min) + max/min, created
    date, location. **Editable name + notes** (inline edit, like Logbook notes).
  - **Linked flights:** the list of flights that reference this mission
    (`SELECT … WHERE mission_id = ?`, paginated). Doubles as the delete reference check.
  - **Mini-map preview** _(last, optional gimmick)_: a static preview drawn from `waypoints_json`
    — simple connection lines in the theme accent on a mini map (current map type), centered via
    `bndbox`.

### Actions
- **Per selected mission (top-right, above the metadata):**
  - **Load to Map** — parse `waypoints_json` → `missionSetWaypoints` → set `loadedMissionId` +
    `markMissionSynced('db')`. If the current map mission is **modified/unsaved**, show a
    replace-confirm first (like the connect-download prompt).
  - **Export** — export *this DB mission* to file. **INAV** `.mission` works; **ArduPilot**
    `.waypoints` is **greyed out** until the ArduPilot mission layer exists. (Needs an
    export-from-`waypoints_json` path, or load-then-export.)
- **Import (top-left, above the list)** — and **drag & drop** onto the Manager. After file
  selection, a popup asks: **"Import to library (+ load to map)"** vs **"Load to map only"**.
  - Both paths run the **dedup-match** (hash → `mission_db_find_by_hash`) and set
    `loadedMissionId` if the mission already exists.
  - "Import to library" upserts (saving with the default name if none) then loads.
  - "Load to map only" just loads (no insert), but still sets `loadedMissionId` on a hash match.
- **Delete** (per mission) — **unlink + delete, with a warning**: show how many flights link it
  (the reverse lookup above); on confirm, set those flights' `mission_id = NULL` (they keep their
  telemetry + the Blackbox-header WP fallback) then delete the mission. Needs a `delete_mission`
  backend command (the FK has no `ON DELETE`, so a bare delete of a referenced mission fails).

### Empty state
Just the frame + buttons, with the action buttons **greyed** (nothing selected / no missions).

### Default mission name
Auto-created missions (arm-save, and import-to-library without a name) get
**`New Mission - <YYYY-MM-DD HH:MM>`**. The planner Save dialog pre-fills the same default;
the name is freely editable later in the Manager.

---

## B. Mission Editor — save / export actions

- **Save to library** (new, primary DB save) — opens a small dialog (**name + notes**, name
  pre-filled with the default). Branching on the in-memory identity:
  - `loadedMissionId == null` (fresh) → **insert NEW** → set `loadedMissionId`.
  - `loadedMissionId != null` and content unchanged (DB flag valid) → already saved (no-op /
    feedback).
  - `loadedMissionId != null` and **modified** → **NEW / OVERWRITE / CANCEL**:
    - **NEW** → dialog → insert new → `loadedMissionId` = new.
    - **OVERWRITE** → `mission_db_update(loadedMissionId, …)` (pre-check `find_by_hash` to avoid
      colliding with a *different* existing row) → `markMissionSynced('db')`.
- **Export** (the current bottom **"Save File"**, renamed) — exports the **currently loaded map
  mission** to file. **INAV** `.mission` works; **ArduPilot** greyed out.
- FC **Download / Upload** unchanged.

Two export buttons by design: the **Manager** exports straight from the DB (encourages using the
library), the **Editor** exports what's on the map (intuitive for an in-progress plan).

---

## C. Logbook — mission link

Extend the flight metadata panel:
- **Mission name** — the linked mission's name, or **N/A** if none.
- **Waypoint count** below it — from the linked mission, else the Blackbox-header
  `logged_wp_count`.
- **Link control** (behind the mission name):
  - **No mission linked → "Link Mission"** button. Links the **currently map-loaded** mission to
    the selected flight (this forces the user to validate that the map mission matches the loaded
    log). Enabled only if a mission is on the map **and** it has a provenance flag (**DB**, FILE,
    or FC) — a pure unsaved scratch mission can't be linked.
    - Map mission has **DB** flag → link directly (`flight_link_mission(flightId,
      loadedMissionId)`).
    - Map mission has **FILE/FC** only (not yet in the library) → prompt **"Save to library &
      link"** → save (upsert) → link → set `loadedMissionId` + `markMissionSynced('db')` +
      geocode. (Removes the two-step friction.)
    - Otherwise **greyed**.
  - **Mission linked → show the name + an Unlink (✕)**. Unlink sets `flights.mission_id = NULL`;
    the **Link Mission** button then reappears. (No separate re-link — unlink, then link the new
    one.)
- After link/unlink, refresh the panel (re-fetch `mission_db_for_flight`) and the replay
  `WP N/X` source.

---

## Backend additions needed for the UI
- `delete_mission(id)` — NULL out referencing `flights.mission_id`, then delete; + command.
- Mission **export from `waypoints_json`** (INAV `.mission`) without loading to the backend
  store — or a "load then export" path. ArduPilot export deferred.
- `flight_unlink_mission(flight_id)` (or reuse a SET NULL) — for the Logbook unlink; + command.
- (Have already: upsert, get, get-for-flight, find-by-hash, update, link, geocode,
  logged-wp-count.)

## Build order — all done ✅
1. ✅ Backend: `delete_mission`, `flight_unlink_mission`, `list_missions`, `update_mission`,
   `find_mission_by_hash`, `update_mission_meta`, `list_flights_for_mission`, INAV
   export-from-waypoints (+ commands + frontend wrappers).
2. ✅ Mission Editor: **Save to library** dialog (`MissionSaveDialog.svelte`) + NEW/OVERWRITE/
   CANCEL. (Kept the file button labelled "Save" and the "Open" button — files vs. library is
   the user's choice.)
3. ✅ Logbook: mission name + WP count + **Link/Unlink** (with the "Save & link" prompt for
   FILE/FC missions). Loading a flight also loads its linked mission onto the map.
4. ✅ Mission Manager (`MissionManager.svelte`): grouped list, metadata + editable name/notes,
   linked-flights (clickable → jump to flight), Load to Map / Export / Import (popup +
   drag&drop) / Delete (unlink+warn) + empty state. Logbook design language + panel width +
   state persistence.
5. ✅ Mini-map preview (`MissionPreviewMap.svelte`) — Leaflet on the current provider.

**Deferred (not part of this feature):** list search/filter; ArduPilot export (with the
ArduPilot mission layer); flown-vs-loaded validation (separate roadmap item); persisting view
state for the other panels (Logbook/Settings/UAV-info).
