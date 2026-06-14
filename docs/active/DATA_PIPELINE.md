# Kite Ground Control — Data Pipeline Architecture

This document describes how telemetry data flows through the application — from source to storage to widget display — for both live and replay scenarios.

---

## Overview

```
┌──────────────────────────────────────────────────────────────────┐
│                        DATA SOURCES                              │
├──────────────┬──────────────┬──────────────┬─────────────────────┤
│  Serial/MSP  │  MAVLink     │  LTM / CRSF  │  Blackbox .TXT     │
│  (live)      │  (live)      │  (planned)    │  (import)          │
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

┌──────────────────────────────────────────────────────────────────┐
│                     Map Views (Svelte)                            │
│                                                                   │
│  Map.svelte (2D, Leaflet)  ←── mapViewMode toggle ──→            │
│  Map3D.svelte (3D, CesiumJS)                                     │
│                                                                   │
│  Both consume the SAME live TelemetryData + playback track as     │
│  the widgets — no separate data path. (Rendering specifics live   │
│  in Map3DRework.md / TerrainFeatures.md, not here.)               │
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

## 4. Multi-Protocol Architecture (MSP + MAVLink, M6 — shipped)

### Same pipeline for MSP and MAVLink (see [PROTOCOL_REFACTORING.md](../archive/PROTOCOL_REFACTORING.md))

```
                    ┌──────────────┐
                    │ Serial Port  │
                    │ TCP / UDP    │
                    │ Bluetooth    │
                    └──────┬───────┘
                           │
                    ByteTransport trait
                    (read/write/close)
                           │
              ┌────────────┼────────────┐
              │                         │
              ▼                         ▼
        ┌──────────────┐       ┌──────────────────┐
        │ MspTransport │       │  MavlinkHandler   │
        │ (framing)    │       │  (reader thread +  │
        │      +       │       │   heartbeat writer) │
        │ MspScheduler │       │                    │
        │ (poll loop)  │       │  mavlink crate     │
        └──────┬───────┘       └────────┬───────────┘
               │                        │
               ▼                        ▼
        Same normalized payloads (AttitudeData, GpsData, etc.)
               │                        │
               └────────────┬───────────┘
                            │
                    ┌───────┴────────┐
                    │  Tauri Events  │
                    │  DB Recording  │
                    │  Raw Logging   │
                    └────────────────┘
```

### Key Architecture Decisions

- **ByteTransport trait** (Layer 1): Protocol-agnostic byte I/O — all transports (Serial, TCP, UDP, BLE) implement this once
- **Protocol handlers** (Layer 2): MSP uses polling scheduler, MAVLink uses push-based reader thread — separate modules, not a unified trait
- **Protocol selection**: Explicit UI dropdown (MSP / MAVLink), no auto-detection
- **Raw recording**: a pre-parsed telemetry text log for MSP (written in parallel to the DB as a backup), standard tlog (`.tlog`) for MAVLink — crash-safe, optional continuous (pre-arm) capture
- **DB recording**: live telemetry is written to SQLite `telemetry_records` during the flight (the raw logs are a parallel backup, not the primary DB path)

### Layer impact (this refactor — shipped)

| Layer | Change | Scope |
|---|---|---|
| Transport | New `ByteTransport` trait, existing serial refactored | Medium |
| MSP Scheduler | Uses `MspTransport<ByteTransport>` instead of `Transport` | Medium |
| MAVLink Handler | New module — reader thread + heartbeat + command channel | Large |
| Payloads | Already protocol-agnostic — no change | None |
| Tauri Events | Same event names — no change | None |
| Frontend Stores | Same listeners — no change | None |
| Widgets | Same `TelemetryData` prop — no change | None |
| DB Schema | Unified — NULL where protocol doesn't provide a field | None |
| Adapter | Same `toTelemetryData()` — no change | None |

### Flight Mode — unified canonical model (shipped, ADR-044)

Flight mode is fully protocol-agnostic. Each protocol **input adapter** classifies its raw mode data
(INAV box bitmask / ArduPilot `custom_mode`) into a canonical model — the raw value never reaches the
widget. Plan: [FLIGHT_MODE_UNIFIED.md](FLIGHT_MODE_UNIFIED.md); decision: ADR-044.

```
INAV bits  ─► classify_inav ─┐
                             ├─► FlightModeState ─► "telemetry-flightmode" ─► store.flightMode ─► widget
Ardu mode  ─► classify_ardu ─┘   { primary, modifiers[] }            └─► recorder (mode_primary/modifiers)
                                       (string ids)                          └─► DB ─► replay (read directly)
```

- **Backend** (`flightmode/mod.rs`): `classify_inav` (priority-selected primary + stacked modifiers) and
  `classify_ardupilot` (per-vehicle table). Emitted by the MSP scheduler (`poll_slot`, Status payload)
  and the MAVLink handler (HEARTBEAT). Raw `flight_mode_flags` stays in `StatusData` as forensic only.
- **Frontend output registry** (`helpers/flightModeRegistry.ts`): the single presentation source —
  id → `{ label, category }`, `category → colour`. The **category** is the shared colour axis (widget
  badge + track-coloring); the **label** stays exact per mode, so equivalent modes share a colour with no
  information lost. `FlightModeWidget`, `trackColors.ts`, Map/Map3D, `LogPlayer` and the replay adapter
  all consume only this — no `fcVariant` branching anywhere.
- **Recording/replay**: `telemetry_records.mode_primary` / `mode_modifiers` (DB v12); imports classify on
  import; replay reads the canonical fields directly. Pre-v12 rows → NULL → neutral `other` category.

New protocols (CRSF / Smartport / Betaflight) = a new adapter that emits `FlightModeState` + a few
registry ids. No pipeline or widget changes.

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
| `src/lib/components/logbook/LogbookPanel.svelte` | Frontend | Button UI, multi-select, source indicators |

---

## 7. Map Views (2D / 3D)

The 2D (Leaflet) and 3D (Cesium) map views are **consumers of the same pipeline** — they have
no separate data path:

- **Live**: the GPS/attitude fields of the `TelemetryData` store (position, heading, altitude).
- **Replay**: the `playbackTrack` (`TelemetryRecord[]`) for the track polyline plus the current
  `playbackPoint` for the moving marker — driven by the same `$derived(telem)` live/replay switch
  as the widgets (see §3).

Everything beyond *what data goes in* is a **rendering concern and out of scope for this
document**: terrain + geoid correction, track colouring, UAV symbols, the chase camera, and the
shared map-tile cache. Those are documented in `Map3DRework.md` and `TerrainFeatures.md`.

---

## 8. File Index

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
| `src/routes/+page.svelte` | Frontend | Live/replay switch (`$derived(telem)`), widget wiring, 2D/3D toggle |
| `src/lib/components/Map3D.svelte` | Frontend | CesiumJS 3D globe view, chase camera, geoid correction |
| `src/lib/config/mapProviders.ts` | Frontend | Map provider registry (URLs, attribution, cesiumMaxZoom) |
| `src/lib/stores/settings.ts` | Frontend | App settings (cesiumIonToken, locale, map provider, etc.) |

---

*Last updated: 2026-06-04*
