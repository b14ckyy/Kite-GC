# Kite Ground Control — Data Pipeline Architecture

This document describes how data flows through the application — for the **inbound telemetry** path
(source → store → widget, live + replay) and the **parallel data networks** that don't go through the
telemetry store (Radar, outbound RC control, Telemetry Relay, live recording). It is a living reference;
keep it in sync with the code.

> Reference doc (in `docs/reference/`). Architecture decisions live in [../ARCHITECTURE.md](../ARCHITECTURE.md)
> (ADR-010 multi-protocol, ADR-044 unified flight mode, ADR-040/041/042 recording, ADR-051 relay).

---

## Overview

```
                         INBOUND  (FC → GCS)                              IMPORT
 ┌───────────────┬───────────────┬───────────────────────────┐   ┌──────────────────┐
 │  Serial/MSP   │  MAVLink      │  Passive telemetry        │   │  Blackbox .TXT   │
 │  (INAV, poll) │  (Ardu/PX4)   │  (listen-only: SmartPort/ │   │  .rawmsp / .tlog │
 │               │  (push)       │   CRSF/LTM/MAVLink, auto) │   │  (import)        │
 └──────┬────────┴──────┬────────┴──────────────┬────────────┘   └────────┬─────────┘
        │ ByteTransport (Serial/TCP/UDP/BLE)     │                         │
        ▼               ▼                        ▼                         ▼
 ┌──────────────┐ ┌──────────────┐ ┌──────────────────────┐      ┌──────────────────┐
 │ MspScheduler │ │ MavlinkHandler│ │ PassiveHandler        │      │ blackbox_decode  │
 │ (poll loop)  │ │ (reader+HB)   │ │ (detect + decode)     │      │ / raw parser     │
 └──────┬───────┘ └──────┬────────┘ └──────────┬───────────┘      └────────┬─────────┘
        └────────────────┴─────── same normalized payloads ──────┐         │
                 AttitudeData / GpsData / AnalogData / StatusData │         │
                 AltitudeData / SensorStatusData / AirspeedData   │         │
                          │                                       │         │
          ┌───────────────┼───────────────────────┐              │         ▼
          ▼               ▼                        ▼              │   SQLite DB
   Tauri "telemetry-*"  FlightRecorder        Telemetry Relay     │   flights +
   events (frontend)    (DB + temp .ktmp)     (re-encode, ADR-051)│   telemetry_records
          │                                                       │         │
          ▼                                                       │  Replay │
   telemetry.ts store (TelemetryData) ◄── telemetryAdapter.ts ◄───┴─────────┘
          │
          ▼
   HUD widgets + Map (2D Leaflet / 3D Cesium)   ── same TelemetryData for live AND replay

 PARALLEL networks (NOT through telemetry.ts):
   • Radar  : RadarManager + sources → "radar-vehicles" / "radar-adsb-status" → radar stores/map
   • RC out : HID → rcEngine/rcManual → rc_stream_* → RcTxState → scheduler/handler → FC (uplink)
```

---

## 1. Live Telemetry Flow

Three live protocols share **one** decode-to-event contract. Each owns its link on a dedicated thread
(ADR-010) and emits the **same** protocol-agnostic payload structs + Tauri events; the frontend never
branches on protocol. Selected explicitly in the connect UI — `ActiveProtocol::{Msp, Mavlink,
PassiveTelemetry}` (`state.rs`).

| Protocol | Module | Model | Notes |
|---|---|---|---|
| **MSP** (INAV) | `scheduler/` | request→response **poll loop** | priority polling; owns serial exclusively |
| **MAVLink** (ArduPilot/PX4) | `mavlink_proto/` | **push** reader + 1 Hz GCS heartbeat | stream rates requested at connect (ADR-043) |
| **Passive telemetry** | `passive_telemetry/` | **listen-only**, sub-protocol auto-detected | SmartPort / CRSF / LTM / MAVLink-passive; emits `telemetry-protocol` once locked, `telemetry-fc-link` for FC-origin liveness |

### MSP path (example): Serial → Scheduler → Events → Store → Widgets

```
Scheduler thread (dedicated std::thread) — owns the link
  Priority poll: Attitude(5) > Status(4) > Analog(3) > GPS(2) > Secondary(1)
    MSP_ATTITUDE (108)          → AttitudeData
    MSP_RAW_GPS (106)           → GpsData
    MSPV2_INAV_ANALOG (0x2002)  → AnalogData
    MSPV2_INAV_STATUS (0x2000)  → StatusData (+ classify_inav → flight mode)
    MSP_SENSOR_STATUS (151)     → SensorStatusData
    MSP_ALTITUDE (109)          → AltitudeData
  ├─ Feed FlightRecorder (when logging; temp .ktmp + DB — see §9.3)
  ├─ Inject outbound RC between polls (RC control — see §9.2)
  └─ Emit Tauri "telemetry-*" events
```

### Tauri telemetry events (authoritative — emitted by all live protocols where applicable)

| Event | Payload | Notes |
|---|---|---|
| `telemetry-attitude` | roll, pitch, yaw | |
| `telemetry-gps` | lat, lon, alt_msl, ground_speed, course, fix_type, num_sat | |
| `telemetry-gps-stats` | hdop | |
| `telemetry-altitude` | altitude (rel), vario | |
| `telemetry-alt-ref` | ground-MSL anchor (relative-only protocols) | |
| `telemetry-analog` | voltage, current, power, mah_drawn, battery_percentage, rssi, cell_count | |
| `telemetry-status` | arming_flags, flight_mode_flags, cpu_load, sensor_status, **msp_rc_override** | |
| `telemetry-sensor-status` | gyro, acc, mag, baro, gps, rangefinder, pitot, opflow, prearm | (note: event is `-sensor-status`) |
| `telemetry-airspeed` | airspeed | now populated (MSP2_INAV_AIR_SPEED / VFR_HUD) |
| `telemetry-flightmode` | canonical `{ primary, modifiers[] }` | classified in the adapter (ADR-044, §4) |
| `telemetry-nav-status` | active_wp_number, nav_state | mission active-WP highlight |
| `telemetry-ekf-status` / `telemetry-ekf-type` | EKF health / EKF2-3 | MAVLink |
| `telemetry-linkstats` | rssi/lq/snr (normalized) | RC-link widget |
| `telemetry-vehicle` | quadplane flag, … | MAVLink vehicle traits |
| `telemetry-rc-channels` | current RC channel µs | **RC-control engage seed** (MSP_RC / MAVLink RC_CHANNELS) |
| `home-position` | lat, lon, alt | authoritative FC home (ADR-039) |
| `telemetry-protocol` / `telemetry-fc-link` | sub-protocol name / FC-origin liveness | passive telemetry |
| `telemetry-disconnected` / `mavlink-disconnected` / `connection-lost` | — | teardown signals |

### Frontend reception

`stores/telemetry.ts` listens to the `telemetry-*` events and merges them into one reactive
`TelemetryData` object (see §5). `+page.svelte` subscribes; widgets receive it as a prop.

---

## 2. Blackbox / raw-log Import Flow

### Path: .TXT/.rawmsp/.tlog → decode/parse → SQLite

```
INAV Blackbox .TXT ─► blackbox_decode (external bin) ─► CSV ─► blackbox.rs parser ─┐
.rawmsp (MWP v2) / .tlog (MAVLink) ─► flightlog_import_raw parser ─────────────────┤
                                                                                   ▼
                                          downsample → telemetry_records (+ blackbox_records BLOB)
                                          split by arm/disarm → one flight per arm (ADR-049)
```

Blackbox specifics (looptime/interval → downsample, column-alias resolution, unit conversions in Rust)
are unchanged — see the table below. Raw `.rawmsp`/`.tlog` import (ADR-049) shares the same
`telemetry_records` sink and splits a session into flights on arm/disarm edges.

### Unit Conversion Rules (Blackbox parser)

| Field | Raw Blackbox Unit | DB Unit | Conversion | Location |
|---|---|---|---|---|
| roll, pitch, yaw | decidegrees (`attitude[*]`) | degrees (f64, 0.1°) | ÷10 (unconditional) | blackbox.rs |
| heading (COG) | degrees (`gps_ground_course`) | degrees (f64) | as-is (÷10 fallback if >360) | blackbox.rs |
| vario (gps_velned[2]) | cm/s, NED down | m/s, climb positive | negate, ÷100 | blackbox.rs |
| altitude (baro) | cm | m | ÷100 | blackbox.rs |
| speed | m/s | m/s | none (`--unit-gps-speed mps`) | decode flag |

**Design principle**: unit conversions happen in the parser (Rust); the DB stores SI-like units. Widgets
may add display-unit conversion but always receive normalized data.

---

## 3. Replay Flow (Log Playback)

```
Logbook select → getFlightTrack(flightId) → TelemetryRecord[] (telemetry_records)
  ├─ homePosition from flight.start_lat/lon
  ├─ load track polyline
  └─ PlaybackController (100 ms tick; 1×/2×/4×/10×; play/pause/seek) → currentIndex
        │
        ▼  playbackPoint = track[currentIndex]
   telemetryAdapter.toTelemetryData(playbackPoint)   // DB snake_case → TelemetryData; NULL → safe default
        │                                            // canonical mode_primary/mode_modifiers read directly
        ▼
   +page.svelte:  telem = $derived(isConnected ? liveTelem : toTelemetryData(playbackPoint))
        ▼
   widgets + map — identical interface for live and replay
```

### Key design decisions

1. **Replay always from DB** — never from raw CSV at runtime; all data passes through import first.
2. **Same `TelemetryData` type** — widgets never know live vs replay; the `$derived(telem)` switch is the
   only branching point.
3. **Home from flight metadata** on replay; set on ARM + GPS fix when live (ADR-039) — same `home` store.
4. **Adapter handles NULL** → 0 so widgets always get numbers.

---

## 4. Multi-Protocol + unified flight mode (ADR-010, ADR-044)

### Layered transport (ADR-010)

```
Serial / TCP / UDP / BLE ─► ByteTransport trait (read/write/close/set_read_timeout)
   ├─ MspTransport + MspScheduler   (poll loop)            → same normalized payloads
   ├─ MavlinkHandler                (push reader + HB)      → same normalized payloads
   └─ PassiveHandler                (detect + decode)       → same normalized payloads
                                                               │
                              same "telemetry-*" events · same DB recording · same raw logging
```

- **ByteTransport** (Layer 1): protocol-agnostic byte I/O — every transport implements it once.
- **Protocol handlers** (Layer 2): separate modules (poll vs push vs listen), **not** a unified trait.
- **Selection**: explicit connect UI (MSP / MAVLink / Telemetry-passive). Passive auto-detects its
  sub-protocol and announces it via `telemetry-protocol`.
- **Payloads / events / stores / widgets / DB / adapter**: unchanged across protocols (NULL where a
  protocol doesn't provide a field).

### Flight mode — unified canonical model (ADR-044)

```
INAV bits  ─► classify_inav ─┐
                             ├─► FlightModeState ─► "telemetry-flightmode" ─► store.flightMode ─► widget
Ardu mode  ─► classify_ardu ─┘   { primary, modifiers[] }            └─► recorder (mode_primary/modifiers)
                                       (string ids)                          └─► DB ─► replay (read directly)
```

Each protocol input adapter classifies its raw mode data into the canonical model (the raw value never
reaches the widget). `flightmode/mod.rs` does the classification; `helpers/flightModeRegistry.ts` is the
single presentation source (id → label/category; category → colour) consumed by the widget, track
colouring, Map/Map3D and the replay adapter — no `fcVariant` branching. Recording/replay use
`telemetry_records.mode_primary` / `mode_modifiers` (DB v12). Plan archived in
[../archive/FLIGHT_MODE_UNIFIED.md](../archive/FLIGHT_MODE_UNIFIED.md). New protocols = a new adapter +
a few registry ids.

---

## 5. Data Format Reference

### TelemetryData (frontend — widget input)

```typescript
interface TelemetryData {
    latitude; longitude; altitude;            // deg, deg, m (baro preferred, GPS fallback)
    speed; yaw; roll; pitch; vario;           // m/s, deg(0-360 COG preferred), deg, deg, m/s(+=climb)
    voltage; current; mahDrawn; power;        // V, A, mAh, W (power derived)
    numSat; fixType;                          // count, 0=NoGPS 1=NoFix 2=2D 3=3D
    rssi; cpuLoad; armingFlags;               // bit 2 = ARMED
    flightModeFlags;                          // raw (forensic only — widget uses canonical flightMode)
    mspRcOverride;                            // INAV MSP-RC-OVERRIDE box active
    gyroStatus; accStatus; magStatus; baroStatus; gpsStatus; rangefinderStatus; pitotStatus;
    airspeed; batteryPercentage; cellCount; linkQuality;   // now populated where the protocol provides them
}
```

Plus the canonical `flightMode` (`{ primary, modifiers[] }`) carried in its own store slot.

### TelemetryRecord (database — stored format)

See [FLIGHTLOG_DATABASE.md](FLIGHTLOG_DATABASE.md) for the schema. Differences from `TelemetryData`:
snake_case columns; NULL allowed; no derived fields (`power` computed by the adapter); extra replay fields
(`baro_alt_m`, `gps_hdop`, `active_wp_number`, `nav_state`, `mode_primary`, `mode_modifiers`, `wind_*`,
`rc_data_json`, `utc_offset_min`, …).

---

## 6. Data Exchange Pipeline (.kflight)

`.kflight` is a self-contained SQLite file for sharing flights between installations.

- **Export** (`exchange::export_flights`): fresh SQLite → create tables → per flight copy
  flights/telemetry_records/blackbox_records + blackbox_files BLOBs → `_kflight_meta` → VACUUM.
- **Import** (`exchange::import_flights`): ATTACH source → list → per flight dedup
  (craft_name + start_time ±10 s) → copy non-duplicates into the main DB.
- **Raw Blackbox export**: `db::get_blackbox_file()` → write the archived BLOB back to a `.TXT`.

Orchestrated by `controllers/logbookController.ts` ↔ `commands/flightlog.rs`.

---

## 7. Map Views (2D / 3D)

The 2D (Leaflet) and 3D (Cesium) maps are **consumers of the same pipeline** — no separate data path:
live = the GPS/attitude fields of `TelemetryData`; replay = `playbackTrack` + `playbackPoint` via the
same `$derived(telem)` switch (§3). Rendering specifics (terrain/geoid, track colouring, symbols, chase
camera, tile cache) are out of scope here — see [../archive/Map3DRework.md](../archive/Map3DRework.md)
and [../archive/TerrainFeatures.md](../archive/TerrainFeatures.md). The map also draws the **Radar** and
**Airspace** overlays, which have their own data paths (§9.1).

---

## 8. File Index (telemetry pipeline)

| File | Layer | Purpose |
|---|---|---|
| `src-tauri/src/scheduler/mod.rs` | Backend | MSP poll loop, event emission, recorder + RC feed |
| `src-tauri/src/scheduler/telemetry.rs` | Backend | MSP decode → normalized payload structs |
| `src-tauri/src/mavlink_proto/handler.rs` | Backend | MAVLink reader/heartbeat, decode → same events, RC stream |
| `src-tauri/src/passive_telemetry/` | Backend | Listen-only detect + decode (SmartPort/CRSF/LTM/MAVLink) |
| `src-tauri/src/flightlog/recorder.rs` | Backend | Arm/disarm detection, temp `.ktmp`, DB batch writes |
| `src-tauri/src/flightlog/blackbox.rs` | Backend | Blackbox CSV parsing, unit conversion, downsampling |
| `src-tauri/src/flightlog/db.rs` | Backend | SQLite schema, migrations, CRUD |
| `src-tauri/src/flightlog/exchange.rs` | Backend | `.kflight` export/import |
| `src/lib/stores/telemetry.ts` | Frontend | `telemetry-*` listeners → reactive `TelemetryData` |
| `src/lib/adapters/telemetryAdapter.ts` | Frontend | DB `TelemetryRecord` → `TelemetryData` (replay) |
| `src/lib/controllers/playbackController.ts` | Frontend | Timer-based playback engine |
| `src/routes/+page.svelte` | Frontend | live/replay switch, widget wiring, 2D/3D toggle |

---

## 9. Parallel data networks (not through telemetry.ts)

These subsystems are independent of the inbound `TelemetryData` pipeline.

### 9.1 Radar — foreign-vehicle tracking (inbound, separate)

`RadarManager` (`radar/`) runs independently of the FC `protocol` lifecycle. Sources feed an aggregator
thread over an ingest channel:

- **ADS-B** — online provider, serial MAVLink receiver, and ADS-B-from-the-UAV via MSP (INAV 8.0+).
- **FormationFlight** — ESP32 INAV-Radar mesh peers over a serial MSP link (ADR-036).

The aggregator merges per-system by id, applies TTL expiry, and emits (throttled) **`radar-vehicles`**
(consolidated snapshot) + **`radar-adsb-status`** (per-source status). The frontend radar stores + the
map/3D overlay consume these. Conflict alerts (ADR-035) run on the ADS-B subset only. Archived plan:
[../archive/RADAR_TRACKING_CORE.md](../archive/RADAR_TRACKING_CORE.md). (The static **Airspace** overlay,
ADR-038, is a separate on-demand aeronautical-data fetch — not a live stream.)

### 9.2 RC control — outbound (uplink) injection

The RC path is **outbound** and independent of inbound telemetry (RC over MSP archived in
`../archive/MSP_RC_CONTROL.md`; MAVLink in [../archive/MAVLINK_RC_CONTROL.md](../archive/MAVLINK_RC_CONTROL.md)):

```
HID device → stores/hid.ts → rcEngine (channel methods) / rcManual (PX4 4-axis)
   → rc_stream_{update,set_aux,set_override,set_manual,enable,set_rate}  (Tauri commands)
   → shared RcTxState (state.rs, independent of `protocol`)
   → owning protocol thread streams to the FC:
        INAV/MSP   : MSP_SET_RAW_RC + MSP2_INAV_SET_AUX_RC      (woven into the poll loop)
        ArduPilot  : RC_CHANNELS_OVERRIDE (#70)                 (MAVLink handler)
        PX4        : MANUAL_CONTROL (#69)                       (MAVLink handler)
```

Engage **seeds** from the FC's current channels (`telemetry-rc-channels` / one-shot `MSP_RC`) so there's
no jump. A frontend heartbeat drives a backend **deadman**; a 2 s link-speed probe emits `rc-link-slow`.

### 9.3 Live recording — temp store + deferred commit (ADR-040/041/042)

`FlightRecorder` is fed by all three live protocols (one instance per connection). On arm it opens a
per-session temp `.ktmp` SQLite file (crash-safe); on disarm/disconnect the session becomes **pending**
in app-state and is committed to the main DB only via the End-Flight dialog or a re-arm grace
(`flight-recording-{started,ended,committed,interrupted,resumed}` events; commands
`flightlog_{commit,discard,continue}_pending_session`). Orphan `.ktmp` recovery on next connect.
Archived plan: `../archive/LIVE_RECORDING_TEMP_STORE.md`.

### 9.4 Telemetry Relay — outbound transcode (ADR-051)

`telemetry_forward/` taps the live decoded telemetry (a self-event tap on the same payloads), re-encodes
it into **LTM / MAVLink / CRSF / SmartPort**, and sends it out a chosen transport (Serial / BLE / TCP /
UDP). Persisted relay configs auto-connect on primary connect; live rate via `relay-stats`. Archived
plan: `../archive/TELEMETRY_FORWARDING.md`.

---

*Last updated: 2026-06-21 — multi-protocol (MSP/MAVLink/passive), unified flight mode, Radar, RC control,
live-recording temp store and Telemetry Relay reflected.*
