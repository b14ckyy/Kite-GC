// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Hi-res replay cache (Dev-Docs active/HIRES_REPLAY.md): re-parse a flight's archived original
//! log at FULL temporal resolution into a disposable per-flight SQLite file under
//! `<db_dir>/hires-cache/`. The 10 Hz track in the main DB stays the replay timeline (scrubber,
//! map, playback index); these rows only feed the instrument values, sampled by timestamp
//! (`db::get_hires_sample`). Cache files are always reproducible from the archived blob — they are
//! wiped at app start (crash leftovers) and dropped when the flight is deselected.

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde::Serialize;

use super::db;

/// Result of a hi-res parse, handed to the frontend to switch the sample source over.
#[derive(Serialize)]
pub struct HiresParseOutcome {
    pub cache_path: String,
    pub rows: usize,
    pub size_bytes: u64,
    /// Effective sample rate of the full-resolution rows (Hz), for display.
    pub rate_hz: f64,
}

/// Deterministic per-flight cache file, so toggling hi-res off/on can reuse an existing parse.
pub fn cache_path_for(cache_dir: &Path, flight_id: i64) -> PathBuf {
    cache_dir.join(format!("hires_{flight_id}.db"))
}

/// Which archived originals the hi-res parse can decode. `.tlog`/`.rawmsp` flights are excluded by
/// nature — they are already stored at their native rate, and no blob is archived for them anyway.
pub fn supported_extension(filename: &str) -> bool {
    matches!(
        Path::new(filename)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("bbl" | "bfl" | "txt" | "bin" | "ulg")
    )
}

/// Wipe the hi-res cache directory (app start: files left by a crash are worthless — every cache is
/// reproducible on demand). Only removes files, never subdirectories.
pub fn cleanup_cache_dir(cache_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(cache_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            if let Err(e) = std::fs::remove_file(&path) {
                log::warn!("Failed to remove hi-res cache file {}: {}", path.display(), e);
            }
        }
    }
}

/// Re-parse the flight's archived log at full rate into `<cache_dir>/hires_<flight_id>.db`.
/// `get_blackbox_file` transparently falls back to the linked partner flight, so a REC flight of a
/// linked pair yields the BBX blob — matching the player, which only offers hi-res on the blackbox
/// track. Rows are written with `flight_id` 0 (the cache has no `flights` table).
pub fn parse_to_cache<F>(
    main_conn: &Connection,
    flight_id: i64,
    cache_dir: &Path,
    mut report: F,
) -> Result<HiresParseOutcome, String>
where
    F: FnMut(u8, &str, &str),
{
    report(2, "hires-load", "Loading archived log...");
    let (filename, blob) = db::get_blackbox_file(main_conn, flight_id)
        .map_err(|e| format!("Failed to read archived log: {}", e))?
        .ok_or_else(|| "This flight has no archived original log".to_string())?;
    if !supported_extension(&filename) {
        return Err(format!("Hi-res parse does not support '{}'", filename));
    }
    let ext = Path::new(&filename)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();

    std::fs::create_dir_all(cache_dir)
        .map_err(|e| format!("Failed to create hi-res cache dir: {}", e))?;

    report(10, "hires-decode", "Decoding log at full resolution...");
    let mut rows = match ext.as_str() {
        "bin" => super::ardupilot::hires_telemetry_rows(&blob)?,
        "ulg" => super::ulog::hires_telemetry_rows(&blob)?,
        // INAV blackbox (.bbl/.bfl/.txt): blackbox_decode needs a real file → spill the blob.
        _ => {
            let scratch = cache_dir.join(format!("hires_{flight_id}.src.tmp"));
            super::blackbox::hires_telemetry_rows(&blob, &scratch)?
        }
    };
    if rows.is_empty() {
        return Err("Hi-res parse produced no rows".into());
    }
    for r in &mut rows {
        r.flight_id = 0;
    }

    report(70, "hires-store", "Writing hi-res cache...");
    let cache_path = cache_path_for(cache_dir, flight_id);
    db::remove_temp_session(&cache_path); // replace any stale cache atomically enough — it's disposable
    let conn = db::open_temp_session(&cache_path)
        .map_err(|e| format!("Failed to create hi-res cache DB: {}", e))?;
    // The file is disposable and rebuilt on demand — trade durability for bulk-insert speed.
    conn.execute_batch("PRAGMA journal_mode = MEMORY; PRAGMA synchronous = OFF;")
        .map_err(|e| format!("Failed to configure hi-res cache DB: {}", e))?;
    db::insert_telemetry_batch(&conn, &rows)
        .map_err(|e| format!("Failed to write hi-res cache: {}", e))?;
    drop(conn);

    let size_bytes = std::fs::metadata(&cache_path).map(|m| m.len()).unwrap_or(0);
    let span_ms = rows.last().map(|r| r.timestamp_ms).unwrap_or(0)
        - rows.first().map(|r| r.timestamp_ms).unwrap_or(0);
    let rate_hz = if span_ms > 0 {
        rows.len() as f64 / (span_ms as f64 / 1000.0)
    } else {
        0.0
    };
    log::info!(
        "Hi-res cache for flight {}: {} rows ({:.0} Hz, {} bytes) at {}",
        flight_id,
        rows.len(),
        rate_hz,
        size_bytes,
        cache_path.display()
    );

    report(100, "done", "Hi-res cache ready.");
    Ok(HiresParseOutcome {
        cache_path: cache_path.to_string_lossy().to_string(),
        rows: rows.len(),
        size_bytes,
        rate_hz,
    })
}
