# ArduPilot DataFlash Import — Architecture Plan

This document describes the native Rust-based ArduPilot `.bin` import pipeline
for Kite Ground Control.  No external binaries, no Python dependency.

---

## Design Principle

The ArduPilot import reuses the **same pipeline as Blackbox/MSP** from the
normalization step onward:

```
.bin file
    │
    ▼
DataFlash binary parser  ← NEW (this document)
    │
    ▼
NormalizedRecord stream  ← same concept as MSP telemetry payloads
    │
    ├──── CSV export (Step 2 — verification / debug output)
    │
    └──── TelemetryRecord + Flight builder
              │
              ▼
          SQLite DB  ←─── same tables, same DB insert path as Blackbox
```

Only the **first stage** differs (binary DataFlash parser instead of an
external decode binary).  Everything from normalization onward is identical
to the existing pipeline.

---

## Implementation Phases

### Phase 1 — DataFlash decoder → CSV  ✅ (this PR)

File: `src-tauri/src/flightlog/ardupilot.rs`

- `DataFlashScanner` — low-level binary reader, self-describing via FMT records
- `decode_to_normalized_csv()` — full decode → merged NormalizedRecord → CSV
- `probe_message_types()` — inventory of FMT records in the file (debug aid)
- Tauri commands: `flightlog_probe_ardupilot`, `flightlog_decode_ardupilot_csv`

### Phase 2 — Normalized data → DB  (next step)

- Map `NormalizedRecord` → `TelemetryRecord` + `Flight`
- Follow identical pattern to `blackbox.rs::import_blackbox_log_with_progress()`
- Add DB migration v6 for new ArduPilot-specific columns
- Add `flightlog_import_ardupilot` Tauri command

### Phase 3 — Frontend integration  (after Phase 2 verified)

- Add ArduPilot import button in LogbookPanel
- `importArduPilot()` controller method
- i18n keys (en/de)
- No widget changes needed — all widgets consume the same `TelemetryData`

---

## DataFlash .bin Format

### Message framing
```
[0xA3] [0x95] [type_id] [data bytes...]
```
- 3-byte header per message
- `length` for each type is registered by FMT records (total incl. header)

### FMT record (type_id = 128 / 0x80)
Total length: 89 bytes (3 header + 86 data)

| Offset | Size | Field   | Description                        |
|--------|------|---------|------------------------------------|
| 3      | 1    | type_id | The type being registered          |
| 4      | 1    | length  | Total record length in bytes       |
| 5      | 4    | name    | ASCII name (null-padded)           |
| 9      | 16   | format  | Format string (see below)          |
| 25     | 64   | labels  | Comma-separated column names       |

### Format string characters

| Char | Type      | Width | Notes                              |
|------|-----------|-------|------------------------------------|
| `b`  | i8        | 1     |                                    |
| `B`  | u8        | 1     |                                    |
| `M`  | u8        | 1     | Flight mode (same as B)            |
| `h`  | i16       | 2     |                                    |
| `H`  | u16       | 2     |                                    |
| `c`  | i16×0.01  | 2     | Divide by 100 → real value         |
| `C`  | u16×0.01  | 2     | Divide by 100 → real value         |
| `i`  | i32       | 4     |                                    |
| `I`  | u32       | 4     |                                    |
| `e`  | i32×0.01  | 4     | Divide by 100 → real value         |
| `E`  | u32×0.01  | 4     | Divide by 100 → real value         |
| `L`  | i32 LatLon| 4     | Divide by 1e7 → decimal degrees    |
| `f`  | f32       | 4     |                                    |
| `d`  | f64       | 8     |                                    |
| `q`  | i64       | 8     |                                    |
| `Q`  | u64       | 8     | TimeUS (microseconds since boot)   |
| `n`  | char[4]   | 4     | Short string                       |
| `N`  | char[16]  | 16    | Medium string                      |
| `Z`  | char[64]  | 64    | Long string (MSG.Message, VER.FWS) |
| `a`  | i16[32]   | 64    | Integer array (RC data etc.)       |

---

## Message Types Decoded

| Type   | Fields used                                      | Maps to                                  |
|--------|--------------------------------------------------|------------------------------------------|
| `GPS`  | TimeUS, Status, GWk, GMS, NSats, HDop, Lat, Lng, Alt, Spd, GCrs, VZ | lat, lon, alt_m, speed_ms, heading, fix_type, num_sat, hdop, gps_vz |
| `ATT`  | TimeUS, Roll, Pitch, Yaw                        | roll, pitch, yaw                         |
| `BAT`  | TimeUS, Volt, Curr, CurrTot                     | voltage, current_a, mah_drawn            |
| `CURR` | TimeUS, Volt, Curr, CurrTot                     | voltage, current_a, mah_drawn (fallback) |
| `BARO` | TimeUS, Alt, CRt, Temp                          | baro_alt_m, baro_climb_rate, baro_temp   |
| `CTUN` | TimeUS, BAlt, CRt                               | baro_alt_m fallback, climb rate fallback |
| `MODE` | TimeUS, Mode                                    | custom_mode                              |
| `EV`   | TimeUS, Id (10=ARM, 11=DISARM)                  | armed flag                               |
| `MSG`  | TimeUS, Message                                 | vehicle_type, fw_version (from string)   |
| `VER`  | TimeUS, FWS                                     | fw_version                               |
| `PARM` | TimeUS, Name, Value                             | stored for future parameter display      |
| `FMT`  | (all fields)                                    | type registry — mandatory                |

---

## NormalizedRecord → TelemetryRecord field mapping

| NormalizedRecord field  | TelemetryRecord field         | Notes                             |
|-------------------------|-------------------------------|-----------------------------------|
| timestamp_us (relative) | timestamp_ms                  | (timestamp_us - boot_us) / 1000   |
| lat                     | lat                           | from GPS.Lat                      |
| lon                     | lon                           | from GPS.Lng                      |
| gps_alt_m               | alt_m                         | GPS altitude                      |
| baro_alt_m              | baro_alt_m                    | BARO.Alt or CTUN.BAlt             |
| speed_ms                | speed_ms                      | GPS.Spd (m/s ground speed)        |
| ground_course_deg       | heading                       | GPS.GCrs (degrees)                |
| roll_deg                | roll                          | ATT.Roll                          |
| pitch_deg               | pitch                         | ATT.Pitch                         |
| yaw_deg                 | yaw                           | ATT.Yaw                           |
| voltage_v               | voltage                       | BAT.Volt                          |
| current_a               | current_a                     | BAT.Curr                          |
| mah_drawn               | mah_drawn                     | BAT.CurrTot                       |
| baro_climb_rate_ms      | vario_ms                      | BARO.CRt (best vario source)      |
| gps_vz_ms               | vario_ms (fallback)           | GPS.VZ if no BARO                 |
| hdop                    | gps_hdop                      | GPS.HDop                          |
| num_sat                 | num_sat                       | GPS.NSats                         |
| fix_type                | fix_type                      | GPS.Status                        |
| custom_mode             | active_flight_mode_flags      | Mode integer (ArduPilot-specific) |
| baro_temp_c             | baro_temperature              | BARO.Temp                         |

### Fields not yet mapped (Phase 2 additions if needed)
- `custom_mode` integer: currently stored in `active_flight_mode_flags` (reuse)
- Vehicle type (Copter/Plane): stored in `flights.fc_variant`
- `base_mode` (MAVLink concept): not present in DataFlash, skip for now

---

## DB Schema — No new columns required for Phase 2

All needed fields already exist in `telemetry_records` (schema v5).

`active_flight_mode_flags` is reused for ArduPilot's mode integer.
`fc_variant` on `flights` carries vehicle type ("ArduCopter", "ArduPlane", etc.).
`source` on `flights` = `"ardupilot"`, `protocol` = `"DATAFLASH"`.

If a dedicated `custom_mode` INTEGER column is desired later, it can be added
as migration v6 without breaking existing data.

---

## GPS Time to UTC

```
GPS epoch:     1980-01-06 00:00:00 UTC  (Unix: 315964800)
Leap seconds:  18 (valid since 2017-01-01; ArduPilot corrects GPS internally
               from GPS week 1929+)

UTC = GPS_epoch_unix_ms + GWk × 604800000 + GMS − 18000
```

Time base in TelemetryRecord:
```
timestamp_ms = (TimeUS − first_valid_gps_TimeUS) / 1000
```

---

## Merge Strategy

The GPS message drives the output cadence (typically 5 Hz):
- When a valid GPS record is received → emit one NormalizedRecord
- All other message types update rolling state (latest-value semantics)
- State carried: ATT (attitude), BAT (battery), BARO (barometric), MODE, armed

**Fallback** (GPS-denied flight): if no GPS fix throughout, use BARO altitude
ticks instead — not implemented in Phase 1, documented for Phase 3.

---

## Duplicate Detection

Same strategy as Blackbox import:
- Key: `(craft_name, start_time)`
- `craft_name` from PARM `SYSID_THISMAV` or filename fallback
- `start_time` = UTC of first valid GPS fix

---

## File: `src-tauri/src/flightlog/ardupilot.rs`

```
pub fn probe_message_types(file_data: &[u8]) -> Vec<FmtDef>
    → Scan only FMT records, return type registry for inspection

pub fn decode_to_normalized_csv(file_data: &[u8], out_path: &Path) -> Result<DecodeStats, String>
    → Full decode pipeline → writes CSV → returns stats

pub struct DecodeStats {
    total_records, gps_rows,
    vehicle_type, fw_version,
    first_fix_time, last_fix_time,
    arm_count, disarm_count,
    message_type_counts: HashMap<String, usize>,
}
```
