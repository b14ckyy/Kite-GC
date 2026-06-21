# Terrain Features — Concept & Implementation Plan

> 📦 ARCHIVED (2026-06-21) — All four terrain features shipped (Feature 4 lives on in `RF_LINK_ANALYSIS.md`). Kept for reference.

**Status**: Implemented (all features done: elevation provider, AGL waypoints, terrain analysis + correction, Live AGL widget, Terrain Radar widget, **and Feature 4 — LOS analysis, shipped 2026-06-15 as the layered RF link analysis — see [RF_LINK_ANALYSIS.md](RF_LINK_ANALYSIS.md)**). COG performance follow-up cancelled.
**Last Updated**: 2026-06-15

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

**Performance follow-up — CANCELLED (not pursued):**
Full-tile decode is ~1 s on a fast CPU → ~5–10 s on a field laptop/tablet, but it runs on
`spawn_blocking` (no UI freeze) and decoded tiles stay in the in-memory LRU, so the latency is
a **one-time, first-sample-per-tile** cost that never recurs while flying/planning the same
area. In practice this is a non-issue, so the COG-partial-read optimisation below was dropped.
Kept for reference; revisit **only** if dedicated offline area pre-download becomes a goal.
- ~~**COG partial reads**: HTTP range requests to fetch + decode only the internal blocks
  covering the needed points instead of the whole 42 MB image.~~
- ~~Optionally persist decoded blocks; pre-fetch around a mission bounding box.~~

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
- **Terrain Correction** sub-panel (**Waypoint mode only**) — mode selector + WP range + manual *Add WP* + APPLY; see below.
- **Datum toggle** MSL / AGL:
  - **MSL view** — terrain filled to its elevation + the flight/track altitude line, both in MSL.
  - **AGL view** — plots the **clearance curve** (flight altitude − terrain) with the ground flattened to a 0 baseline; clearance violations read straight off the zero line. (Value-add over the reference tools, which only show the MSL side-view.)

**Readouts (bottom):** min clearance (with warning icon), max climb angle, total distance, value under cursor.

**State persistence:** all panel parameters (view mode, datum, compact, Ground Clearance, correction mode, climb/descent-angle limits, Fixed-Wing toggle, WP range, zoom/pan) are kept for the **whole app session** — closing and reopening the panel restores the last state. Reset only when the app itself closes (in-memory store, **not** written to disk).

**Waypoint editing:** **no dragging.** Click a waypoint to select it → set its altitude via a numeric field, respecting its alt mode (REL/AMSL/AGL).

**Terrain Correction (Waypoint mode only — explicit, never automatic):**

Two sub-features chosen by a mode selector. Both operate on a **WP range** (two fields *Start WP* / *End WP*, default first/last) so corrections target a **section only** — often just part of a mission must follow terrain while other legs stay at a safe travel altitude. Changing **Ground Clearance** (or any parameter) live-renders a **differently-colored preview** of the corrected track; nothing is written until **APPLY**, which updates the affected waypoints in **bulk**. All corrected waypoints are set to **AGL mode** afterwards (so the map shows the correct reference and the user sees the right datum).

- **Terrain Follow** — adjust WPs *to* the target height where possible:
  1. Set every WP in range to Ground-Clearance AGL.
  2. Check each track (leg) between WPs.
  3. Where a leg drops below clearance, raise **both** adjacent WPs until the leg is clear.
  4. *(no automatic insertion — see manual "Add WP" below)* If a leg still can't follow terrain well enough, the user pins a marker on the chart and clicks **Add WP** to drop a waypoint exactly on the track there, then re-runs Terrain Follow. (Auto-insertion was dropped — it added too many WPs and wasn't reliable enough.)

- **Clearance Check** — raise only, never lower:
  1. Check WPs, raise any below clearance.
  2. Check legs, raise **both** endpoints of an offending leg simultaneously until clear.
  3. No waypoint insertion.

**Climb-angle limit** (Fixed-Wing) applies on top of both modes: the ground climb/descent angle between consecutive WPs must stay ≤ limit; if a required raise would exceed it, **propagate the climb earlier** (raise preceding WPs *within the range*) so the climb starts sooner.

Raising **iterates to convergence** (one raised WP affects both its neighbouring legs). Waypoint insertion is **manual** (*Add WP*), not automatic — see the implementation notes below.

**Datum advantage:** we plot terrain in MSL (Copernicus EGM2008), consistent with FC GPS MSL and AMSL waypoints — more correct than INAV Configurator, which labels its terrain as WGS84/ellipsoid (see §1).

**Phasing:**
1. ✅ **Read-only chart** (Waypoint + Track modes): terrain + path + clearance coloring + zoom/pan + readouts + hover. *(implemented — see notes below)*
2. ✅ **Terrain Correction** — both modes (Terrain Follow / Clearance Check) + Ground Clearance + WP range + climb/descent-angle limit + manual *Add WP*, live preview → APPLY. *(implemented — see notes below)*
3. ✅ **Jump-loop simulation** — branch + cut, shared-altitude correction. *(implemented — see notes below)*
4. Map-marker hover link; polish.

**Phase 2 implementation notes (Terrain Correction):**
- Pure-function engine `helpers/terrainCorrection.ts` over the existing `ProfileData` — no new backend calls. Correctable WPs = Waypoint + PosHold within the range; **Land/RTH/Jump/SetHead and out-of-range WPs are fixed anchors** (never edited, leg endpoints only).
- **Terrain Follow**: set correctable WPs to `ground + clearance`, then lift legs to clear. **Clearance Check**: raise-only from the original altitudes.
- **No automatic WP insertion.** Instead a **manual "Add WP"**: click the chart to pin a marker, press *Add WP* → inserts a waypoint at that lat/lon on the current track (interpolated AMSL, so it sits on the path), respecting the WP limit; the user then re-runs Terrain Follow. Removal is done by editing WPs on the map.
- **Convergence loop** (monotonic raises): WP clearance → leg deficit (raise both endpoints by the max deficit; if one endpoint is an anchor, raise only the correctable one to the exact requirement) → climb/descent-angle pass (raise the *lower* endpoint of any too-steep leg; only-raise → never costs clearance). Climb/descent are two params, 0 = off, gated by Fixed-Wing.
- **Clearance warning at 95%** of the target (5% grace) — exact-clearance is no longer flagged red; at 50 m clearance the warning/red trips below 47 m.
- Output: changed WPs (→ **AGL mode**, value = `alt − ground`), a corrected **preview path** (green dashed, live as params change), min-clearance-after, and flags (climb forced above clearance / unresolvable anchor leg). **APPLY** updates the changed WPs in place via a confirm dialog.

**Live Track mode (FC connected):** When a live MSP/MAVLink connection is active, Track mode follows the **live flown track**:
- A shared **`liveTrack` store** (`stores/liveTrack.ts`) accumulates `{lat, lon, alt_m}` from telemetry **while armed** (5 m move filter); cleared on each new arm. Independent of the map trail (lat/lon only) and the flight-log DB — exists from arm onward regardless of whether the panel is open. The accumulator lives in `+page` (telemetry subscription); on arm it also **pre-fetches the Copernicus tile** for the current area (`terrain_elevation` warms the cache).
- The panel polls every **5 s** via an **incremental `LiveTrackProfiler`** (`terrainProfile.ts`) — only the *new* points are terrain-sampled (last known point as segment start for continuity); the cheap JS folding (clearance/min/climb) is recomputed over the accumulation. No full re-sample each tick.
- A **Follow** toggle (header, live only): **on** = view pinned to the right edge, zoom-only (no pan); **off** = fixed window, free pan + zoom over the growing data. Default live window **250 m** (matches the 30 m terrain resolution floor), building up left→right then scrolling. **Full zoom-out auto-fits** the whole growing range regardless of Follow. Disarm leaves the track complete for review (poll idles, controls unlock).

**Jump-loop simulation (Phase 2 step 3):** A jump is simulated as **one loop**, branch + cut, with no duplicate WP dots (per the agreed `4J2` example: plot `4→2`, a cut, then resume `4→5`). `expandRoute()` walks the sequence and, at each jump (once, ignoring the repeat count), appends a **branch** to the target and a **resume** at the WP before the jump (`cutBefore`); `expandRoute` runs in `terrainProfile.ts`. Each continuous segment is terrain-sampled separately and stitched with a small gap (`CUT_GAP_M`); the cut is a `cut:true` terrain sample that breaks the terrain fill, path line, clearance and preview, and is drawn as a dashed marker (`ProfileData.cuts`). Revisits are `repeat` markers (no map dot). **Correction stays correct:** the engine keys altitude **per WP index** (one `Cell` shared by all revisits), so the jump-back leg constrains the same WP as its first-pass legs; cut legs are skipped (`isCut`). `MAV_FRAME`/`JUMP` target is resolved by 1-based WP number (verify empirically with a real mission). 3D map cuts are not handled (Cesium rework later).

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

### Feature 3 — Live AGL widget (forward-looking terrain HUD) — ✅ **DONE**

A HUD widget (`widgets/LiveAglWidget.svelte`) in a new **`wide` (2×1) widget class** — *not* a reuse of the Feature 2 chart, but a **dedicated lightweight renderer** built for live update rates. Side-view terrain HUD:

- **Left 1/3** = recently flown terrain + flight-history line; a neutral, **airframe-agnostic UAV marker** sits at the "now" divider (horizontal 1/3) and **tracks the current altitude** vertically.
- **Right 2/3** = **estimated terrain ahead along the current heading**, with a dashed **projected flight line** so climb/descent and ground-intersection are visible at a glance.
- **AGL** readout (centered over the UAV) = `GPS MSL altitude − terrain ahead at the UAV`; **min-clearance-ahead** readout (warns red < 0).

**Data sources**
- **History accumulated internally from the telemetry stream** (lat/lon/MSL/timestamp, 5 m dedup), *not* the shared `liveTrack` store — that store only fills while *armed* on a live link, so it is empty during blackbox/flight-log replay. The internal buffer resets when time runs backwards (scrub / new flight), so the widget works **live and in replay**. Terrain for the history is folded incrementally via `LiveTrackProfiler`.
- **Forward terrain** sampled along the heading with a single `terrain_profile([uav, destPoint], 30 m)` call, re-queried only on meaningful change (> 5 m moved / > 2° heading / scale change / > 1 s) so a jittering yaw at standstill doesn't hammer the backend.
- **Heading** mirrors the compass logic: filtered 5-point GPS track ≥ 2 m/s, compass `yaw` below.
- **Vario** for the projected line uses the **FC's own vario** (smooth baro/nav-filtered source — same as the Vario widget), 5-sample averaged. (Differencing GPS-MSL over the sparse history points was too coarse and made the angle snap.)

**Scaling**
- **Horizontal**: total render distance steps **300 / 900 / 1800 / 3600 m** (≈ `speed · 120 s`), split **1:2** history:forward. **Boundary hysteresis**: step up immediately when the window is outgrown, step *down* only when `need < 0.7 × the step below` — cruising right on a boundary (e.g. ~54 km/h on the 1800↔3600 edge) no longer flaps.
- **Vertical**: auto-fit over the visible window, expand-fast / shrink-slow; the steep projected line is **not** a scaling reference (only the UAV altitude + real terrain are).
- **Axes**: left = altitude **relative to the UAV** (0 = current flight level, incl. negatives — like the Altitude widget); bottom = visible **distance** (0 under the UAV, positive both ways). Readout text scales with the widget size.

Visuals follow the Feature 2 panel (grid, ground gradient) inside a standard widget card (blur / semi-transparent / rounded). Update is driven by each telemetry frame, self-throttled (drops a frame while a backend sample is in flight). Default **off**.

### Feature 3b — Terrain Radar widget (top-down EGPWS-style) — ✅ **DONE**

A 1×1 (`large`) **top-down, track-up** terrain-awareness display (`widgets/TerrainRadarWidget.svelte`), a simplified take on a Honeywell EGPWS terrain display.

- **120° forward fan**, fixed pointing up; terrain is sampled relative to the heading so the picture is **track-up** (turning rotates the terrain). The fan **fills the square** vertically — its wide ±60° flanks overflow the left/right edges and are clipped by the card (no dead space). The same **UAV ring+dot marker** sits at the apex (bottom-centre).
- **Two independent ranges** (the easy thing to confuse):
  1. *Horizontal fan distance* — how far ahead it looks: **speed-driven** 300/900/1800/3600 m with the same boundary hysteresis as the Live AGL widget. Drawn as range **arcs + distance labels** along the heading line.
  2. *Clearance colour scale* — how terrain height vs the reference altitude maps to colour: a **dedicated setting** (`radarScale`, **left toggle 60/120/250 m**, default 120; coarse-rounded **200/400/800 ft** in imperial). This is deliberately **not** the Terrain-Analysis `groundClearance` (that's a planning value, not a radar scale).
- **Colouring** = `clearance = referenceAlt(dist) − terrain`, on a **continuous red→orange→yellow→green ramp** over 0…scale (`< 0` clamps to red, `> scale` unpainted). Reference altitude toggles **REL/PRED** (right button): static current MSL, or sink-angle predicted (`MSL + slope·dist`, averaged FC vario) — both share one code path.
- **Heatmap look** — cells (32×16 polar grid) are textured with an SVG **`feTurbulence` + `feDisplacementMap`** filter (dissolves the grid blocks organically) plus a very light `feGaussianBlur`, clipped to the fan sector. Chosen over a plain blur so terrain detail survives.
- **Backend**: new `terrain_fan(lat, lon, heading, half_angle, range, ang_cells, rad_cells)` command — one IPC call per refresh, server-side polar sampling via the existing tile cache. Re-sampled only on meaningful change (movement > ½ radial cell / turn > 2° / scale change / > 1 s). Default **off**.

### Feature 4 — LOS (line-of-sight) analysis — ✅ **DONE (shipped 2026-06-15)**

Shipped, and expanded well beyond naïve LOS into a layered **RF link / radio-shadow analysis** —
geometric LOS occlusion + Fresnel/knife-edge diffraction + two-ray ground reflection, rendered as a
background "rainbow" loss field in the Terrain Analyzer profile (plus a LOS-clearance line and a
logged-RSSI overlay in Track mode for prediction-vs-measurement comparison). Radial 1° sampling from
the launch point; per-band (5.8/2.4/0.9/0.433 GHz); clutter/vegetation offset. **Full design + as-built
detail in [RF_LINK_ANALYSIS.md](RF_LINK_ANALYSIS.md).**

---

## 4. Implementation order (summary)

1. ✅ **Shared elevation provider** (foundation) — Copernicus GLO-30, Rust backend, validated
2. ✅ **AGL waypoints** — WP editor alt-mode (REL/AMSL/AGL) with terrain conversion, survey-pattern `ground`/AGL, export AGL→AMSL, launch point + `<mwp>` persistence. Validated against INAV Configurator terrain analysis.
3. ✅ **Terrain analysis** — full-width NavRail overlay; view modes Waypoint / Track; SVG profile chart with zoom/pan + clearance coloring; Terrain Correction (Terrain Follow / Clearance Check) over a WP range, preview → APPLY, manual Add WP, jump-loop simulation — all done
4. ✅ **Live AGL widget** — 2×1 `wide` forward-looking terrain HUD; dedicated renderer, history from the telemetry stream (live + replay), heading-projected terrain ahead + vario flight line
5. ✅ **Terrain Radar widget** — 1×1 top-down track-up EGPWS-style fan; `terrain_fan` backend, continuous clearance heatmap (REL/PRED), own 60/120/250 m colour scale
6. ✅ **LOS analysis** — shipped as the layered RF link analysis (LOS + Fresnel/diffraction + two-ray, rainbow loss field + RSSI overlay); see `RF_LINK_ANALYSIS.md`

## 5. Protocol scope (TBD)

AGL is currently implemented for the **INAV mission path only** (MSP WP / MW XML). The **ArduPilot/PX4** mission path is still rudimentary (separate MAVLink WP implementation; no test hardware), so **AGL there is TBD**. ArduPilot/PX4 expose a native terrain-follow frame (`MAV_FRAME_GLOBAL_TERRAIN_ALT`); a future implementation would likely map AGL onto that frame rather than resolving to AMSL — decided once test hardware exists.

---

*This is a living planning doc. Details in each "TBD" are resolved as we work through the features one at a time.*
