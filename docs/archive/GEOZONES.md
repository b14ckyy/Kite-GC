# INAV Geozones — Feature Plan

> ARCHIVED (2026-06-28) — fully shipped (P1 + P2 + P3). Kept for the byte-exact MSP + editor reasoning.
>
> STATUS: P1 (display) + P2 (editor) + mission safety check + P3 (in-flight breach toast) SHIPPED.
> The P3 toast is GCS-computed geometry (commit `3f0e53b`) — INAV doesn't expose its geozone-avoidance
> flight mode over MSP, so the planned mode-detection approach was dropped in favour of the geometric one.
> Second of the three "Airspace Manager" safety subsystems
> (Autoland → **Geozones** → Geofence). Distinct from the worldwide-NFZ idea (that one is generic
> drone no-fly maps from external providers; see the future note). This is the **INAV FC geozone
> editor**: read/write the flight controller's own geozone config over MSP.

## Concept (from INAV `docs/Geozones.md`)
Geozones let the pilot define map areas that keep the aircraft inside allowed zones (**Inclusive** /
Flight-Zone) or out of restricted areas (**Exclusive** / No-Flight-Zone). The FC enforces them with
autonomous avoidance in self-levelling modes and NFZ-avoiding RTH path planning. Two shapes
(**circle**, **polygon**); per zone a min/max altitude band, an AGL/AMSL reference, and a fence action
(None / Avoid / Pos-Hold / RTH).

We mirror the data + the **sanity checks**; the UI is **our own** (map-first editing, minimal panel),
not a copy of the INAV-configurator layout. The UI lives **inside the Airspace Manager** panel.

## Firmware facts (verified against INAV master)
- Limits: **63 zones** (`MAX_GEOZONES_IN_CONFIG`, id 0..62), **126 vertices total**
  (`MAX_VERTICES_IN_CONFIG`, one shared pool across all zones). A **circle = 2 vertices**
  (centre + a hidden radius-holder, but `vertexCount` is **1**); a **polygon ≥ 3 vertices**
  (`vertexCount = N`). A zone is "used" when `vertexCount > 0`.
- The `MAX_GEOZONES`/`MAX_VERTICES` `+1` reserve is the firmware-internal "safehome-as-inclusive"
  zone (setting `geozone_safehome_as_inclusive`) — derived, **not** an editable slot for us.
- Enums: `shape` 0=Circular, 1=Polygon · `type` 0=Exclusive, 1=Inclusive ·
  `fenceAction` 0=None, 1=Avoid, 2=Pos-Hold, 3=RTH · `isSealevelRef` 0=AGL (rel. launch), 1=AMSL.
- Altitudes in **cm**: `minAltitude` 0 = ground (use −1 for "action at ground"); `maxAltitude`
  **0 = no upper limit (∞)**. Vertex lat/lon in deg×1e7. Polygon vertices **counter-clockwise**,
  ascending order, non self-intersecting.
- Min firmware **INAV 8.0** — `Feature::Geozones` already exists in `msp/features.rs`
  (`FeatureSet.geozones`).

### MSP wire format (byte-exact, from `fc_msp.c`)
- `MSP2_INAV_GEOZONE` **0x2210** / `SET` **0x2211** — 14 bytes:
  `[id u8, type u8, shape u8, minAlt i32, maxAlt i32, isSealevelRef u8, fenceAction u8, vertexCount u8]`.
  **SET resets that zone's vertices** as a side-effect → always write the zone header **before** its
  vertices.
- `MSP2_INAV_GEOZONE_VERTEX` **0x2212** / `SET` **0x2213** —
  `[zoneId u8, vertexId u8, lat i32, lon i32]` (10 B); for a **circular** zone an extra `radius u32`
  is appended (14 B), which the FC stores internally as vertex `vertexId+1` (lat=radius, lon=0).
  Requires the zone's `shape` to be set first (the SET handler branches on the stored shape).

We deliberately **skip the 7 global `geozone_*` settings** (detection distance, avoid-altitude-range,
safe-altitude-distance, safehome-as-inclusive, safehome-zone-action, mr-stop-distance,
no-way-home-action). They tune global fence *behaviour*, not the zones; out of scope for now. Possible
later "Geozone Behaviour" block (alongside the alerts work).

## Locked UI decisions
- **Map-first editing.** Fence points are edited like waypoints: click a vertex → a WP-style popup
  (polygon: lat/lon; circle: lat/lon + radius). The panel stays minimal.
- **Top toggles:** Geozones is the **5th layer** (2D + 3D), directly under "Airspaces", **default on**.
- **Panel "Geozones" section** (below the airspace settings, built **like the Safe Home list**):
  - Collapsed 1-line row: zone number · type (Circle/Polygon) · small text: vertex count (polygon)
    or radius (circle).
  - **Colours: Inclusive = blue (`#37a8db`), Exclusive = amber (`#f5a623`).** (Intentionally
    different from the configurator's green/red.)
  - **Map representation by FENCE ACTION** (scheme adopted from MWPTools `mwp-geozonemgr.vala`, kept with
    our colours): `None` → **dashed** thin, no fill · `Avoid` → solid thin · `Pos-Hold`/`RTH` → solid
    **thick**. Translucent **area fill** for every **exclusive** zone with a real action (Avoid/Pos-Hold/
    RTH) and only for an **inclusive RTH** zone. (`geozoneFilled` + `geozonePathStyle` in
    `helpers/geozoneStyle.ts`.) **3D mirrors the full scheme**: the fill as a translucent extruded
    volume, and the line variants (dash/width) as real boundary **polylines** at the floor (always) +
    ceiling (real ceilings only) — Cesium outline width/dash on extruded volumes is unreliable, so
    `PolylineDashMaterialProperty` + polyline `width` carry the dashed/thick distinction.
  - Expanded (edit): Inclusive/Exclusive slide toggle · Lower altitude (default 0 m, 10 m steps) ·
    Upper altitude (default 0 m, 10 m steps). 0 m upper = ∞ (shown as "∞").
  - An **edit toggle** above the list that locks/unlocks the map markers.
- **Visibility:** the whole Geozone UI is shown **only** when an INAV UAV with compatible firmware
  (≥8.0) is connected.
- **Data flow:** geozones are downloaded **at every handshake** and are **always shown in the Mission
  Planner while it is in edit mode** (regardless of the layer toggle); otherwise they show only when
  the 2D/3D layer toggle is on.

## Phasing
### Phase 1 — Display (read-only) — *this iteration*
Backend `commands/geozone.rs` (mirrors `safehome.rs`): `geozone_read_all` gated on
`FeatureSet.geozones`; loop id 0..63, read the header, keep `vertexCount>0` zones, read each zone's
vertices (circle: 1 read → centre+radius; polygon: `vertexCount` reads). Register the command; add
the four MSP codes to `msp/types.rs`. Frontend `stores/geozone.ts` (loaded snapshot; download on INAV
connect; clear on disconnect). `settings.airspace.layers` gains a **`geozones`** entry (default
`d2:true, d3:true`). `helpers/geozoneStyle.ts` (colours, labels). 2D `Map.svelte::updateGeozones()`
(circles + polygons, blue/amber, gated by the toggle **or** mission-edit-mode). 3D
`Map3D.svelte::updateGeozones3D()` (extruded volumes between min/max alt; circle→cylinder,
polygon→extruded hull; AGL via terrain sample, AMSL via geoid; max=0 → capped tall column). The panel
"Geozones" list section (collapsed + expanded showing values **read-only** in P1) + the 5th layer
toggle. i18n en/de/fr.

### Phase 2 — Editor (interactive map + write) — *SHIPPED*
Working/dirty store (`geozoneWorking`/`geozoneDirty`/`geozoneEditing` + mutations) — 2D/3D/panel now
render from the working copy. **Backend** `geozone_write_all`: writes all 63 slots (active with data,
empties cleared via `vertexCount 0`), **header before vertices** (the vertex SET handler branches on the
stored shape to read a circle radius), circle radius appended, then one `MSP_EEPROM_WRITE`. **Map-first
editing** (`Map.svelte`, edit-lock gate so handles only show in edit mode): labelled drag handles
(vertex number / "GZn"), edge-midpoint click to insert, WP-style popup for exact lat/lon (+ radius) with
per-vertex delete; new zones placed at the map centre, **sized to the current zoom** (circle = 2 tiles
radius; polygon = ~2×3-tile trapezoid). **Panel editor**: type toggle, action select, lower/upper alt
(10 m steps), radius, AGL/AMSL (auto-converts the altitudes via `terrain_elevation`), delete,
Save/Revert (+ ConfirmDialog). **Sanity** (`helpers/geozoneSanity.ts`): ≤63 zones, ≤126 vertices total,
circle = radius > 0, polygon ≥ 3 + non-self-intersecting, upper > lower (except 0); **CCW auto-fix at
save**. (The advanced multi-zone overlap warnings — ≥2 shared borders ≥2.5× loiter apart; ≥50 m vertical
overlap — were not needed and are deferred.)

### Phase 2.5 — Mission ↔ geozone safety check — *SHIPPED*
Hints only, never a blocker. `helpers/geozoneMissionCheck.ts` (pure, **altitude-aware**): normalises the
path to metres-above-launch via `resolveMissionAltitudes` (same launch-ground + per-WP `altMsl` as the
3D mission renderer), samples each leg ~40 m. **Inclusion** zones are enforced only when launch/home is
laterally inside one (else ignored, like INAV); a leg is bad if a sample leaves the **union** of the
inclusive zones (overlap = corridor, handled naturally). **NFZ**: launch/home inside (red — arming may
be blocked) and path crossing (amber). Driven by a debounced controller in `stores/geozone.ts`
(`geozoneMissionResult`; immediate recompute on the edit-mode toggle + a 400 ms max-wait so frequent
telemetry can't starve it). Warning bar in `InavMissionPanel` (above the WP list); offending legs drawn
**red** in 2D (a pane *below* the mission line → red outline/halo) and 3D (redrawn inside
`drawMission3DModel`, placed at `altMsl + geoidOffset` so it sits exactly on the mission line). i18n
`warn*` keys. NOTE: altitude for terrain-following (AGL) WPs is approximated as launch-relative.

### Phase 3 — Breach toast (scoped down, 2026-06-23)
INAV itself performs the geozone avoidance (RTH/Pos-Hold/Avoid) — we do **not** re-implement
proximity prediction. Instead, keep it **simple**: when INAV reports its explicit geozone-avoidance
**flight mode** (the dedicated AUTO/avoidance state, distinct from plain WP/NAV), show a short **toast**
("Geozone avoidance active") so the pilot knows the FC took over. TODO when building: verify the exact
INAV flight-mode / box-flag that marks the avoidance state, and reuse it for future avoidance maneuvers
too. (The earlier idea of a full breach-proximity pipeline hooked into the ADS-B conflict infra is
dropped as overkill.) Possibly the "Geozone Behaviour" global-settings block lands here.

## Reused infrastructure
- Backend: `safehome.rs` (per-index read loop + batch write + EEPROM), `fc_settings.rs`
  read/set-by-name helpers, `FeatureSet.geozones`.
- Frontend: `stores/safehome.ts` (store + connect/disconnect lifecycle) as the store template;
  `AirspaceManagerPanel.svelte` (panel + layer toggles) as the UI home; the Safe Home list rows as
  the list-row pattern; `Map.svelte` mission-waypoint drag + polygon render + point-in-polygon (2D
  editing in P2); `Map3D.svelte` `updateAirspaces3D`/`updateObstacles3D` (3D extrusion/columns).

## Out of scope / notes
- No global `geozone_*` settings (see above).
- No new ADR expected for P1 (pure reuse of existing patterns); reconsider a short ADR for the
  edit/data model when P2 lands if it introduces a non-obvious decision.
