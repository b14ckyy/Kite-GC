# Airspace Manager — Feature Plan

> Status: **Planned** (2026-06-08). A dedicated nav-rail panel (**directly under Radar**) + a backend
> aeronautical-data subsystem over **OpenAIP**: airspaces, obstacles, RC/model airfields and airports as
> toggleable map (2D) and globe (3D) layers. The **static counterpart to the Radar subsystem**.
> Architecture in ADR-038. Roadmap: *Map Overlays → Airspace Manager*.

## Mental model: the static counterpart to Radar
Same shape as the Radar subsystem, but for **static aeronautical features** instead of moving vehicles:
backend provider + region cache · a dedicated panel · 2D + 3D rendering · per-layer config. Updates only on
region change (data is static → long TTL).

## Data layers (OpenAIP) — scope locked to four

| Layer | 2D | 3D | Notes |
|---|---|---|---|
| **Airspaces** | **all** shown (when on) | **only relevant** ones | 3D relevance filter (below) — avoids clutter + cost |
| **Obstacles** (wind turbines, masts, towers, cranes) | small type-icon (height-labelled) | **vertical column** ground→height (start simple) | ⭐ critical for low flight; icon mapped from OpenAIP `type` |
| **RC / model airfields** | marker | **ground-projected highlight** | community-relevant |
| **Airports** (typed) | marker | **ground-projected highlight** | avoid / situational |

_Navaids, hotspots, reporting points: **out of scope** (not needed)._

### Airspace 3D relevance filter
2D shows **all** airspaces (when the layer is on). 3D shows an airspace **only if it's relevant to the UAV**:
- a boundary within **500 m above** the UAV's altitude, **or**
- within **5000 m laterally** of the UAV, **or**
- the UAV is **inside** it.

Everything else is hidden in 3D as irrelevant. (Reference = the UAV; falls back to the GCS/camera when no UAV.)

## Panel — "Airspace Manager" (nav rail, under Radar, `PanelShell` **`advanced`** variant, two-column)
The `advanced` (left controls + right data) variant is chosen specifically because the right column is a
**grouped nearby-feature list** — exactly like the Radar panel's three system groups.
- **Left = settings/controls:**
  - **Cache capacity readout + reset.**
  - **Per-layer visibility — separate 2D and 3D toggles** for each of the four layers ("what's visible in 2D
    / in 3D"). This replaces a single master mode.
  - Per-layer filters as needed (e.g. obstacle min-height, airspace classes, airport types).
  - **Alerts per relevant type** (airspace-level alert, obstacle alert) — **config space reserved; built
    later once the layers run.**
  - Whatever else settings-wise comes up.
- **Right = nearby-feature list, grouped per layer** — **Obstacles · Airspaces · Airfields** (RC + airports),
  distance-sorted within each group and **capped by count** (limit TBD), with the relevant info per entry
  (e.g. obstacle height, airspace floor/ceiling, field type) + the class/type **legend**. Click an entry →
  centre the map + highlight. Mirrors the Radar contact list (per-system groups). _Later this can also be
  presented as an `info` minimized card (like the logbook), so the variant stays multi-format._

## Settings (main app, Data tab — under the Cesium Ion token)
Kept minimal, like Radar / Flight Logbook:
- **Global feature toggle** — enables the subsystem + shows the panel in the nav rail.
- **Provider dropdown** (None / OpenAIP / future) — one active provider at a time.
- **API key** (shown only when the provider needs one, persisted).

Everything else (cache, per-layer 2D/3D visibility, filters, alerts) lives **in the panel**.

## Data source — single pluggable provider, OpenAIP first
A backend `AeroProvider` trait; **OpenAIP** first (`api.core.openaip.net`, `apiKey` query param, free
**non-commercial → user-supplied key**, licence obligation on the user). FAA (US, public domain),
openFlightMaps, national open-data are future impls behind the same trait. One active provider only (ADR-038).

## Architecture

### Backend — `src-tauri/src/aero/`
- **`AeroProvider` trait**: `fetch(center, radius_km, layers, api_key) -> AeroData`. `OpenAipProvider`: per
  enabled layer hit the matching endpoint (`/api/airspaces`, `/api/obstacles`, `/api/airports`, RC airfields)
  for the region (`pos=lat,lon&dist=` or `bbox`), paginate, map into normalized models.
- **Normalized models**:
  - `Airspace` — `name`, `class`, `lower`/`upper` `{value_m, datum: Gnd|Msl|Std}`, `polygon: Vec<[lon,lat]>`.
  - `PointFeature` — `kind` (obstacle/airport/rc), `subtype` (→ icon), `name`, `lat`, `lon`,
    `elevation_m?`, `height_m?` (obstacles → 3D column height), `lighting?`.
  Units/datum normalized to metres in the backend.
- **Cache**: last fetched region (~**500 km radius** around the reference) in RAM, long TTL. Refetch only on
  reference move beyond a fraction of the radius, manual clear, or provider/key/layer change. Commands:
  `aero_fetch(center, radiusKm, layers)`, `aero_cache_stats()`, `aero_cache_clear()`.
- Attribution string per provider ("© OpenAIP contributors").

### Frontend
- **Stores**: normalized `aeroData` + `aeroCacheStats` + the panel's per-layer 2D/3D visibility config.
- **Fetch trigger**: when the feature is enabled and a provider (+ key) is set — debounced on the reference
  moving. Stops when disabled.
- **2D (Leaflet)**: airspace polygons (class-coloured, dashed for R/D/P) + category markers (obstacle /
  airport / RC) with a small SVG icon per `subtype`; click → info; a **legend**.
- **3D (Cesium)**: airspace **extruded volumes** for the **relevant** set only (datum: GND→terrain,
  MSL→geoid, FL/STD→pragmatic MSL); **obstacle columns** (ground→height); **ground-projected** highlights for
  RC/airports; billboards. View-cull + count cap.
- **Icons**: a small flat-SVG set mapped from OpenAIP `type`/`subtype` (per obstacle/airport kind),
  optionally seeded from OpenAIP's open icon set (licence permitting) — shared 2D/3D + legend.

## Phasing
- **P1 — foundation + the four layers in 2D + the panel.** Backend `aero/` (provider + OpenAIP + region
  cache + 3 commands) · Data settings (global toggle + provider + key) · the **Airspace Manager panel**
  (`advanced`: left = per-layer 2D/3D visibility + cache readout/reset; right = the **grouped nearby list**,
  basic) · **2D** rendering for the four layers + icon set + legend.
- **P2 — 3D + list polish.** 3D volumes (with the airspace relevance filter) + obstacle columns + RC/airport
  ground projection; nearby-list search + click-to-centre/info + the `info` minimized variant.
- **P3 — alerts + polish.** Per-type alerts (airspace-level, obstacle proximity), more filters, more
  providers (FAA / open-data), OpenAIP raster chart as an optional tile overlay.

## Open (resolve in P1, against a live key)
- OpenAIP endpoint set + per-layer params (pagination `page`/`limit`, `pos+dist` vs `bbox`) and the enum
  encodings (airspace `type`/`icaoClass`, limit `unit`/`referenceDatum`, obstacle/airport `type`).
- Obstacle height vs. elevation fields + datum; airspace FL/STD precision (start = MSL).
- Icon set: reuse OpenAIP's open icons (licence) vs. own SVG set.
- Naming: backend module `aero/`; user-facing panel "Airspace Manager" (kept though it covers more).
