# Mission Library & DB — reusable missions linked to the flight log

**Status:** Phase 1 in progress (2026-06-02). Complements
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
   If (3) happened → **prompt at disarm**: update the linked mission to the new version?

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
Mission browser (list + metadata, rename/delete with reference guard, "load into planner") ·
"flights flown with this mission" list.

---

## Open points (non-blocking)

- **Multi-mission (INAV, ≤ 9 segments / 120 WP):** a library mission = the currently displayed /
  flown mission as one entry; segment-level splitting is a later refinement.
- **ArduPilot:** the `format` column keeps the path open; canonical `waypoints_json` is
  protocol-tagged (ties to `ARDUPILOT_WAYPOINT_ARCHITECTURE.md`).
- **Blackbox `waypoints:N,M`:** `N` = count; the second field's exact meaning (valid / version)
  is to be verified — only `N` is used.
