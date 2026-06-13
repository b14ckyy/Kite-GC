# Live Recording — Temp Session Store + Capture Completeness

> Status: **Planned** (2026-06-13). Backend recorder rework. Schema-neutral for the telemetry
> field set (columns already exist at `v11`); adds a separate per-session temp SQLite file.
> Related: [DATA_PIPELINE.md](DATA_PIPELINE.md), [FLIGHTLOG_DATABASE.md](FLIGHTLOG_DATABASE.md), ADR-040.

## Why

Two independent problems with how a **live flight** is recorded today:

1. **The main DB is written mid-flight.** `recorder.rs` inserts the `flights` row on the
   disarmed→armed edge and batch-flushes `telemetry_records` every 50 samples **directly into
   `flights.db`**. An app crash mid-flight therefore leaves a **half-written, non-finalized flight**
   in the main database (rows up to the last batch present, `end_time`/stats `NULL`), and the
   in-flight stream **cannot be resumed**. WAL prevents corruption, but not the half-flight pollution
   or the lost-session problem.

2. **The captured record is incomplete.** The unified `TelemetryRecord` already carries the full
   replay field set, but the live recorder hard-codes several columns to `None` even though the FC
   exposes the data **and we already poll it**:
   - `active_wp_number` + `nav_state` — `MSP_NAV_STATUS` is polled (Status slot) and emitted to the
     frontend, but `feed_recorder()` has no NAV_STATUS branch and the recorder has no
     `on_nav_status()`. → A live-recorded mission shows **no active-WP tracking on replay**, even
     though live tracking now works. (`active_flight_mode_flags` *is* fed, via the Status slot.)
   - `gps_hdop` — `MSP_GPSSTATISTICS` is polled + emitted, not fed to the recorder.
   - `hw_health_status` — `MSP_SENSOR_STATUS` is polled + emitted (sensor health), not packed/fed.

Both are fixed together so the new temp store carries the **complete** unified record from day one.

## Design

The protocol-independent unit already exists: the recorder builds `TelemetryRecord` from the
normalized Rust payloads (MSP **and** MAVLink converge here — the recorder does not know the
protocol at that point). So the temp store sits at the `TelemetryRecord` level and is inherently
protocol-agnostic.

### Temp session store = a separate SQLite file

- **One file per armed session**, alongside the main DB:
  `<db_dir>/sessions/active_<YYYY-MM-DD_HHMMSS>.ktmp`.
- Same `telemetry_records` DDL as the main DB (WAL, `synchronous = NORMAL`), **plus** a
  `session_meta` table so the file is **self-describing** for recovery (start time, `FcInfo`
  fields, protocol, start lat/lon). No external state is needed to interpret an orphaned temp file.
- The temp DB is the **durable buffer** — not memory. Samples are still grouped into small batches
  (≈10–25) purely for write efficiency; the point is that committed batches survive a crash.

### Lifecycle (this phase)

```
ARM (disarmed→armed edge, live)
  └─ create sessions/active_<ts>.ktmp  (telemetry_records + session_meta)
  └─ write session_meta row (start_time, fc_info, protocol, start_lat/lon)
  └─ NOTE: nothing is written to the main flights.db yet  ← behavioural change

… during flight …
  └─ each unified sample → INSERT into the temp DB (small batches)
  └─ statistics (max alt/speed/distance, total distance, start_mah) accumulated in memory as today

DISARM (armed→disarmed edge)
  └─ main DB transaction:
        INSERT finalized flights row (end_time, duration, stats, battery_used)
        ATTACH the temp .ktmp
        INSERT INTO telemetry_records SELECT … (rewriting flight_id to the new main id)
        COMMIT  → DETACH → delete the .ktmp
  └─ the main DB sees the flight only as a finished whole — never as a half-flight
```

The copy-with-id-rewrite step reuses the flight-copy machinery already in
[`exchange.rs`](../../src-tauri/src/flightlog/exchange.rs) (the `.kflight` export copies
`telemetry_records` for a flight the same way).

### Behavioural change: `flight_id` is assigned at DISARM, not ARM

Today `on_arm` inserts the main `flights` row immediately to obtain a `flight_id`, which is carried
in the `flight-recording-started` event. With deferred insert the real id only exists at disarm.
Three downstream touch-points adjust:

1. **Lifecycle events.** `flight-recording-started` becomes an **id-less** "recording started"
   signal; mission save+link moves to **`flight-recording-ended`** (which already fires at disarm
   with the id). The exact frontend usage in `+page.svelte` / `logbookController` is verified before
   changing the event payload, so the flown-mission link still lands on the right flight.
2. **Weather / geocode enrichment.** Currently spawned at arm against the main `flight_id`. Moves to
   **disarm**, against the newly inserted id (same `enrich_flight_async`, just reordered).
3. **DB-disabled / raw-only mode.** When `db_enabled` is false there is no main flight to commit to;
   the temp store is skipped (or kept only as the raw-log backup path — unchanged from today, since
   raw logging is explicitly out of scope here).

### Capture completeness (folded in)

Schema is untouched (columns exist since `v4`). Wiring only:
- `recorder.rs`: extend `TelemetrySnapshot` with `active_wp_number`, `nav_state`, `gps_hdop`,
  `hw_health_status`; add `on_nav_status()` / `on_gps_stats()` / `on_sensor_status()`; populate the
  new fields in **both** `TelemetryRecord` builders (active-flight + continuous).
- `scheduler/telemetry.rs`: add `MSP_NAV_STATUS`, `MSP_GPSSTATISTICS`, `MSP_SENSOR_STATUS` branches
  to `feed_recorder()` (decoders already exist for the event path; reuse them).
- `hw_health_status` is packed from the per-sensor `SensorStatusData` into the 2-bit-per-sensor
  layout documented in [FLIGHTLOG_DATABASE.md](FLIGHTLOG_DATABASE.md#hw_health_status).

## Out of scope (deferred — next phase)

Agreed to handle after the core store lands:

- **Crash recovery / resume.** Startup scan of `sessions/*.ktmp` → offer to recover an unfinished
  flight from `session_meta` (finalize with `end_time` = last sample). The self-describing temp file
  is designed for this, but the recovery UI/flow is deferred.
- **Reconnect during an active flight.** Continue appending to the **same** `.ktmp` session on
  reconnect instead of starting a new flight (the `.ktmp` is the session anchor). Needs the
  arm-edge / reconnect interplay (see ADR-039) worked through.
- **Save trigger tuning.** Batch size, fsync cadence, and whether to also flush on a timer.
- **Raw recording** (`raw_logger` / tlog) — unchanged here; it stays the parallel write-only backup.

## Fields explicitly left `NULL` for live MSP (correct, not a gap)

- `link_quality` — MSP exposes no LQ (CRSF/Blackbox only).
- `nav_lat` / `nav_lon` — always `NULL` by design (local-frame `navPos`, see FLIGHTLOG_DATABASE.md).
- `nav_alt_m` — Blackbox EKF fused altitude; live MSP altitude is already in `alt_m` / `baro_alt_m`.
- `wind_*`, `rc_data_json`, `rc_command_json`, `gps_eph`/`gps_epv`, `baro_temperature`, `state_flags`
  — **Blackbox-sourced** in the current model; whether any have a clean MSP poll worth the bandwidth
  is a **separate, later investigation** (verified against the INAV repo, version-safe), not part of
  this rework.

## File index

| File | Change |
|---|---|
| `src-tauri/src/flightlog/recorder.rs` | Temp-store lifecycle (create on arm, commit+delete on disarm); deferred main-DB insert; snapshot + record-builder completeness |
| `src-tauri/src/flightlog/db.rs` | Temp-DB open/DDL helpers; attach-and-copy commit into main DB (id rewrite) |
| `src-tauri/src/flightlog/types.rs` | (only if a `session_meta` struct is warranted) |
| `src-tauri/src/scheduler/telemetry.rs` | `feed_recorder()` NAV_STATUS / GPSSTATISTICS / SENSOR_STATUS branches |
| `src/routes/+page.svelte`, `logbookController.ts` | Move flown-mission link to the `ended` event if the id-less `started` change requires it |
