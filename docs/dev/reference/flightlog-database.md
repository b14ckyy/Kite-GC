# Flight-log database (schema reference)

This document describes the local SQLite schema Kite Ground Control uses for the flight log, vehicle, and battery libraries.

It is intentionally scoped for GCS and replay use cases, not full Blackbox tuning or firmware-debug workflows.

## Scope

Included fields should help with at least one of these goals:

- Replay the flight path, orientation, altitude, and mission context
- Detect clear pilot mistakes or navigation mistakes automatically
- Detect obvious hardware or sensor issues automatically
- Support later UI overlays such as sensor state, wind, or active waypoint

Excluded fields are typically Blackbox tuning data, PID internals, or low-level diagnostics that are not easy to interpret automatically inside a GCS.

## Current schema

Current implemented schema version: `v21`. The core replay-focused telemetry field set was complete at
`v5`; `v6`–`v20` add flight-level mission/pilot/battery/vehicle links and a handful of additional
live/import telemetry fields (link metrics, airspeed, throttle, canonical flight mode); `v21` drops the
stored decoded Blackbox rows.

Migration history (tracked via `PRAGMA user_version`):

- `v1`: initial `flights` and `telemetry_records`
- `v2`: `blackbox_records` (decoded Blackbox CSV rows, dropped again in `v21`), `blackbox_files`, `flights.source`
- `v3`: `telemetry_records.link_quality`
- `v4`: replay-focused telemetry fields (baro/GPS quality/nav/state/wind/RC arrays/sensor health)
- `v5`: `nav_lat`, `nav_lon`, `nav_alt_m` columns for INAV navigation filter data
- `v6`: `flights.linked_flight_id` (live↔blackbox pairing)
- `v7`/`v8`: mission library (`missions` table + `flights.mission_id` + `flights.logged_wp_count`)
- `v9`: `flights.pilot_name` + `flights.pilot_id`
- `v10`: `battery_packs` table + soft `flights.battery_serial` link
- `v11`: `missions.home_lat` + `missions.home_lon` (planned launch/home reference for REL altitudes)
- `v12`: `telemetry_records.mode_primary` + `telemetry_records.mode_modifiers` (canonical flight-mode columns)
- `v13`: `flights.utc_offset_min` (flight-location UTC offset in minutes, east-positive, DST-aware)
- `v14`: `telemetry_records.link_snr` + `telemetry_records.link_rssi_dbm` (RC-link metrics)
- `v15`: `telemetry_records.airspeed_ms` (airspeed in m/s)
- `v16`: `vehicles` table (vehicle library; soft-linked via `flights.craft_name`)
- `v17`: `telemetry_records.throttle_pct` (throttle in %)
- `v18`: `battery_records` table + `idx_battery_flight` index (per-instance multi-battery samples)
- `v19`: `flights.fc_uid` + `vehicles.fc_uid` (FC hardware id)
- `v20`: `flight_groups` table + `flights.group_id`, `flight_events` table, `telemetry_records.wall_ms`,
  `flights.vehicle_id` (flight → library vehicle link)
- `v21`: `flights.source_before_link`; `blackbox_records` and its index dropped (nothing read the stored
  decoded rows — replay uses `telemetry_records`, hi-res replay re-decodes the archived original in
  `blackbox_files`). The back-fill sets `source_before_link` on every linked flight: `blackbox` if a
  log importer made it (`protocol` `BLACKBOX` / `DATAFLASH` / `ULOG`) or it holds decoded rows or an
  archived log, otherwise `live`. On unlink, a flight without a stored value falls back to the same
  protocol / archived-log check.

Freed space: the database runs with `auto_vacuum = INCREMENTAL`, so deleted data (e.g. a blackbox
BLOB) first lands on the SQLite freelist. A background thread hands the freelist back to the file
system (`db::spawn_reclaim`). It starts after every delete of a flight, a group or an archived log. It
also starts whenever the database is opened with more than 4096 free pages (16 MiB at 4 KiB pages):
that covers a schema step that dropped a table and a reclaim cut short by an app exit.

The thread runs on its own connection, in chunks of `PRAGMA incremental_vacuum(1024)` pages, until
the freelist is empty. Each chunk is its own short write transaction (tens of ms on an SSD, a few
hundred ms on a hard disk), with a 5 ms pause between chunks, and the WAL stays small. A recording or
import waits at most about one chunk, but only because every write transaction on the flight database
is `BEGIN IMMEDIATE` (`db::write_tx`). A deferred transaction that reads before it writes would fail
with `SQLITE_BUSY` at once instead of waiting. Only one such thread runs per database file. The
scratch store of "open a log without importing it" is never reclaimed.

## Replay schema

The core replay-focused telemetry fields are complete as of `v5`; later migrations (`v6`–`v20`) add
flight-level metadata (mission/pilot/battery/vehicle links, time-zone offset) plus a few additional
telemetry fields (`v12` canonical flight mode, `v14` link metrics, `v15` airspeed, `v17` throttle).
See the migration history above.

### flights

These are flight-level metadata values, stored once per flight.

| Field | Type | Status | Source | Purpose |
|---|---|---|---|---|
| `id` | `INTEGER` | existing | internal | Primary key |
| `start_time` | `TEXT` | existing | live / blackbox header | Flight start timestamp |
| `end_time` | `TEXT` | existing | live / derived | Flight end timestamp |
| `duration_sec` | `INTEGER` | existing | derived | Flight duration |
| `source` | `TEXT` | existing | internal | `live`, `blackbox` (any imported log), or `both` for a linked live/log pair |
| `source_before_link` | `TEXT` | `v21` | internal | The flight's own source (`live` / `blackbox`) while `source` reads `both`; restored on unlink. NULL for an unlinked flight |
| `craft_name` | `TEXT` | existing | MSP / blackbox header | Aircraft name |
| `fc_variant` | `TEXT` | existing | MSP / importer | FC family |
| `fc_version` | `TEXT` | existing | MSP / blackbox header | Firmware version |
| `board_id` | `TEXT` | existing | MSP / blackbox header | FC target / board |
| `platform_type` | `INTEGER` | existing | MSP | Aircraft platform type |
| `protocol` | `TEXT` | existing | internal | Telemetry/import source protocol |
| `start_lat` | `REAL` | existing | telemetry | Start coordinate |
| `start_lon` | `REAL` | existing | telemetry | Start coordinate |
| `location_name` | `TEXT` | existing | reverse geocode | Human-readable location |
| `weather_temp_c` | `REAL` | existing | weather API / user | Flight weather |
| `weather_wind_ms` | `REAL` | existing | weather API / user | Flight weather |
| `weather_wind_deg` | `INTEGER` | existing | weather API / user | Flight weather |
| `weather_desc` | `TEXT` | existing | weather API / user | Flight weather |
| `max_alt_m` | `REAL` | existing | derived | Max altitude |
| `max_speed_ms` | `REAL` | existing | derived | Max speed |
| `max_distance_m` | `REAL` | existing | derived | Max distance from home/start |
| `total_distance_m` | `REAL` | existing | derived | Total travelled distance |
| `battery_used_mah` | `INTEGER` | existing | derived | Flight consumption |
| `notes` | `TEXT` | existing | user | User notes |
| `pilot_name` | `TEXT` | `v9` | user | Pilot / operator name (manually editable) |
| `pilot_id` | `TEXT` | `v9` | user | Pilot / operator ID (manually editable) |
| `battery_serial` | `TEXT` | `v10` | user | Serial of the battery pack flown — **soft link** to `battery_packs.serial`, resolved at read time (no FK) |
| `utc_offset_min` | `INTEGER` | `v13` | derived | Local UTC offset at the flight location, in minutes (east-positive), DST-aware; `start_time` stays true UTC |

`pilot_name` / `pilot_id` (schema `v9`) are manually editable in the flight detail panel.
A future operator/login system can prefill them on new recordings; the columns are the
forward-looking anchor for that. Added idempotently (`ensure_v9_schema`, self-healing like the
mission-library `v8` columns) — existing DBs gain them automatically on next open, no data loss.

`battery_serial` (schema `v10`) links the flight to a battery pack **by serial** (a soft link,
resolved against `battery_packs.serial` at read time — there is no foreign key). The
`battery_packs` table is owned by the Battery Manager. Added idempotently (`ensure_v10_schema`).

### telemetry_records

These are sampled replay records. The `v4` fields below are the current target set.

| Field | Type | Status | Source | Purpose |
|---|---|---|---|---|
| `id` | `INTEGER` | existing | internal | Primary key |
| `flight_id` | `INTEGER` | existing | internal | Parent flight |
| `timestamp_ms` | `INTEGER` | existing | telemetry | Milliseconds since flight start |
| `lat` | `REAL` | existing | GPS / blackbox | Replay track |
| `lon` | `REAL` | existing | GPS / blackbox | Replay track |
| `alt_m` | `REAL` | existing | GPS altitude | Replay altitude |
| `speed_ms` | `REAL` | existing | GPS speed | Replay speed |
| `heading` | `INTEGER`¹ | existing | GPS course over ground | Replay orientation |
| `vario_ms` | `REAL` | existing | `gps_velned[2]` (negated ÷100) or `vario` (÷100) | GPS vertical speed (m/s, positive = climbing) |
| `voltage` | `REAL` | existing | battery | Power analysis |
| `current_a` | `REAL` | existing | current meter | Power analysis |
| `mah_drawn` | `INTEGER` | existing | cumulative current | Power analysis |
| `rssi` | `INTEGER` | existing | receiver | Link quality fallback |
| `roll` | `REAL` | existing | attitude | Replay orientation |
| `pitch` | `REAL` | existing | attitude | Replay orientation |
| `yaw` | `INTEGER`¹ | existing | FC fused heading (attitude) | Replay orientation |
| `fix_type` | `INTEGER` | existing | GPS | GPS quality |
| `num_sat` | `INTEGER` | existing | GPS | GPS quality |
| `cpu_load` | `INTEGER` | existing | MSP live only | FC load |
| `link_quality` | `INTEGER` | existing | CRSF / blackbox `lq` | Link quality |
| `baro_alt_m` | `REAL` | existing (`v4`) | `BaroAlt` | Barometric altitude for comparison and later derived baro vario |
| `gps_hdop` | `REAL` or `INTEGER` | existing (`v4`) | `GPS_hdop` | GPS quality |
| `gps_eph` | `REAL` or `INTEGER` | existing (`v4`) | `GPS_eph` | Horizontal GPS error |
| `gps_epv` | `REAL` or `INTEGER` | existing (`v4`) | `GPS_epv` | Vertical GPS error |
| `active_wp_number` | `INTEGER` | existing (`v4`) | blackbox slow frame / MSP nav status | Mission context |
| `active_flight_mode_flags` | `INTEGER` | existing (`v4`) | `activeFlightModeFlags` | Actual active flight modes |
| `state_flags` | `INTEGER` | existing (`v4`) | `stateFlags` | Runtime state, including vehicle-type and VTOL mode context |
| `nav_state` | `INTEGER` | existing (`v4`) | `navState` | Navigation state machine state |
| `nav_flags` | `INTEGER` | existing (`v4`) | `navFlags` | Navigation status flags |
| `rx_signal_received` | `INTEGER` | existing (`v4`) | slow frame | RX signal availability |
| `hw_health_status` | `INTEGER` | existing (`v4`) | slow frame | Packed hardware sensor health bits |
| `baro_temperature` | `REAL` or `INTEGER` | existing (`v4`) | slow frame | Barometer temperature |
| `wind_n_ms` | `REAL` | existing (`v4`) | `wind[0]` | Wind overlay / replay context |
| `wind_e_ms` | `REAL` | existing (`v4`) | `wind[1]` | Wind overlay / replay context |
| `wind_d_ms` | `REAL` | existing (`v4`) | `wind[2]` | Wind overlay / replay context |
| `rc_data_json` | `TEXT` | existing (`v4`) | `rcData[]` | Raw pilot inputs |
| `rc_command_json` | `TEXT` | existing (`v4`) | `rcCommand[]` | FC-processed inputs |
| `nav_lat` | `REAL` | existing (`v5`) | — | Always NULL (see note below) |
| `nav_lon` | `REAL` | existing (`v5`) | — | Always NULL (see note below) |
| `nav_alt_m` | `REAL` | existing (`v5`) | `navPos[2]` / 100 | Fused altitude relative to home (m) |
| `mode_primary` | `TEXT` | existing (`v12`) | derived | Canonical primary flight mode (NULL on older rows) |
| `mode_modifiers` | `TEXT` | existing (`v12`) | derived | Canonical flight-mode modifiers (NULL on older rows) |
| `link_snr` | `INTEGER` | existing (`v14`) | CRSF 0x14 / MSP2_INAV_GET_LINK_STATS | RC-link signal-to-noise ratio (dB) |
| `link_rssi_dbm` | `INTEGER` | existing (`v14`) | CRSF 0x14 / MSP2_INAV_GET_LINK_STATS | RC-link RSSI (raw dBm) |
| `airspeed_ms` | `REAL` | existing (`v15`) | VFR_HUD / MSP2_INAV_AIR_SPEED / ARSP / blackbox `airspeed` | Airspeed (m/s) |
| `throttle_pct` | `REAL` | existing (`v17`) | MSP2_INAV_MISC2 / VFR_HUD / blackbox / ArduPilot CTUN | Throttle (%) |

¹ `heading` and `yaw` are **decimal degrees** (`f64`, 0.1° resolution). The column affinity is still
`INTEGER` (historical), but SQLite stores the values as `REAL` automatically — no schema migration needed.

### nav_lat / nav_lon / nav_alt_m

INAV's `navPos[0,1,2]` values from the Blackbox log represent **local-frame North-East-Up centimeter offsets** relative to the GPS home position. They are NOT geographic coordinates.

- `nav_lat` and `nav_lon` are always `NULL`. An earlier attempt to convert `navPos[0,1]` to geographic coordinates using `home + offset / 111320` produced inaccurate tracks (verified by comparison with the `flightlog2kml` reference tool in Google Earth). The local tangent plane approximation introduces systematic offset.
- `nav_alt_m` stores `navPos[2] / 100` — the INAV EKF fused altitude in meters, relative to the home/launch point. This is actively used by the track export module and telemetry adapter for smooth altitude data (raw GPS altitude has 1m integer stepping).
- The columns are retained in the schema for potential future use if a more accurate conversion method is found.

**Track export and map display** always use raw GPS (`lat`/`lon`) for position and `nav_alt_m` for altitude.

### battery_records

Per-instance battery samples (schema `v18`), for ArduPilot/PX4 multi-monitor logging — imported
(`BAT.Instance`) and live (`BATTERY_STATUS.id`). INAV stays single-battery, so it writes no rows here;
the frontend then falls back to the denormalised primary battery values on `telemetry_records`.
Indexed by `(flight_id, timestamp_ms)` via `idx_battery_flight`.

| Field | Type | Status | Source | Purpose |
|---|---|---|---|---|
| `id` | `INTEGER` | `v18` | internal | Primary key |
| `flight_id` | `INTEGER` | `v18` | internal | Parent flight |
| `timestamp_ms` | `INTEGER` | `v18` | telemetry | Milliseconds since flight start |
| `instance` | `INTEGER` | `v18` | telemetry | Battery monitor instance index |
| `voltage` | `REAL` | `v18` | battery | Pack voltage |
| `current_a` | `REAL` | `v18` | current meter | Pack current (A) |
| `mah_drawn` | `INTEGER` | `v18` | cumulative current | Pack consumption (mAh) |
| `battery_percentage` | `INTEGER` | `v18` | battery | Remaining capacity (%) |
| `cell_count` | `INTEGER` | `v18` | battery | Cell count |
| `temperature` | `REAL` | `v18` | battery | Pack temperature |

## Vehicle library

### vehicles

The vehicle library (schema `v16`) holds the per-aircraft build sheet. Flights soft-link to a vehicle by
the existing `flights.craft_name` (no foreign key, no flight-row change), so the link applies
retroactively. Records (max time/distance/altitude) are derived from the linked flights, not stored here.

| Field | Type | Status | Purpose |
|---|---|---|---|
| `id` | `INTEGER` | `v16` | Primary key |
| `name` | `TEXT` | `v16` | Display name (required) |
| `craft_name` | `TEXT` | `v16` | Craft name used to soft-link flights |
| `vehicle_type` | `TEXT` | `v16` | Vehicle type (default `other`) |
| `status` | `TEXT` | `v16` | Lifecycle status (default `active`) |
| `image` | `TEXT` | `v16` | Stored image reference |
| `notes` | `TEXT` | `v16` | User notes |
| `model` | `TEXT` | `v16` | Airframe model |
| `wingspan_mm` | `INTEGER` | `v16` | Wingspan (mm) |
| `length_mm` | `INTEGER` | `v16` | Length (mm) |
| `weight_auw_g` | `INTEGER` | `v16` | All-up weight (g) |
| `weight_dry_g` | `INTEGER` | `v16` | Dry weight (g) |
| `motors` | `TEXT` | `v16` | Motor description |
| `props` | `TEXT` | `v16` | Propeller description |
| `esc` | `TEXT` | `v16` | ESC description |
| `recommended_cells` | `TEXT` | `v16` | Recommended cell configuration |
| `recommended_capacity_mah` | `INTEGER` | `v16` | Recommended battery capacity (mAh) |
| `rx` | `TEXT` | `v16` | Receiver description |
| `vtx` | `TEXT` | `v16` | Video transmitter description |
| `camera` | `TEXT` | `v16` | Camera description |
| `gimbal_camera` | `TEXT` | `v16` | Gimbal camera description |
| `datalink` | `TEXT` | `v16` | Datalink description |
| `sensor_airspeed` | `INTEGER` | `v16` | Has airspeed sensor (0/1) |
| `sensor_rangefinder` | `INTEGER` | `v16` | Has rangefinder (0/1) |
| `sensor_optical_flow` | `INTEGER` | `v16` | Has optical-flow sensor (0/1) |
| `sensor_gps` | `INTEGER` | `v16` | Has GPS (0/1) |
| `sensor_rtk` | `INTEGER` | `v16` | Has RTK GPS (0/1) |
| `sensor_compass` | `INTEGER` | `v16` | Has compass (0/1) |
| `fc_model` | `TEXT` | `v16` | Flight controller model |
| `fc_manufacturer` | `TEXT` | `v16` | Flight controller manufacturer |
| `fc_firmware` | `TEXT` | `v16` | Firmware family |
| `fc_firmware_version` | `TEXT` | `v16` | Firmware version |
| `blackbox_available` | `INTEGER` | `v16` | Blackbox logging available (0/1) |
| `base_flight_count` | `INTEGER` | `v16` | Baseline flight count carried over from prior records |
| `base_total_time_s` | `INTEGER` | `v16` | Baseline total flight time (s) |
| `base_total_dist_m` | `INTEGER` | `v16` | Baseline total distance (m) |
| `base_total_energy` | `INTEGER` | `v16` | Baseline total energy consumed |
| `created_at` | `TEXT` | `v16` | Creation timestamp |
| `updated_at` | `TEXT` | `v16` | Last-update timestamp |

## Field semantics

### active_flight_mode_flags

This field stores the real active INAV flight mode bitmask, not RC box activation.

- Stored as a raw integer bitmask
- Decoded in UI or replay overlays later
- Preferred over legacy `flightModeFlags` / `rcModeFlags`

### state_flags

This field stores INAV runtime state flags.

It is useful for:

- Vehicle type detection (`AIRPLANE`, `MULTIROTOR`, `ROVER`, `BOAT`)
- VTOL / transition-aware replay context
- Detecting state such as `LANDING_DETECTED`, `GPS_FIX`, `AIRMODE_ACTIVE`, etc.

Stored as a raw integer bitmask and decoded later in UI logic.

### hw_health_status

This field stores packed 2-bit hardware sensor status values.

Current INAV packing order is:

1. Gyro
2. Accelerometer
3. Compass
4. Barometer
5. GPS
6. Rangefinder
7. Pitot

Recommended usage:

- Store the raw packed integer in the DB
- Decode in the replay UI to drive sensor-health indicators over time

### nav_state

This is the current navigation state machine state, such as RTH, waypoint processing, loitering, landing, or emergency landing stages.

Stored as a raw integer and decoded in the UI if needed.

### nav_flags

This is the navigation status flag bitmask.

At minimum it can indicate states such as:

- adjusting position
- adjusting altitude

Stored as a raw integer and decoded later.

### rc_data_json / rc_command_json

These are stored as JSON arrays in `TEXT` columns.

Reason:

- Blackbox can log more than 4 channels depending on configuration
- JSON avoids hard-coding a fixed channel count in the schema
- Replay and analysis logic can still parse the arrays efficiently

Examples:

- `rc_data_json`: `[1500,1500,1500,1000]`
- `rc_command_json`: `[0,0,0,1041]`

### wind_n_ms / wind_e_ms / wind_d_ms

Store the INAV wind estimator output as separate axis values.

Purpose:

- future map overlay
- replay context for navigation behavior
- simple automatic detection of strong wind / crosswind scenarios

## Explicitly excluded fields

These are intentionally not part of the replay-focused DB target at this time:

- `flight_mode_flags` / `flightModeFlags2` (RC box activation, not actual active modes)
- PID internals and tuning terms
- gyro/filter/debug internals
- most header-only tuning/config values beyond existing aircraft identity metadata
- Blackbox-only diagnostic data that is not easily auto-interpretable in a GCS

## Data exchange (.kflight)

The `.kflight` file format is a self-contained SQLite database used for sharing flight records between Kite installations.

### Schema

An export holds the per-flight data tables `flights`, `telemetry_records`, `battery_records` and `blackbox_files`, with the same columns as in the main database. The libraries (vehicles, missions, battery packs), group flights and `flight_events` are not exported. On top of that come `blackbox_records` and a metadata table.

`blackbox_records` holds no decoded rows, only one **marker row** per log flight: `flight_id`, `timestamp_us = 0`, `csv_data = ''`. A log flight is one whose own source is `blackbox`: either `source = 'blackbox'`, or `source = 'both'` with `source_before_link = 'blackbox'`. If `source_before_link` is NULL, the same fallback as on unlink decides (log-importer `protocol`, or an archived log). The table and its markers are there for Kite 1.0. Its importer reads the table for every flight and fails the flight when the table is missing. It copies the marker rows like decoded rows, and its unlink restores a flight to `blackbox` when it has rows there.

| Table | Purpose |
|---|---|
| `blackbox_records` | Marker rows for Kite 1.0 (see above); ignored by this version's importer |
| `_kflight_meta` | Export metadata: `schema_version`, `app_id`, `exported_at`, `flight_count` |

### Export

`exchange::export_flights(flight_ids, src_db, dest_path)`:

1. Creates a fresh SQLite file at `dest_path`
2. Creates the tables listed above
3. Copies each flight (with `source_before_link`) and its associated `telemetry_records`, `battery_records`, and `blackbox_files`. A linked pair keeps its link only if both flights are exported. A flight exported without its partner is written as the single flight it was before the link: its own source, `source_before_link` NULL, no link
4. Writes the `blackbox_records` marker rows and the `_kflight_meta` rows
5. VACUUMs the file for minimal size

### Import

`exchange::import_flights(src_path, dest_db)`:

1. Opens the `.kflight` file and validates `_kflight_meta`
2. Lists all flights in the file
3. Duplicate detection: skips flights matching `craft_name` + `start_time` within ±10 seconds
4. Copies non-duplicate flights with all associated records into the main DB, but never the file's `blackbox_records` rows. A file from before `v21` has no `source_before_link` column. For its `both` flights the value is derived from the file: `blackbox` when the flight's `protocol` is `BLACKBOX`, `DATAFLASH` or `ULOG` (only the log importers write these), or when the file holds `blackbox_records` rows (decoded rows or a marker) or a `blackbox_files` row for the flight; `live` otherwise
5. Restores the links between imported flights. An imported flight that arrived as `both` but ends up without a partner becomes the single flight it was, with its own source and `source_before_link` NULL
6. Returns `(imported_count, skipped_count)`

### Raw Blackbox export

`db::get_blackbox_file(conn, flight_id)` retrieves the original binary file from the `blackbox_files.file_data` BLOB. The Tauri command `flightlog_export_blackbox` writes it to a user-selected path. Only available when `flights.source` is `"blackbox"` or `"both"`.

## Replay-focused analysis enabled by this schema

This schema supports all of the following without building a dedicated Blackbox tuning tool:

- map replay with orientation, altitude, speed, and mission context
- GPS quality detection and overlays
- RX loss and link degradation detection
- battery sag, current spikes, and consumption analysis
- sensor-health replay overlays via `hw_health_status`
- VTOL / vehicle-type aware display logic via `state_flags`
- wind display on the map
- pilot-input vs FC-command comparisons via `rc_data_json` and `rc_command_json`
