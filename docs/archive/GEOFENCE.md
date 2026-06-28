# ArduPilot / PX4 Geofence — Feature Plan

> ARCHIVED (2026-06-28) — feature complete; full editor + geometric breach fallback shipped. ArduPilot
> validated in SITL; PX4 / on-hardware validation is user-side and not kept open.

> STATUS: shipped-local · 2026-06-23 (awaiting SITL verification). Third "Airspace Manager" safety
> subsystem (Autoland → Geozones → **Geofence**). The MAVLink counterpart to the INAV geozone editor
> ([[GEOZONES]]); much simpler (no per-zone altitude bands / actions — those are global params).
> Backend + store + wiring + the full UI (2D/3D render + map editor, panel Fence section, i18n) are
> implemented and pass `npm run check` (0/0) + `npm run build`. Next: SITL test (ArduPilot first, then
> PX4), then commit (docs first, then code).

## Concept
ArduPilot **and** PX4 expose the geofence over the **MAVLink mission protocol** using
`MAV_MISSION_TYPE_FENCE` (same COUNT / REQUEST_INT / ITEM_INT / ACK exchange as a normal mission, just a
different `mission_type` and a fence-specific set of `MAV_CMD`s). Fence geometry = **inclusion / exclusion
× polygon / circle** (+ an optional return point). Enforcement (enable / action / altitude / radius) is
**global parameters**, not part of the fence items. QGC is the reference implementation.

## Wire format (verified — mavlink.io mission service)
Fence items are `MISSION_ITEM_INT` (`mission_type = FENCE`), `frame = MAV_FRAME_GLOBAL`, `x/y = lat/lon`
(deg×1e7), `z` unused. Commands:
- `MAV_CMD_NAV_FENCE_RETURN_POINT` 5000 — one return point (ArduPilot).
- `MAV_CMD_NAV_FENCE_POLYGON_VERTEX_INCLUSION` 5001 — one polygon vertex; **param1 = vertex count of
  this polygon**. N consecutive items with the same command + count form one polygon.
- `MAV_CMD_NAV_FENCE_POLYGON_VERTEX_EXCLUSION` 5002 — exclusion polygon vertex.
- `MAV_CMD_NAV_FENCE_CIRCLE_INCLUSION` 5003 — circle; **param1 = radius (m)**, x/y = centre.
- `MAV_CMD_NAV_FENCE_CIRCLE_EXCLUSION` 5004 — exclusion circle.

PX4 polygons wind **clockwise**. No per-item altitude.

## Global params (core set, via the existing param protocol)
- **ArduPilot:** `FENCE_ENABLE`, `FENCE_ACTION`, `FENCE_TYPE` (bitmask), `FENCE_ALT_MAX`, `FENCE_ALT_MIN`,
  `FENCE_RADIUS` (home circle), `FENCE_MARGIN`.
- **PX4:** `GF_ACTION`, `GF_MAX_HOR_DIST`, `GF_MAX_VER_DIST`, `GF_SOURCE`, `GF_PREDICT`.
The panel reads/writes a curated subset (enable/action/alt-max/radius), picking the param names by the
connected autopilot family.

## Data model (mirrors the geozone editor for UX reuse)
```
FenceZone { kind: inclusion | exclusion, shape: polygon | circle, vertices: [(latE7,lonE7)], radius_cm }
Fence { zones: FenceZone[], return_point: (latE7,lonE7) | null, params: FenceParams, has_fence: bool }
```
A circle zone = 1 vertex (centre) + `radius_cm`; a polygon = N vertices. (Same shape as `GeoZone` minus
altitude/action → the 2D/3D rendering + map editor are reused with blue=inclusion / amber=exclusion.)

## Decisions (locked)
- **Full editor now** (read + on-map create/edit + upload), not display-first.
- **UI in the Airspace Manager** (third safety subsystem). The panel already appears for a geozone-capable
  INAV FC; extend it to also appear + host the **Fence editor** when connected via **MAVLink (ArduPilot/
  PX4)**. Reuse the geozone map-edit UX (drag handles, WP-style popup, panel list, sanity).
- **Geometry + core params** (enable/action/alt-max/radius).

## Plan
### Backend
- **Parametrise** `mavlink_proto/mission.rs`: the download/upload helpers currently hard-code
  `MAV_MISSION_TYPE_MISSION` — thread a `mission_type` through so the same protocol drives FENCE (and
  later RALLY). Keep the deprecated-`MISSION_REQUEST` fallback.
- **Fence codec** (`mavlink_proto/`): encode/decode `MISSION_ITEM_INT` ↔ `Fence` model (group polygon
  vertices by command + param1 count; circle param1=radius; return point). Winding kept as drawn.
- **Commands** `commands/fence.rs`: `fence_read_all` (download fence items + read the core params by the
  autopilot family) and `fence_write_all` (upload items via the mission protocol + write changed params).
  MAVLink-only (error on MSP). Capability = MAVLink connected (ArduPilot/PX4).

### Frontend
- `stores/fence.ts`: `Fence` model + working/dirty + load on MAVLink connect + save (mirrors
  `stores/geozone.ts`). Mutations (add/delete zone, set kind, set vertex, insert/remove vertex, set
  radius, set return point).
- Map 2D/3D: render + edit the fence reusing the geozone overlay/edit code (generalise or parallel;
  inclusion=blue / exclusion=amber). 3D extrude uses the global `FENCE_ALT_MAX` (fallback height) since
  fences have no per-zone altitude.
- **Airspace Manager panel**: a **Fence** section (parallel to the geozone section) shown when
  MAVLink-connected; zone list + add (incl/excl × polygon/circle) + the core params + Save/Upload. Tab
  visibility: `airspace.enabled || geozonesAvailable || fenceAvailable`.
- i18n en/de/fr.

### Sanity
Polygon ≥ 3 vertices; circle radius > 0; (PX4) clockwise winding auto-applied on save. No per-zone alt.

## Rally points (shipped — same subsystem)
ArduPilot/PX4 **rally points** (RTL divert/return locations) ride the same path: `MAV_MISSION_TYPE_RALLY`
over the parametrised mission protocol. Geometry is just points (`MAV_CMD_NAV_RALLY_POINT` 5100, lat/lon
+ alt rel. home). `commands/rally.rs` (`rally_read_all`/`rally_write_all` + `RALLY_LIMIT_KM` /
`RALLY_INCL_HOME` params, ArduPilot-only), `stores/rally.ts` (index-based), green **R** markers in 2D
(`Map.svelte::updateRally`, draggable + lat/lon/alt popup) + 3D (`Map3D.svelte::updateRally3D`), a Rally
section in the Airspace panel + a `rally` layer toggle. Loaded on MAVLink connect; no reboot; armed-locked.

## Out of scope / later
INAV (uses its own geozones); in-flight breach alerts (shared with the geozone P3 alert work).
