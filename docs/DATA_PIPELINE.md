# Kite Ground Control — Data Pipeline Architecture

This document describes how telemetry data flows through the application — from source to storage to widget display — for both live and replay scenarios.

---

## Overview

```
┌──────────────────────────────────────────────────────────────────┐
│                        DATA SOURCES                              │
├──────────────┬──────────────┬──────────────┬─────────────────────┤
│  Serial/MSP  │  MAVLink     │  LTM / CRSF  │  Blackbox .TXT     │
│  (live)      │  (planned)   │  (planned)    │  (import)          │
└──────┬───────┴──────┬───────┴──────┬────────┴────────┬──────────┘
       │              │              │                 │
       ▼              ▼              ▼                 ▼
┌──────────────────────────────────────────┐  ┌──────────────────┐
│         Protocol Adapters (Rust)         │  │  blackbox_decode  │
│  MspSource  MavSource  LtmSource  Crsf   │  │  (external bin)   │
└──────────────────┬───────────────────────┘  └────────┬─────────┘
                   │                                   │
                   ▼                                   ▼
┌──────────────────────────────────────────────────────────────────┐
│              Normalized Telemetry Payloads (Rust)                 │
│  AttitudeData, GpsData, AnalogData, StatusData, AltitudeData     │
│  (protocol-agnostic structs, same regardless of source)          │
└──────────┬──────────────────────────────────┬────────────────────┘
           │                                  │
           ▼                                  ▼
┌─────────────────────┐          ┌────────────────────────────────┐
│   Tauri Events       │          │   SQLite Database               │
│   (frontend push)    │          │   flights + telemetry_records   │
│                      │          │   blackbox_records/files        │
│  telemetry-attitude  │          └───────────────┬────────────────┘
│  telemetry-gps       │                          │
│  telemetry-analog    │                          │ Replay
│  telemetry-status    │                          ▼
│  telemetry-altitude  │          ┌────────────────────────────────┐
└──────────┬───────────┘          │   telemetryAdapter.ts           │
           │                      │   toTelemetryData()             │
           ▼                      │   TelemetryRecord → TelemetryData│
┌─────────────────────┐          └───────────────┬────────────────┘
│  telemetry.ts store  │                          │
│  (TelemetryData)     │◄─────────────────────────┘
└──────────┬───────────┘
           │
           ▼
┌──────────────────────────────────────────────────────────────────┐
│                     HUD Widgets (Svelte)                          │
│  AHI  Compass  Vario  Speed  Battery  GPS  Home  RawTelemetry    │
│                                                                   │
│  All widgets take `telem: TelemetryData` as prop                 │
│  Same interface for live AND replay — widgets don't know the      │
│  difference                                                       │
└──────────────────────────────────────────────────────────────────┘

                    ┌─────────────────────────────────────┐
                    │     Data Exchange (.kflight)         │
                    │                                     │
                    │  Export: DB → .kflight SQLite file   │
                    │  Import: .kflight → DB (dedup)      │
                    │  Blackbox: BLOB → raw .TXT file     │
                    └─────────────────────────────────────┘
```

---

## 1. Live Telemetry Flow (MSP)

### Path: Serial Port → Scheduler → Events → Store → Widgets

```
Serial Port (USB / Wireless)
    │
    ▼
Scheduler Thread (Rust, dedicated std::thread)
    │  Owns SerialConnection exclusively
    │  Priority-based polling: Attitude(5) > Status(4) > Analog(3) > GPS(2) > Secondary(1)
    │
    ├── poll MSP_ATTITUDE (108)         → AttitudeData { roll, pitch, yaw }
    ├── poll MSP_RAW_GPS (106)          → GpsData { lat, lon, speed, cog, numSat, fixType }
    ├── poll MSPV2_INAV_ANALOG (0x2002) → AnalogData { voltage, current, mAh, rssi, power }
    ├── poll MSPV2_INAV_STATUS (0x2000) → StatusData { armingFlags, flightModes, cpuLoad }
    ├── poll MSP_SENSOR_STATUS (151)    → SensorData { gyro, acc, mag, baro, gps, pitot }
    └── poll MSP_ALTITUDE (109)         → AltitudeData { altitude, vario }
    │
    ├── Feed FlightRecorder (if logging enabled, on ARM/DISARM transitions)
    │   └── Batch-write to SQLite: telemetry_records table
    │
    └── Emit Tauri Events
        ├── "telemetry-attitude"  → { roll, pitch, yaw }
        ├── "telemetry-gps"       → { lat, lon, speed, heading, numSat, fixType }
        ├── "telemetry-analog"    → { voltage, current, mahDrawn, rssi }
        ├── "telemetry-status"    → { armingFlags, flightModeFlags, cpuLoad }
        ├── "telemetry-altitude"  → { altitude, vario }
        └── "telemetry-sensors"   → { gyro, acc, mag, baro, gps, rangefinder, pitot }
```

### Frontend Reception

```typescript
// telemetry.ts store — listens to Tauri events, merges into single TelemetryData object
telemetryData = {
    latitude, longitude, altitude, speed, yaw,
    roll, pitch, vario,
    voltage, current, mahDrawn, power,
    numSat, fixType, rssi,
    armingFlags, flightModeFlags, cpuLoad,
    gyroStatus, accStatus, magStatus, baroStatus, gpsStatus, ...
}

// +page.svelte — live telemetry is always available
let liveTelem = $derived(/* subscribe to telemetry store */);
```

---

## 2. Blackbox Import Flow

### Path: .TXT File → blackbox_decode → CSV → Rust Parser → SQLite

```
.TXT File (INAV Blackbox binary log)
    │
    ▼
probe_blackbox_logs()
    │  Tries --index 0..31, returns list of available logs
    │
    ▼
User selects log index (if multiple found)
    │
    ▼
Read raw file header:
    │  H looptime: 500    (µs per loop iteration)
    │  H P interval: 1/4  (log every 4th loop)
    │  → effective rate = 500µs × 4 = 2000µs = 500 Hz
    │  → keep_every = 500 / 10 = 50 (downsample to 10 Hz)
    │
    ▼
blackbox_decode --merge-gps --datetime --unit-height m --unit-gps-speed mps --stdout --index N <file>
    │  Child process, stdout captured
    │
    ▼
CSV text (dynamic columns, INAV-version-dependent)
    │
    ▼
Rust CSV Parser (blackbox.rs)
    │
    ├── Pre-build HashMap<String, usize> header index (once)
    ├── Resolve ColumnIndices struct (once):
    │     roll → ["roll", "attitude0", "attitude_roll"]
    │     pitch → ["pitch", "attitude1", "attitude_pitch"]
    │     yaw → ["yaw", "attitude2", "attitude_yaw"]
    │     heading → ["heading", "GPS_ground_course"]
    │     vario → ["gps_velned2", "vario"]
    │     ... (all field aliases)
    │
    ├── Unit conversions in parser:
    │     roll, pitch:  ALWAYS ÷10 (INAV decidegrees → degrees)
    │     yaw, heading: ÷10 only if > 360 (heuristic: may be degrees or decidegrees)
    │     vario:        gps_velned[2] → negate (NED down→up) ÷100 (cm/s → m/s)
    │                   vario fallback → ÷100 (cm/s → m/s)
    │     altitude:     baroAlt_cm → ÷100 (cm → m)
    │
    ├── Skip (keep_every − 1) rows (downsampling)
    │
    └── Per kept row:
        ├── Insert telemetry_records (lat, lon, alt, speed, heading, vario, voltage, ...)
        └── Insert blackbox_records (raw comma-joined CSV line for future analysis)
    │
    ▼
blackbox_files table: original .TXT archived as BLOB (re-downloadable)
flights table: new entry with source="blackbox", metadata from header
```

### Unit Conversion Rules

| Field | Raw Blackbox Unit | DB Unit | Conversion | Location |
|---|---|---|---|---|
| roll, pitch | decidegrees | degrees | ÷10 (unconditional) | blackbox.rs |
| yaw, heading | degrees or decidegrees | degrees | ÷10 if >360 | blackbox.rs |
| vario (gps_velned[2]) | cm/s, NED down | m/s, climb positive | negate, ÷100 | blackbox.rs |
| vario (fallback) | cm/s | m/s | ÷100 | blackbox.rs |
| altitude (baro) | cm | m | ÷100 | blackbox.rs |
| speed | m/s | m/s | none (--unit-gps-speed mps) | blackbox_decode flag |
| altitude (GPS) | m | m | none (--unit-height m) | blackbox_decode flag |

**Design principle**: Unit conversions happen in the parser (Rust), not in widgets. The DB stores values in standard SI-like units (degrees, m/s, m). Widgets may later add user-selectable display units (mph, ft, etc.) but always receive normalized data.

---

## 3. Replay Flow (Log Playback)

### Path: SQLite → PlaybackController → Adapter → Widgets

```
User selects flight in Logbook
    │
    ▼
getFlightTrack(flightId)
    │  Returns TelemetryRecord[] from telemetry_records table
    │
    ├── Set homePosition from flight.start_lat/lon
    ├── Load track into selectedFlightTrack (map polyline)
    └── Initialize PlaybackController with track data
    │
    ▼
PlaybackController (playbackController.ts)
    │  Timer-based: 100ms tick interval
    │  Speed modes: 1× (real-time), 2×, 4×, 10×
    │  Controls: Play, Pause, Reset, Seek (scrubber)
    │  Outputs: currentIndex (reactive)
    │
    ▼
playbackPoint = track[currentIndex]  (TelemetryRecord from DB)
    │
    ▼
telemetryAdapter.ts — toTelemetryData(playbackPoint)
    │
    │  Maps DB column names → TelemetryData fields:
    │    r.lat → latitude
    │    r.lon → longitude
    │    r.alt_m → altitude (baro_alt_m preferred if available)
    │    r.speed_ms → speed
    │    r.heading ?? r.yaw → yaw  (GPS COG preferred for compass)
    │    r.roll → roll
    │    r.pitch → pitch
    │    r.vario_ms → vario
    │    r.voltage → voltage
    │    r.current_a → current
    │    r.mah_drawn → mahDrawn
    │    r.voltage * r.current_a → power (derived)
    │    r.num_sat → numSat
    │    r.fix_type → fixType
    │    r.rssi → rssi
    │    r.cpu_load → cpuLoad
    │    NULL fields → 0 (safe defaults)
    │
    └── Returns TelemetryData (same type as live store)
    │
    ▼
+page.svelte — reactive switch:
    │
    │  let telem = $derived(
    │      isConnected ? liveTelem : toTelemetryData(playbackPoint)
    │  );
    │
    ▼
All widgets receive `telem` prop → identical interface for live and replay
```

### Key Design Decisions

1. **Replay always from DB** — never from raw Blackbox CSV at runtime. All data passes through the import pipeline first.

2. **Same TelemetryData type** — widgets never know if data is live or replayed. The `$derived(telem)` switch in `+page.svelte` is the only branching point.

3. **Home position from flight metadata** — during replay, `homePosition` store is set from `flight.start_lat/lon`. During live, it's set on ARM + GPS fix. Widgets use the same `homePosition` store in both cases.

4. **Adapter handles NULL** — DB records may have NULL fields (protocol didn't provide that value). The adapter maps NULL → 0 so widgets always get numbers.

---

## 4. Future: Multi-Protocol Architecture (Planned, M6)

### Goal: Same pipeline for MSP, MAVLink, LTM, CRSF

```
                    ┌──────────────┐
                    │ Serial Port  │
                    │ TCP/UDP      │
                    │ Bluetooth    │
                    └──────┬───────┘
                           │
              ┌────────────┼────────────┐
              │            │            │
              ▼            ▼            ▼
        ┌──────────┐ ┌──────────┐ ┌──────────┐
        │MspSource │ │MavSource │ │LtmSource │  ... (impl TelemetrySource)
        └─────┬────┘ └─────┬────┘ └─────┬────┘
              │            │            │
              ▼            ▼            ▼
        Same normalized payloads (AttitudeData, GpsData, etc.)
              │            │            │
              └────────────┼────────────┘
                           │
                    ┌──────┴───────┐
                    │  Scheduler   │
                    │  Thread      │
                    ├──────────────┤
                    │ Tauri Events │
                    │ DB Recording │
                    └──────────────┘
```

### TelemetrySource Trait (Planned)

```rust
trait TelemetrySource: Send {
    fn poll(&mut self) -> Vec<(String, TelemetryPayload)>;
    fn stop(&mut self);
}
```

- **MspSource**: Extracted from current `poll_slot()` — request/response, active polling
- **LtmSource**: Passive listener — LTM frames arrive continuously, no requests needed
- **MavlinkSource**: MAVLink v1/v2 heartbeat + telemetry stream (ArduPilot, PX4)
- **CrsfSource**: CRSF/ELRS telemetry frames
- **ReplaySource**: Timed playback from DB records at original rate

### What Changes

| Layer | Changes Needed | Scope |
|---|---|---|
| Scheduler | Owns `Box<dyn TelemetrySource>` instead of calling MSP directly | Medium |
| Payloads | Already protocol-agnostic — no change | None |
| Tauri Events | Same event names — no change | None |
| Frontend Stores | Same listeners — no change | None |
| Widgets | Same `TelemetryData` prop — no change | None |
| DB Schema | Unified — NULL where protocol doesn't provide a field | None |
| Adapter | Same `toTelemetryData()` — no change | None |

### Protocol Auto-Detection (Planned)

On connect: try MSP handshake → if fails, listen for MAVLink heartbeat → if fails, try LTM frame detection. First successful detection selects the `TelemetrySource` implementation.

---

## 5. Data Format Reference

### TelemetryData (Frontend — widget input)

```typescript
interface TelemetryData {
    // Position
    latitude: number;      // degrees
    longitude: number;     // degrees
    altitude: number;      // meters (baro preferred, GPS fallback)
    speed: number;         // m/s (ground speed)
    
    // Orientation
    yaw: number;           // degrees (0-360, GPS COG preferred)
    roll: number;          // degrees (-180 to +180)
    pitch: number;         // degrees (-90 to +90)
    vario: number;         // m/s (positive = climbing)
    
    // Power
    voltage: number;       // volts
    current: number;       // amps
    mahDrawn: number;      // mAh consumed
    power: number;         // watts (derived: voltage × current)
    
    // GPS
    numSat: number;        // satellite count
    fixType: number;       // 0=NO GPS, 1=NO FIX, 2=2D, 3=3D
    
    // System
    rssi: number;          // 0-255 or 0-100 depending on source
    cpuLoad: number;       // FC CPU load (0-100)
    armingFlags: number;   // bitfield (bit 2 = ARMED)
    flightModeFlags: number;
    
    // Sensor health
    gyroStatus: number;    // 0=none, 1=OK, 2=unhealthy
    accStatus: number;
    magStatus: number;
    baroStatus: number;
    gpsStatus: number;
    rangefinderStatus: number;
    pitotStatus: number;
    
    // Extended (not yet populated)
    airspeed: number;
    batteryPercentage: number;
    cellCount: number;
    linkQuality: number;
}
```

### TelemetryRecord (Database — stored format)

See [FLIGHTLOG_DATABASE.md](FLIGHTLOG_DATABASE.md) for the complete schema. Key differences from `TelemetryData`:
- Snake_case column names (`alt_m`, `speed_ms`, `vario_ms`, `current_a`, etc.)
- NULL allowed (protocol may not provide all fields)
- No derived fields (e.g. `power` is not stored, computed by adapter)
- Additional replay fields: `baro_alt_m`, `gps_hdop`, `active_wp_number`, `nav_state`, `wind_*`, `rc_data_json`, etc.

---

## 6. Data Exchange Pipeline (.kflight)

The `.kflight` format enables sharing flight data between KiteGC installations.

### Export Flow

```
User selects flights (single or Ctrl+click multi-select)
         │
         ▼
+page.svelte ─► logbookController.exportKflight()
         │
         ▼
flightlog.ts ─► invoke("flightlog_export_kflight")
         │
         ▼
commands/flightlog.rs ─► exchange::export_flights()
         │
         ├── create fresh SQLite (.kflight)
         ├── CREATE TABLE flights/telemetry_records/blackbox_records/blackbox_files
         ├── for each flight_id:
         │     ├── copy_flight() (flights row)
         │     ├── copy telemetry_records
         │     ├── copy_blackbox_records()
         │     └── copy_blackbox_files() (BLOBs)
         ├── INSERT _kflight_meta
         └── VACUUM
```

### Import Flow

```
User clicks "Import .kflight" or drag & drops file
         │
         ▼
+page.svelte ─► logbookController.importKflight()
         │
         ▼
flightlog.ts ─► invoke("flightlog_import_kflight")
         │
         ▼
commands/flightlog.rs ─► exchange::import_flights()
         │
         ├── ATTACH source .kflight as 'import_db'
         ├── list_flights_in_file() → all flights
         ├── for each flight:
         │     ├── duplicate check (craft_name + start_time ±10s)
         │     ├── skip if duplicate
         │     └── copy flight + telemetry + blackbox into main DB
         └── return (imported, skipped)
```

### Raw Blackbox Export

```
User clicks "Export Blackbox" (single flight, source = blackbox|both)
         │
         ▼
+page.svelte ─► logbookController.exportBlackbox()
         │
         ▼
flightlog.ts ─► invoke("flightlog_export_blackbox")
         │
         ▼
commands/flightlog.rs ─► db::get_blackbox_file()
         │
         ├── SELECT original_filename, file_data FROM blackbox_files
         └── std::fs::write(output_path, blob_bytes)
```

### File Index (Exchange)

| File | Layer | Purpose |
|---|---|---|
| `src-tauri/src/flightlog/exchange.rs` | Backend | .kflight export/import logic |
| `src-tauri/src/flightlog/db.rs` | Backend | `get_blackbox_file()` BLOB retrieval |
| `src-tauri/src/commands/flightlog.rs` | Backend | Tauri commands for export/import |
| `src/lib/stores/flightlog.ts` | Frontend | TS invoke wrappers |
| `src/lib/controllers/logbookController.ts` | Frontend | Export/import orchestration |
| `src/lib/components/LogbookPanel.svelte` | Frontend | Button UI, multi-select, source indicators |

---

## 7. File Index

| File | Layer | Purpose |
|---|---|---|
| `src-tauri/src/scheduler/mod.rs` | Backend | MSP polling loop, event emission, recorder feed |
| `src-tauri/src/scheduler/telemetry.rs` | Backend | MSP decode → normalized payload structs |
| `src-tauri/src/flightlog/recorder.rs` | Backend | ARM/DISARM detection, DB batch writes |
| `src-tauri/src/flightlog/blackbox.rs` | Backend | Blackbox CSV parsing, unit conversion, downsampling |
| `src-tauri/src/flightlog/db.rs` | Backend | SQLite schema, migrations, CRUD operations |
| `src-tauri/src/flightlog/exchange.rs` | Backend | .kflight export/import, flight copy logic |
| `src/lib/stores/telemetry.ts` | Frontend | Tauri event listeners → reactive TelemetryData store |
| `src/lib/adapters/telemetryAdapter.ts` | Frontend | DB TelemetryRecord → TelemetryData mapper |
| `src/lib/controllers/playbackController.ts` | Frontend | Timer-based playback engine |
| `src/lib/controllers/logbookController.ts` | Frontend | Logbook CRUD, export/import orchestration |
| `src/lib/stores/home.ts` | Frontend | Home position (set on ARM or replay start) |
| `src/routes/+page.svelte` | Frontend | Live/replay switch (`$derived(telem)`), widget wiring |

---

*Last updated: 2026-04-18*
