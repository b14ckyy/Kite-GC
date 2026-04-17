# Flightlog Database

This document defines the flight logging database model that KiteGC (Kite Ground Control) should target for replay, mission context, and simple automatic fault analysis.

It is intentionally scoped for GCS and replay use cases, not full Blackbox tuning or firmware-debug workflows.

## Scope

Included fields should help with at least one of these goals:

- Replay the flight path, orientation, altitude, and mission context
- Detect clear pilot mistakes or navigation mistakes automatically
- Detect obvious hardware or sensor issues automatically
- Support later UI overlays such as sensor state, wind, or active waypoint

Excluded fields are typically Blackbox tuning data, PID internals, or low-level diagnostics that are not easy to interpret automatically inside a GCS.

## Current Schema

Current implemented schema version: `v4`

Migration history:

- `v1`: initial `flights` and `telemetry_records`
- `v2`: `blackbox_records`, `blackbox_files`, `flights.source`
- `v3`: `telemetry_records.link_quality`
- `v4`: replay-focused telemetry fields (baro/GPS quality/nav/state/wind/RC arrays/sensor health)

## Replay Schema Direction

Current replay target schema: `v4` (implemented)

### flights

These are flight-level metadata values, stored once per flight.

| Field | Type | Status | Source | Purpose |
|---|---|---|---|---|
| `id` | `INTEGER` | existing | internal | Primary key |
| `start_time` | `TEXT` | existing | live / blackbox header | Flight start timestamp |
| `end_time` | `TEXT` | existing | live / derived | Flight end timestamp |
| `duration_sec` | `INTEGER` | existing | derived | Flight duration |
| `source` | `TEXT` | existing | internal | `live`, `blackbox`, or future combined source |
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

No additional flight-level columns are currently planned for replay.

### telemetry_records

These are sampled replay records. New `v4` fields below are the current target set.

| Field | Type | Status | Source | Purpose |
|---|---|---|---|---|
| `id` | `INTEGER` | existing | internal | Primary key |
| `flight_id` | `INTEGER` | existing | internal | Parent flight |
| `timestamp_ms` | `INTEGER` | existing | telemetry | Milliseconds since flight start |
| `lat` | `REAL` | existing | GPS / blackbox | Replay track |
| `lon` | `REAL` | existing | GPS / blackbox | Replay track |
| `alt_m` | `REAL` | existing | GPS altitude | Replay altitude |
| `speed_ms` | `REAL` | existing | GPS speed | Replay speed |
| `heading` | `INTEGER` | existing | GPS course / heading | Replay orientation |
| `vario_ms` | `REAL` | existing, newly populated in v4 plan | `GPS_velned[2]` | GPS vertical speed |
| `voltage` | `REAL` | existing | battery | Power analysis |
| `current_a` | `REAL` | existing | current meter | Power analysis |
| `mah_drawn` | `INTEGER` | existing | cumulative current | Power analysis |
| `rssi` | `INTEGER` | existing | receiver | Link quality fallback |
| `roll` | `REAL` | existing | attitude | Replay orientation |
| `pitch` | `REAL` | existing | attitude | Replay orientation |
| `yaw` | `INTEGER` | existing | attitude | Replay orientation |
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

## Field Semantics

### active_flight_mode_flags

This field should store the real active INAV flight mode bitmask, not RC box activation.

- Store as raw integer bitmask
- Decode in UI or replay overlays later
- Preferred over legacy `flightModeFlags` / `rcModeFlags`

### state_flags

This field stores INAV runtime state flags.

It is useful for:

- Vehicle type detection (`AIRPLANE`, `MULTIROTOR`, `ROVER`, `BOAT`)
- VTOL / transition-aware replay context
- Detecting state such as `LANDING_DETECTED`, `GPS_FIX`, `AIRMODE_ACTIVE`, etc.

Store as raw integer bitmask and decode later in UI logic.

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

- Store raw packed integer in DB
- Decode in replay UI to drive sensor-health indicators over time

### nav_state

This is the current navigation state machine state, such as RTH, waypoint processing, loitering, landing, or emergency landing stages.

Store raw integer and decode in UI if needed.

### nav_flags

This is the navigation status flag bitmask.

At minimum it can indicate states such as:

- adjusting position
- adjusting altitude

Store raw integer and decode later.

### rc_data_json / rc_command_json

These should be stored as JSON arrays in `TEXT` columns.

Reason:

- Blackbox can log more than 4 channels depending on configuration
- JSON avoids hard-coding a fixed channel count in the schema
- Replay and analysis logic can still parse the arrays efficiently

Recommended examples:

- `rc_data_json`: `[1500,1500,1500,1000]`
- `rc_command_json`: `[0,0,0,1041]`

### wind_n_ms / wind_e_ms / wind_d_ms

Store the INAV wind estimator output as separate axis values.

Purpose:

- future map overlay
- replay context for navigation behavior
- simple automatic detection of strong wind / crosswind scenarios

## Explicitly Excluded Fields

These are intentionally not part of the replay-focused DB target at this time:

- `flight_mode_flags` / `flightModeFlags2` (RC box activation, not actual active modes)
- PID internals and tuning terms
- gyro/filter/debug internals
- most header-only tuning/config values beyond existing aircraft identity metadata
- Blackbox-only diagnostic data that is not easily auto-interpretable in a GCS

## Replay-Focused Analysis Enabled By This Schema

This schema should support all of the following without building a dedicated Blackbox tuning tool:

- map replay with orientation, altitude, speed, and mission context
- GPS quality detection and overlays
- RX loss and link degradation detection
- battery sag, current spikes, and consumption analysis
- sensor-health replay overlays via `hw_health_status`
- VTOL / vehicle-type aware display logic via `state_flags`
- wind display on the map
- pilot-input vs FC-command comparisons via `rc_data_json` and `rc_command_json`

## Backlog Notes

- Milestone 4: decode Blackbox header `features` into a human-readable feature decode field for analysis and UI display