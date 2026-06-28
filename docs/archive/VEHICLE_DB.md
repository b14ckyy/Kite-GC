# Vehicle Database — Feature Plan

> ARCHIVED (2026-06-28) — feature complete (Phases A–C shipped). On-device INAV validation is user-side;
> not kept open.

> STATUS: SHIPPED-LOCAL (2026-06-25), uncommitted-then-committed, **awaiting on-device INAV/SITL test**.
> Phases A–C all implemented; `npm run check` 0/0, `cargo test --no-run` + `npm run build` green.
> Mirrors the Battery DB architecture 1:1 (table + soft-link + manager panel + `.k*` export/import +
> FC baseline). Last major pre-1.0 subsystem.
>
> **As-built deltas from the plan below (decided during implementation):**
> - **Records derived, not stored** — max flight time/distance/altitude come live from the linked
>   flights (argmax query, incl. the achieving flight), like `battery_aggregate`. No stored record
>   columns → always correct after relink/delete.
> - **`image` is TEXT (base64 data URI)**, not BLOB — trivial across the Tauri boundary and makes
>   `.kvehicle` self-contained for free.
> - **`id` is INTEGER PK AUTOINCREMENT** (mirrors `battery_packs`), not a uuid.
> - **Craft-name link uses the existing `flightlog_update_craft_name`** (no new `flight_set_craft_name`
>   command); craft name normalized = trimmed (case preserved), matched case-insensitively (COLLATE NOCASE).
> - **Phase C is implemented** (it was flagged "investigate"): the INAV `stats` totals ARE readable as
>   settings via `MSP2_COMMON_SETTING` by name → `inav_read_stats` reads `stats` + `stats_flight_count`/
>   `stats_total_time`/`stats_total_dist`/`stats_total_energy`. Adopted on request into `base_*` columns.
> - **Open questions resolved:** Q1 craft-name = trim + case-insensitive match, store as-typed. Q2
>   sensor set = airspeed · rangefinder · optical_flow · gps · rtk · compass_ext (shipped as-is).
> - **Energy unit caveat:** `stats_total_energy` shown as Wh assuming the FC reports mWh (÷1000) —
>   verify against a real INAV FC; one-line fix if the unit differs.

## Goal
A library of the user's aircraft/vehicles to (a) link flights to a specific craft and (b) hold a
structured **setup overview** (the build sheet) per vehicle. Flights link by **`craft_name`** — already
recorded in every flight row — exactly as the Battery DB links by `battery_serial`.

## Linking model (decided)
- **Link key = `flights.craft_name`** (soft-link, no FK). Already stored per flight → works retroactively
  for existing logs, zero migration.
- `craft_name` is normalized the same way as battery serials (the existing `normalize_serial` policy:
  trim/clean; we keep case for craft names — see Open Q1) so equal names match reliably.
- **INAV**: `craft_name` comes from the FC automatically. On first link we can offer:
  *"Write craft name «XYZ» to the INAV FC and store it here?"* (MSP set-name + EEPROM).
- **ArduPilot / PX4**: no craft name from the FC → the user assigns the vehicle **post-flight** in the
  Flight Detail / End-Flight flow (manual link, same UX as the battery picker).
- A flight whose `craft_name` matches no vehicle triggers an **"Create vehicle from «XYZ»"** suggestion.

## Schema — table `vehicles` (DB **v16**)
All columns nullable/optional unless noted. TEXT unless stated. Mirrors `battery_packs` creation
(idempotent `CREATE TABLE IF NOT EXISTS` in its own `ensure_*` fn; `flights.craft_name` already exists).

### Identity & status
| col | type | notes |
|---|---|---|
| `id` | TEXT PK | uuid |
| `name` | TEXT NOT NULL | display name (free) |
| `craft_name` | TEXT | link key to `flights.craft_name`; may differ from `name` |
| `type` | TEXT | `fixed_wing` · `flying_wing` · **`vtol`** (incl. quad-plane / any transitioning) · `multirotor` · `helicopter` · `rover` · `boat` · `other` |
| `status` | TEXT | `active` · `storage` · `retired` · `damaged` · `crashed` |
| `image` | BLOB | embedded (downscaled on import) → `.kvehicle` export stays self-contained |
| `notes` | TEXT | freetext |
| `created_at` / `updated_at` | INTEGER | epoch ms |

### Airframe
| col | type | notes |
|---|---|---|
| `model` | TEXT | off-the-shelf model name (freetext) |
| `wingspan_mm` | INTEGER | wingspan / frame size |
| `length_mm` | INTEGER | length |
| `weight_auw_g` | INTEGER | all-up weight |
| `weight_dry_g` | INTEGER | without battery |

### Propulsion (all freetext — paste specs as needed)
`motors` · `props` · `esc`

### Power recommendation (documentation only — no battery link)
`recommended_cells` (TEXT, e.g. "4S–6S") · `recommended_capacity_mah` (INTEGER)

### Radio / FPV / Link (all freetext)
| col | notes |
|---|---|
| `rx` | receiver + protocol |
| `vtx` | video transmitter |
| `camera` | FPV camera |
| `gimbal_camera` | payload / gimbal camera |
| `datalink` | secondary telemetry / link: SiK, 5G/LTE, ESP32 INAV-Radar, ADS-B, etc. |

*(VTX antenna and a standalone GPS field dropped — GPS moves to Sensors.)*

### Sensors (boolean checkboxes)
`airspeed` · `rangefinder` · `optical_flow` · `gps` · `rtk` · `compass_ext` (external mag)
*(Open Q2: confirm/trim this set — candidates also: current sensor, baro.)*

### Flight Controller (stored; can be auto-prefilled from the latest linked flight, editable)
| col | notes |
|---|---|
| `fc_model` | board model |
| `fc_manufacturer` | maker |
| `fc_firmware` | INAV / Betaflight / ArduPilot / PX4 (from `fc_variant`) |
| `fc_firmware_version` | from `fc_version` |
| `blackbox_available` | BOOL — Blackbox / Dataflash present |

### Records (stored on the vehicle, updated per linked flight)
| col | type | notes |
|---|---|---|
| `max_flight_time_s` | INTEGER | + `max_flight_time_flight_id` (which flight) |
| `max_distance_m` | INTEGER | + `max_distance_flight_id` |
| `max_altitude_m` | INTEGER | + `max_altitude_flight_id` |

### Optional INAV FC baseline (Phase C — investigate MSP availability first)
Like the battery `set_baseline`: if INAV `stats` are enabled, read the FC's cumulative
**total flights / total time / total distance** once at first link as a baseline. Stored as
`fc_stat_*_baseline` columns. **No existing MSP read for this yet** → needs protocol investigation;
deferred to Phase C, not a v1 blocker.

### Live-derived (NOT stored — computed by join over `craft_name`)
Flight count · total logged time · total distance · last-flown date · latest FC info. Same pattern as
`battery_db_aggregate` / `battery_db_flights`.

## Backend (Rust) — mirror `battery_db_*`
- `flightlog/db.rs`: `ensure_vehicle_objects()` (v16 table create, idempotent); structs `Vehicle`,
  `VehicleRecords`; fns `create_vehicle`, `update_vehicle`, `list_vehicles`, `get_vehicle`,
  `find_vehicle_by_craft_name`, `delete_vehicle` (flights keep `craft_name`, fall back to "not in
  library"), `vehicle_aggregate(craft_name)`, `vehicle_flights(craft_name)`, `update_vehicle_records`
  (called when a flight is finalized/linked), `set_vehicle_fc_baseline` (Phase C).
- `flightlog/normalize`: reuse the serial-normalize approach for craft names (Open Q1 on case).
- `commands/`: `vehicle_db_create/update/list/get/find_by_craft/delete/aggregate/flights`,
  `flight_set_craft_name` (manual relink, like `flight_set_battery_serial`),
  `vehicle_file_write/read` (`.kvehicle`, like `.kbatt`), `vehicle_set_baseline` (Phase C),
  `inav_set_craft_name` (MSP write craft name + EEPROM — INAV only, the "save to FC" offer).
- `lib.rs`: register all of the above next to the `battery_db_*` block ([lib.rs:251](../../src-tauri/src/lib.rs#L251)).
- Records update: on flight finalize/relink, compare the flight's duration/distance/alt against the
  linked vehicle's stored records and update (with the flight id) if exceeded.

## Frontend (Svelte 5 runes)
- `stores/vehicleManager.ts`: `vehicleManagerOpen` store (mirror `batteryManager.ts`) + `Vehicle` types
  in `stores/flightlogTypes.ts`; `normalizeCraftName` helper (matches Rust).
- `components/logbook/VehicleManager.svelte`: full-panel library (list + editor), mirror
  `BatteryManager.svelte`. **Image frame on top** over the vehicle info (same presentation as the WP
  mission preview header). NumberStepper for all numeric fields, Toggle for sensor/blackbox checkboxes,
  `<select>` for `type`/`status`, freetext inputs for the spec fields. `.kvehicle` import/export buttons.
- `components/logbook/LogbookPanel.svelte`: add a **Vehicles** button **before** the Batteries button
  ([LogbookPanel.svelte:273](../../src/lib/components/logbook/LogbookPanel.svelte#L273)); Vehicles 1st,
  Batteries 2nd. Render `{#if $vehicleManagerOpen}<VehicleManager .../>`.
- `FlightDetail.svelte` / `EndFlightDialog.svelte`: craft-name link UI for Ardu/PX4 (manual picker, like
  the battery serial picker); INAV shows the linked vehicle + offers the "write craft name to FC" action.
- Stats view: vehicle detail shows live-derived totals (join) + the stored records (max time/dist/alt
  with a link to the achieving flight).

## i18n
New `vehicleMgr.*` keys (and `endFlight.*` / `flightDetail.*` additions for the craft link) in
**en + de** (and fr). NumberStepper, dark theme, no `any`.

## Phasing
- **Phase A — DB + backend** ✅: v16 `vehicles` table, CRUD/find/aggregate/flights commands, `.kvehicle`
  IO. (Craft-name relink reuses the existing `flightlog_update_craft_name`.)
- **Phase B — Vehicle Manager UI** ✅: panel, editor, image header, logbook button, list/detail/stats.
- **Phase B2 — linking UX** ✅: craft-name link + real-time picker in FlightDetail, flight-jump fix,
  "create vehicle from craft name" (flight detail + post-flight), `inav_set_craft_name` + "write to FC"
  button (INAV connected + disarmed).
- **Phase C — FC stats baseline (INAV)** ✅: `inav_read_stats` reads the `stats` settings; adopt-on-request
  into the vehicle `base_*` baseline; lifetime display = baseline + Σ logged flights; carried in `.kvehicle`.

## Open questions (carry into Phase A)
1. **Craft-name normalization/case**: battery serials are forced UPPER + alnum-only. Craft names are
   user-facing display strings (mixed case, spaces) → propose **case-insensitive, whitespace-trimmed
   match** but **store as-typed** (don't mangle display). Confirm before coding the link.
2. **Sensor checkbox set**: confirm/trim `airspeed · rangefinder · optical_flow · gps · rtk ·
   compass_ext` (+ optional current sensor / baro?).

## Out of scope (explicit)
- **No battery↔vehicle link** — batteries are cross-build (DIY INAV/Ardu), used across vehicles.
- No per-component sub-tables (the freetext propulsion/RF fields cover ad-hoc specs).
- No live FC config sync beyond the optional INAV craft-name write + Phase-C stats baseline.

## Verification
`npm run check` (0 errors) + `npm run build`; `cargo check` + `cargo test --no-run`. On-device:
INAV auto-link + "write to FC"; Ardu/PX4 manual link; records update across multiple flights;
`.kvehicle` round-trip; image frame render; type/status enums; sensor toggles persist.
