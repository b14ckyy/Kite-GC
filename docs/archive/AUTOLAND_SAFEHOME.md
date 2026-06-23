<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

# Autoland & Safe Home Manager (INAV) — Feature Plan

> ARCHIVED (2026-06-23) — SHIPPED. Phases **A–E done** (backend read/write + connect-load + Safe Home
> Manager editor + 2D **and 3D** map overlays). The first of the three "Airspace Manager" safety
> subsystems (Autoland → Geozones → Geofence). This iteration covered **safehome display + the Safe Home
> Manager (safehome editing + fixed-wing autoland approach config)**. Deliberate follow-ups: the **mission
> LAND-waypoint** autoland (fwapproach indices 8+, waypoint placement) and the home-marker→safehome swap
> on arm. Next safety subsystem: **Geozones**.
>
> Phase E (3D, `Map3D.svelte::updateSafehome3D`): teardrop "H" billboards + green `max_distance` ring
> (disarmed-only) + yellow `loiter_radius` ring at the approach altitude + the approach drawn as a real
> descent (downwind level → base −33 % → final to ground), all height-anchored to the safehome's Cesium
> ground sample (purely visual — no MSL/geoid dependency). Per-slot **Clear** button + empty-slot vs
> set-slot default handling added.

## Context / why

INAV fixed-wing autoland (7.1+) lands the aircraft automatically at a **safehome** (on RTH) or a mission
**LAND** waypoint, flying a wind-aware approach (loiter → downwind → base → final → glide → flare).
Configuration is a set of **safehome points** (lat/lon, since INAV 2.6), **per-site approach settings**
(`fwapproach`) and **global `nav_fw_land_*` settings**. Kite currently shows none of this. We want to
(a) always display safehomes on the map, and (b) give a connected-INAV editor for safehomes + autoland.

Reference docs: INAV `docs/Fixed Wing Landing.md`, `docs/Safehomes.md`; firmware PR
[iNavFlight/inav#9713]. Approach geometry to be ported 1:1 from inav-configurator.

## Gating (two decoupled capabilities)

- **Safehome display (read-only):** any connected INAV (≥7.0, our minimum). Safehomes + the two radius
  settings are downloaded on every connect (**always on**) and drawn on 2D + 3D. No editing.
- **Safe Home Manager (edit + autoland):** INAV **≥7.1** only — the house button appears, per-site
  approaches + `nav_fw_land_*` are loaded/editable, "Save to FC" persists. Validated **7.1.0–9.1.x**;
  a connected version **> 9.1.x** still works but shows a "version not validated" hint in the config
  panel (the config UI will change in a later INAV; re-gate then).
- **< 7.1:** no house button, no editing, no autoland — map display only. (No partial-edit filters: no
  one should fly < 7.1.)

`features.rs` already gates `AutolandConfig` at 7.1.0; add an upper validated bound (`autoland_validated`
→ true for 7.1.0..=9.1.x). The house button is **INAV-only** (not on the Ardu/PX4 mission panel).

## MSP (INAV)

Per-index request/response (loop slots 0–7), via the scheduler handle (`msp_request`), mirroring
`commands/rc.rs::rc_read_fc_config`:

| Message | Code | Payload |
|---|---|---|
| `MSP2_INAV_SAFEHOME` | `0x2038` | req `u8 idx` → resp `u8 idx, u8 enabled, i32 lat(1e7), i32 lon(1e7)` |
| `MSP2_INAV_SET_SAFEHOME` | `0x2039` | `u8 idx, u8 enabled, i32 lat, i32 lon` |
| `MSP2_INAV_FW_APPROACH` | `0x204A` | req `u8 idx` → resp `u8 idx, i32 approachAlt(cm), i32 landAlt(cm), u8 approachDirection(0=L/1=R), i16 heading1, i16 heading2, u8 isSeaLevelRef` |
| `MSP2_INAV_SET_FW_APPROACH` | `0x204B` | same field order as the read response |
| `MSP_EEPROM_WRITE` | `250` (exists) | persist after the SET batch |

- Headings: positive = bidirectional (e.g. 90 ⇒ 90°+270°), negative = exclusive direction, 0 = off.
- Global settings via the existing `read_setting`/`set_setting` (MSP2_COMMON_SETTING/SET_SETTING):
  `nav_fw_land_approach_length`, `..._final_approach_pitch2throttle_mod`, `..._glide_alt`,
  `..._flare_alt`, `..._glide_pitch`, `..._flare_pitch`, `..._max_tailwind`, plus `safehome_max_distance`,
  `safehome_usage_mode`, `nav_rth_allow_landing`, `nav_fw_loiter_radius`.
- **Open detail (verify at impl):** the exact integer width per setting (from INAV `settings.yaml`) — read
  returns the setting's raw bytes; SET must write the same width.

## Save semantics

Edits accumulate in a frontend **working copy** (not live). **"Save to FC"** sends the whole package in
one go — all changed `SET_SAFEHOME` + `SET_FW_APPROACH` + `set_setting` — then a single
`MSP_EEPROM_WRITE`. No per-keystroke writes.

## Backend

- `msp/types.rs`: add the four MSP codes above.
- Factor `read_setting`/`set_setting` out of `commands/rc.rs` into a shared `msp/settings.rs` (`pub(crate)`).
- `commands/safehome.rs`:
  - `safehome_read_all(state) -> SafeHomeConfig` — always reads 8 safehomes + `safehome_max_distance` +
    `nav_fw_loiter_radius`; **only when ≥7.1** also reads 8 approaches + the `nav_fw_land_*` +
    `safehome_usage_mode` + `nav_rth_allow_landing` settings (avoid MSP errors on old fw).
  - `safehome_write_all(config, state)` — batch SET (safehomes + approaches + settings) + `MSP_EEPROM_WRITE`.
  - Register both in `lib.rs`.
- `features.rs`: `autoland_validated(version)` upper bound (9.1.x).

## Frontend

- `stores/safehome.ts`: `SafeHomeConfig` (safehomes[8], approaches[8], autoland/global settings),
  working copy + dirty flag, `safeHomeManagerOpen`, plus a persisted `showSafehomes` setting (default on).
- Connect `$effect` (INAV connected) → `safehome_read_all` → store. (Always; the ≥7.1 extras are filled
  only when present.) Mirror the RC panel's connect effect.
- `components/mission/SafeHomeManager.svelte` — slim panel (no transition), swapped into the INAV mission
  body when `safeHomeManagerOpen` (like `MissionManager`): top = autoland/global config form (editable
  only when connected INAV ≥7.1; "not validated" hint > 9.1.x; "connect to INAV" when offline) + the
  display toggle + "Save to FC"; bottom = safehome list (8 slots, each expandable: enabled, lat/lon,
  approachAlt, landAlt, direction L/R, heading1/2, sea-level ref).
- House button in `InavMissionPanel.svelte` toolbar (home icon, app style, at the marked toolbar slot,
  ≥7.1) → toggles `safeHomeManagerOpen`.
- `helpers/autolandGeometry.ts` — pure helper: from safehome pos + approach config + `nav_fw_loiter_radius`
  + `approach_length`, compute the loiter circle, downwind/base/final legs and the landing-direction
  arrow(s). Ported from inav-configurator; shared by 2D + 3D.
- i18n en/de/fr throughout.

## Map visualization (per enabled safehome; gated by `showSafehomes`)

- **`safehome_max_distance`** ring: green dashed + black outline. **Disarmed only** — hidden on arm.
- **`nav_fw_loiter_radius`** ring: yellow dashed. Always (overlay on). In **3D** at `approach_alt` above
  ground (per-safehome, ≥7.1); on the **ground** for older INAV.
- Safehome **marker** (distinct icon); draggable only when editing (≥7.1) → updates working-copy lat/lon.
- **Approach geometry** overlay only when approach data exists (≥7.1).
- **Arm behaviour (this iteration):** on arm, only hide the green `max_distance` ring. The home-marker →
  safehome swap is **deferred** (would need client-side active-safehome selection; revisit later).
- 2D = `Map.svelte` (Leaflet markers/polylines/circles, reuse the GCS-marker drag pattern); 3D =
  `Map3D.svelte` (billboards + polylines via the cesium overlay manager).

## Phases

- **A — Backend:** MSP codes + `safehome.rs` read_all/write_all + EEPROM + `settings.rs` extract +
  validated-bound. Verify against INAV SITL (read, write batch, reboot → persisted).
- **B — Data load:** store + connect effect + read-only readout in a temporary spot.
- **C — Safe Home Manager panel:** house button + slim panel + config form + safehome list + Save to FC.
- **D — 2D map:** marker + both rings + approach geometry + drag placement + arm-state ring hide.
- **E — 3D map:** safehome billboards + rings (loiter at approach_alt) + approach lines.

## Verification

INAV **SITL** (available): connect → safehomes/approaches/settings load; edit in the manager; Save to FC;
reboot SITL → values persisted. Cross-check map geometry against inav-configurator for the same config.
Old-version path: a <7.1 SITL/build shows markers + rings, no house button, no editing.

## Open details (resolve during impl, non-blocking)

- Exact integer widths of the `nav_fw_land_*` / `safehome_*` settings (INAV `settings.yaml`).
- Exact approach geometry (port from inav-configurator).
- Later: mission LAND-waypoint autoland (fwapproach idx 8+) + waypoint placement; home-marker→safehome
  swap on arm; the planned INAV config-scheme change (new version gate).
