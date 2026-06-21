# Radar Tracking — Panel & Map Visualization (Plan B)

> 📦 ARCHIVED (2026-06-21) — Shipped (B0–B4). Residual polish only: exclude the primary connection's own port from radar source lists (minor TODO via a store). Kept for reference.

> Status: **B0–B4 shipped** (2026-06-07). The user-facing half of the foreign-vehicle tracking feature
> ("Radar"). Covers the **Advanced Radar Panel** (settings + tracked-vehicle lists) and the **map
> visualization** (2D + 3D). The backend subsystem, data model, sources and the `radar-vehicles`
> event are specified in `RADAR_TRACKING_CORE.md` (Plan A) — this plan consumes them.
>
> Depends on Plan A **Phase 0–1**: the `radar-vehicles` event + the first real source (**ADS-B online**,
> no hardware) drive the panel — no simulator needed. Map work is the **last phase** by request.
>
> Framing decisions carried from Plan A: name = **Radar**; **per-system dedup** (ADS-B / FormationFlight /
> Radio are separate stacked lists; within ADS-B, merged by ICAO). **Heterogeneous data per system** —
> each system's list shows **its own columns** (ADS-B has FL/squawk/category; FormationFlight has peer
> name/LoRa-RSSI/relative pos; radio whatever the link gives). No forced common column set.

---

## 1. Frontend store

`src/lib/stores/radarTracking.ts` — listens to `radar-vehicles`, mirrors the consolidated snapshot:

```ts
interface RadarState {
  adsb: TrackedVehicle[];
  formationFlight: TrackedVehicle[];
  radio: TrackedVehicle[];
  stats: { sources: SourceStatus[]; lastUpdate: number };
}
```

- `startRadarListeners()` / `stopRadarListeners()` mirror the telemetry-store pattern
  ([telemetry.ts](../../src/lib/stores/telemetry.ts)); started once at app init.
- A derived helper enriches each vehicle with **relative bearing / distance / relative altitude**
  from `resolveUserLocation()` ([userLocation.ts](../../src/lib/helpers/userLocation.ts)) — purely
  frontend, no backend location coupling.
- `SourceStatus` (per feed: enabled, connected/polling, last-ok, error, count) drives the settings
  side's live status dots.

---

## 2. Where the master + system switches live (Main Settings)

The **enable switches are in Main Settings → Data tab**, not in the Radar panel. The existing
*"Telemetry Rates"* subsection in [SettingsPanel.svelte](../../src/lib/components/SettingsPanel.svelte)
is **renamed to "Telemetry"** and gains:

- **Radar master switch** — off by default. Off ⇒ all radar functions stop **and the Radar nav-rail
  tab/panel is hidden entirely**.
- When on, three **per-system toggles**: **ADS-B**, **FormationFlight** (the ESP32 mesh radar, formerly
  INAV-Radar), **Radio Telemetry**. Each enables/disables its system independently (turn off what you
  don't need to keep the UI clean).

So Settings owns *which systems exist*; the Radar panel owns *each system's source configuration*.
These map to `settings.radar.enabled` + `settings.radar.{adsb,formationFlight,radio}.enabled` (Plan A §8).

## 3. Advanced Radar Panel

New NavRail tab `radar`, icon = **radar dish**, placed **above Video and below Logbook** in the rail
order. Rendered via the `{#if activeTab === 'radar'}` block in
[+page.svelte](../../src/routes/+page.svelte#L1685) → `RadarPanel.svelte` on the panel framework
(`PanelShell`, `advanced` 1:2 split; see `PANEL_FRAMEWORK.md` + ADR-029). The tab only appears when the
radar master switch is on.

```
┌─ Radar ─────────────────────────────────────────[ Compact ]┐
│ ┌─[ ADS-B | FormationFlight | Radio Tel. ]─(dynamic tabs)─┐       │
├──────────────────────────┬─────────────────────────────────┤
│ SOURCES (selected tab)   │  TRACKED VEHICLES (all, grouped) │
│                          │                                  │
│  Online sources          │   ── ADS-B ──                    │
│  ┌─────────────────────┐ │   DLH7AB  12km 041° FL90 210kt 2s│
│  │Name  API-URI  Key  ✕│ │   RYR41X  6km  330° FL120 250 1s │
│  │OpenSky ...  ...    ✕│ │   ...                            │
│  └─────────────────────┘ │   ── FormationFlight ──                │
│  [        + add        ] │   PEER-2  340m 120° +15m 8m/s 1s │
│                          │   ── Radio Telemetry ──          │
│  Hard sources            │   (rows…)                        │
│  ┌─────────────────────┐ │                                  │
│  │[Serial▾] port… ✕    │ │                                  │
│  └─────────────────────┘ │                                  │
│  [        + add        ] │                                  │
└──────────────────────────┴─────────────────────────────────┘
```

### 3.1 Header
- **Dynamic system tabs** over the left field, as a `SegmentedToggle`. The tab set is **derived from
  the enabled systems** in Settings (only ADS-B on ⇒ a single segment; all three on ⇒ three). The
  `SegmentedToggle` already takes `options` as a prop and sizes its indicator from `--n`/`--i`, so
  **no control-framework change is needed** — we pass a `$derived` options array. Panel-side logic
  keeps the selected tab valid when a system is toggled off (fall back to the first available) and
  shows a hint if the master is on but no system is enabled.
- **Compact button** sits on the **detail (vehicle-list) toolbar row, right-aligned** (same level as
  the tab switcher on the left), with a `←` prefix (standard Button, like the Mission/Battery manager
  *Back*). It collapses the whole panel to the framework's **`info` variant** showing *only* the
  vehicle list with reduced per-row data (exact reduced layout defined later). In `info` mode there is
  **no button** — clicking the list re-expands it (like the logbook), but it does **not** auto-collapse
  (we may want full-panel map interactions later).

### 3.2 Left field — per-system source configuration
The selected tab swaps the left content. Composition per system:

- **ADS-B (SHIPPED):** **Online sources** then **Local sources**, both under one heading group.
  - **Online sources** — two *built-in* rows (adsb.lol / adsb.one: fixed URL, toggle-only, not
    removable) + *custom* rows (e.g. adsb.fi: `Name · URL · API-Key`, removable). Plus **Radius**
    (10/25/50/75/100 km) and **Online poll interval** (2/5/10/30 s) dropdowns, and a **"UAV Source"**
    toggle (ADS-B from the FC via MSP, with a contact counter) — shown only on **INAV 8.0+**
    (`Feature::AdsbMsp`), hidden + polling off otherwise.
  - **Local sources** — hardware receivers; Phase 2 = **serial MAVLink** (a **serial-port dropdown**
    from `list_serial_ports` + a ⟳ refresh + baud select). `+ Add receiver`.
  - **Collapse-on-enable:** an enabled source row collapses to a single line (name + status + toggle);
    disabled rows expand to show their config fields + delete. New rows start disabled (expanded) for
    editing.
  - **Serial-port conflict:** a port already used by another local source appears **disabled** in
    other pickers, labelled *(in use: \<name\>)* — cross-platform (string match). (The active FC
    connection's port should also be excluded — TODO via a store.)
  - Per-source **status badge**: green contact-count / red ✕ on error (event `radar-adsb-status`,
    merged by name).
- **FormationFlight:** *Hard sources only*, transport restricted to **Serial** (AFAIK only serial). Same
  row/delete/`+ add` pattern.
- **Radio Telemetry:** *Hard sources only*, transports **Serial / Bluetooth**. Same pattern. (Built
  last — gated on the shared telemetry parser, Plan A §7.3.)

All rows use the shared `Button` / `Toggle` / selects (28px). **Each source row has its own on/off
`Toggle`** (decided) so a feed can be muted without deleting it, plus a **status dot** from
`SourceStatus` (connected / polling / error). Add/remove/toggle edits `settings.radar.*` via
`applySettingsPatch`; the backend is reconfigured through `radar_configure` /
`radar_set_source_enabled`.

### 3.3 Right field — all tracked vehicles
A single scrollable list of **all** vehicles, **one per row**, **grouped by ADS-B → FormationFlight → Radio
Telemetry** (in that order) with a small group header + count. **Disabled systems are hidden entirely**
(decided) — a system's group only appears when it's enabled in Settings. **Columns are per-system**
(heterogeneous data): a shared core (id/callsign, distance, bearing rel. to user, age) plus
system-specific columns — ADS-B: altitude/FL, ground speed, vertical trend (▲/▼), squawk/category;
FormationFlight: relative altitude, LoRa signal; Radio Telemetry: whatever the link gives. Distance / bearing
/ relative-alt are the frontend-derived fields. Stale rows fade toward TTL. Selecting a row selects the
vehicle (→ map highlight once the map phase lands).

**Shipped layout:** advanced row = `callsign · type · dist · bearing · alt · speed · age` — the **type**
is a short abbreviation from the ADS-B emitter category (A1…C7 → LGT/SML/LRG/HVY/HELI/GLD/UAV/…; the
MAVLink receiver maps `emitter_type` to the same code; the aggregator preserves callsign/category/squawk
across sources). The **info (compact)** view shows `callsign · dist · bearing · alt` (alt column widened
so the unit never wraps >10 000 m) and caps **ADS-B to the nearest 10** (distance-sorted); the advanced
view shows all. Distance/bearing reference = connected UAV (valid fix) else GCS; the **online query
centre** = the map view (2D centre / 3D camera focus), distinct from the reference (Plan A §7.1).

All strings via `$t()` with keys in **en / de / fr**.

---

## 4. Map visualization (final phase)

Foreign vehicles render on the **same** Map/Map3D instances, in dedicated, isolated layers so the
existing UAV/track/mission rendering is untouched. Design decided 2026-06-06 (see §6).

### 4.0 Relevance model — proximity focus, adjustable, "show all"
- Default = **proximity focus**, fully operator-adjustable, with a **"Show all"** override.
- Live-adjustable params: max **radius** (km), **altitude band** ± relative to the reference, per-system
  **map visibility**, and **Show all** (disables radius+band relevance weighting).
- **Distance relevance is soft only** (decided): the radius filter **never hides** a contact — contacts
  outside the radius render **dimmed + smaller**. Only a **per-system visibility** toggle (or the master
  map toggle) actually hides contacts. **Colour depends on altitude only, never on distance.**
- **Altitude is a hard map cutoff** (decided): ADS-B contacts above a **configurable ABSOLUTE ceiling
  (default 10 000 m)** are **hidden from the map** — high airliners are pure clutter for a low UAV.
  - They **always remain in the Radar panel list** — the cutoff only affects map rendering.
  - **Relative-altitude override:** any contact within **Δ ≤ +2000 m** relative to the reference (i.e. it
    has entered the top of the colour scale, §4.2) is **always shown on the map regardless of the
    absolute ceiling** — a high cruiser descending toward us must never disappear.
  - Net rule: *hidden from map ⟺ (absolute alt > ceiling) AND (Δ > +2000 m)*. Disabled by "Show all".
- A separate **performance safety cap** (render budget, e.g. nearest-N by distance) bounds entity count
  in dense airspace independently of the relevance dimming; updates **diff** the entity set rather than
  full-rebuilding once counts grow.
- **Reference** for distance / altitude-band = same as the lists: connected UAV (valid fix) else GCS
  location ([userLocation.ts](../../src/lib/helpers/userLocation.ts)). (Distinct from the ADS-B online
  *query centre*, which is the map view — Plan A §7.1.)

### 4.1 Controls — Radar panel "Map" section
- A **"Map" tab** appended to the panel's dynamic `SegmentedToggle` (after the per-system tabs). Its
  left pane hosts the map controls; the right vehicle list stays as-is.
- One **"Map visibility"** group: **Show all** (top — when on, radius + max-altitude are disabled since
  they have no effect) · **Radius** (soft dim) · **Max altitude** (hard absolute ADS-B map ceiling,
  default 10 km, adjustable) · **per-system map visibility** toggles (ADS-B / FormationFlight / Radio,
  independent of the data-enable in Settings — track a system but hide it on the map). Below it the
  **altitude colour legend** (§4.2). No separate "show on map" master — per-system visibility (all off)
  covers it. The +2000 m relative override (§4.0) is fixed (tied to the top of the colour scale).
- State persists under `settings.radar.map.*` (`radiusKm`, `maxAltM`, `showAll`, per-system `visible`),
  reactive to both 2D and 3D.

### 4.2 Icons — category silhouettes, colour = altitude  ⚠️ representation still brainstorming
- **Top-down silhouettes per ADS-B emitter category**, rotated to heading: light/GA, small, large,
  heavy/airliner, **helicopter** (rotor glyph), glider, **UAV/drone**, generic/unknown fallback. Built
  ourselves (inspired by MWPTools' per-category SVG set — A0–A7/B2/C0–C3 — **not copied**).
- **Recolourable two-element SVG** (MWP-inspired): a **fill** element tinted at runtime by the
  **altitude colour** + a **contrast outline** (white/black by brightness). This frees colour from
  encoding the *system*.
- **System encoded by glyph family** instead of colour: ADS-B uses the category silhouettes,
  FormationFlight peers the UAV/drone glyph + callsign, Radio Telemetry a generic glyph.
- **Colour encodes ALTITUDE — relative-to-reference, fixed danger-centred scale (decided):** based purely
  on the **altitude difference** `Δ = contact.alt − reference.alt`. **Reference altitude** = the UAV's
  **GPS MSL** altitude when connected with a fix, else the **GCS terrain ground level** from
  `terrain_elevation(lat,lon)` (so the scale works without a UAV), else null → absolute fallback.
  **Distance never affects colour.** Violet at our level = highest danger and grabs the eye; it cools off
  steeply going up and goes hard blue going down (legally nothing should be below us):

  | Δ relative altitude | colour |
  |---|---|
  | ≤ −500 m | strong **blue** (constant, contrast outline) |
  | −500 → 0 m | blue → violet |
  | **0 m (our level)** | **violet** — max danger |
  | 0 → +500 m | violet → red |
  | +500 m | red |
  | +500 → +1500 m | red → yellow → green |
  | +1500 m | green |
  | +1500 → +2000 m | green → white |
  | +2000 m | white |
  | > +2000 m | **semi-transparent white + black outline** (fading out — irrelevant) |

  - **Black/contrast outline** throughout for legibility (essential for the blue and the translucent
    white). Above +2000 m is effectively faded out (and subject to the absolute map cutoff, §4.0).
  - **No reference at all** ⇒ fall back to the **absolute** MWP-style HSV ramp (red→green→violet, 0–12 km).
  - **Legend** in the panel Map tab (title *"Altitude Scale (Rel. to UAV/GCS)"*): a **horizontal** bar
    linear over Δ −500…+2000 m (blue → violet → red → yellow → green → white), with a tick at the
    **20 %** mark = our level (Δ = 0, violet); labels below / level / above.
- **Age → opacity** (fade toward TTL); **out-of-relevance → dim + scale-down**. No-heading contacts use
  a non-directional dot variant.

### 4.3 Labels — minimal, full on hover/select
- Default label = **callsign only** (glyph alone when callsign unknown).
- **Hover / select** reveals the full readout: callsign · category · alt/FL · ground speed · vertical
  trend (▲/▼) · distance · bearing.
- Declutter at low zoom: collapse to dots, label on hover/select.

### 4.4 2D (Leaflet, [Map.svelte](../../src/lib/components/Map.svelte)) — B3
- Dedicated `L.LayerGroup`(s) per system, **diffed** from the enriched store on update.
- Directional category `divIcon` SVG markers (rotated to heading), callsign label; dim/scale by
  relevance, opacity by age.
- Click → **select** (syncs the panel list via the `radarSelection` store). Optional leader line /
  range ring to the reference.

### 4.5 3D (Cesium, [Map3D.svelte](../../src/lib/components/Map3D.svelte)) — ✅ SHIPPED
- Each contact is a real **glb model** (rendered like the UAV model — `minimumPixelSize` floor +
  `colorBlendMode=MIX` altitude tint), **oriented to heading** via the shared `uavOrientation()`, at real
  altitude (`alt_m` as MSL + geoid). Per-class models live in **their own folder** `static/models/radar/`
  (`contactModelClass()` → `radarModelUri()`; currently placeholder copies, see the folder README). We
  tried flat extruded silhouettes and camera-facing billboards first — both fought Cesium
  (`requestRenderMode` geometry rebuilds / no heading) — glb models are flicker-free and show direction.
- **Floating label** under each model: callsign + altitude; the **full readout** (alt/vs/speed/dist/brg,
  like the 2D hover) when the contact is **selected**.
- **Ground projection** (only within the colour-scale zone, Δ ≤ +2000 m; debug+show-all = always): a
  thin **dashed drop-line** (height-coloured over a black backing) + a filled **CLAMP_TO_GROUND circle**
  (1 km radius, brightened, translucent, unlit) + a **heading arrow** in it.
- Unknown-alt contacts are **hidden in 3D** (can't be placed). Show-all keeps far stationary local
  contacts (hide radius 1000 km vs the size curve's 100 km).
- **Selection / picking:** click a contact (or its ground projection) to (de)select — synced to list/2D;
  hover → pointer cursor; selected → cyan model silhouette + cyan drop-line/ring.
- **No-blink rule (learned):** never re-touch an unchanged contact's ground geometry per snapshot — a
  per-contact change *signature* gates `syncRadarGround`, and the drop-line colour updates in place via a
  `ConstantProperty` (no material rebuild). Kept isolated so 2D↔3D continuity (ADR-031) and the FPV/HUD
  work (ADR-034) are unaffected.

### 4.6 Selection
- Small new `radarSelection` store (selected vehicle id) shared by the panel list ↔ 2D ↔ 3D. Selecting a
  contact in any view highlights it in all.

---

## 5. Phasing

- **B0 — Settings + panel shell + store** (after Plan A Phase 0–1): rename *Telemetry Rates → Telemetry*
  + add the master & per-system toggles; NavRail tab (radar-dish icon, above Video / below Logbook);
  `RadarPanel` with the dynamic-tab header, left source tables (online + hard for ADS-B), the grouped
  right-side vehicle list, the Compact→`info` collapse; store + enrichment; i18n. Demo with **ADS-B
  online** (real data, no hardware).
- **B1 — Lists polished**: per-system columns, sorting, age fade, status dots, empty/no-system states,
  selection store, the reduced `info`-compact row layout.
- **B2 — Source tables complete**: online add/remove (Name/URI/Key) + hard-source transport rows
  (Serial/Network/Bluetooth) wired to `radar_configure` / `radar_set_source_enabled`.
- **B3 — Map 2D. ✅ SHIPPED.** `radarSelection` store + `settings.radar.map.*` + the panel **"Map" tab**
  controls (show-all, radius, max-altitude, per-system visibility) + horizontal altitude legend;
  category silhouette icon set + relative-altitude colormap helper; Leaflet layer with diffed directional
  markers, distance dim+scale, altitude cutoff/override, callsign label / hover tooltip, selection sync.
- **B4 — Map 3D. ✅ SHIPPED.** Per-contact glb models (own `static/models/radar/` folder) oriented to
  heading + altitude-tinted; floating callsign/alt label (full readout on select); dashed drop-line +
  filled 1 km ground circle + heading arrow (gated to the colour zone); click/hover picking; incremental
  entity creation + change-signature guard (no stutter / no blink); 3D online query = "auto from view".

**Hidden everywhere (decided):** ADS-B ground obstacles/reserved (C‑, C3–C7) + the all-reserved D‑ set
are dropped from the list and both maps (`isHiddenCategory()`); surface vehicles C1/C2 are kept.

**3D online query (decided):** centre = camera ground-focus (else nadir); radius = camera→focus distance,
floored at the manual web radius, capped ~250 NM (provider limit) — sized to what the camera sees.

B0–B2 can proceed in parallel with Plan A Phases 1–4 (real sources); B3–B4 are the final phase.

---

## 6. Open questions

**Decided (2026-06-06):** per-source rows get their own on/off toggle (mute without delete) · disabled
systems are hidden from the right list · third system UI label = **Radio Telemetry** (internal `radio`) ·
ESP32 mesh-radar system = **FormationFlight** (formerly INAV-Radar; repos in Plan A §7.2).

**Map decided (2026-06-06):** **distance** relevance is **soft (dim + scale-down) only — never hidden**;
**altitude** is a **hard absolute map cutoff** (default 10 km, adjustable) with a **+2000 m relative
override** (always show what's within the colour scale) — hidden-from-map contacts **stay in the list** ·
filter/visibility controls live in a **panel "Map" tab** (state in `settings.radar.map.*`) · **3D = real
altitude + drop-line to ground** · icons = **category silhouettes** rotated to heading, **system encoded
by glyph family** (colour is free for altitude) · **colour = relative altitude, fixed danger-centred
scale** (§4.2: violet at level → red/yellow/green/white going up, blue going down) — **never distance** ·
labels = **minimal (callsign), full on hover/select** · **selection** = new small `radarSelection` store
(panel ↔ 2D ↔ 3D) · **performance** = render-budget cap (nearest-N) + diff updates.

- **Silhouette asset granularity** — final category→glyph set (how many distinct silhouettes) — settle
  while building the B3 icon set. (MWP ships A0–A7, B2, C0–C3 as a reference grouping.)
- **Units** — bearing relative vs true; altitude as FL vs display units for ADS-B (likely: ADS-B in FL,
  others in display units) — confirm in B1.
