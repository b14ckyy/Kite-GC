# Mission Library & DB — reusable missions linked to the flight log

**Status:** Phase 1 + the UI implemented (2026-06-03) — backend, logic, **and** the UI (Mission
Manager, editor Save-to-library, logbook link/unlink) are complete; **awaiting hardware/
simulator testing**. UI surface is documented in
[`MISSION_LIBRARY_UI.md`](MISSION_LIBRARY_UI.md). Complements
[`MISSION_TRACKING_AND_PROVENANCE.md`](MISSION_TRACKING_AND_PROVENANCE.md) — that doc
defines *when* the active-WP highlight is trusted (the FC/FILE/DB provenance flags); **this**
doc defines the **persistence layer**: missions as first-class, reusable database entities
that recorded flights link to. Wiring the `DB` provenance flag is part of this work.

---

## Goal

Missions become **first-class DB entities** in a reusable library. Recorded flights link the
mission that was flown. The mission planner uses the DB as its **primary store**; `.mission` /
`.waypoints` files become **import / export** only. A mission is shared (one row) across any
number of UAVs and any number of recorded flights.

---

## Data model (migration v7 → v8)

### New table `missions`

Identity = **content hash** (the same `hashWaypoints()` algorithm used by the provenance
system in `stores/mission.ts`), so DB identity and provenance stay consistent and the same
mission is stored only once (dedup).

| Column | Purpose |
|---|---|
| `id` PK | |
| `content_hash` TEXT UNIQUE | identity → dedup; matches the provenance hash |
| `name` TEXT | mutable label (the hash, not the name, is identity) |
| `format` TEXT | `inav` \| `ardupilot` (forward-looking) |
| `waypoints_json` TEXT | canonical `Waypoint[]` (protocol-agnostic, the planner format) |
| `source_xml` TEXT | optional original `.mission` XML (round-trip fidelity) |
| `wp_count` INTEGER | total entries (matches `active_wp_number` range) |
| `total_distance_m` REAL | sum of haversine over consecutive geo waypoints (legs only) |
| `alt_diff_m` REAL | highest − lowest waypoint altitude |
| `max_alt_m`, `min_alt_m` REAL | absolute bounds (kept separate from the diff) |
| `bndbox_min_lat`, `bndbox_min_lon`, `bndbox_max_lat`, `bndbox_max_lon` REAL | bounding box — map centering / region grouping in the Phase 2 browser (`bndbox` not `bbox`: "bbox" means *Blackbox* in the FC community) |
| `location_name` TEXT | reverse-geocoded location (bounding-box centroid) via the same Nominatim helper as the flight log; drives the Phase 2 browser's location grouping |
| `created_at` TEXT, `notes` TEXT | |

### `flights` additions

- `mission_id INTEGER REFERENCES missions(id)` (nullable) — the flown mission.
- `logged_wp_count INTEGER` (nullable) — fallback `X` parsed from the Blackbox header
  (`H waypoints:N,…`) when no mission is linked (pure telemetry / blackbox-only recording).

**Link direction is flightlog → mission only.** No reverse table. "Which flights used this
mission" is a later `SELECT … WHERE mission_id = ?` (paginated for commercial use); its UI is
deferred to Phase 2 and needs no preparation now.

### Identity, dedup & metadata

- The **content hash is computed in TS** (where provenance lives) and passed to Rust — one
  source of truth for identity.
- **Mission metadata is also computed in TS** (the planner already has the geo utils, and every
  mission — planned, imported, or FC-downloaded — flows through the TS `mission` store). Rust's
  `mission_save` is a dumb **upsert by `content_hash`**; it does not recompute geometry.
- **`location_name` is the one exception** — it is filled by `mission_db_geocode` (Rust, async,
  after save) from the bounding-box centroid, reusing the flight log's Nominatim helper. Skipped
  if already set (dedup → geocoded once). Fire-and-forget, like the flight log's geocoding.
- `total_distance_m` covers mission legs (WP→WP); the launch→WP1 leg is **not** included.

---

## In-memory mission identity (Phase 1 — store logic, not just UI)

The `mission` store gains:

- `loadedMissionId: number | null` — the DB id of the currently loaded/imported mission
  (`null` = fresh / never saved). Set on **DB load** and on **import when the dedup hash matches
  an existing row**.

**Save semantics** (the dialog is UI, but the branching is store logic):

- `loadedMissionId == null` → save dialog (name + notes) → **insert NEW** → set `loadedMissionId`.
- `loadedMissionId != null` and content unchanged (hash equal) → already saved, no-op.
- `loadedMissionId != null` and **modified** → **NEW / OVERWRITE / CANCEL**:
  - **OVERWRITE** updates the row by `id` (new hash/json/metadata). If the new hash collides with
    a *different* existing mission, reuse that row instead of creating a duplicate (edge case,
    handled in logic).

**Import** (replaces "open"): `.mission` / `.waypoints` → compute hash → dedup check → load onto
the map. If the hash matches a DB mission, `loadedMissionId` = that row; otherwise
`loadedMissionId = null` (fresh-from-file — saving to the DB stays an **explicit** action).
Import never auto-inserts.

---

## Recording flows (arm-save / disarm-link)

Telemetry is written to the DB only on **disarm**, but the mission is captured at **arm** — so
the two are decoupled:

1. **On arm** (recording active): immediately `mission_save` the displayed mission (dedup; if it
   was never manually saved, auto-save with an auto-name, renamable later). **Remember per flight
   session:** `flightMissionId` + `flightMissionHash`.
2. **In-flight edit without upload:** the FC keeps flying the arm-time version → we link exactly
   that snapshot (this is why we save at arm).
3. **In-flight upload of a new mission:** the upload yields a new id/hash → set a session flag
   "flown mission changed".
4. **On disarm:** the recorder writes the flight → link `flights.mission_id = flightMissionId`.

> **As built (2026-06-03):** the disarm handling now lives in the **End-Flight dialog**
> (`EndFlightDialog`, see `BATTERY_MANAGEMENT.md`). The standalone "mission changed?" prompt was
> folded in. The rule simplified to **FC-sync** rather than "changed since arm": an **FC-synced**
> mission is trusted and (re)linked automatically (covers a mid-flight re-upload); a **non-FC**
> mission is offered with an **opt-in checkbox** (FILE flag ignored). `linkMissionToFlight` upserts
> (saves if not in the DB) + links in one step.

**Integration point (logic now):** the arm-save is triggered from the **frontend** (it owns the
mission + dedup hash). The Rust recorder must expose the **new `flight_id`** at disarm (event or
return) so the frontend can link. Verify/extend the recorder for this.

Other provenance flows (connect prompt, replay "track?" prompt, file-load prompt) are unchanged —
see `MISSION_TRACKING_AND_PROVENANCE.md`. On linking (either direction) we call
`markMissionSynced('db')`, finally making the `DB` flag real.

---

## Replay `WP N/X` resolution order

For the active-waypoint readout (Flight-Mode widget) and the map highlight, `X` resolves as:

1. linked `mission.wp_count` (flight has `mission_id`), else
2. `flights.logged_wp_count` (Blackbox header), else
3. nothing (`WP N`).

`N` = `active_wp_number` (live `MSP_NAV_STATUS` / replay record).

---

## Phasing

**Phase 1 — backbone (now):**
schema v8 (missions + `flights.mission_id` + `flights.logged_wp_count`) · Blackbox `waypoints`
header → `logged_wp_count` · Rust commands (`mission_save` upsert, `mission_get`,
`mission_for_flight`, `flight_link_mission`) · store `loadedMissionId` + maintenance · arm-save /
disarm-link / in-flight-upload detection · `markMissionSynced('db')` · replay `X`.

**Phase 1b — make planner-save usable (right after):**
save dialog (name/notes) · NEW/OVERWRITE/CANCEL · import flow (replaces "open") · disarm
update-prompt.

**Phase 2 — management UI:**
Mission browser modeled on the flight-log UI — a collapsible list on the left grouped by
**location → name/id**, with the selected mission's metadata on the right · rename/delete with
reference guard · "load into planner" · "flights flown with this mission" list.

---

## Open points (non-blocking)

- **Multi-mission (INAV, ≤ 9 segments / 120 WP):** a library mission = the currently displayed /
  flown mission as one entry; segment-level splitting is a later refinement.
- **ArduPilot:** the `format` column keeps the path open; canonical `waypoints_json` is
  protocol-tagged (ties to `ARDUPILOT_WAYPOINT_ARCHITECTURE.md`).
- **Blackbox `waypoints:N,M`:** `N` = count; the second field's exact meaning (valid / version)
  is to be verified — only `N` is used.

---

## Functional behaviour & manual test checklist (Phase 1 — implemented)

Plain-language description of what Phase 1 *does*, so it can be verified against intent. The
**planner save dialog, the import flow, and the mission browser are NOT part of Phase 1** (UI
phase — see Phasing above); this list covers only the logic/backend that is in place now.

### 1. Active-waypoint readout in the Flight-Mode widget (testable now, no FC)
- While the FC is in **WP/Mission mode**, the Flight-Mode widget shows **`WP N/X`** below the
  mode badge (replaces the old hex flags dump).
  - `N` = the FC's current target waypoint (live `MSP_NAV_STATUS`; in replay, from the log).
  - `X` (replay) resolves in order: **linked library mission's WP count → Blackbox header
    `waypoints:` count → nothing** (then just `WP N`).
  - `X` (live) = the loaded planner mission's waypoint count.
- In mission mode with **no active waypoint** (FC falls back to RTH) it shows **`WP-RTH`**.
- Outside mission mode: no WP line.
- **To test now:** (re)import a Blackbox log that has an `H waypoints:N` header *with the new
  build*, then replay it — the widget should count `WP 1/N`, `WP 2/N`, … A log imported by an
  older build has no stored count; re-import it.

### 2. Mission auto-save + link on a recorded flight (needs FC / simulator)
- **Preconditions:** flight-log **DB recording enabled**, connected to the FC, and the displayed
  mission is **in sync with the FC** (the **FC** flag in the mission panel — i.e. it was up- or
  downloaded and not edited since).
- **On ARM:** the displayed mission is saved into the library (deduplicated by content) and
  **linked to the new flight**.
- **On DISARM:** the flight's telemetry is written to the DB (as before) and the mission link
  is in place. Replaying that flight afterwards shows `WP N/X` from the **linked** mission.
- **Deliberately NOT linked:** if at arm the mission is empty, or **not FC-synced** (a stale
  plan, or edited-but-not-reuploaded), nothing is linked — because that is not what the FC
  flies. (This also keeps the feature INAV-only for now.)
- **In-flight mission change:** if a *different* mission is uploaded during the flight, **on
  disarm a dialog asks** whether to update the recording's linked mission to the current
  version ("Mission changed" / "Mission geändert").
- A **disconnect while armed** ends the recording the same way (end-of-flight handling runs).

### 3. Mission library in the DB (backend in place; no browser UI yet)
- A mission is stored **once per unique content** (content hash) and shared across any number of
  flights/UAVs. Re-saving the same mission reuses the existing row.
- Stored per mission: waypoint count, total distance (leg sum), altitude difference
  (max−min) + max/min altitude, bounding box, and a **reverse-geocoded location name**
  (bounding-box centroid, same Nominatim service as the flight log; fetched once per mission).
- **Self-healing schema:** an existing flight-log DB gets the new `missions` table and the
  `flights.mission_id` / `flights.logged_wp_count` columns added automatically on next open —
  **no data loss**, no manual migration.

### What is intentionally still missing (UI phase, after we design the UI)
- Planner **"Save to library"** dialog (name + notes) and **NEW / OVERWRITE / CANCEL** on save.
- **Import** replacing "Open" for `.mission` / `.waypoints` (load + dedup-match → sets the
  in-memory mission identity).
- **Mission browser** (flight-log-style, location-grouped list + metadata, load into planner,
  "flights flown with this mission").

### Test matrix
| Test | Needs | Expected |
|---|---|---|
| Replay `WP N/X` | a re-imported Blackbox log w/ mission | counts up `WP n/N`; `WP-RTH` when no active WP |
| Arm → mission saved + linked | simulator + FC-synced mission + DB recording | after disarm, replaying that flight shows `WP N/X` from the linked mission |
| Stale/edited plan at arm | simulator, mission **not** FC-synced | nothing linked (replay falls back to header count or `WP N`) |
| In-flight upload | simulator, upload a different mission while armed | "Mission changed" dialog on disarm; on *Update*, link points to the new mission |
| Existing DB upgrade | any pre-existing flights.db | opens fine; new schema present; old flights intact |
