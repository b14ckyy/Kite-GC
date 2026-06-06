# Radar Tracking — Panel & Map Visualization (Plan B)

> Status: **Planned** (2026-06-06). The user-facing half of the foreign-vehicle tracking feature
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
existing UAV/track/mission rendering is untouched.

### 2D (Leaflet, [Map.svelte](../../src/lib/components/Map.svelte))
- A dedicated `L.LayerGroup` per system (or one group with per-system styling), rebuilt from the
  enriched store on update.
- **Icons by system + category**: ADS-B aircraft (directional, rotated to heading, FL label),
  FormationFlight peers (UAV glyph + callsign), Radio Telemetry (generic). Reuse the icon approach in
  [uavIcons.ts](../../src/lib/helpers/uavIcons.ts) / [uavTopDown.ts](../../src/lib/helpers/uavTopDown.ts).
- Label shows callsign + relative alt; declutter at low zoom (collapse to dots, label on
  hover/select). Color/opacity encode age (fade toward TTL).
- Click → select (syncs the panel list). Optional leader line / distance ring to the user.

### 3D (Cesium, [Map3D.svelte](../../src/lib/components/Map3D.svelte))
- A dedicated set of entities (billboards + optional 3D models) positioned by lat/lon/alt, kept in a
  separate collection so 2D↔3D continuity (ADR-031) and the FPV/HUD work (ADR-034) are unaffected.
- Altitude handling consistent with the existing geoid/terrain offset logic; vehicles with unknown
  alt clamp to a configurable plane or draw a drop-line.
- Same selection sync; optional billboards always face the camera.

### Cross-cutting
- A map toggle (and/or per-system visibility) to show/hide radar contacts without disabling the feeds.
- Performance: cap rendered contacts (nearest-N / within-radius); diff updates rather than full
  rebuilds if counts get large (decide after measuring Plan A Phase 1).

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
- **B3 — Map 2D**: Leaflet layer, icons, declutter, selection sync.
- **B4 — Map 3D**: Cesium entities, altitude handling, selection sync.

B0–B2 can proceed in parallel with Plan A Phases 1–4 (real sources); B3–B4 are the final phase.

---

## 6. Open questions

**Decided (2026-06-06):** per-source rows get their own on/off toggle (mute without delete) · disabled
systems are hidden from the right list · third system UI label = **Radio Telemetry** (internal `radio`) ·
ESP32 mesh-radar system = **FormationFlight** (formerly INAV-Radar; repos in Plan A §7.2).

- **Icon language** — how visually distinct should ADS-B vs FormationFlight vs Radio Telemetry be on the map?
  (decide with a quick mock in B3.)
- **Declutter strategy** in dense ADS-B airspace (nearest-N, radius, zoom-based) — measure first.
- **Selection model** — reuse an existing selection store or add a small `radarSelection` store.
- **Units** — bearing relative vs true; altitude as FL vs display units for ADS-B (likely: ADS-B in FL,
  others in display units) — confirm in B1.
