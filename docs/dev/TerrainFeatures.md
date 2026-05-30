# Terrain Features — Concept & Implementation Plan

**Status**: Planning (temporary working doc — remove once implemented & documented in ARCHITECTURE.md/DEVLOG.md, like PatternGenerator.md was)
**Last Updated**: 2026-05-30

This doc plans the four terrain-based features for the **2D map / mission planning** path. The 3D map (Cesium) is out of scope here — its quirks are tracked separately under Milestone 7.

---

## 1. Decision: data source & datum

**Source: Copernicus DEM GLO-30** (free, no API key, AWS Open Data, GeoTIFF, 1°×1° tiles, ~30 m resolution).

Chosen over SRTM / Mapzen-Terrarium:
- Accuracy ~±4 m RMSE vs ~±9 m (Terrarium is SRTM-grade globally)
- Global coverage **including > 60°N** (SRTM/Terrarium have none)
- **Geoid-referenced (EGM2008 ≈ MSL)**

### Why the datum matters (the key simplification)

| Quantity | Reference |
|---|---|
| GPS altitude reported by FC | MSL (orthometric, ≈ geoid) |
| INAV waypoint altitude (AMSL mode) | MSL |
| Copernicus GLO-30 elevation | EGM2008 geoid ≈ MSL |

Because all three are **≈ MSL**, terrain elevation, GPS altitude, and AMSL waypoints are **directly comparable** — no geoid-undulation conversion needed (unlike Cesium World Terrain, which is ellipsoid-referenced and needs the `geoidOffset` hack). This is the core reason Copernicus fits the 2D/planning use cases cleanly.

> **Verified: reference tools disagree on datum.** Per the mwp docs, INAV Configurator uses Bing **"Ellipsoid"** elevations while mwp uses Bing **"Sea Level"** — at one test point 526 m vs 470 m (real ≈ 483 m). Picking a geoid/MSL source (Copernicus EGM2008) keeps us consistent with the FC's MSL GPS altitude and with mwp's sea-level approach, rather than the ellipsoid mismatch.

> Residual nuance to confirm during implementation: GLO-30 is a **DSM** (surface model — includes vegetation/buildings), not a bare-earth DTM. For clearance this is arguably safer (conservative), but worth noting. FABDEM (bare-earth, Copernicus-derived) is a possible later alternative if DSM proves problematic.

Used **locally only** — for visualization/validation. Independent of any onboard FC terrain (INAV 10 will add onboard SRTM1; irrelevant to us).

---

## 2. Foundation: shared elevation provider

All four features depend on one elevation-sampling abstraction. Build this first.

**Capabilities:**
- `elevation(lat, lon) -> meters MSL` — single point (bilinear interpolation within the tile)
- `profile(points[], spacing) -> samples[]` — batch sample along a polyline at a fixed ground spacing (for the clearance graph)
- Tile fetch + decode (GeoTIFF) + on-disk cache; region pre-download for offline use (respect portable mode, like `flights.db`)

**Decided & implemented (Phase A):** Rust backend module `src-tauri/src/terrain/`. Source `copernicus-dem-30m` S3 (no key); tile `Copernicus_DSM_COG_10_{N|S}lat_00_{E|W}lon_00_DEM`; full-tile fetch → disk cache (portable-aware) → `tiff`-crate decode (Float32, DEFLATE, predictor 3 — verified, Zugspitze 2943.8 m) → in-memory LRU (4 tiles) → bilinear sample. CPU decode + disk I/O run on `spawn_blocking`; loads serialized/coalesced via async lock. Commands `terrain_elevation` / `terrain_profile`.

**Performance follow-up (planned — important for weak hardware):**
Full-tile decode is ~1 s on a fast CPU → ~5–10 s on a field laptop/tablet. `spawn_blocking` prevents runtime stalls (no freeze), but it's a latency cost on the first sample per tile.
- **COG partial reads**: Copernicus tiles are internally tiled Cloud-Optimized GeoTIFFs. Use HTTP **range requests** to fetch only the internal blocks covering the needed points (a few hundred KB) and decode only those chunks (`tiff` `read_chunk`) instead of the whole 42 MB / 13 M-pixel image. Turns multi-second decodes into sub-100 ms and slashes bandwidth.
- Optionally persist decoded/needed blocks; pre-fetch around a mission bounding box.

**Open questions (TBD):**
- Cache eviction tuning; pre-fetch radius around a mission.
- Void / nodata handling (GLO-30 ocean = 0).
- DSM vs DTM (GLO-30 includes surface objects — conservative for clearance).

---

## 3. Features (implementation order)

### Feature 1 — AGL waypoints (mission planning) — **FIRST**

Add **AGL (above-ground-level)** as a waypoint altitude reference, alongside the existing REL (relative-to-home) and AMSL.

**Verified INAV format facts (research 2026-05-29):**
- INAV waypoints (firmware + `.mission` XML) have **only two** altitude modes, encoded in **P3 bit 0**: `0` = relative-to-home, `1` = AMSL. There is **no AGL flag**. The `.mission` XML `missionitem` carries `action, lat, lon, alt, parameter1/2/3, flag` — no AGL attribute.
- The reference tool **mwp also does not store AGL** — it does terrain *clearance analysis* on REL/AMSL missions; it does not add an AGL waypoint mode.
- → **AGL is a GCS-only authoring concept.** It must be converted to AMSL on every export.

**Design (decided):**
- In-memory waypoint model stores `alt` + `mode` where mode ∈ {REL, AMSL, **AGL**}. AGL value is kept as-is in RAM (and across AGL on/off toggles) — **no repeated re-conversion** (avoids the rounding/drift errors other tools hit).
- **Convert AGL → AMSL only at export** (both MSP upload and `.mission` save): `AMSL = terrain_elevation(lat, lon) + agl_offset`, written with P3 bit 0 = 1 (AMSL). Clean because Copernicus ≈ MSL.
- **Single waypoints**: WP editor altitude-mode selector gains "AGL".
- **Pattern system**: the survey `altMode` already lists `ground` as a disabled "coming soon" placeholder → becomes the AGL option (enable it).
- **Not round-trippable**: since `.mission`/FC store AMSL, a loaded/downloaded mission returns as AMSL — AGL intent is lost on save/upload. UI must communicate this ("AGL is converted to AMSL on export").

**Launch-point reference & mode conversions:**

A GCS-side **launch-point marker** represents the home/arming location and provides the base altitude for any REL↔AGL/terrain math. (INAV resolves REL waypoints against the arming altitude at arm time; this marker is our planning-time proxy.)

- **Auto-placed** when needed (default: FC home if connected/armed, else first geo-WP / map center); **user-movable**. Its terrain elevation `terrain_MSL(launch)` = the home-altitude reference.
- **Persisted in the `.mission` file** via the MW XML `<mwp home-x=… home-y=…/>` meta element (verified 2026-05-29): mwp already stores a planned home here; other tools (INAV Configurator) read only `<missionitem>` and ignore unknown meta → safe to write, no interop break. On load, restore the launch marker from `home-x`/`home-y` if present, else auto-place. Keeps the launch point consistent across save/load/archive.
  - Our codec ([`mission/store.rs`](../../src-tauri/src/mission/store.rs)) currently emits only `<version>` + `<missionitem>`; extend `mission_to_xml` / `mission_from_xml` to emit & parse the `<mwp>` element. **TBD**: confirm exact attribute semantics (`x` = lon, `y` = lat?) and scaling against a real mwp sample.
  - The format stores home *position* only, no altitude — we derive `terrain_MSL(launch)` from Copernicus at that position.
- **Correctness of the launch location is the user's responsibility** — the *planned* home is now persisted/consistent, but if the *real* arming position differs at flight time, absolute altitudes shift (same caveat as any REL mission).
- Setting **any** waypoint to AGL ensures a launch point exists (auto-place if missing).

Conversion when switching a waypoint's mode **to AGL**:

| From | Rule | Launch point |
|---|---|---|
| AMSL | `AGL = AMSL_alt − terrain_MSL(wp)` | not required for the math (auto-placed anyway for consistency / clearance) |
| REL | `AGL = terrain_MSL(launch) + rel_alt − terrain_MSL(wp)` | **required** — if none set, block with a popup prompting the user to place it; retry after |

Export **AGL → AMSL** (MSP + `.mission`): `AMSL = terrain_MSL(wp) + agl_offset` — launch point not needed for this direction.

Downstream: once a launch point exists, REL waypoints gain an MSL reference, so **terrain clearance analysis (Feature 2) works for REL missions too**.

Mixed-mode missions: INAV supports per-waypoint mode (P3 bit 0), so REL and AMSL can coexist; loaded waypoints keep their original mode until the user explicitly switches one to AGL.

**Open questions (TBD):**
- Launch point scope: per active mission vs global (lean: per mission).
- Show computed AMSL alongside the AGL input in the editor (so the user sees the exported value).
- Behaviour when terrain data for a WP location isn't available (offline / void) — warn & block export, or fall back to REL/AMSL?
- Whether to surface a per-mission "convert all to AMSL" preview before upload.

### Feature 2 — Terrain analysis (clearance validation & correction) — **SECOND / NEXT**

A complex analysis tool opened as a **full-width, viewport-centered overlay** over the map, triggered from the **NavRail** (not a narrow side panel — the chart is wide/short by nature, and the tool has no relevant map representation, so overlaying the map is fine). A simplified live glance is the later 2×1 widget (Feature 3), not this.

**Two modes (toggle in the header):**
- **Waypoint Analysis** — the planned mission: editable WP altitudes + auto-correction.
- **Track Analysis** — a flown track, **read-only** (a flown track can't be corrected). Source is whichever track is on the map: the **live MSP temp-log** *or* a **loaded blackbox log**. These are mutually exclusive (log loading is blocked while an MSP connection exists), so **no source selector** is needed — same on-map data structure either way → one render pipeline.

**Chart — hand-rolled SVG, no external runtime dependency** (matches our widget stack; themeable, crisp, natively interactive — unlike mwp which shells out to an external graph tool):
- **X-axis**: cumulative ground distance along the route/track
- **Y-axis**: altitude (MSL)
- **Terrain fill** (orange): sampled via `terrain_profile()` at **30 m spacing** (= GLO-30 native resolution)
- **Path line** (blue): waypoint/track MSL altitudes connected, with waypoints marked
- **Clearance** = path altitude − terrain; segments below the safety margin highlighted red
- **Hover crosshair** showing distance / altitude / clearance under the cursor (later: linked to a marker on the map)
- **Horizontal zoom + pan** (essential — 120 INAV WPs, or hundreds on ArduPilot, get crowded otherwise): mouse-wheel zoom **stretches the X-axis only**, drag scrolls along the distance axis; touch = pinch + drag. Implemented via SVG `viewBox` on the X domain.

**Controls (left column):**
- **Ground Clearance** (m) — the *single* height parameter. Serves as **target AGL** in Terrain Follow and as **minimum AGL** in Clearance Check. (Not called "safety margin": the point is to not go *below* a height, so one clearance value is the natural concept.) Changing it live-updates the correction preview.
- **Climb-angle limit (°)** + **Fixed-Wing** toggle. Angle, *not* climb rate, is the edited parameter: a fixed wing's *vertical* climb rate is roughly constant, but the resulting *ground* climb angle varies with wind (headwind → steeper, tailwind → shallower) and airspeed — so the operator edits the geometric angle directly.
- **Average airspeed** (optional, default **0 = off**): when set, show the *calculated required climb rate* = `airspeed × sin(angle)` as an informational readout (vertical component of the path velocity vector; wind ignored). Operator-supplied because INAV fixed-wing has no speed control yet (ArduPilot does). Future extension: refine with wind/weather data — needs a paid API (e.g. Windy), out of scope for now.
- **Vertical exaggeration**.
- **Terrain Correction** sub-panel (**Waypoint mode only**) — mode selector + WP range + Add-Waypoints toggle + APPLY; see below.
- **Datum toggle** MSL / AGL:
  - **MSL view** — terrain filled to its elevation + the flight/track altitude line, both in MSL.
  - **AGL view** — plots the **clearance curve** (flight altitude − terrain) with the ground flattened to a 0 baseline; clearance violations read straight off the zero line. (Value-add over the reference tools, which only show the MSL side-view.)

**Readouts (bottom):** min clearance (with warning icon), max climb angle, total distance, value under cursor.

**State persistence:** all panel parameters (mode, Ground Clearance, climb-angle limit, Fixed-Wing toggle, airspeed, vertical exaggeration, WP range, Add-Waypoints toggle, zoom/pan, datum) are kept for the **whole app session** — closing and reopening the panel restores the last state. Reset only when the app itself closes (in-memory store, **not** written to disk).

**Waypoint editing:** **no dragging.** Click a waypoint to select it → set its altitude via a numeric field, respecting its alt mode (REL/AMSL/AGL).

**Terrain Correction (Waypoint mode only — explicit, never automatic):**

Two sub-features chosen by a mode selector. Both operate on a **WP range** (two fields *Start WP* / *End WP*, default first/last) so corrections target a **section only** — often just part of a mission must follow terrain while other legs stay at a safe travel altitude. Changing **Ground Clearance** (or any parameter) live-renders a **differently-colored preview** of the corrected track; nothing is written until **APPLY**, which updates the affected waypoints in **bulk**. All corrected waypoints are set to **AGL mode** afterwards (so the map shows the correct reference and the user sees the right datum).

- **Terrain Follow** — adjust WPs *to* the target height where possible:
  1. Set every WP in range to Ground-Clearance AGL.
  2. Check each track (leg) between WPs.
  3. Where a leg drops below clearance, raise **both** adjacent WPs until the leg is clear.
  4. *(optional **Add Waypoints** toggle)* insert **one** WP at the leg's highest terrain point to lift that leg locally — at most **one extra WP per leg per analysis run**.

- **Clearance Check** — raise only, never lower:
  1. Check WPs, raise any below clearance.
  2. Check legs, raise **both** endpoints of an offending leg simultaneously until clear.
  3. No waypoint insertion.

**Climb-angle limit** (Fixed-Wing) applies on top of both modes: the ground climb/descent angle between consecutive WPs must stay ≤ limit; if a required raise would exceed it, **propagate the climb earlier** (raise preceding WPs *within the range*) so the climb starts sooner.

Raising **iterates to convergence** (one raised WP affects both its neighbouring legs); only **waypoint insertion** is capped at one per leg per run.

**Datum advantage:** we plot terrain in MSL (Copernicus EGM2008), consistent with FC GPS MSL and AMSL waypoints — more correct than INAV Configurator, which labels its terrain as WGS84/ellipsoid (see §1).

**Phasing:**
1. ✅ **Read-only chart** (Waypoint + Track modes): terrain + path + clearance coloring + zoom/pan + readouts + hover. *(implemented — see notes below)*
2. **Terrain Correction** — both modes (Terrain Follow / Clearance Check) + Ground Clearance + WP range + climb-angle limit + Add-Waypoints, live preview → APPLY; click-to-select + numeric WP altitude edit.
3. Map-marker hover link; polish.

**Phase 1 implementation notes (decided while building):**
- **Full-width NavRail overlay**, mutually exclusive with the nav panel (only one panel open at a time). All params live in an in-memory session store (`stores/terrainAnalysis.ts`).
- **Hand-rolled SVG** chart (`TerrainProfileChart.svelte`); profile data built in `helpers/terrainProfile.ts` from `terrain_profile` + per-WP MSL resolution.
- **Clearance analysis ignores take-off & landing**: the leading/trailing runs that sit *below* clearance (we start/land on the ground) are trimmed, so only the en-route portion drives the min-clearance alert and red coloring. A *mid-route* dip below clearance still alerts.
- **Climb angle is low-pass filtered for tracks**: flown-track altitude jitter spikes per-sample slopes toward 90°, so the track climb angle is computed over a ~10-sample smoothed altitude measured per ≥20 m segment. Waypoint vertices are used as-is.
- **Rendering scales with zoom**: only the visible distance slice is drawn, decimated to ~screen resolution (per-bucket worst-clearance / peak-terrain sample, so peaks and unsafe spots survive). Full-resolution data still drives the readouts. Zooming in reveals more detail without rendering the whole route.
- **Datum toggle** MSL ↔ AGL (clearance curve) as above.
- **Per-mode RAM cache**: Waypoint and Track profiles are cached by signature, so toggling between them is instant (no re-sampling) until the route/track changes.

**Compact mode (map-linked cursor)** — a *"Show Map"* toggle (first button in the header) collapses the analyzer to a short **top-docked strip** (≈20 vh, stops short of the side widget dock so widgets stay visible), leaving the map usable above. The chart cursor is mirrored onto the 2D map (`TerrainCursorLayer`), giving the spatial *where* that a side-view alone lacks:
- a **transient hover dot** follows the mouse over the profile;
- a **click pins a persistent marker** (click again to clear) that sits on the mission track / flight trail (per view mode), and is **mirrored back into the chart** as a vertical pin line (nearest-sample by lat/lon; hidden if the pin isn't on the current path).
- The pinned marker is **visual-only** (not editable) and **persists in the session store even when the panel is closed**, so it stays as a reference on the map while editing in mission control. State lives in `terrainCursor`; the chart emits lat/lon per sample. (2D only for now; 3D follows the later Cesium rework.)

**Still TBD:** handling of RTH / LAND / loiter / jump legs in the distance domain; void/nodata terrain segments; exact climb-angle propagation when constraints conflict.

### Feature 3 — AGL widget (live flight) — **THIRD**

A **mini, informational version of the Track-Analysis chart** as a HUD widget in **2×1 full-size horizontal format**: live AGL = `GPS MSL altitude − terrain_elevation(current lat, lon)`, shown with a small terrain profile. Reuses the Feature 2 SVG chart component (compact instance, live track source).

**Design TBD**: forward-looking window vs current point only, scale/axis, update rate (likely the same 30 m / 1–2 s cadence as the overlay's live mode).

### Feature 4 — LOS (line-of-sight) analysis — **LAST, no priority**

Line-of-sight / radio-horizon analysis along the route (à la MWPTools): detect where terrain occludes the line between the GCS/home and points along the mission.

**TBD**: home/GCS reference point, occlusion sampling, visualization (on map and/or in the profile view), antenna height assumptions.

---

## 4. Implementation order (summary)

1. ✅ **Shared elevation provider** (foundation) — Copernicus GLO-30, Rust backend, validated
2. ✅ **AGL waypoints** — WP editor alt-mode (REL/AMSL/AGL) with terrain conversion, survey-pattern `ground`/AGL, export AGL→AMSL, launch point + `<mwp>` persistence. Validated against INAV Configurator terrain analysis.
3. **Terrain analysis** — full-width NavRail overlay; view modes Waypoint / Track; SVG profile chart with zoom/pan + clearance coloring; Terrain Correction (Terrain Follow / Clearance Check) over a WP range, preview → APPLY *(next)*
4. **AGL widget** — mini 2×1 profile, live (reuses the Feature 2 chart)
5. **LOS analysis** — deferred, low priority

## 5. Protocol scope (TBD)

AGL is currently implemented for the **INAV mission path only** (MSP WP / MW XML). The **ArduPilot/PX4** mission path is still rudimentary (separate MAVLink WP implementation; no test hardware), so **AGL there is TBD**. ArduPilot/PX4 expose a native terrain-follow frame (`MAV_FRAME_GLOBAL_TERRAIN_ALT`); a future implementation would likely map AGL onto that frame rather than resolving to AMSL — decided once test hardware exists.

---

*This is a living planning doc. Details in each "TBD" are resolved as we work through the features one at a time.*
