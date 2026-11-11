// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Coverage index of a custom map: per zoom level the tile count and the footprint (stored only as a
//! count of merged tile-range rectangles), plus the covered AREAS derived from the footprints. An area
//! is found top-down (see [`areas`]) and carries its highest tile level `top`, its footprint outline
//! and a tight box. The 2D map frames every area (a ×4 finder box while that fits the view, the real
//! outline beyond), "Show map area" fits the view to its box.
//!
//! The scan reads directory listings only and never opens a tile file. Its result is stored as a small
//! JSON index inside the map's folder (`.kite-coverage.json`; in the app data folder when the map
//! folder is not writable) and reused while the folder signature — the zoom and X folders with their
//! modification times — is unchanged. Adding or removing tiles touches the X folder's mtime, adding a
//! column touches the zoom folder's, so both invalidate the index; the Settings row has a Rescan too.
//!
//! Every per-level footprint is capped at [`CELL_LIMIT`] cells: a level with more tiles is coarsened
//! (each cell stands for a 2^k x 2^k block of tiles) until it fits. Everything derived from it is an
//! outline aid, so that loss of detail is fine — the tiles themselves are never decided from it.

use super::{parse_index, parse_tile_file_name, MAX_LEVEL};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

/// Index file name inside a map folder.
pub const INDEX_FILE: &str = ".kite-coverage.json";
/// Bump when the index layout or the derivation changes — an old index is then rebuilt.
pub const INDEX_VERSION: u32 = 4;
/// Max cells per level footprint (a 128 x 128 block worth).
pub const CELL_LIMIT: usize = 16_384;
/// Max outline segments of one area — beyond it the area's outline is coarsened.
pub const AREA_SEG_LIMIT: usize = 200;
/// Max areas kept (frames + "Show map area").
pub const AREA_LIMIT: usize = 64;

type Cell = (u32, u32);
type CellSet = HashSet<Cell>;

/// All tiles of one zoom level: column x → sorted, de-duplicated rows.
#[derive(Debug, Default)]
pub struct LevelTiles {
    pub z: u8,
    pub columns: BTreeMap<u32, Vec<u32>>,
    pub count: u64,
}

/// One covered area (see [`areas`]).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Area {
    /// Highest tile level in this area (diagnostics; the areas are found top-down by it).
    pub top: u8,
    /// Tight box `[west, south, east, north]` ("Show map area").
    pub bounds: [f64; 4],
    /// Footprint outline as merged segments `[lat0, lon0, lat1, lon1]`, at most [`AREA_SEG_LIMIT`].
    pub segments: Vec<[f64; 4]>,
}

/// One level of the index: tile count, footprint grid level (= `z` when uncoarsened) and the number
/// of merged tile-range rectangles of that footprint (Debug Monitor).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LevelSummary {
    pub z: u8,
    pub tiles: u64,
    pub grid: u8,
    pub rects: usize,
}

/// What the frontend gets: level range, areas, plus diagnostics.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Coverage {
    pub min_zoom: Option<u8>,
    pub max_zoom: Option<u8>,
    pub tile_count: u64,
    /// Covered areas, highest `top` first.
    pub areas: Vec<Area>,
    /// True when [`AREA_LIMIT`] cut off further areas.
    pub areas_truncated: bool,
    pub levels: Vec<LevelSummary>,
    /// Duration of the scan that built this coverage.
    pub scan_ms: u64,
}

/// [`Coverage`] plus where it came from — what the command returns.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageReport {
    #[serde(flatten)]
    pub coverage: Coverage,
    /// Index file read or written (`None` if it could not be stored anywhere).
    pub index_path: Option<String>,
    /// True when the stored index was still valid (no scan ran).
    pub index_hit: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct IndexFile {
    version: u32,
    signature: String,
    coverage: Coverage,
}

// ── Scan ──────────────────────────────────────────────────────────────────────────────

fn is_dir_entry(entry: &std::fs::DirEntry) -> bool {
    match entry.file_type() {
        Ok(t) if t.is_dir() => true,
        Ok(t) if t.is_symlink() => entry.path().is_dir(),
        _ => false,
    }
}

/// Numeric child folders of `dir` (canonical names only), sorted.
fn numeric_dirs(dir: &Path) -> io::Result<Vec<(u32, std::fs::DirEntry)>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)?.flatten() {
        let name = entry.file_name();
        let Some(n) = name.to_str().and_then(parse_index) else { continue };
        if is_dir_entry(&entry) {
            out.push((n, entry));
        }
    }
    out.sort_by_key(|(n, _)| *n);
    Ok(out)
}

/// Scan a map folder: every tile per level, from directory listings only.
pub fn scan_levels(map_dir: &Path) -> io::Result<Vec<LevelTiles>> {
    let mut levels = Vec::new();
    for (zn, zentry) in numeric_dirs(map_dir)? {
        if zn > MAX_LEVEL as u32 {
            continue;
        }
        let z = zn as u8;
        let n = 1u64 << z;
        let mut lt = LevelTiles { z, ..Default::default() };
        for (x, xentry) in numeric_dirs(&zentry.path())? {
            if x as u64 >= n {
                continue;
            }
            let Ok(rd) = std::fs::read_dir(xentry.path()) else { continue };
            let mut ys: Vec<u32> = rd
                .flatten()
                .filter_map(|f| {
                    let name = f.file_name();
                    name.to_str().and_then(parse_tile_file_name).map(|(y, _)| y)
                })
                .filter(|y| (*y as u64) < n)
                .collect();
            if ys.is_empty() {
                continue;
            }
            ys.sort_unstable();
            ys.dedup(); // 5.png + 5.jpg is one tile
            lt.count += ys.len() as u64;
            lt.columns.insert(x, ys);
        }
        if lt.count > 0 {
            levels.push(lt);
        }
    }
    Ok(levels)
}

// ── Cell-set helpers ──────────────────────────────────────────────────────────────────

fn project(cells: &CellSet, k: u8) -> CellSet {
    if k == 0 {
        return cells.clone();
    }
    cells.iter().map(|&(x, y)| (x >> k, y >> k)).collect()
}

fn project_columns(columns: &BTreeMap<u32, Vec<u32>>, k: u8) -> CellSet {
    let mut out = CellSet::new();
    for (&x, ys) in columns {
        let px = x >> k;
        let mut last: Option<u32> = None;
        for &y in ys {
            let py = y >> k;
            if last != Some(py) {
                out.insert((px, py));
                last = Some(py);
            }
        }
    }
    out
}

/// The level's footprint at the finest grid that fits [`CELL_LIMIT`]: (grid level, cells).
pub fn footprint(level: &LevelTiles) -> (u8, CellSet) {
    let mut k: u8 = 0;
    // Lower bound: projecting by k divides the count by at most 4^k.
    while k < level.z && (level.count >> (2 * k as u32)) > CELL_LIMIT as u64 {
        k += 1;
    }
    let mut cells = project_columns(&level.columns, k);
    while cells.len() > CELL_LIMIT && k < level.z {
        cells = project(&cells, 1);
        k += 1;
    }
    (level.z - k, cells)
}

/// Merge cells into rectangles `[x0, y0, x1, y1]` (inclusive): vertical runs per column, then equal
/// runs of neighbouring columns. Exact — the rectangles cover precisely the cells.
pub fn rects_from_cells(cells: &CellSet) -> Vec<[u32; 4]> {
    let mut cols: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for &(x, y) in cells {
        cols.entry(x).or_default().push(y);
    }
    let mut out: Vec<[u32; 4]> = Vec::new();
    // run (y0, y1) → (x0, last x)
    let mut open: HashMap<(u32, u32), (u32, u32)> = HashMap::new();
    for (x, mut ys) in cols {
        ys.sort_unstable();
        let mut runs: Vec<(u32, u32)> = Vec::new();
        for y in ys {
            match runs.last_mut() {
                Some(r) if r.1 + 1 == y => r.1 = y,
                _ => runs.push((y, y)),
            }
        }
        let mut next: HashMap<(u32, u32), (u32, u32)> = HashMap::new();
        for run in runs {
            match open.remove(&run) {
                Some((x0, xl)) if xl + 1 == x => {
                    next.insert(run, (x0, x));
                }
                Some((x0, xl)) => {
                    out.push([x0, run.0, xl, run.1]);
                    next.insert(run, (x, x));
                }
                None => {
                    next.insert(run, (x, x));
                }
            }
        }
        for (run, (x0, xl)) in open.drain() {
            out.push([x0, run.0, xl, run.1]);
        }
        open = next;
    }
    for (run, (x0, xl)) in open {
        out.push([x0, run.0, xl, run.1]);
    }
    out.sort_unstable();
    out
}

#[cfg(test)]
fn cells_from_rects(rects: &[[u32; 4]]) -> CellSet {
    let mut out = CellSet::new();
    for r in rects {
        for x in r[0]..=r[2] {
            for y in r[1]..=r[3] {
                out.insert((x, y));
            }
        }
    }
    out
}

/// Boundary of a cell set as merged axis-parallel segments `[x0, y0, x1, y1]` in tile-corner
/// coordinates of its grid (a cell (x, y) spans corners (x, y)..(x+1, y+1)).
pub fn boundary_segments(cells: &CellSet) -> Vec<[u32; 4]> {
    let mut hor: BTreeMap<u32, Vec<u32>> = BTreeMap::new(); // line y → unit edges starting at x
    let mut ver: BTreeMap<u32, Vec<u32>> = BTreeMap::new(); // line x → unit edges starting at y
    for &(x, y) in cells {
        if y == 0 || !cells.contains(&(x, y - 1)) {
            hor.entry(y).or_default().push(x);
        }
        if !cells.contains(&(x, y + 1)) {
            hor.entry(y + 1).or_default().push(x);
        }
        if x == 0 || !cells.contains(&(x - 1, y)) {
            ver.entry(x).or_default().push(y);
        }
        if !cells.contains(&(x + 1, y)) {
            ver.entry(x + 1).or_default().push(y);
        }
    }
    let mut out = Vec::new();
    let mut merge = |line: u32, mut starts: Vec<u32>, horizontal: bool| {
        starts.sort_unstable();
        starts.dedup();
        let mut i = 0;
        while i < starts.len() {
            let a = starts[i];
            let mut b = a + 1;
            i += 1;
            while i < starts.len() && starts[i] == b {
                b += 1;
                i += 1;
            }
            out.push(if horizontal { [a, line, b, line] } else { [line, a, line, b] });
        }
    };
    for (y, xs) in hor {
        merge(y, xs, true);
    }
    for (x, ys) in ver {
        merge(x, ys, false);
    }
    out
}

/// Tile corner (x, y) at grid level `level` → (lat, lon) in degrees (Web Mercator).
pub fn corner_to_latlon(x: u32, y: u32, level: u8) -> (f64, f64) {
    let n = (1u64 << level) as f64;
    let lon = x as f64 / n * 360.0 - 180.0;
    let lat = (std::f64::consts::PI * (1.0 - 2.0 * y as f64 / n)).sinh().atan().to_degrees();
    (lat, lon)
}

/// `[west, south, east, north]` of an inclusive cell rectangle at `level`.
fn cell_rect_bounds(x0: u32, y0: u32, x1: u32, y1: u32, level: u8) -> [f64; 4] {
    let (north, west) = corner_to_latlon(x0, y0, level);
    let (south, east) = corner_to_latlon(x1 + 1, y1 + 1, level);
    [west, south, east, north]
}

/// One level as used by the derivations: (tile level, grid level, cells).
struct Footprint {
    z: u8,
    level: u8,
    cells: CellSet,
}

/// 8-connected components of a cell set with their inclusive bounding box, largest first.
fn components(all: &CellSet) -> Vec<(CellSet, [u32; 4])> {
    let mut seen = CellSet::new();
    let mut comps: Vec<(CellSet, [u32; 4])> = Vec::new();
    let mut start: Vec<Cell> = all.iter().copied().collect();
    start.sort_unstable();
    for s in start {
        if !seen.insert(s) {
            continue;
        }
        let mut q = VecDeque::from([s]);
        let mut cells = CellSet::new();
        let (mut x0, mut y0, mut x1, mut y1) = (s.0, s.1, s.0, s.1);
        while let Some((x, y)) = q.pop_front() {
            cells.insert((x, y));
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
            for dx in -1i64..=1 {
                for dy in -1i64..=1 {
                    let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                    if nx < 0 || ny < 0 {
                        continue;
                    }
                    let n = (nx as u32, ny as u32);
                    if all.contains(&n) && seen.insert(n) {
                        q.push_back(n);
                    }
                }
            }
        }
        comps.push((cells, [x0, y0, x1, y1]));
    }
    comps.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.1.cmp(&b.1)));
    comps
}

/// Outline of a cell set at grid `level` as lat/lon segments, coarsened until at most
/// [`AREA_SEG_LIMIT`] segments remain.
fn outline_segments(cells: &CellSet, level: u8) -> Vec<[f64; 4]> {
    let mut w = level;
    let mut h = cells.clone();
    let mut segs = boundary_segments(&h);
    while segs.len() > AREA_SEG_LIMIT && w > 0 {
        h = project(&h, 1);
        w -= 1;
        segs = boundary_segments(&h);
    }
    segs.into_iter()
        .map(|s| {
            let (lat0, lon0) = corner_to_latlon(s[0], s[1], w);
            let (lat1, lon1) = corner_to_latlon(s[2], s[3], w);
            [lat0, lon0, lat1, lon1]
        })
        .collect()
}

/// The covered areas, found top-down: the highest level's connected components are areas with
/// `top` = that level; each lower level adds areas only from its footprint OUTSIDE the areas found so
/// far (`top` = that lower level). A pyramid with a low overview (z0–18 for one airfield) therefore
/// yields ONE area — the airfield with top 18 — not the world, and a sparse set with separate patches
/// keeps one area per patch with its own top. A lower-level component whose box lies completely
/// inside the box of a higher area AND that touches that area (see [`touches`]) is a hole or an edge
/// notch in the area's top level (a downloader that skipped blank tiles) and is dropped — no frame
/// inside a frame. A detached field that merely sits inside the box of, say, a diagonal corridor keeps
/// its own area. Bounded by [`AREA_LIMIT`] (the flag reports a cut); highest `top` first.
fn areas(fps: &[Footprint]) -> (Vec<Area>, bool) {
    let mut order: Vec<&Footprint> = fps.iter().collect();
    order.sort_by(|a, b| b.z.cmp(&a.z));
    // Footprint cells of the areas found so far, with their grid level.
    let mut claimed: Vec<(u8, CellSet)> = Vec::new();
    // Normalised boxes of the areas found so far, with their top.
    let mut boxes: Vec<(u8, [f64; 4])> = Vec::new();
    let mut out: Vec<Area> = Vec::new();
    let mut truncated = false;
    for f in order {
        if truncated {
            break;
        }
        let g = f.level;
        // The claimed cells as seen from this level's grid: finer grids projected down onto it,
        // coarser ones tested by ancestor (a cell inside a coarse claimed cell counts as claimed).
        let mut finer = CellSet::new();
        let mut coarser: Vec<(u8, &CellSet)> = Vec::new();
        for (cg, cells) in &claimed {
            if *cg >= g {
                finer.extend(project(cells, cg - g));
            } else {
                coarser.push((*cg, cells));
            }
        }
        let rest: CellSet = f
            .cells
            .iter()
            .copied()
            .filter(|&(x, y)| {
                !finer.contains(&(x, y))
                    && !coarser.iter().any(|(cg, cs)| cs.contains(&(x >> (g - cg), y >> (g - cg))))
            })
            .collect();
        for (cells, b) in components(&rest) {
            let nb = norm_box(b, g);
            // `boxes[i]` and `claimed[i]` belong to the same area (pushed together below).
            let in_hole = boxes.iter().zip(claimed.iter()).any(|((top, ab), (ag, acells))| {
                *top > f.z
                    && nb[0] >= ab[0]
                    && nb[1] >= ab[1]
                    && nb[2] <= ab[2]
                    && nb[3] <= ab[3]
                    && touches(&cells, g, *ag, acells)
            });
            if in_hole {
                continue;
            }
            if out.len() >= AREA_LIMIT {
                truncated = true;
                break;
            }
            out.push(Area {
                top: f.z,
                bounds: cell_rect_bounds(b[0], b[1], b[2], b[3], g),
                segments: outline_segments(&cells, g),
            });
            boxes.push((f.z, nb));
            claimed.push((g, cells));
        }
    }
    (out, truncated)
}

/// Does a component (cells on grid `g`) touch an area (cells on grid `area_grid`)? True when one of
/// its cells has an 8-neighbour on grid `g` that the area covers — a finer area is projected down onto
/// `g`, a coarser one is tested by ancestor.
fn touches(comp: &CellSet, g: u8, area_grid: u8, area_cells: &CellSet) -> bool {
    let view = (area_grid >= g).then(|| project(area_cells, area_grid - g));
    let covered = |x: i64, y: i64| -> bool {
        if x < 0 || y < 0 {
            return false;
        }
        let (x, y) = (x as u32, y as u32);
        match &view {
            Some(v) => v.contains(&(x, y)),
            None => {
                let k = g - area_grid;
                area_cells.contains(&(x >> k, y >> k))
            }
        }
    };
    comp.iter().any(|&(x, y)| {
        (-1i64..=1).any(|dx| (-1i64..=1).any(|dy| covered(x as i64 + dx, y as i64 + dy)))
    })
}

/// Inclusive cell box at grid `level` → normalised Web-Mercator box `[x0, y0, x1, y1]` in 0..1, so
/// boxes of different grids compare directly (exact in f64 up to level 30).
fn norm_box(b: [u32; 4], level: u8) -> [f64; 4] {
    let n = (1u64 << level) as f64;
    [b[0] as f64 / n, b[1] as f64 / n, (b[2] as f64 + 1.0) / n, (b[3] as f64 + 1.0) / n]
}

/// Build the coverage (index content + what the frontend gets) from a scan.
pub fn build(levels: &[LevelTiles]) -> Coverage {
    let mut fps = Vec::new();
    let mut summaries = Vec::new();
    for lt in levels {
        let (level, cells) = footprint(lt);
        summaries.push(LevelSummary { z: lt.z, tiles: lt.count, grid: level, rects: rects_from_cells(&cells).len() });
        fps.push(Footprint { z: lt.z, level, cells });
    }
    let (areas, areas_truncated) = areas(&fps);
    Coverage {
        min_zoom: levels.iter().map(|l| l.z).min(),
        max_zoom: levels.iter().map(|l| l.z).max(),
        tile_count: levels.iter().map(|l| l.count).sum(),
        areas,
        areas_truncated,
        levels: summaries,
        scan_ms: 0,
    }
}

// ── Signature + index file ────────────────────────────────────────────────────────────

/// FNV-1a — stable across builds (std's `DefaultHasher` is not guaranteed to be).
struct Fnv(u64);
impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= *b as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

/// A folder's mtime, from a fresh `stat` of its path — NOT the `DirEntry` metadata: on Windows that
/// comes from the parent's directory listing (FindNextFile), which can lag behind the folder itself.
fn mtime_nanos(path: &Path) -> u128 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// Folder signature: every zoom folder and every X folder with its mtime. Listing + one stat per
/// folder (no tile files) — far cheaper than the scan, which is what makes the index worth keeping.
/// A file system that does not update folder mtimes when files change (some network shares) cannot
/// be tracked this way; Settings → Rescan is the way there.
pub fn signature(map_dir: &Path) -> io::Result<String> {
    let mut h = Fnv::new();
    for (z, zentry) in numeric_dirs(map_dir)? {
        let zpath = zentry.path();
        h.write(b"z");
        h.write(&z.to_le_bytes());
        h.write(&mtime_nanos(&zpath).to_le_bytes());
        for (x, xentry) in numeric_dirs(&zpath)? {
            h.write(&x.to_le_bytes());
            h.write(&mtime_nanos(&xentry.path()).to_le_bytes());
        }
    }
    Ok(format!("{:016x}", h.0))
}

/// Where the index of `map_dir` lives when its own folder is not writable.
fn fallback_index_path(map_dir: &Path) -> PathBuf {
    let mut h = Fnv::new();
    h.write(map_dir.to_string_lossy().as_bytes());
    let app_dir = super::default_root()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    app_dir.join("custom_maps_index").join(format!("{:016x}.json", h.0))
}

fn read_index(path: &Path) -> Option<IndexFile> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Store the index; returns where it went (`None` if nowhere).
fn write_index(map_dir: &Path, fallback: &Path, index: &IndexFile) -> Option<PathBuf> {
    let json = serde_json::to_string(index).ok()?;
    let primary = map_dir.join(INDEX_FILE);
    match std::fs::write(&primary, &json) {
        Ok(()) => {
            // A stale fallback copy would only be read if the primary vanished — drop it.
            let _ = std::fs::remove_file(fallback);
            Some(primary)
        }
        Err(e) => {
            log::debug!("custom maps: index not writable in {} ({e}), using app data", map_dir.display());
            let res = fallback
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(fallback, &json));
            match res {
                Ok(()) => Some(fallback.to_path_buf()),
                Err(e2) => {
                    log::warn!(
                        "custom maps: could not store the coverage index for {} ({e}; fallback {}: {e2})",
                        map_dir.display(),
                        fallback.display()
                    );
                    None
                }
            }
        }
    }
}

fn load_valid(map_dir: &Path, fallback: &Path, sig: &str) -> Option<(Coverage, PathBuf)> {
    [map_dir.join(INDEX_FILE), fallback.to_path_buf()]
        .into_iter()
        .filter_map(|p| read_index(&p).map(|i| (i, p)))
        .find(|(i, _)| i.version == INDEX_VERSION && i.signature == sig)
        .map(|(i, p)| (i.coverage, p))
}

/// Coverage of a map folder: from its index when the signature still matches (and `force` is off),
/// else a fresh scan that rewrites the index. Blocking — call from a blocking thread. Scans are
/// serialised so two requests for the same map never scan twice in parallel.
pub fn load_or_scan(map_dir: &Path, force: bool) -> io::Result<CoverageReport> {
    static SCAN_LOCK: Mutex<()> = Mutex::new(());
    let _guard = SCAN_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let fallback = fallback_index_path(map_dir);
    // Signature BEFORE the scan: a change during the scan leaves the stored signature behind the
    // folder, so the next request rescans instead of trusting an incomplete index.
    let sig = signature(map_dir)?;
    if !force {
        if let Some((coverage, path)) = load_valid(map_dir, &fallback, &sig) {
            log::debug!("custom maps: coverage index of {} is current", map_dir.display());
            return Ok(CoverageReport {
                coverage,
                index_path: Some(path.to_string_lossy().into_owned()),
                index_hit: true,
            });
        }
    }
    let t0 = std::time::Instant::now();
    let levels = scan_levels(map_dir)?;
    let mut coverage = build(&levels);
    coverage.scan_ms = t0.elapsed().as_millis() as u64;
    log::info!(
        "custom maps: scanned {} — {} tiles on {} level(s), {} area(s), {} ms",
        map_dir.display(),
        coverage.tile_count,
        levels.len(),
        coverage.areas.len(),
        coverage.scan_ms
    );
    if coverage.areas_truncated {
        log::warn!(
            "custom maps: {} has more than {AREA_LIMIT} separate areas — frames and \"Show map area\" \
             cover the first {AREA_LIMIT} (highest levels first) only",
            map_dir.display()
        );
    }
    let index = IndexFile { version: INDEX_VERSION, signature: sig, coverage };
    let written = write_index(map_dir, &fallback, &index);
    Ok(CoverageReport {
        coverage: index.coverage,
        index_path: written.map(|p| p.to_string_lossy().into_owned()),
        index_hit: false,
    })
}

#[cfg(test)]
mod tests {
    use super::super::test_util::*;
    use super::*;

    fn cells(list: &[(u32, u32)]) -> CellSet {
        list.iter().copied().collect()
    }

    #[test]
    fn rects_merge_exactly() {
        // An L shape: column 0 rows 0..=2, columns 1..=2 row 2.
        let c = cells(&[(0, 0), (0, 1), (0, 2), (1, 2), (2, 2)]);
        let r = rects_from_cells(&c);
        assert_eq!(cells_from_rects(&r), c);
        assert_eq!(r.len(), 2);
        // A full block is one rectangle.
        let mut block = CellSet::new();
        for x in 10..20 {
            for y in 5..9 {
                block.insert((x, y));
            }
        }
        assert_eq!(rects_from_cells(&block), vec![[10, 5, 19, 8]]);
        // Equal runs separated by a gap column stay separate.
        let gap = cells(&[(0, 0), (2, 0)]);
        assert_eq!(rects_from_cells(&gap), vec![[0, 0, 0, 0], [2, 0, 2, 0]]);
    }

    #[test]
    fn boundary_of_a_block_is_four_segments() {
        let mut block = CellSet::new();
        for x in 2..5 {
            for y in 3..5 {
                block.insert((x, y));
            }
        }
        let mut s = boundary_segments(&block);
        s.sort_unstable();
        assert_eq!(s, vec![[2, 3, 2, 5], [2, 3, 5, 3], [2, 5, 5, 5], [5, 3, 5, 5]]);
        // Two separate squares: eight segments.
        assert_eq!(boundary_segments(&cells(&[(0, 0), (2, 2)])).len(), 8);
        // Diagonal neighbours share two edge lines, which merge into straight runs (same drawing).
        assert_eq!(boundary_segments(&cells(&[(0, 0), (1, 1)])).len(), 6);
    }

    #[test]
    fn footprint_coarsens_to_the_cell_limit() {
        let mut lt = LevelTiles { z: 12, ..Default::default() };
        for x in 0..300u32 {
            lt.columns.insert(x, (0..300u32).collect());
        }
        lt.count = 300 * 300;
        let (level, c) = footprint(&lt);
        assert!(c.len() <= CELL_LIMIT);
        assert!(level < 12);
        // Coarsened just enough: one level finer would not fit.
        assert!(project_columns(&lt.columns, 12 - level - 1).len() > CELL_LIMIT);
    }

    /// Two areas: A at z5..=8 around (16..18, 10..12)@5, B only at z12..=13 far away (not even
    /// diagonal neighbours of A at world zoom, so their outlines never share an edge line).
    fn two_area_tree(root: &Path) -> PathBuf {
        let m = root.join("map");
        for z in 5u8..=8 {
            let s = 1u32 << (z - 5);
            for x in 16 * s..18 * s {
                for y in 10 * s..12 * s {
                    touch_tile(&m, z, x, y, "png");
                }
            }
        }
        for z in 12u8..=13 {
            let s = 1u32 << (z - 12);
            for x in 3500 * s..3502 * s {
                for y in 1000 * s..1001 * s {
                    touch_tile(&m, z, x, y, "jpg");
                }
            }
        }
        m
    }

    #[test]
    fn scan_counts_tiles_and_ignores_junk() {
        let root = temp_root("scan");
        let m = two_area_tree(&root);
        std::fs::write(m.join("5").join("16").join("notes.txt"), b"x").unwrap();
        std::fs::create_dir_all(m.join("abc").join("1")).unwrap();
        std::fs::write(m.join("5").join("16").join("10.jpg"), b"dup").unwrap(); // same tile, other ext
        std::fs::create_dir_all(m.join("3").join("99")).unwrap(); // x out of range at z3
        std::fs::write(m.join("3").join("99").join("0.png"), b"x").unwrap();
        let levels = scan_levels(&m).unwrap();
        let zs: Vec<u8> = levels.iter().map(|l| l.z).collect();
        assert_eq!(zs, [5, 6, 7, 8, 12, 13]);
        assert_eq!(levels[0].count, 4);
        assert_eq!(levels[3].count, 4 * 64);
        assert_eq!(levels[5].count, 4 * 2);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn areas_of_a_sparse_set_carry_their_own_top() {
        let root = temp_root("areas");
        let m = two_area_tree(&root);
        let levels = scan_levels(&m).unwrap();
        let cov = build(&levels);
        assert_eq!(cov.levels.len(), 6);
        assert_eq!((cov.min_zoom, cov.max_zoom), (Some(5), Some(13)));
        // B (z12..=13) and A (z5..=8): one area each, highest top first. A's lower levels lie inside
        // A's z8 footprint, B's z12 inside B's z13 footprint — no extra areas.
        let tops: Vec<u8> = cov.areas.iter().map(|a| a.top).collect();
        assert_eq!(tops, [13, 8]);
        // Tight box around B (4 x 2 tiles at z13).
        let (lat_b, lon_b) = corner_to_latlon(3500, 1000, 12);
        let b = cov.areas[0].bounds;
        assert!(b[0] <= lon_b && lon_b < b[2] && b[1] < lat_b && lat_b <= b[3], "{b:?}");
        assert!(b[2] - b[0] < 0.2, "B box too wide: {b:?}");
        // Both footprints are rectangles: the real outline is 4 merged segments each.
        assert_eq!(cov.areas[0].segments.len(), 4);
        assert_eq!(cov.areas[1].segments.len(), 4);
        // The outline runs along the box.
        let s = &cov.areas[0].segments;
        assert!(s.iter().all(|g| (g[1] - b[0]).abs() < 1e-9 || (g[1] - b[2]).abs() < 1e-9
            || (g[3] - b[0]).abs() < 1e-9 || (g[3] - b[2]).abs() < 1e-9));
        assert_eq!(cov.levels.len(), 6);
        assert_eq!(cov.levels[0].tiles, 4);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_pyramid_is_one_area_with_the_detail_top() {
        // One airfield as a full pyramid z0..=16: every overview tile contains the detail, so the
        // only area is the airfield (2 x 2 tiles at z16, top 16), not the world.
        let mut levels = Vec::new();
        for z in 0u8..=16 {
            let k = 16 - z;
            let mut lt = LevelTiles { z, ..Default::default() };
            for x in (34_000u32 >> k)..=(34_001u32 >> k) {
                lt.columns.insert(x, ((22_000u32 >> k)..=(22_001u32 >> k)).collect());
            }
            lt.count = lt.columns.values().map(|c| c.len() as u64).sum();
            levels.push(lt);
        }
        let cov = build(&levels);
        assert_eq!(cov.areas.len(), 1, "{:?}", cov.areas);
        assert_eq!(cov.areas[0].top, 16);
        let air = cov.areas[0].bounds;
        assert!(air[2] - air[0] < 0.02 && air[3] - air[1] < 0.02, "airfield box {air:?}");

        // A second, overview-only patch far away at z3..=5 adds its own area with top 5.
        for z in 3u8..=5 {
            let s = 1u32 << (z - 3);
            let l = levels.iter_mut().find(|l| l.z == z).unwrap();
            l.columns.insert(s, vec![s]);
            l.count += 1;
        }
        let cov = build(&levels);
        let tops: Vec<u8> = cov.areas.iter().map(|a| a.top).collect();
        assert_eq!(tops, [16, 5]);
        let far = cov.areas[1].bounds;
        assert!(far[2] - far[0] > 1.0, "overview-only area {far:?}");
    }

    #[test]
    fn a_hole_in_the_top_level_is_no_extra_area() {
        // z14: an 8 x 8 block with a 2 x 2 hole (a downloader skipped blank tiles); z13: the full
        // 4 x 4 parent block. The z13 tile under the hole is outside the z14 footprint but inside its
        // box — it must not become a second area (a frame inside a frame).
        let (x0, y0) = (8000u32, 5000u32);
        let mut top = LevelTiles { z: 14, ..Default::default() };
        for x in x0..x0 + 8 {
            let ys: Vec<u32> = (y0..y0 + 8)
                .filter(|&y| !((x0 + 2..x0 + 4).contains(&x) && (y0 + 2..y0 + 4).contains(&y)))
                .collect();
            top.count += ys.len() as u64;
            top.columns.insert(x, ys);
        }
        let mut low = LevelTiles { z: 13, ..Default::default() };
        for x in x0 / 2..x0 / 2 + 4 {
            low.columns.insert(x, (y0 / 2..y0 / 2 + 4).collect());
        }
        low.count = 16;
        let cov = build(&[low, top]);
        let tops: Vec<u8> = cov.areas.iter().map(|a| a.top).collect();
        assert_eq!(tops, [14]);
        assert!(!cov.areas_truncated);
    }

    #[test]
    fn an_edge_notch_in_the_top_level_is_no_extra_area() {
        // z14: an 8 x 8 block missing a 2 x 2 notch at its top edge; z13: the full parent block. The
        // z13 tile under the notch touches the z14 area and lies in its box → dropped.
        let (x0, y0) = (8000u32, 5000u32);
        let mut top = LevelTiles { z: 14, ..Default::default() };
        for x in x0..x0 + 8 {
            let ys: Vec<u32> = (y0..y0 + 8)
                .filter(|&y| !((x0 + 2..x0 + 4).contains(&x) && y < y0 + 2))
                .collect();
            top.count += ys.len() as u64;
            top.columns.insert(x, ys);
        }
        let mut low = LevelTiles { z: 13, ..Default::default() };
        for x in x0 / 2..x0 / 2 + 4 {
            low.columns.insert(x, (y0 / 2..y0 / 2 + 4).collect());
        }
        low.count = 16;
        let cov = build(&[low, top]);
        let tops: Vec<u8> = cov.areas.iter().map(|a| a.top).collect();
        assert_eq!(tops, [14]);
    }

    #[test]
    fn a_detached_field_inside_a_corridor_box_keeps_its_area() {
        // z14: a diagonal corridor of 16 tiles (one area, box 16 x 16). z13: the corridor's parents
        // plus a detached block far from the corridor but inside its box — that block is its own
        // field (loaded only to z13) and must keep its frame and "Show map area" target.
        let (x0, y0) = (8000u32, 5000u32);
        let mut top = LevelTiles { z: 14, ..Default::default() };
        for i in 0..16u32 {
            top.columns.insert(x0 + i, vec![y0 + i]);
        }
        top.count = 16;
        let mut low = LevelTiles { z: 13, ..Default::default() };
        for j in 0..8u32 {
            low.columns.entry(x0 / 2 + j).or_default().push(y0 / 2 + j);
        }
        // Detached: covers z14 x0+12..=13, y0+2..=3 — the corridor is at (x0+2, y0+2) there.
        low.columns.entry(x0 / 2 + 6).or_default().push(y0 / 2 + 1);
        for ys in low.columns.values_mut() {
            ys.sort_unstable();
        }
        low.count = 9;
        let cov = build(&[low, top]);
        let tops: Vec<u8> = cov.areas.iter().map(|a| a.top).collect();
        assert_eq!(tops, [14, 13]);
    }

    #[test]
    fn area_count_is_bounded() {
        // 200 isolated tiles at z12 (every 3rd column/row) → capped.
        let mut lt = LevelTiles { z: 12, ..Default::default() };
        for i in 0..200u32 {
            lt.columns.insert(i * 3, vec![(i % 20) * 3]);
        }
        lt.count = 200;
        let cov = build(&[lt]);
        assert_eq!(cov.areas.len(), AREA_LIMIT);
        assert!(cov.areas_truncated);
    }

    #[test]
    fn area_outline_stays_bounded() {
        // One connected comb at z14: a full row with 300 one-cell teeth below it — far more boundary
        // segments than allowed, so the outline is coarsened.
        let mut lt = LevelTiles { z: 14, ..Default::default() };
        for x in 0..600u32 {
            let ys = if x % 2 == 0 { vec![0, 1, 2, 3] } else { vec![0] };
            lt.columns.insert(x, ys);
        }
        lt.count = lt.columns.values().map(|c| c.len() as u64).sum();
        let cov = build(&[lt]);
        assert_eq!(cov.areas.len(), 1);
        let n = cov.areas[0].segments.len();
        assert!((4..=AREA_SEG_LIMIT).contains(&n), "{n} segments");
    }

    #[test]
    fn index_is_reused_and_invalidated() {
        let root = temp_root("index");
        let m = two_area_tree(&root);
        let report = load_or_scan(&m, false).unwrap();
        assert!(!report.index_hit);
        assert_eq!(report.index_path.as_deref(), Some(m.join(INDEX_FILE).to_string_lossy().as_ref()));
        let first = report.coverage;
        assert!(m.join(INDEX_FILE).is_file(), "index written into the map folder");
        let sig = signature(&m).unwrap();
        // Unchanged folder → same signature, index reused (identical result).
        assert_eq!(signature(&m).unwrap(), sig);
        let again = load_or_scan(&m, false).unwrap();
        assert!(again.index_hit);
        assert_eq!(again.coverage, first);
        // The index file itself must not change the signature.
        std::fs::write(m.join(INDEX_FILE), std::fs::read(m.join(INDEX_FILE)).unwrap()).unwrap();
        assert_eq!(signature(&m).unwrap(), sig);
        // A new column (new X folder) → zoom folder changed → signature differs → rescan.
        touch_tile(&m, 8, 200, 100, "png");
        assert_ne!(signature(&m).unwrap(), sig);
        let second = load_or_scan(&m, false).unwrap().coverage;
        assert_eq!(second.tile_count, first.tile_count + 1);
        // A tampered index with the right signature is trusted — that is what Rescan (force) is for.
        let sig2 = signature(&m).unwrap();
        let fake = IndexFile {
            version: INDEX_VERSION,
            signature: sig2,
            coverage: Coverage::default(),
        };
        std::fs::write(m.join(INDEX_FILE), serde_json::to_string(&fake).unwrap()).unwrap();
        assert_eq!(load_or_scan(&m, false).unwrap().coverage.tile_count, 0);
        assert_eq!(load_or_scan(&m, true).unwrap().coverage.tile_count, second.tile_count);
        // An index of another version is ignored.
        let mut old: IndexFile = read_index(&m.join(INDEX_FILE)).unwrap();
        old.version = INDEX_VERSION + 1;
        old.coverage.tile_count = 1;
        std::fs::write(m.join(INDEX_FILE), serde_json::to_string(&old).unwrap()).unwrap();
        assert_eq!(load_or_scan(&m, false).unwrap().coverage.tile_count, second.tile_count);
        let _ = std::fs::remove_dir_all(&root);
    }
}
