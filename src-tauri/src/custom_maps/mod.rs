// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Custom (offline) maps from a local tile folder — desktop only.
//!
//! Layout: `<root>/<map>/<z>/<x>/<y>.<ext>` — strict XYZ (OSM order, y counted from the top, no TMS
//! flip). Each direct subfolder of the root is one map, its folder name is the label. Tiles may be
//! png / jpg / jpeg / webp (case-insensitive); the extension is detected once per map from the first
//! tile found, the others stay as a fallback.
//!
//! Tiles are read through the [`TileSource`] trait, so a different source (e.g. a single-file archive)
//! can later sit behind the same commands without touching the frontend. The coverage index (the
//! covered areas behind the frames and "Show map area") lives in [`coverage`].

pub mod coverage;

use serde::Serialize;
use std::collections::HashMap;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

/// Supported tile file extensions (compared case-insensitively).
pub const SUPPORTED_EXTS: [&str; 4] = ["png", "jpg", "jpeg", "webp"];

/// Highest zoom level we accept anywhere (folder names, tile requests). XYZ servers stop around 22-24.
pub const MAX_LEVEL: u8 = 30;

/// How many X folders of a zoom level are probed for a tile when listing the maps. Listing must stay
/// cheap (it runs at startup), so a level counts as present when one of its first X folders has a tile.
const PROBE_X_DIRS: usize = 16;

/// Default root: `custom_maps` in the app data folder — the folder the flight database uses when no
/// custom database path is set (`%APPDATA%\kite-gc` on Windows, `~/.local/share/kite-gc` on Linux,
/// `~/Library/Application Support/kite-gc` on macOS, `<exe>/data` in portable mode). A custom
/// database path does not move it.
pub fn default_root() -> PathBuf {
    let db = crate::flightlog::db::resolve_db_path("", crate::is_portable());
    match db.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.join("custom_maps"),
        _ => PathBuf::from("custom_maps"),
    }
}

/// Validate a map (folder) name: it must name exactly one direct child of the root — no separators,
/// no `.`/`..`, no drive prefix, no control characters.
pub fn sanitize_map_name(name: &str) -> Result<&str, String> {
    let bad = name.is_empty()
        || name.len() > 255
        || name == "."
        || name == ".."
        || name.chars().any(|c| matches!(c, '/' | '\\' | ':' | '\0') || c.is_control());
    if bad {
        return Err(format!("invalid custom map name: {name:?}"));
    }
    let mut comps = Path::new(name).components();
    match (comps.next(), comps.next()) {
        (Some(Component::Normal(_)), None) => Ok(name),
        _ => Err(format!("invalid custom map name: {name:?}")),
    }
}

/// Normalise a file extension to one of [`SUPPORTED_EXTS`] (case-insensitive), or `None`.
pub fn supported_ext(ext: &str) -> Option<&'static str> {
    SUPPORTED_EXTS.iter().copied().find(|e| e.eq_ignore_ascii_case(ext))
}

/// Parse a canonical decimal folder/file stem (`"12"`, not `"012"` or `"+12"`) — the XYZ layout writes
/// plain numbers, and a non-canonical name would never match the path we build for a request.
pub fn parse_index(s: &str) -> Option<u32> {
    if s.is_empty() || s.len() > 10 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if s.len() > 1 && s.starts_with('0') {
        return None;
    }
    s.parse().ok()
}

/// Parse a zoom-level folder name (`0..=MAX_LEVEL`).
pub fn parse_level(s: &str) -> Option<u8> {
    parse_index(s).filter(|z| *z <= MAX_LEVEL as u32).map(|z| z as u8)
}

/// Split a tile file name `<y>.<ext>` into the row and the extension AS WRITTEN (its case is kept, so
/// a Linux folder of `.PNG` files is read back with `.PNG`). `None` for anything else.
pub fn parse_tile_file_name(name: &str) -> Option<(u32, &str)> {
    let (stem, ext) = name.rsplit_once('.')?;
    supported_ext(ext)?;
    Some((parse_index(stem)?, ext))
}

/// Is (z, x, y) a valid XYZ tile address?
pub fn valid_tile(z: u8, x: u32, y: u32) -> bool {
    if z > MAX_LEVEL {
        return false;
    }
    let n = 1u64 << z;
    (x as u64) < n && (y as u64) < n
}

/// Extensions to try for a tile, in order: the detected one first (its case as found), then the other
/// supported ones. Windows and macOS file systems are case-insensitive by default, Linux is not — there
/// the upper-case spellings are tried as well.
pub fn ext_candidates(detected: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(d) = detected {
        out.push(d.to_string());
    }
    for e in SUPPORTED_EXTS {
        if !out.iter().any(|o| o == e) {
            out.push(e.to_string());
        }
    }
    if cfg!(target_os = "linux") {
        for e in SUPPORTED_EXTS {
            let up = e.to_ascii_uppercase();
            if !out.contains(&up) {
                out.push(up);
            }
        }
    }
    out
}

/// A source of map tiles. Today only a folder tree; an archive format would be another impl.
pub trait TileSource: Send + Sync {
    /// The tile's bytes, `Ok(None)` when it does not exist.
    fn read(&self, z: u8, x: u32, y: u32) -> io::Result<Option<Vec<u8>>>;
    /// Whether the tile exists (no read).
    fn exists(&self, z: u8, x: u32, y: u32) -> bool;
}

/// `<dir>/<z>/<x>/<y>.<ext>`.
pub struct FolderSource {
    dir: PathBuf,
    exts: Vec<String>,
}

impl FolderSource {
    pub fn new(dir: PathBuf, detected_ext: Option<&str>) -> Self {
        Self { dir, exts: ext_candidates(detected_ext) }
    }

    fn base(&self, z: u8, x: u32) -> PathBuf {
        self.dir.join(z.to_string()).join(x.to_string())
    }
}

impl TileSource for FolderSource {
    fn read(&self, z: u8, x: u32, y: u32) -> io::Result<Option<Vec<u8>>> {
        let base = self.base(z, x);
        for ext in &self.exts {
            match std::fs::read(base.join(format!("{y}.{ext}"))) {
                Ok(bytes) => return Ok(Some(bytes)),
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                // A missing X folder surfaces as NotFound too; anything else is a real I/O problem.
                Err(e) => return Err(e),
            }
        }
        Ok(None)
    }

    fn exists(&self, z: u8, x: u32, y: u32) -> bool {
        let base = self.base(z, x);
        self.exts.iter().any(|ext| base.join(format!("{y}.{ext}")).is_file())
    }
}

/// What the 2D map should draw for a tile that is not on disk at its own level.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TilePlan {
    /// `tile` = draw tile `z` (the tile itself, or its nearest existing ancestor, scaled up);
    /// `children` = compose the existing children at `z` (one level down, scaled down);
    /// `none` = nothing to draw.
    pub kind: &'static str,
    pub z: u8,
    /// For `children`: which of the four children exist, in the order (2x,2y) (2x+1,2y) (2x,2y+1)
    /// (2x+1,2y+1).
    pub children: [bool; 4],
}

/// Resolve what to draw for (z, x, y): the tile itself or its nearest existing ancestor down to
/// `min_zoom`; failing that, the children one level up (the "one more level below the lowest" rule —
/// never deeper, so at most four tiles feed one displayed tile); else nothing.
pub fn resolve_tile(src: &dyn TileSource, z: u8, x: u32, y: u32, min_zoom: u8, max_zoom: u8) -> TilePlan {
    let top = z.min(max_zoom);
    if top >= min_zoom {
        for l in (min_zoom..=top).rev() {
            let d = z - l;
            if src.exists(l, x >> d, y >> d) {
                return TilePlan { kind: "tile", z: l, children: [false; 4] };
            }
        }
    }
    if z < max_zoom {
        let (cx, cy) = (x * 2, y * 2);
        let children = [
            src.exists(z + 1, cx, cy),
            src.exists(z + 1, cx + 1, cy),
            src.exists(z + 1, cx, cy + 1),
            src.exists(z + 1, cx + 1, cy + 1),
        ];
        if children.iter().any(|c| *c) {
            return TilePlan { kind: "children", z: z + 1, children };
        }
    }
    TilePlan { kind: "none", z, children: [false; 4] }
}

/// One map as listed for the selector.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapInfo {
    pub name: String,
    pub min_zoom: u8,
    pub max_zoom: u8,
    /// Detected tile extension (lower-case), `None` if no tile was found while probing.
    pub ext: Option<String>,
}

/// A registered map: its folder, its tile source and its level range.
pub struct MapEntry {
    pub dir: PathBuf,
    pub source: Arc<dyn TileSource>,
    pub min_zoom: u8,
    pub max_zoom: u8,
}

#[derive(Default)]
struct Registry {
    maps: HashMap<String, Arc<MapEntry>>,
}

fn registry() -> &'static RwLock<Registry> {
    static REG: OnceLock<RwLock<Registry>> = OnceLock::new();
    REG.get_or_init(|| RwLock::new(Registry::default()))
}

/// Look up a listed map. Only maps found by the last [`list_maps`] can be read — together with
/// [`sanitize_map_name`] this keeps tile reads inside the configured root.
pub fn lookup(name: &str) -> Result<Arc<MapEntry>, String> {
    sanitize_map_name(name)?;
    let reg = registry().read().unwrap_or_else(|e| e.into_inner());
    reg.maps
        .get(name)
        .cloned()
        .ok_or_else(|| format!("unknown custom map: {name}"))
}

/// The first supported tile file among the first [`PROBE_X_DIRS`] X folders of a zoom-level folder —
/// returns its extension as written. Directory listings only.
fn probe_level(level_dir: &Path, z: u8) -> Option<String> {
    let n = 1u64 << z;
    let rd = std::fs::read_dir(level_dir).ok()?;
    let mut probed = 0usize;
    for entry in rd.flatten() {
        let name = entry.file_name();
        let Some(x) = name.to_str().and_then(parse_index) else { continue };
        if x as u64 >= n || !entry.path().is_dir() {
            continue;
        }
        probed += 1;
        if let Ok(files) = std::fs::read_dir(entry.path()) {
            for f in files.flatten() {
                let fname = f.file_name();
                if let Some((y, ext)) = fname.to_str().and_then(parse_tile_file_name) {
                    if (y as u64) < n {
                        return Some(ext.to_string());
                    }
                }
            }
        }
        if probed >= PROBE_X_DIRS {
            break;
        }
    }
    None
}

/// Probe one map folder: the zoom levels that hold tiles and the tile extension.
pub fn probe_map(dir: &Path) -> Option<(u8, u8, Option<String>)> {
    let mut min: Option<u8> = None;
    let mut max: Option<u8> = None;
    let mut ext: Option<String> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let name = entry.file_name();
        let Some(z) = name.to_str().and_then(parse_level) else { continue };
        if !entry.path().is_dir() {
            continue;
        }
        let Some(found) = probe_level(&entry.path(), z) else { continue };
        // The extension of the lowest level wins (deterministic regardless of listing order).
        if min.is_none_or(|m| z < m) {
            ext = Some(found);
        }
        min = Some(min.map_or(z, |m| m.min(z)));
        max = Some(max.map_or(z, |m| m.max(z)));
    }
    Some((min?, max?, ext))
}

/// Resolve the configured root (`""` = default, created if missing).
pub fn resolve_root(setting: &str) -> Result<PathBuf, String> {
    let trimmed = setting.trim();
    if trimmed.is_empty() {
        let root = default_root();
        std::fs::create_dir_all(&root)
            .map_err(|e| format!("cannot create custom map folder {}: {e}", root.display()))?;
        return Ok(root);
    }
    let root = PathBuf::from(trimmed);
    if !root.is_dir() {
        return Err(format!("custom map folder not found: {}", root.display()));
    }
    Ok(root)
}

/// List the maps under `root` and make them the registered set (replacing the previous one).
pub fn list_maps(root: &Path) -> Result<Vec<MapInfo>, String> {
    let rd = std::fs::read_dir(root).map_err(|e| format!("cannot read {}: {e}", root.display()))?;
    let mut infos: Vec<MapInfo> = Vec::new();
    let mut maps: HashMap<String, Arc<MapEntry>> = HashMap::new();
    for entry in rd.flatten() {
        let fname = entry.file_name();
        let Some(name) = fname.to_str() else { continue };
        // Dot-folders are hidden/system folders, never a map.
        if name.starts_with('.') || sanitize_map_name(name).is_err() {
            continue;
        }
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let Some((min_zoom, max_zoom, ext)) = probe_map(&dir) else { continue };
        let source: Arc<dyn TileSource> = Arc::new(FolderSource::new(dir.clone(), ext.as_deref()));
        maps.insert(
            name.to_string(),
            Arc::new(MapEntry { dir, source, min_zoom, max_zoom }),
        );
        infos.push(MapInfo {
            name: name.to_string(),
            min_zoom,
            max_zoom,
            ext: ext.map(|e| e.to_ascii_lowercase()),
        });
    }
    infos.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.name.cmp(&b.name)));
    log::info!("custom maps: {} map(s) under {}", infos.len(), root.display());
    for m in &infos {
        log::debug!(
            "custom maps: '{}' z{}-{} ext {:?}",
            m.name, m.min_zoom, m.max_zoom, m.ext
        );
    }
    registry().write().unwrap_or_else(|e| e.into_inner()).maps = maps;
    Ok(infos)
}

/// Forget all registered maps (the configured root could not be read).
pub fn clear_registry() {
    registry().write().unwrap_or_else(|e| e.into_inner()).maps.clear();
}

#[cfg(test)]
pub(crate) mod test_util {
    use std::path::{Path, PathBuf};

    /// A fresh, empty temp folder for one test.
    pub fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kite-custom-maps-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Write a tiny placeholder tile file (the scan never opens tiles, so no real image is needed).
    pub fn touch_tile(map_dir: &Path, z: u8, x: u32, y: u32, ext: &str) {
        let d = map_dir.join(z.to_string()).join(x.to_string());
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(format!("{y}.{ext}")), [z, (x & 0xff) as u8, (y & 0xff) as u8]).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::test_util::*;
    use super::*;

    #[test]
    fn map_names_are_sanitized() {
        assert!(sanitize_map_name("Airfield 2026").is_ok());
        assert!(sanitize_map_name("Karte_ä").is_ok());
        for bad in ["", ".", "..", "a/b", "a\\b", "C:", "..\\x", "../x", "a\0b", "tab\there"] {
            assert!(sanitize_map_name(bad).is_err(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn indices_are_canonical() {
        assert_eq!(parse_index("0"), Some(0));
        assert_eq!(parse_index("123"), Some(123));
        assert_eq!(parse_index("012"), None);
        assert_eq!(parse_index("+1"), None);
        assert_eq!(parse_index("-1"), None);
        assert_eq!(parse_index("1.5"), None);
        assert_eq!(parse_index(""), None);
        assert_eq!(parse_level("30"), Some(30));
        assert_eq!(parse_level("31"), None);
    }

    #[test]
    fn tile_addresses_are_validated() {
        assert!(valid_tile(0, 0, 0));
        assert!(!valid_tile(0, 1, 0));
        assert!(valid_tile(3, 7, 7));
        assert!(!valid_tile(3, 8, 0));
        assert!(!valid_tile(31, 0, 0));
    }

    #[test]
    fn tile_file_names_and_extensions() {
        assert_eq!(parse_tile_file_name("12.png"), Some((12, "png")));
        assert_eq!(parse_tile_file_name("12.JPG"), Some((12, "JPG")));
        assert_eq!(parse_tile_file_name("7.jpeg"), Some((7, "jpeg")));
        assert_eq!(parse_tile_file_name("7.WebP"), Some((7, "WebP")));
        assert_eq!(parse_tile_file_name("7.gif"), None);
        assert_eq!(parse_tile_file_name("7.png.tmp"), None);
        assert_eq!(parse_tile_file_name("x.png"), None);
        assert_eq!(parse_tile_file_name("png"), None);
        assert_eq!(supported_ext("JPEG"), Some("jpeg"));
        assert_eq!(supported_ext("tif"), None);
        // Detected extension first, the rest as fallback, no duplicates.
        let c = ext_candidates(Some("jpg"));
        assert_eq!(c[0], "jpg");
        assert!(c.iter().filter(|e| e.as_str() == "jpg").count() == 1);
        for e in SUPPORTED_EXTS {
            assert!(c.iter().any(|x| x == e));
        }
    }

    #[test]
    fn extension_is_detected_per_map_with_fallback() {
        let root = temp_root("ext");
        let a = root.join("jpgmap");
        touch_tile(&a, 5, 3, 4, "jpg");
        touch_tile(&a, 6, 6, 8, "jpg");
        // A stray png in the same map must still be found through the fallback list.
        touch_tile(&a, 6, 7, 8, "png");
        let (min, max, ext) = probe_map(&a).unwrap();
        assert_eq!((min, max, ext.as_deref()), (5, 6, Some("jpg")));
        let src = FolderSource::new(a.clone(), ext.as_deref());
        assert!(src.read(5, 3, 4).unwrap().is_some());
        assert!(src.read(6, 7, 8).unwrap().is_some());
        assert!(src.read(6, 9, 9).unwrap().is_none());
        assert!(src.exists(6, 7, 8));
        assert!(!src.exists(4, 0, 0));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn listing_registers_maps_and_skips_junk() {
        let root = temp_root("list");
        touch_tile(&root.join("Beta"), 10, 1, 1, "png");
        touch_tile(&root.join("alpha"), 2, 1, 1, "webp");
        std::fs::create_dir_all(root.join("empty/5/3")).unwrap(); // levels but no tiles
        std::fs::create_dir_all(root.join(".hidden/1/1")).unwrap();
        std::fs::write(root.join("readme.txt"), b"x").unwrap();
        let maps = list_maps(&root).unwrap();
        let names: Vec<&str> = maps.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["alpha", "Beta"]);
        assert_eq!(maps[0].ext.as_deref(), Some("webp"));
        assert!(lookup("alpha").is_ok());
        assert!(lookup("empty").is_err());
        assert!(lookup("../alpha").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_prefers_self_then_ancestor_then_children() {
        let root = temp_root("resolve");
        let m = root.join("m");
        touch_tile(&m, 10, 100, 200, "png");
        touch_tile(&m, 13, 800, 1600, "png"); // a descendant of (10,100,200)
        touch_tile(&m, 14, 2000, 2000, "png"); // an isolated deep area
        let src = FolderSource::new(m.clone(), Some("png"));
        // Exact tile.
        assert_eq!(resolve_tile(&src, 13, 800, 1600, 10, 14).kind, "tile");
        assert_eq!(resolve_tile(&src, 13, 800, 1600, 10, 14).z, 13);
        // Missing at 12 → nearest existing ancestor at 10 (two levels down, not 11).
        let p = resolve_tile(&src, 12, 401, 802, 10, 14);
        assert_eq!((p.kind, p.z), ("tile", 10));
        // Below the lowest level of an area → its children one level up, never deeper.
        let p = resolve_tile(&src, 13, 1000, 1000, 10, 14);
        assert_eq!((p.kind, p.z, p.children), ("children", 14, [true, false, false, false]));
        let p = resolve_tile(&src, 12, 500, 500, 10, 14);
        assert_eq!(p.kind, "none", "two levels above the data shows nothing");
        // Over-zoom above the top level resolves to the top tile.
        let p = resolve_tile(&src, 16, 8000, 8000, 10, 14);
        assert_eq!((p.kind, p.z), ("tile", 14));
        let _ = std::fs::remove_dir_all(&root);
    }
}
