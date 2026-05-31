# 3D Map Rework — Plan

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
  `cycleCameraMode` cycles the three. UAV = SVG arrow billboard.

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

- [ ] **Heading-follow jitter fix**: when heading is locked (heading-follow), only **pitch**
  should be mouse-adjustable. Sideways drag jitters today because the chase loop forces
  heading back to `behindHeading` every frame while Cesium's default rotate also turns the
  heading → they fight. Fix: disable Cesium's horizontal rotate in this mode and drive pitch
  from a custom vertical-drag handler.
- [ ] **FPV cockpit** view: camera **at** the UAV looking along the flight direction, **no
  visible UAV model**. Must be **stabilized** (smoothed heading/pitch/position) — tune.
- [ ] **Follow tuning**: orientation detail, smoother/tunable defaults (distance/pitch),
  possibly exposed in settings later.

## Phase 4 — Jagged-track smoothing (after inspecting a decoded log)

The user's main visual gripe is a **stair-stepped / jagged** track. Investigate against a
**decoded log** before fixing. Hypotheses:

- **Horizontal**: GPS is ~**2 Hz** → sparse points → straight segments / sharp corners.
- **Vertical**: the 3D track uses **raw `alt_m` (GPS MSL)**, which is noisy/quantized.
  Smoother sources exist in `TelemetryRecord`: **`nav_alt_m`** (INAV nav estimate),
  **`baro_alt_m`**. Switching the vertical source must stay **MSL-consistent** (the
  `geoidOffset` is derived from GPS MSL at arming — baro/nav are relative, so a conversion or
  re-reference is needed).

Likely fix: **Catmull-Rom / Hermite spline resample** of the polyline (horizontal smoothing)
+ a smoother altitude source (vertical). Verify point spacing and compare the three altitude
columns on a real log first.

## Notes

- Views are intentionally unchanged.
- The marker height-line idea was dropped (redundant with the track + wall behind the UAV).
- Commit docs-before-code; run `npm run check` + `npm run build` before delivering.
