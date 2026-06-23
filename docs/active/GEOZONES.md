# INAV Geozones — Feature Plan

> STATUS: planned · 2026-06-23. Second of the three "Airspace Manager" safety subsystems
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

### Phase 2 — Editor (interactive map + write)
Working/dirty store; the expanded panel fields + type toggle become live; the edit-lock toggle; map
markers become draggable with the WP-style popup (polygon vertex add/move/delete, circle centre +
radius); **Save to FC** (SET_GEOZONE header → SET_GEOZONE_VERTEX per vertex → `MSP_EEPROM_WRITE`);
**sanity checks**: ≤63 zones, ≤126 vertices total, circle=2 / polygon≥3, **CCW + non
self-intersecting**, upper>lower (except 0). Advanced multi-zone overlap checks (≥2 shared borders
≥2.5× loiter radius apart; ≥50 m vertical overlap) surfaced as **warnings**, not hard blocks.

### Phase 3 — Alerts (deferred)
Geozone breach-proximity alerts, hooked into the existing alert infrastructure (the ADS-B conflict
pipeline). Possibly the "Geozone Behaviour" global-settings block lands here.

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
