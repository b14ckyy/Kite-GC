# Live Recording — Temp Session Store + Capture Completeness

> Status: **In progress** (2026-06-13). Temp store + capture completeness shipped; deferred commit +
> End-Flight dialog gate + 5 s re-arm grace this round; recovery prompt on start/reconnect is the next
> phase. Schema-neutral (columns exist at `v11`); adds a per-session temp SQLite file.
> Related: [DATA_PIPELINE.md](DATA_PIPELINE.md), [FLIGHTLOG_DATABASE.md](FLIGHTLOG_DATABASE.md), ADR-040, ADR-041.

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

### Lifecycle (deferred commit — ADR-041)

The commit into the main DB is **deferred**: while the End-Flight summary dialog is open after a
disarm, **nothing** is written to `flights.db` — not even after the grace window. The temp session
is committed **only** on an explicit **Save** or when the **grace lapses and a new arm starts the
next flight**. This makes **Discard** trivial (just drop the temp) and lets an accidental disarm be
re-armed into the **same** log.

```
ARM — fresh session (no pending)
  └─ create sessions/active_<ts>.ktmp (telemetry_records + session_meta)
  └─ write session_meta (start_time, fc_info, protocol, start_lat/lon)
  └─ emit id-less "flight-recording-started"; nothing in flights.db

… during flight …
  └─ each unified sample → INSERT into the temp DB (small batches)
  └─ stats (max alt/speed/distance, total distance, start_mah) accumulated as today

DISARM
  └─ flush tail + close the temp DB (WAL checkpoint); do NOT commit, do NOT delete
  └─ build the finalized Flight (end_time, duration, stats, battery_used) and store it as the
     PENDING SESSION in app-state (temp_path, db_path, Flight, start_mah, last_ts, disarm_instant)
  └─ emit "flight-recording-ended" carrying the STATS (no flight_id yet → dialog reads the payload)

The pending session is then resolved by exactly one of:
  SAVE  (user, command)         → commit pending → main DB, return flight_id, enrich, delete temp
  re-ARM after grace (≥5 s)     → commit pending (the previous flight) → main DB, enrich, delete temp,
                                  emit "flight-recording-committed{flight_id}", then start a fresh session
  re-ARM within grace (<5 s)    → reopen the SAME .ktmp and CONTINUE the flight (timestamps continue
                                  from max(timestamp_ms)); emit "flight-recording-resumed"; no commit
  DISCARD (user, confirmed)     → delete temp; no commit
```

**Commit = atomic main-DB transaction** (same for the SAVE command and the grace-arm path): insert the
finalized `flights` row → `ATTACH` the temp `.ktmp` → `INSERT … SELECT` its `telemetry_records`
(rewriting `flight_id` to the new main id) → `COMMIT` → `DETACH` → delete the `.ktmp`. Reuses the
flight-copy approach already in [`exchange.rs`](../../src-tauri/src/flightlog/exchange.rs). The main DB
only ever sees a finished flight, never a half-flight.

### Why the pending session lives in app-state

It must survive a **disconnect while the dialog is open** (Save/Discard must still work afterwards) and
be reachable by the `flightlog_commit_pending_session` / `flightlog_discard_pending_session` Tauri
commands. So it lives in a shared `Arc<Mutex<Option<PendingSession>>>` in app-state — **not** in the
recorder (which is torn down with the connection). The recorder holds the same `Arc` and, on a new arm,
`take()`s it under the mutex for the grace decision (so the command thread and the recorder can never
both commit the same session).

### Behavioural change: `flight_id` is assigned at COMMIT, not ARM

`on_arm` no longer inserts a `flights` row; the id is born at commit (Save / grace-arm). Touch-points:

1. **Events.** `flight-recording-started` is an **id-less** signal. `flight-recording-ended` carries
   the **stats** (the dialog no longer reads a committed flight). New: `flight-recording-committed
   {flight_id}` (a pending session was auto-committed on a grace-arm → frontend links the captured
   mission + closes the dialog) and `flight-recording-resumed` (re-arm within grace → frontend just
   closes the dialog; the flight continues).
2. **Mission save+link.** The FC-synced flown mission is **captured at disarm** (waypoints + hash,
   while FC-sync still holds) and **linked at commit** (Save → the returned id; grace-arm → the
   committed event), even if FC-sync was lost meanwhile.
3. **Weather / geocode enrichment.** Runs at **commit** against the new id (same `enrich_flight_async`).
4. **DB-disabled / raw-only mode.** No temp store / no pending; the raw-log backup path is unchanged.
5. **Disconnect with an active (armed) flight** (interim): `shutdown()` finalizes the active flight as
   a **pending session** + emits `flight-recording-ended` (the dialog appears), instead of committing
   outright. A reconnect+arm within grace then continues the same `.ktmp`. The richer
   recovery-on-reconnect prompt is the next phase.

### Capture completeness (folded in)

Schema is untouched (columns exist since `v4`). Wiring only:
- `recorder.rs`: extend `TelemetrySnapshot` with `active_wp_number`, `nav_state`, `gps_hdop`,
  `hw_health_status`; add `on_nav_status()` / `on_gps_stats()` / `on_sensor_status()`; populate the
  new fields in **both** `TelemetryRecord` builders (active-flight + continuous).
- `scheduler/telemetry.rs`: add `MSP_NAV_STATUS`, `MSP_GPSSTATISTICS`, `MSP_SENSOR_STATUS` branches
  to `feed_recorder()` (decoders already exist for the event path; reuse them).
- `hw_health_status` is packed from the per-sensor `SensorStatusData` into the 2-bit-per-sensor
  layout documented in [FLIGHTLOG_DATABASE.md](FLIGHTLOG_DATABASE.md#hw_health_status).

### End-Flight summary dialog (ADR-041)

The disarm summary ([`EndFlightDialog.svelte`](../../src/lib/components/logbook/EndFlightDialog.svelte))
is the commit gate when DB recording is on:
- **Modal** — no backdrop-click / Escape dismissal (a stray click must not lose the recording).
- **Save** commits the pending session; **Discard** (with an in-dialog confirmation, irreversible)
  drops it. The old **Skip** button is gone.
- A **re-arm** while it is open force-closes it (the flight is continued within grace, or already
  auto-committed beyond grace — the backend decides).

## Out of scope (deferred — next phase)

- **Recovery prompt on start / reconnect.** Startup scan of `sessions/*.ktmp`; for an orphan (app
  was killed) show the 3-option prompt (Discard / Save Incomplete / Continue on Reconnect). The
  temp store is already self-describing for this; the prompt + the explicit "continue on reconnect"
  wait-state are the next phase. (The 5 s grace-continue across a normal disarm→re-arm is **done**
  here; the cross-disconnect reconnect prompt is not.)
- **Single-temp invariant enforcement.** There must never be more than one `.ktmp`; the deferred
  flow already removes it on commit/discard, but the startup scan that guarantees a clean slate is
  part of the recovery phase.
- **Save trigger tuning.** Batch size, fsync cadence, and whether to also flush on a timer.
- **Raw recording** (`raw_logger` / tlog) — unchanged here; it stays the parallel write-only backup.

## Fields explicitly left `NULL` for live MSP (correct, not a gap)

- `link_quality` — MSP exposes no LQ (CRSF/Blackbox only).
- `nav_lat` / `nav_lon` — always `NULL` by design (local-frame `navPos`, see FLIGHTLOG_DATABASE.md).
- `nav_alt_m` — Blackbox EKF fused altitude; live MSP altitude is already in `alt_m` / `baro_alt_m`.
- `wind_*`, `rc_data_json`, `rc_command_json`, `baro_temperature`, `state_flags` — **Blackbox-sourced**
  in the current model; whether any have a clean MSP poll worth the bandwidth is a **separate, later
  investigation** (verified against the INAV repo, version-safe), not part of this rework.
  (`gps_hdop` + `gps_eph`/`gps_epv` are now captured — they ride along in `MSP_GPSSTATISTICS`.)

## File index

| File | Change |
|---|---|
| `src-tauri/src/state.rs` | `PendingSession` + shared `Arc<Mutex<Option<PendingSession>>>` |
| `src-tauri/src/flightlog/recorder.rs` | Deferred-commit lifecycle: on_disarm → pending + ended-stats; on_arm grace (continue / commit+new); shutdown → pending; snapshot + record-builder completeness; altitude (`alt_m` = GPS MSL) |
| `src-tauri/src/flightlog/db.rs` | Temp-DB open/DDL + session_meta; attach-and-copy `commit_session_to_main` (id rewrite); `remove_temp_session` |
| `src-tauri/src/commands/flightlog.rs` | `flightlog_commit_pending_session` / `flightlog_discard_pending_session` commands |
| `src-tauri/src/scheduler/telemetry.rs` | `feed_recorder()` NAV_STATUS / GPSSTATISTICS / SENSOR_STATUS branches; GpsStatsData eph/epv |
| `src/lib/components/logbook/EndFlightDialog.svelte` | Modal; Discard + confirm; Skip removed |
| `src/routes/+page.svelte` | ended-stats → dialog; Save/Discard → commit/discard commands; committed/resumed listeners; FC-mission snapshot at disarm |
