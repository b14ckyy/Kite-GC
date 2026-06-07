# Radar / ADS-B 3D models

3D models for foreign-vehicle (radar) contacts in the 3D map. **Separate from the UAV models** in
`../` so they can be replaced independently. Each ADS-B class has its own file — currently these are
**placeholder copies** of the UAV models (we don't have proper aircraft models yet). Replace any file
with a better model of the same name; no code change needed.

Resolved in `src/lib/helpers/radar3d.ts` (`contactModelClass()` → `radarModelUri()`); rendered in
`src/lib/components/Map3D.svelte` (oriented to heading, altitude-tinted).

| File                | Class    | ADS-B emitter categories / source                          | Placeholder copy of |
|---------------------|----------|------------------------------------------------------------|---------------------|
| `adsb-light.glb`    | light    | A‑ (unspecified powered), A1 (light < 15 500 lb)           | `uav-plane.glb`     |
| `adsb-small.glb`    | small    | A2 (15 500–75 000 lb)                                       | `uav-plane.glb`     |
| `adsb-heavy.glb`    | heavy    | A3, A4 (B757), A5 (heavy)                                   | `uav-plane.glb`     |
| `adsb-jet.glb`      | jet      | A6 (high performance, >5 g & >400 kt)                       | `uav-plane.glb`     |
| `adsb-heli.glb`     | heli     | A7 (rotorcraft)                                            | `uav-quad.glb`      |
| `adsb-glider.glb`   | glider   | B1 (glider / sailplane)                                    | `uav-plane.glb`     |
| `adsb-balloon.glb`  | balloon  | B2 (lighter-than-air)                                      | `uav-arrow.glb`     |
| `adsb-arrow.glb`    | arrow    | B‑, B3, B4, B6 (UAV), B7 · FormationFlight · Radio · no category received | `uav-arrow.glb` |
| `adsb-ground.glb`   | ground   | C1 (emergency vehicle), C2 (service vehicle)               | `uav-arrow.glb`     |
| `adsb-dot.glb`      | dot      | **any contact with no heading** (non-directional)         | `uav-quad.glb`      |

**Resolution order:** no heading → `dot`; FormationFlight / Radio → `arrow`; otherwise by ADS-B emitter
category (above). Unmapped powered/unpowered (B‑/B3/B4/B6/B7, B5 reserved) falls through to `arrow`.

**Hidden entirely — not on the map, not in the list** (`isHiddenCategory()` in `radar3d.ts`): obstacles
and reserved/unspecified ground — **C‑** (unspecified ground), **C3** (fixed/tethered obstruction),
**C4** (cluster obstacle), **C5** (line obstacle), **C6/C7** (reserved) — plus the all-reserved **D‑**
set. Surface **vehicles** C1 (emergency) / C2 (service) are kept → `ground`.

## Model conventions (match the UAV models so orientation works)
- glTF **Y-up**; after Cesium's Y-up→Z-up load the local frame is **nose = +X, up = +Z, left = +Y**.
- Modelled at a small real-world size; on the map `minimumPixelSize` gives a screen-size floor and
  `scale` controls the world size (see `Map3D.svelte`, `createRadarEntities`).
- **Model colour doesn't matter** — contacts are rendered with `colorBlendMode = REPLACE`, so the glb's
  own colours/textures are fully replaced by the exact relative-altitude colour. Grab any model; only
  the shape/orientation matters.

## To replace with proper models
Drop a new `.glb` with the **same filename** here (keeping the conventions above). To split a class
into finer categories, add a file + extend `contactModelClass()` / the file map in `radar3d.ts`.
