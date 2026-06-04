# M5 Test Checklist — Flight Recording & Logbook

This checklist covers the current M5 implementation state.

## Scope (Current Implementation)

- Flight recording on ARM -> DISARM (MSP status arming flag bit 2)
- SQLite log storage with schema migrations (`user_version`)
- Configurable DB folder in Settings (default path + custom path)
- Optional raw text logs per flight
- Craft name handshake (`MSP_NAME`) in FC info
- Logbook tab with grouped sort modes
- Flight details, notes update, and delete
- Lazy metadata enrichment (reverse geocode + weather fetch)

## Preconditions

1. Build starts successfully:
   - Backend: `cargo check` (in `src-tauri`)
   - Frontend: `npm run check`
2. FC is running INAV and can ARM/DISARM safely on bench setup.
3. GPS lock available for location/weather tests (optional but recommended).

## A. Settings Tests

1. Open Settings -> Flight Logging section.
2. Verify defaults on fresh settings:
   - `Enable Flight Logging` is OFF
   - `Save Raw Text Logs` is OFF
   - DB path shows default path (or fallback text)
3. Click `Choose` and select a folder.
4. Restart app and verify selected folder is persisted.
5. Click `Use Default` and verify path resets to default.

Expected:
- Values persist in local storage.
- No crash when path is empty (default mode).

## B. Connection/Handshake Tests

1. Connect to FC.
2. Open UAV Info tab.
3. Verify craft name row is visible.

Expected:
- If FC has a craft name configured, it is shown.
- If not configured, `(not set)` / localized equivalent is shown.

## C. Recording Lifecycle Tests

1. Enable Flight Logging in settings.
2. Connect to FC.
3. Arm FC for ~10-20 seconds while telemetry is active.
4. Disarm FC.
5. Open Logbook tab and refresh.

Expected:
- A new flight entry appears.
- Duration is non-zero.
- Track point count is > 0.
- Max altitude/speed and battery usage fields have plausible values.

## D. Raw Log File Tests

1. Enable `Save Raw Text Logs`.
2. Repeat ARM -> DISARM flight.
3. Open DB folder and check `raw_logs/` subfolder.
4. Open created file.

Expected:
- One file per recorded flight exists.
- Header lines present.
- Multiple telemetry lines present with timestamp and parsed fields.

## E. DB Path Behavior Tests

1. Use custom folder and record a flight.
2. Verify `flights.db` is created in custom folder.
3. Switch back to default path and record again.

Expected:
- Separate databases are used per configured folder.
- Logbook reflects the selected DB path context.

## F. Logbook UI Tests

1. Open Logbook tab with recorded flights.
2. Cycle all 4 sort modes:
   - Aircraft -> Location -> Date
   - Location -> Date -> Aircraft
   - Date -> Location -> Aircraft
   - Aircraft -> Date -> Location
3. Select a flight entry.
4. Edit notes and save.
5. Delete selected flight.

Expected:
- Grouping changes with sort mode.
- Detail panel updates for selected flight.
- Notes persist after refresh.
- Delete removes flight and associated track data.

## G. Metadata Enrichment Tests (Network)

1. Select a flight with valid start GPS.
2. Wait for detail panel refresh.

Expected:
- Missing location may be filled via Nominatim.
- Missing weather may be filled via Open-Meteo.
- Failures should not crash UI or recording.

## H. Regression Checks

1. Telemetry widgets still update while recording enabled.
2. Connect/disconnect still works.
3. Mission tab workflows are unchanged.

Expected:
- No functional regression in M1-M4 core features.

## Known Gaps (Not Yet Implemented)

- Protocol-agnostic recorder abstraction (currently MSP-integrated)
- Animated flight path replay on map (marker moves along track — currently playback only feeds widgets)
- Collapsible group headers with aggregates
- Search/filter UI in logbook
- Export (`KML`, `GPX`, `CSV`)
- Blackbox import/archive workflow

## I. Blackbox Import Tests

1. Import a single-log .TXT Blackbox file.
2. Import a multi-log .TXT and verify log selector appears.
3. Open the imported flight in the logbook.

Expected:
- Flight appears with `source: blackbox` indicator.
- Metadata (FW version, date, GPS start) extracted from header.
- Re-importing same file triggers duplicate detection dialog.

## J. Telemetry Replay Tests (Widgets)

1. Select an imported Blackbox flight in the logbook.
2. Press Play in the playback controls.
3. Observe all HUD widgets during playback.

Expected:
- **AHI**: Roll/pitch move smoothly, values in plausible range (±30° gentle flight, ±60° acro).
- **Compass**: Heading rotates, shows cardinal directions, matches GPS COG.
- **Vario**: Shows climb/descent in m/s, positive = climbing, negative = descending.
- **Speed**: Ground speed in m/s, matches GPS speed from log.
- **Battery**: Voltage/current/mAh values from log.
- **GPS**: Satellite count and fix type shown.
- **Home Distance**: Distance and bearing from flight start position.
- **Altitude**: Shows altitude values from log.

4. Test playback controls: Pause, Resume, Reset, Scrubber seek, Speed 1×/2×/4×/10×.
5. Close player and verify widgets return to live telemetry (or zero if disconnected).

Expected:
- Home position cleared on player close.
- No stale replay data in widgets after closing.
