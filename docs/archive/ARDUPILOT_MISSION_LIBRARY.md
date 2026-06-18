# ArduPilot / PX4 Mission Library — Architecture Plan

> **ARCHIVED (2026-06-18)** — Phase 1 (functional library + flight-link parity, ADR-050) shipped. Phase 2
> (provenance / fc-file-db sync chips for the AP store) deferred — tracked in `ROADMAP.md`. Archived ≠
> abandoned: resume here if the AP provenance chips are picked up.

**Status:** **Phase 1 shipped (2026-06-15, ADR-050).** Phase 2 (provenance/sync indicators for the
ArduPilot store) deferred. Extends the multi-autopilot work
([MISSION_MULTIAUTOPILOT_PLAN.md](MISSION_MULTIAUTOPILOT_PLAN.md), Phases 1–4) with the
**mission database / library** parity for the MAVLink-mission family (ArduPilot now, PX4 later),
mirroring the INAV library ([archive/MISSION_LIBRARY_AND_DB.md](../archive/MISSION_LIBRARY_AND_DB.md),
[archive/MISSION_LIBRARY_UI.md](../archive/MISSION_LIBRARY_UI.md)).
**Created:** 2026-06-15

---

## Goal

Bring the ArduPilot mission editor to **full library parity** with INAV: missions can be saved to
the DB, deduplicated by content hash, retrieved, previewed, exported, and **linked to flights** —
the exact same UX and Mission Manager surface, only over the ArduPilot `.waypoints` / `ArduWaypoint`
model instead of INAV WPs. `.waypoints` (QGC WPL 110) file import/export already exists; this plan
adds the **DB layer wiring** on top.

Design constraint (user): **PX4 rides on the same code path.** PX4 uses the same MAVLink mission
protocol and the same `ArduWaypoint` model as ArduPilot; the only per-system differences
(command catalog, flight modes, param semantics) **already branch on the autopilot system**
(ADR-046 vehicle-filtered catalog, `autopilotContext`). So the library code is **one shared path**;
the DB `format` column carries the concrete system (`'ardupilot'` / `'px4'`) for display + filtering,
not a fork in logic.

---

## What already exists (no work needed)

| Piece | Where | State |
|---|---|---|
| `missions` DB table (format-agnostic) | `db.rs` `ensure_v8_schema` | `format TEXT DEFAULT 'inav'`, `waypoints_json`, `content_hash UNIQUE`, `home_lat/lon` |
| DB commands (save/get/update/delete/list/find-by-hash/geocode) | `commands/flightlog.rs`, `mission_db_*` | Protocol-neutral — work on any `waypoints_json` |
| Flight↔mission link commands | `flight_link_mission` / `flight_unlink_mission` / `mission_db_for_flight` / `mission_db_flights` | Protocol-neutral (operate on `flights.mission_id`) |
| `.waypoints` (QGC WPL 110) I/O | `missionArdupilot.ts` `serializeWaypoints` / `parseWaypoints` | Wired in `ArduMissionPanel` (file open/save/drop) |
| AP WP model + store + editor + map layer | `missionArdupilot.ts`, `ArduMissionPanel`, `ArduMissionLayer` | Complete |
| Autopilot context + locking + switch dialog | `autopilotContext.ts` | Complete (`setAutopilotSystem`, `pendingSystemSwitch`, locking when connected) |

So the backend DB layer and the file format are **done**. The gap is entirely **frontend wiring**:
an AP library builder, the AP panel's "save to library", a format-aware Mission Manager, and
flight-link parity.

---

## What's missing (the work)

1. **AP library builder** — an `ArduWaypoint`-side equivalent of `helpers/missionLibrary.ts`:
   content hash + geometry metadata + `buildMissionInput`, with `format` = active system.
2. **AP "Save to library" + "Mission Manager"** entry in `ArduMissionPanel` (mirrors `InavMissionPanel`).
3. **Format-aware Mission Manager** — load dispatches to the right store/editor by `format`;
   preview renders by `format`; list shows a format badge.
4. **Format-aware `MissionPreviewMap`** — render AP missions via the AP renderer.
5. **Cross-format guard on load** — loading a library mission of the *other* system switches the
   editor (via `autopilotContext`) and shows the existing switch dialog (Keep-in-memory / Cancel).
6. **Flight↔mission link parity for AP** — an `arduLoadedMissionId` so arm-time capture, End-Flight
   capture, manual link (FlightDetail) and replay-load resolve the AP library mission.

---

## Design decisions (confirmed with user)

### D1 — Content hash: like INAV, **launch/home excluded**
The hash is the dedup identity. INAV excludes the launch point naturally (it lives in a separate
`launchPoint` store, not in the waypoint array). For AP we mirror that: **the home/launch reference
is NOT part of the hash** — only the mission waypoint items are.

```ts
// missionArdupilot.ts (new) — mirrors mission.ts hashWaypoints
export function hashArduWaypoints(wps: ArduWaypoint[]): string {
  return JSON.stringify(
    wps.map((w) => [w.command, w.frame, w.param1, w.param2, w.param3, w.param4, w.lat, w.lon, w.alt]),
  );
}
```

> **Implementation check — RESOLVED.** `mavlink_proto/mission.rs` **drops mission slot 0 (home) on
> download** and re-injects a home placeholder on upload, so Kite's `arduMission` store holds **only
> authored waypoints, no home**. The hash therefore covers the whole array and home is naturally
> excluded (exactly the INAV behaviour) — `home_lat/lon` are stored as **null** for AP missions. No
> home-splitting needed.

The DB-side `content_hash` is `SHA-256(hashArduWaypoints(...))`, same construction as INAV
(`missionContentHash`).

### D2 — Cross-format guard: reuse the existing switch dialog (separate stores → keep-in-memory)
INAV (`mission` store) and ArduPilot (`arduMission` store) are **independent**. Switching the active
system therefore **never destroys data** — each store retains its own mission. So:

- **Loading a library mission of the active system's format** → loads directly (today's behaviour).
- **Loading a library mission of the *other* format** → call `setAutopilotSystem(targetSystem)` first.
  - If unsaved WPs are present in the *current* editor, the existing `pendingSystemSwitch` dialog
    appears — but since stores are separate, the choice is **Switch (keep both in memory) / Cancel**,
    not "Switch & Clear". (Adapt the dialog copy: the current mission stays in its own store.)
  - On confirm, the editor switches and the library mission loads into the target store.
- **When a FC is connected** the system is **locked** (`autopilotLocked`). Loading a library mission
  whose format ≠ connected system is **blocked** with a status message (consistent with existing
  locking). Same-format load proceeds.

### D3 — Format-aware preview + Mission Manager dispatch
- `MissionManager.loadToMap` and the import flow **branch on `m.format`**:
  - `'inav'` → `missionSetWaypoints(...)` (INAV `mission` store), `loadedMissionId`.
  - `'ardupilot' | 'px4'` → `arduMission.set(...)`, `arduLoadedMissionId`, after the D2 system switch.
- `MissionPreviewMap` takes a `format` prop and renders via the matching renderer (INAV vs AP polyline
  + icon dispatch — the AP map renderer already exists in `ArduMissionLayer`; extract the draw logic
  or add an AP branch in the preview).
- The list shows a small **format badge** (`INAV` / `ArduPilot` / `PX4`) on **every** mission so mixed
  libraries are clear.

### D5 — Format filter (shipped)
A filter dropdown sits next to the Import button and appears **only when ≥2 formats are present** in the
library (pure-INAV users never see it). It lists "All" + **only the formats actually present**, ordered
INAV → ArduPilot → PX4 → other, derived from the live mission list — so **PX4 shows up automatically**
once a PX4 mission exists, with no code change. If the last mission of a filtered format is deleted, the
filter falls back to "All".

### D4 — PX4-forward structure
- DB `format` stores the **concrete system** (`'ardupilot'` / `'px4'`), never a generic `'mavlink'` —
  so display, filtering and future PX4-specific quirks have a key to hang on.
- The builder/hash/metadata code is **system-agnostic** over `ArduWaypoint` (one path). PX4 reuses it
  unchanged; PX4-specific behaviour stays in the layers that already branch on system (command
  catalog, modes, param semantics).
- Helper naming: keep it under the ArduPilot file for now (`missionArdupilot.ts`) but name the
  functions for the model, not the vendor (`hashArduWaypoints`, `buildArduMissionInput`) so PX4 reuse
  reads naturally. No premature "mavMission" abstraction layer until PX4 actually lands.

---

## Phase 1 — Core library parity

### 1a. AP library builder (`missionArdupilot.ts` or a small `missionLibraryArdu.ts`)
- `hashArduWaypoints(wps)` (D1).
- `arduMissionContentHash(wps): Promise<string>` = SHA-256 of the above (reuse the `sha256Hex` util —
  factor it out of `missionLibrary.ts` into a shared helper).
- `computeArduMissionMetadata(wps)` — wp count, total leg distance, alt min/max/diff, bbox, over the
  geo items (`arduHasLocation`). Mirror `computeMissionMetadata`; alt is already metres (no cm convert).
- `buildArduMissionInput(wps, opts)` → `LibraryMissionInput` with `format: activeSystem`,
  `waypoints_json: JSON.stringify(wps)`, `home_lat/lon` per D1, `source_xml: null`.
- `findArduLibraryMissionId(wps, dbPath)` — content-hash lookup (sets `arduLoadedMissionId` on import).

### 1b. `arduLoadedMissionId` store + load helpers (`missionArdupilot.ts`)
- `export const arduLoadedMissionId = writable<number | null>(null);` — mirrors `loadedMissionId`.
- Reset to `null` on `arduMissionClear`, on FC download, on file open (fresh provenance).
- (Phase 2 adds the full fc/file/db provenance flags; Phase 1 only needs the DB id for linking.)

### 1c. `ArduMissionPanel` — library buttons
- Add **"Save to library"** (mirror `InavMissionPanel.handleSaveToLibrary`): NEW vs OVERWRITE by
  `arduLoadedMissionId`, dedup collision → adopt existing id, geocode after save.
- Add a **"Mission Manager"** toggle (same entry point INAV uses) to open `MissionManager`.
- Reuse `ConfirmDialog` + a name/notes save dialog (share the INAV one).

### 1d. `MissionManager` — format-aware
- `loadToMap(m)`: branch on `m.format` (D3) — system switch (D2) then load into the right store.
- Preview: pass `m.format` to `MissionPreviewMap`.
- Import flow (`importMission`): build via the INAV or AP builder by the **active system** /
  imported file type; `.waypoints` drop/open → AP path, `.mission` → INAV path.
- List item: format badge; grouping by location unchanged.

### 1e. `MissionPreviewMap` — format prop
- `let { waypointsJson, format = 'inav' }: {...} = $props();`
- Branch the `draw()` to the AP renderer for `'ardupilot' | 'px4'` (extract from `ArduMissionLayer`
  or add a parallel draw path). INAV path unchanged.

### 1f. Flight↔mission link parity (AP live flights, MAVLink)
- **Arm-time / End-Flight capture** (`+page.svelte` ~1795): when the connected system is ArduPilot/PX4,
  read `arduLoadedMissionId`; if null, build + save via `buildArduMissionInput`, then
  `flightLinkMission(flightId, id)`. (Same shape as the INAV branch — dispatch by active system.)
- **Manual link in `FlightDetail`** (~121): the "link current mission" path resolves the active
  system's loaded mission (INAV `loadedMissionId` or `arduLoadedMissionId`) and builds with the
  matching builder.
- **Replay load** (`+page.svelte` ~1255): `missionDbForFlight` returns the linked mission; load it by
  `format` into the correct store + set the matching `*LoadedMissionId`.

---

## Phase 2 — Provenance parity (sync indicators)

Mirror [MISSION_TRACKING_AND_PROVENANCE.md](MISSION_TRACKING_AND_PROVENANCE.md) for the AP store:
fc / file / db flags computed by content-hash comparison against sync snapshots, surfaced as the
same indicator chips in `ArduMissionPanel`. Lower priority — Phase 1 delivers the functional library;
Phase 2 adds the "is the on-map mission the saved one?" visual parity. (ArduPilot has no multi-mission
slots, so the per-slot snapshot map collapses to a single slot — simpler than INAV.)

---

## Out of scope / future

- **INAV-over-MAVLink** and **ArduPilot-over-MSP** — unchanged from the multi-autopilot plan (future).
- **PX4 hardware validation** — structure is PX4-ready; actual testing waits on hardware.
- **EEPROM/storage save** for AP missions — separate from the library (FC-side `PREFLIGHT_STORAGE`).

---

## Touch list (Phase 1)

| File | Change |
|---|---|
| `src/lib/helpers/missionLibrary.ts` | Extract `sha256Hex` to a shared util (or re-export) |
| `src/lib/stores/missionArdupilot.ts` | `arduLoadedMissionId`, `hashArduWaypoints`, builder/metadata/find helpers (or split into `missionLibraryArdu.ts`) |
| `src/lib/components/mission/ArduMissionPanel.svelte` | Save-to-library + Mission Manager toggle + status |
| `src/lib/components/mission/MissionManager.svelte` | Format-aware load / preview / import / badge |
| `src/lib/components/mission/MissionPreviewMap.svelte` | `format` prop + AP draw branch |
| `src/routes/+page.svelte` | AP branch in arm-time/End-Flight capture + replay load by format |
| `src/lib/components/logbook/FlightDetail.svelte` | AP branch in manual link |
| `src/lib/i18n/locales/{en,de}.json` | New keys (save-to-library, format badge, switch-keep dialog) |

No Rust changes expected — the DB layer and link commands are already format-agnostic.
(Verify the home-item handling of `ardu_mission_download` per D1; that's a read, not necessarily a change.)
```
