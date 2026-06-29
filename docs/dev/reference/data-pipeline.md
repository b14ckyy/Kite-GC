# Data pipeline (reference)

How data flows through Kite Ground Control: the **inbound telemetry** path (source → store → widget, for
both live and replay) and the **parallel data networks** that don't go through the telemetry store
(Radar, outbound RC control, the Telemetry Relay, live recording). See the
[architecture overview](../../user/for-developers/architecture.md) for the higher-level picture.

## Overview

```
                         INBOUND  (FC → GCS)                              IMPORT
 ┌───────────────┬───────────────┬───────────────────────────┐   ┌──────────────────┐
 │  Serial/MSP   │  MAVLink      │  Passive telemetry        │   │  Blackbox .TXT   │
 │  (INAV, poll) │  (Ardu/PX4)   │  (listen-only: SmartPort/ │   │  .rawmsp / .tlog │
 │               │  (push)       │   CRSF/LTM/MAVLink, auto) │   │  (import)        │
 └──────┬────────┴──────┬────────┴──────────────┬────────────┘   └────────┬─────────┘
        │ ByteTransport (Serial/TCP/UDP/BLE)    │                         │
        ▼               ▼                       ▼                         ▼
 ┌──────────────┐ ┌──────────────┐ ┌──────────────────────┐      ┌──────────────────┐
 │ MspScheduler │ │MavlinkHandler│ │ PassiveHandler       │      │ blackbox_decode  │
 │ (poll loop)  │ │ (reader+HB)  │ │ (detect + decode)    │      │ / raw parser     │
 └──────┬───────┘ └──────┬───────┘ └──────────┬───────────┘      └────────┬─────────┘
        └────────────────┴─────── same normalized payloads ──────┐        │
                AttitudeData / GpsData / AnalogData / StatusData │        │
                AltitudeData / SensorStatusData / AirspeedData   │        │
                         │                                       │        │
          ┌──────────────┼────────────────────────┐              │        ▼
          ▼              ▼                        ▼              │   SQLite DB
   Tauri "telemetry-*"  FlightRecorder       Telemetry Relay     │   flights +
   events (frontend)    (DB + temp .ktmp)    (re-encode)         │   telemetry_records
          │                                                      │        │
          ▼                                                      │ Replay │
   telemetry.ts store (TelemetryData) ◄── telemetryAdapter.ts ◄──┴────────┘
          │
          ▼
   HUD widgets + Map (2D Leaflet / 3D Cesium)   ── same TelemetryData for live AND replay
```

The guiding principle: **three live protocols share one decode-to-event contract**, so the frontend
never branches on protocol, and **live and replay feed the exact same `TelemetryData`** so widgets and
the maps don't know the difference.

## Live telemetry

Each protocol owns its link on a dedicated thread and emits the **same** protocol-agnostic payload
structs + Tauri events. The protocol is chosen explicitly in the connect UI.

| Protocol | Module | Model | Notes |
|---|---|---|---|
| **MSP** (INAV) | `scheduler/` | request→response **poll loop** | priority polling; owns serial exclusively |
| **MAVLink** (ArduPilot/PX4) | `mavlink_proto/` | **push** reader + 1 Hz GCS heartbeat | stream rates requested at connect |
| **Passive telemetry** | `passive_telemetry/` | **listen-only**, sub-protocol auto-detected | SmartPort / CRSF / LTM / MAVLink-passive |

MSP path, as an example:

```
Scheduler thread (dedicated) — owns the link
  Priority poll: Attitude(5) > Status(4) > Analog(3) > GPS(2) > Secondary(1)
    MSP_ATTITUDE (108)          → AttitudeData
    MSP_RAW_GPS (106)           → GpsData
    MSPV2_INAV_ANALOG (0x2002)  → AnalogData
    MSPV2_INAV_STATUS (0x2000)  → StatusData (+ flight-mode classification)
    MSP_SENSOR_STATUS (151)     → SensorStatusData
    MSP_ALTITUDE (109)          → AltitudeData
  ├─ Feed the FlightRecorder (when logging)
  ├─ Inject outbound RC between polls (RC control)
  └─ Emit Tauri "telemetry-*" events
```

### Tauri telemetry events

Emitted by all live protocols where applicable (NULL/absent where a protocol doesn't provide a field):

| Event | Payload | Notes |
|---|---|---|
| `telemetry-attitude` | roll, pitch, yaw | |
| `telemetry-gps` | lat, lon, alt_msl, ground_speed, course, fix_type, num_sat | |
| `telemetry-gps-stats` | hdop | |
| `telemetry-altitude` | altitude (rel), vario | |
| `telemetry-alt-ref` | ground-MSL anchor (relative-only protocols) | |
| `telemetry-analog` | voltage, current, power, mah_drawn, battery_percentage, rssi, cell_count | |
| `telemetry-status` | arming_flags, flight_mode_flags, cpu_load, sensor_status, rc-override | |
| `telemetry-sensor-status` | gyro, acc, mag, baro, gps, rangefinder, pitot, opflow, prearm | |
| `telemetry-airspeed` | airspeed | MSP2_INAV_AIR_SPEED / VFR_HUD |
| `telemetry-flightmode` | canonical `{ primary, modifiers[] }` | classified in the adapter |
| `telemetry-nav-status` | active_wp_number, nav_state | mission active-WP highlight |
| `telemetry-ekf-status` / `telemetry-ekf-type` | EKF health / EKF2-3 | MAVLink |
| `telemetry-linkstats` | rssi/lq/snr (normalized) | RC-link widget |
| `telemetry-rc-channels` | current RC channel µs | RC-control engage seed |
| `home-position` | lat, lon, alt | authoritative FC home |
| `telemetry-protocol` / `telemetry-fc-link` | sub-protocol name / FC-origin liveness | passive telemetry |
| `telemetry-disconnected` / `connection-lost` | — | teardown signals |

`stores/telemetry.ts` listens to these and merges them into one reactive `TelemetryData` object;
`+page.svelte` subscribes and passes it to widgets as a prop.

## Import (Blackbox / raw logs)

```
INAV Blackbox .TXT ─► blackbox_decode (external bin) ─► CSV ─► blackbox.rs parser ─┐
.rawmsp (MWP v2) / .tlog (MAVLink) ─► raw parser ─────────────────────────────────┤
                                                                                   ▼
                                          downsample → telemetry_records (+ blackbox BLOB)
                                          split by arm/disarm → one flight per arm
```

Unit conversions happen in the parser (Rust); the database stores SI-like units. Widgets may add
display-unit conversion but always receive normalized data. Common Blackbox conversions:

| Field | Raw unit | Stored unit | Conversion |
|---|---|---|---|
| roll/pitch/yaw | decidegrees | degrees | ÷10 |
| vario (NED down) | cm/s | m/s, climb + | negate, ÷100 |
| altitude (baro) | cm | m | ÷100 |
| speed | m/s | m/s | none |

## Replay

```
Logbook select → getFlightTrack(flightId) → TelemetryRecord[]
  ├─ homePosition from flight.start_lat/lon
  ├─ load track polyline
  └─ PlaybackController (100 ms tick; 1×/2×/4×/10×; play/pause/seek) → currentIndex
        ▼  playbackPoint = track[currentIndex]
   telemetryAdapter.toTelemetryData(playbackPoint)   // DB snake_case → TelemetryData; NULL → safe default
        ▼
   +page.svelte:  telem = $derived(isConnected ? liveTelem : toTelemetryData(playbackPoint))
        ▼
   widgets + map — identical interface for live and replay
```

Key decisions: replay is **always from the DB** (raw logs pass through import first); the **same
`TelemetryData` type** is used for live and replay (the `$derived(telem)` switch is the only branch); the
adapter coerces NULL → 0 so widgets always get numbers.

## Multi-protocol layering & unified flight mode

```
Serial / TCP / UDP / BLE ─► ByteTransport trait (read/write/close/set_read_timeout)
   ├─ MspTransport + MspScheduler   (poll loop)            → same normalized payloads
   ├─ MavlinkHandler                (push reader + HB)      → same normalized payloads
   └─ PassiveHandler                (detect + decode)       → same normalized payloads
```

- **ByteTransport** (layer 1): protocol-agnostic byte I/O — each transport implements it once.
- **Protocol handlers** (layer 2): separate modules (poll vs push vs listen), not a unified trait.
- **Payloads / events / stores / widgets / DB / adapter** are unchanged across protocols.

Each protocol input adapter **classifies** its raw mode data into a canonical
`{ primary, modifiers[] }` flight-mode model in the backend (`flightmode/`); the raw value never reaches
the widget. `helpers/flightModeRegistry.ts` is the single presentation source (id → label/category;
category → colour), consumed by the widget, track colouring, the 2D/3D maps and the replay adapter — no
per-firmware branching. A new protocol = a new adapter + a few registry ids.

## TelemetryData (widget input)

```typescript
interface TelemetryData {
    latitude; longitude; altitude;            // deg, deg, m (baro preferred, GPS fallback)
    speed; yaw; roll; pitch; vario;           // m/s, deg (COG preferred), deg, deg, m/s (+ = climb)
    voltage; current; mahDrawn; power;        // V, A, mAh, W (power derived)
    numSat; fixType;                          // count, 0=NoGPS 1=NoFix 2=2D 3=3D
    rssi; cpuLoad; armingFlags;               // bit 2 = ARMED
    flightModeFlags;                          // raw (forensic only — widget uses canonical flightMode)
    gyroStatus; accStatus; magStatus; baroStatus; gpsStatus; rangefinderStatus; pitotStatus;
    airspeed; batteryPercentage; cellCount; linkQuality;
}
```

The canonical `flightMode` (`{ primary, modifiers[] }`) is carried in its own store slot. The database
record (`TelemetryRecord`) is the snake_case, NULL-allowed, on-disk form — see the
[flight-log database reference](flightlog-database.md).

## Parallel networks (not through telemetry.ts)

These subsystems are independent of the inbound `TelemetryData` pipeline:

- **Radar — foreign-vehicle tracking (inbound, separate).** `RadarManager` (`radar/`) runs independently
  of the FC connection. Sources (ADS-B online / serial / via-MSP, and ESP32 INAV-Radar FormationFlight
  peers) feed an aggregator that merges per-system by id, applies TTL expiry, and emits throttled
  `radar-vehicles` + `radar-adsb-status`. Conflict alerts run on the ADS-B subset only.
- **RC control — outbound (uplink).** HID device → `rcEngine` / `rcManual` → `rc_stream_*` commands →
  shared `RcTxState` → the owning protocol thread streams to the FC (MSP `SET_RAW_RC` + AUX, ArduPilot
  `RC_CHANNELS_OVERRIDE`, PX4 `MANUAL_CONTROL`). Engage seeds from the FC's current channels; a frontend
  heartbeat drives a backend deadman.
- **Live recording — temp store + deferred commit.** `FlightRecorder` (one per connection) opens a
  per-session temp `.ktmp` SQLite file on arm (crash-safe); on disarm/disconnect the session becomes
  *pending* and is only committed to the main DB via the End-Flight dialog or a re-arm grace. Orphan
  `.ktmp` recovery runs on the next connect.
- **Telemetry Relay — outbound transcode.** `telemetry_forward/` taps the live decoded telemetry,
  re-encodes it into LTM / MAVLink / CRSF / SmartPort and sends it out a chosen transport
  (Serial / BLE / TCP / UDP). Persisted relay configs auto-connect on primary connect.

## File index (telemetry pipeline)

| File | Layer | Purpose |
|---|---|---|
| `src-tauri/src/scheduler/mod.rs` | Backend | MSP poll loop, event emission, recorder + RC feed |
| `src-tauri/src/scheduler/telemetry.rs` | Backend | MSP decode → normalized payload structs |
| `src-tauri/src/mavlink_proto/handler.rs` | Backend | MAVLink reader/heartbeat, decode → same events |
| `src-tauri/src/passive_telemetry/` | Backend | Listen-only detect + decode |
| `src-tauri/src/flightlog/recorder.rs` | Backend | Arm/disarm detection, temp `.ktmp`, DB writes |
| `src-tauri/src/flightlog/blackbox.rs` | Backend | Blackbox CSV parsing, unit conversion, downsampling |
| `src-tauri/src/flightlog/db.rs` | Backend | SQLite schema, migrations, CRUD |
| `src/lib/stores/telemetry.ts` | Frontend | `telemetry-*` listeners → reactive `TelemetryData` |
| `src/lib/adapters/telemetryAdapter.ts` | Frontend | DB record → `TelemetryData` (replay) |
| `src/lib/controllers/playbackController.ts` | Frontend | Timer-based playback engine |
| `src/routes/+page.svelte` | Frontend | live/replay switch, widget wiring, 2D/3D toggle |
