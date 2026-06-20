// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// RC control profile storage — user-managed, shareable config files under
// `Documents/KiteGC/HID-Profiles/<name>.json`. A profile bundles the channel assignments/methods/
// behaviour (filled by the mapping UI); the backend only does file I/O and never auto-links a profile
// to anything (the user picks the active profile + matching FC settings themselves). The profile's
// real (display) name lives inside the JSON; the filename is a sanitised form of it.

use std::fs;
use std::path::PathBuf;

/// Resolve (and create) `Documents/KiteGC/HID-Profiles`. Mirrors the flight-log DB dir logic.
fn profiles_dir() -> Result<PathBuf, String> {
    let base = dirs::document_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Documents")))
        .ok_or_else(|| "could not resolve the Documents directory".to_string())?;
    let dir = base.join("KiteGC").join("HID-Profiles");
    fs::create_dir_all(&dir).map_err(|e| format!("create profiles dir: {e}"))?;
    Ok(dir)
}

/// Map a profile name to a safe filename stem (keeps alphanumerics, space, dash, underscore).
fn sanitize(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let s = s.trim().to_string();
    if s.is_empty() { "profile".to_string() } else { s }
}

/// Absolute path of the profiles directory (for display in the UI).
pub fn dir_path() -> Result<String, String> {
    Ok(profiles_dir()?.to_string_lossy().to_string())
}

/// Raw JSON text of every `*.json` profile in the directory (the frontend parses them).
pub fn list() -> Result<Vec<String>, String> {
    let dir = profiles_dir()?;
    let mut out = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|e| format!("read profiles dir: {e}"))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            match fs::read_to_string(&path) {
                Ok(txt) => out.push(txt),
                Err(e) => eprintln!("[hid] skip unreadable profile {path:?}: {e}"),
            }
        }
    }
    Ok(out)
}

/// Write a profile (overwrites a profile with the same sanitised name).
pub fn save(name: &str, json: &str) -> Result<(), String> {
    let file = profiles_dir()?.join(format!("{}.json", sanitize(name)));
    fs::write(&file, json).map_err(|e| format!("write profile: {e}"))
}

/// Delete a profile file (no error if it's already gone).
pub fn delete(name: &str) -> Result<(), String> {
    let file = profiles_dir()?.join(format!("{}.json", sanitize(name)));
    if file.exists() {
        fs::remove_file(&file).map_err(|e| format!("delete profile: {e}"))?;
    }
    Ok(())
}
