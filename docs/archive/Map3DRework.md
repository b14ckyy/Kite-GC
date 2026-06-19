# 3D Map Rework — Plan

> **ARCHIVED (2026-06-19) — fully shipped.** All phases landed: track outline + ground shadow +
> altitude curtain incl. the live trail (Phase 1), mission path in 3D (Phase 2), camera incl. the
> heading-follow jitter fix **and the FPV cockpit view** (Phase 3 — later extended with a vector
> marker), and the jagged-track / geoid-altitude rework (Phase 4). Some `[ ]` checkboxes below were
> simply never ticked; the code is the source of truth (`Map3D.svelte`: `Camera3DMode` incl. `'fpv'`,
> `enterFpv`/`updateFpvCamera`, the altitude curtain, the terrain-derived geoid). Kept for the
> detailed build reasoning. See also [[project_3d_rework]].

Working plan for the CesiumJS 3D view (`src/lib/components/Map3D.svelte`) overhaul.
Agreed 2026-05-31. Built in phases; tick items as they land.

## Goals (user-stated)

1. Better **path/track rendering** (altitude legibility, cleaner line, planned mission in 3D).
2. Better **camera follow** (FPV cockpit, stabilization, follow-orientation detail).
3. **Views**: *no change* — the current `free → follow → orbit` cycle is fine. (The "Other"
   answer in planning was only because the picker required a selection.)

## Current state (baseline)

- **Track**: `updatePlaybackTrack3D` adds flight-mode/altitude/speed-colored `polyline`
  entities at MSL altitude (`width: 3`, `ColorMaterialProperty`, `clampToGround: false`).
  Altitude per point = **`alt_m` (raw GPS MSL)**, corrected by a `geoidOffset` computed once
  from the first GPS point vs. Cesium World Terrain. Live trail = `updateTrail3D` (same idea,
  growing). `flyTo` on track load.
- **Camera**: `free` / `follow` (chase behind UAV, `lockRange` zoom, `followPitch`
  user-adjustable, exponential `CHASE_SMOOTHING` lerp) / `orbit` (locked target, free orbit).
  `cycleCameraMode` cycles the three. UAV = coloured position **point** (the SVG arrow billboard
  was removed 2026-06-04 — its rotation didn't read well in 3D and a proper 3D GLTF model is
  planned to replace it; see ROADMAP M7).

## Phase 1 — Track line: outline + ground shadow + vertical wall

For the playback track first, then the live trail.

- [ ] **Black outline** on the track line — `PolylineOutlineMaterialProperty`
  (`color` = segment color, `outlineColor` = black, `outlineWidth` ≈ 1–2, width ≈ 4–5).
- [ ] **Ground shadow** — a second polyline per segment with `clampToGround: true`, grey,
  ~30% alpha, following the same lat/lon (drape on terrain).
- [ ] **Vertical wall** — a Cesium `wall` entity per segment: top heights = track MSL,
  bottom = terrain ground height (sampled), material = **flight-mode color at 20–25% alpha**
  (fall back to **white** if it reads too busy). Bottom height: prefer terrain-sampled
  `minimumHeights`; if too costly, rely on `depthTestAgainstTerrain` occlusion with a low
  fixed minimum.
- Applies to the live trail too (incremental; the wall grows behind the UAV).
  - **Status**: playback track done (outline + draped shadow + curtain; chunked
    growing build for scale; reverse-scrub debounce; `altitudeCurtain3D` setting
    toggle in Settings→Map). **Live-trail curtain (1b) deferred** until the
    simulator is available for hours-long live MSP tests (near release) — needs
    the same chunking applied to `updateTrail3D`.

## Phase 2 — Mission path in 3D + "Show Mission" toggle  ✅

Decided: the 3D mission must look **exactly like 2D** (no fancy 3D elements) — same line
colours/styles, and the **same marker SVGs** rendered as **viewport-facing billboards**
(fixed pixel size, always visible — "projected onto the viewport"). The *only* 3D addition is
a **drop-line** from each waypoint to the ground (thin **white dashed + black outline**).

- [x] Shared marker SVGs: `missionIcons.ts` now exposes `wpIconSpec()` (2D divIcon + 3D
  billboard use the identical SVG). Shared geometry helpers extracted to `missionGeometry.ts`
  (display numbering, flight-path filter, mission-end, modifiers). New altitude resolver
  `resolveMissionAltitudes()` in `terrainProfile.ts` (per-WP MSL + ground; REL/AMSL/AGL).
- [x] 3D render (`Map3D.renderMission3D`): flight path (blue) + greyed-beyond-end (grey
  dashed) + launch→WP1 (orange dashed) + jump (purple dashed) + RTH (orange dashed) lines, all
  as an **always-visible overlay** (`depthFailMaterial` on lines, `disableDepthTestDistance`
  on billboards) so it reads like the flat 2D map; markers = billboards from the shared SVGs;
  per-WP **drop-lines** (white dashed + black dashed outline) to the ground.
- [x] **Visibility**: a `showMission` store + `replayActive` store. Replay → the **MISSION
  toggle** in the LogPlayer (after REC/BBX) controls it (2D + 3D). Planning/live → always
  shown. Default on.
- **Known limitation**: `geoidOffset` is derived from the flown track at load, so the 3D
  mission sits correctly in **replay**. In pure **planning** 3D (no track loaded) `geoidOffset`
  is 0 → the mission may float off terrain by the local geoid undulation (~tens of m). Resolve
  later (sample Cesium terrain vs Copernicus at a mission point to derive the offset).

## Phase 3 — Camera

- [x] **Heading-follow jitter fix** (done): Cesium's own rotate/tilt/look/pan are disabled in
  follow (`setFollowCameraControls`), so a sideways drag can't fight the per-frame heading
  lock; `followPitch` is driven by a custom vertical-drag `ScreenSpaceEventHandler` (clamped
  0…−90°) instead of being read back from `camera.pitch` each frame (that read-back was also
  why the start angle jumped to the leftover free-cam −45°). **Start pitch lowered to −20°.**
- [ ] **FPV cockpit** view: camera **at** the UAV looking along the flight direction, **no
  visible UAV model**. Must be **stabilized** (smoothed heading/pitch/position) — tune.
- [x] ~~**Follow tuning**~~ — done. The new defaults (start pitch −20°, custom vertical-drag
  pitch, disabled Cesium rotate, range/pitch sliders) feel right; no further tuning planned.
  Settings exposure can wait until there is demand.

## Phase 4 — Jagged-track smoothing  ✅ (resolved without resampling)

**Resolved.** The stair-stepping was a **vertical** problem (raw GPS/quantized baro altitude).
Switching the 3D track to INAV's **fused EKF altitude** (`nav_alt_m`, see the "clean geoid +
altitude rework" section below) made the track smooth — no Catmull-Rom / spline resample was
needed. Horizontal GPS spacing is fine at the colored-segment resolution. The original
analysis is kept below for reference.

The user's main visual gripe was a **stair-stepped / jagged** track. Investigated against a
**decoded log** before fixing. Hypotheses (historical):

- **Horizontal**: GPS is ~**2 Hz** → sparse points → straight segments / sharp corners.
- **Vertical**: the 3D track uses **raw `alt_m` (GPS MSL)**, which is noisy/quantized.
  Smoother sources exist in `TelemetryRecord`: **`nav_alt_m`** (INAV nav estimate),
  **`baro_alt_m`**. Switching the vertical source must stay **MSL-consistent** (the
  `geoidOffset` is derived from GPS MSL at arming — baro/nav are relative, so a conversion or
  re-reference is needed).

Likely fix: **Catmull-Rom / Hermite spline resample** of the polyline (horizontal smoothing)
+ a smoother altitude source (vertical). Verify point spacing and compare the three altitude
columns on a real log first.

### DONE — clean geoid + altitude rework (replay track)

Validated against a decoded blackbox (and `flightlog/blackbox.rs:623`): **`nav_alt_m` =
`navPos[2]/100` = fused EKF altitude, relative to home, 0 at arm** — smooth (the GCS already
uses it for the Altitude widget). `BaroAlt` carries an arm offset (~1.4 m); GPS `alt_m` is
erratic. Implemented in `Map3D`:

- **Geoid `N` is now terrain-derived**: `N = cesiumGround_ellipsoid(firstPt) −
  copernicus terrain_elevation(firstPt)` (was `cesiumGround − GPS_MSL(firstPt)`, which snapped
  the arm point to the ground and shifted a tower/rooftop start — and the whole track — down).
- **Track altitude is now the relative fused source anchored absolutely**:
  `ellipsoid = startMslGps + N + nav_alt_m` where `startMslGps = alt_m` of the first GPS fix.
  Tower/rooftop starts keep their height; track shape stays smooth (nav, not GPS) — this also
  addresses the Phase-4 vertical jaggedness. Applied to the track line, the progressive
  shadow/curtain (`posFromRecord`), and the playback marker. Mission stays `altMsl + N`
  (Copernicus MSL) → consistent with the track (both = trueMSL + N).
- Live path (telemetry) unchanged for now — revisit with the live-trail curtain (1b) during
  simulator testing.
- Caveat seen in a launch-phase snippet: `nav_alt_m` (EKF) lags fast launch transients
  (heavily damped) — fine for a smooth track; verify visually in replay.

### DONE — source-switch clearing, trails, recenter, tile refresh (this session)

Fixes that emerged while testing the above on real logs + a live link:

- **Source-switch clearing** (`clearAllMapData`): switching replay log↔log and replay→live
  wipes the playback track, progressive deco, live trail, live + replay markers and home, and
  resets the geoid/anchor state. A fresh **live connect clears only when DISARMED** (an armed
  reconnect keeps the track — connection recovery); a **disconnect never clears**. The mission
  overlay is intentionally kept and re-placed at the new geoid.
- **Cross-log deco fix**: `clearDeco()` now cancels its pending grow/rebuild timers, and a
  `decoLoading` guard (set across the async track load, with `decoValidTrack` cleared up front)
  stops the `playbackPoint` effect from appending stale/mixed points → no more shadow/curtain
  spanning the old + new track.
- **Live trail armed-only** + a thin plain **black, ground-clamped pre-arm trail** while
  disarmed (2D `Map.svelte` + 3D), cleared on arm.
- **Recenter on 2D→3D switch** (`recenter3D`): the `{#if}{:else}` toggle remounts Map3D, and
  the old inline `flyTo` ran before the canvas had a size → no-op on the first switch. Now
  deferred via rAF until the canvas is laid out; targets the UAV (replay marker / live UAV)
  with the track-start anchor as fallback.
- **Over-zoom placeholder auto-refresh**: when `fetchAndCacheImage` detects a *newly*
  unavailable region, a debounced `scheduleImageryRefresh()` re-applies the provider so the
  1–3 placeholder tiles that slipped through before the hash was confirmed are re-requested
  and replaced by the parent tile — no manual zoom needed.
- **Tile-distance/LOD experiments reverted**: tried `tileCacheSize`, `preloadSiblings`, a
  view-distance cap and higher `maximumScreenSpaceError` to curb the thousands of tiles loaded
  at grazing angles — all either ineffective or too muddy/limiting; left at Cesium defaults.

## Notes

- Views are intentionally unchanged.
- The marker height-line idea was dropped (redundant with the track + wall behind the UAV).
- Commit docs-before-code; run `npm run check` + `npm run build` before delivering.
