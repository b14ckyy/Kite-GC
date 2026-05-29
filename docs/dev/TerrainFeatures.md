# Terrain Features — Concept & Implementation Plan

**Status**: Planning (temporary working doc — remove once implemented & documented in ARCHITECTURE.md/DEVLOG.md, like PatternGenerator.md was)
**Last Updated**: 2026-05-29

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

### Feature 2 — Terrain clearance validation — **SECOND**

A popup window showing a **1-D side-view (profile)** of the planned waypoint mission:
- **X-axis**: cumulative distance along the route
- **Y-axis**: altitude
- **Flight path line**: waypoint MSL altitudes connected, with waypoints marked
- **Terrain graph** below: terrain elevation sampled along the route (via `profile()`)
- **Clearance** = flight altitude − terrain; warn/highlight where below a configurable threshold

**Exact features TBD**: clearance threshold config, color-coding of unsafe segments, min-clearance readout, per-leg warnings, handling of RTH / LAND / loiter / jump legs, vertical exaggeration.

### Feature 3 — AGL widget (live flight) — **THIRD**

A **mini version of the clearance side-view** as a HUD widget: live AGL = `GPS MSL altitude − terrain_elevation(current lat, lon)`, shown with a small terrain profile.

**Design TBD**: forward-looking window vs current point only, scale/axis, how it docks in the widget system, update rate.

### Feature 4 — LOS (line-of-sight) analysis — **LAST, no priority**

Line-of-sight / radio-horizon analysis along the route (à la MWPTools): detect where terrain occludes the line between the GCS/home and points along the mission.

**TBD**: home/GCS reference point, occlusion sampling, visualization (on map and/or in the profile view), antenna height assumptions.

---

## 4. Implementation order (summary)

1. ✅ **Shared elevation provider** (foundation) — Copernicus GLO-30, Rust backend, validated
2. ✅ **AGL waypoints** — WP editor alt-mode (REL/AMSL/AGL) with terrain conversion, survey-pattern `ground`/AGL, export AGL→AMSL, launch point + `<mwp>` persistence. Validated against INAV Configurator terrain analysis.
3. **Terrain clearance validation** — profile popup *(next)*
4. **AGL widget** — mini profile, live
5. **LOS analysis** — deferred, low priority

## 5. Protocol scope (TBD)

AGL is currently implemented for the **INAV mission path only** (MSP WP / MW XML). The **ArduPilot/PX4** mission path is still rudimentary (separate MAVLink WP implementation; no test hardware), so **AGL there is TBD**. ArduPilot/PX4 expose a native terrain-follow frame (`MAV_FRAME_GLOBAL_TERRAIN_ALT`); a future implementation would likely map AGL onto that frame rather than resolving to AMSL — decided once test hardware exists.

---

*This is a living planning doc. Details in each "TBD" are resolved as we work through the features one at a time.*
