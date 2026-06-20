// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Terrain elevation provider — Copernicus DEM GLO-30.
//
// Source: AWS Open Data `copernicus-dem-30m` (Cloud Optimized GeoTIFF, 1°×1°
// tiles, Float32, EGM2008 geoid ≈ MSL, no API key). Used locally only for
// AGL waypoint planning, terrain clearance, AGL widget and LOS analysis.
//
// Because GLO-30 is geoid-referenced (≈ MSL) it is directly comparable with
// the FC's MSL GPS altitude and INAV AMSL waypoints — no geoid conversion.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tiff::decoder::{Decoder, DecodingResult};
use tiff::tags::Tag;

const BASE_URL: &str = "https://copernicus-dem-30m.s3.amazonaws.com";
/// Max decoded tiles kept in memory. Each GLO-30 tile is ~3600×3600 f32 ≈ 52 MB.
const MAX_CACHED_TILES: usize = 4;

/// A decoded DEM tile: full-resolution Float32 grid + geographic transform.
struct TileData {
    width: usize,
    height: usize,
    /// row-major elevation grid, length = width*height (metres, ≈ MSL)
    grid: Vec<f32>,
    /// geographic coordinate of pixel (0,0) — top-left
    origin_lon: f64,
    origin_lat: f64,
    /// degrees per pixel (always positive)
    px_lon: f64,
    px_lat: f64,
}

impl TileData {
    /// Bilinear sample at lat/lon. Returns None if outside the tile.
    fn sample(&self, lat: f64, lon: f64) -> Option<f32> {
        let col = (lon - self.origin_lon) / self.px_lon;
        let row = (self.origin_lat - lat) / self.px_lat;
        if col < 0.0 || row < 0.0 || col > (self.width - 1) as f64 || row > (self.height - 1) as f64 {
            return None;
        }
        let c0 = col.floor() as usize;
        let r0 = row.floor() as usize;
        let c1 = (c0 + 1).min(self.width - 1);
        let r1 = (r0 + 1).min(self.height - 1);
        let fc = col - c0 as f64;
        let fr = row - r0 as f64;

        let v = |r: usize, c: usize| self.grid[r * self.width + c] as f64;
        let top = v(r0, c0) * (1.0 - fc) + v(r0, c1) * fc;
        let bot = v(r1, c0) * (1.0 - fc) + v(r1, c1) * fc;
        Some((top * (1.0 - fr) + bot * fr) as f32)
    }
}

#[derive(Serialize, Clone)]
pub struct ProfileSample {
    /// cumulative distance from the route start, metres
    pub dist_m: f64,
    pub lat: f64,
    pub lon: f64,
    /// terrain elevation (≈ MSL), null if unavailable (ocean tile / out of range)
    pub elev_m: Option<f32>,
}

/// Polar terrain sampling over a forward fan — for the terrain-radar widget.
#[derive(Serialize, Clone)]
pub struct TerrainFan {
    pub ang_cells: usize,
    pub rad_cells: usize,
    pub range_m: f64,
    /// cell-centre elevations (≈ MSL), row-major `[ang_cell * rad_cells + rad_cell]`;
    /// `null` where the tile is missing (ocean / out of coverage)
    pub elev: Vec<Option<f32>>,
}

struct CacheState {
    tiles: HashMap<String, Arc<TileData>>,
    missing: HashSet<String>, // tiles known to not exist (ocean)
    order: VecDeque<String>,  // insertion order for simple eviction
}

pub struct TerrainProvider {
    client: reqwest::Client,
    cache_dir: PathBuf,
    state: Mutex<CacheState>,
    /// Serializes tile loads so concurrent requests coalesce (no redundant
    /// decode of the same tile) and CPU/IO load stays bounded.
    load_lock: tokio::sync::Mutex<()>,
}

impl TerrainProvider {
    pub fn new() -> Self {
        let cache_dir = resolve_terrain_cache_dir();
        std::fs::create_dir_all(&cache_dir).ok();
        TerrainProvider {
            client: reqwest::Client::builder()
                .user_agent("KiteGC/terrain")
                // Bound every DEM fetch: a stalled connection must NOT hang forever while
                // holding `load_lock` (which serializes all tile loads) — that would freeze
                // every terrain call (both HUD widgets + mission + geoid) for the whole replay.
                .connect_timeout(Duration::from_secs(8))
                .timeout(Duration::from_secs(25))
                .build()
                .unwrap_or_default(),
            cache_dir,
            state: Mutex::new(CacheState {
                tiles: HashMap::new(),
                missing: HashSet::new(),
                order: VecDeque::new(),
            }),
            load_lock: tokio::sync::Mutex::new(()),
        }
    }

    fn cached(&self, key: &str) -> Option<Arc<TileData>> {
        self.state.lock().unwrap().tiles.get(key).cloned()
    }

    fn is_missing(&self, key: &str) -> bool {
        self.state.lock().unwrap().missing.contains(key)
    }

    fn insert_tile(&self, key: String, tile: Arc<TileData>) {
        let mut st = self.state.lock().unwrap();
        st.tiles.insert(key.clone(), tile);
        st.order.push_back(key);
        while st.order.len() > MAX_CACHED_TILES {
            if let Some(old) = st.order.pop_front() {
                st.tiles.remove(&old);
            }
        }
    }

    /// Elevation (metres ≈ MSL) at lat/lon, or None if unavailable.
    pub async fn elevation(&self, lat: f64, lon: f64) -> Option<f32> {
        let key = tile_name(lat, lon);

        // Fast path: already cached or known-missing (no blocking, no await)
        if let Some(t) = self.cached(&key) {
            return t.sample(lat, lon);
        }
        if self.is_missing(&key) {
            return None;
        }

        // Slow path: serialize loads so concurrent callers coalesce.
        let _guard = self.load_lock.lock().await;
        // Re-check — another task may have loaded it while we waited for the lock.
        if let Some(t) = self.cached(&key) {
            return t.sample(lat, lon);
        }
        if self.is_missing(&key) {
            return None;
        }

        match self.load_tile(&key).await {
            Some(tile) => {
                let sample = tile.sample(lat, lon);
                self.insert_tile(key, tile);
                sample
            }
            None => {
                self.state.lock().unwrap().missing.insert(key);
                None
            }
        }
    }

    /// Total bytes + file count of the on-disk terrain tile cache (`*.tif`).
    pub fn cache_stats(&self) -> (u64, usize) {
        let mut bytes = 0u64;
        let mut count = 0usize;
        if let Ok(entries) = std::fs::read_dir(&self.cache_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("tif") {
                    if let Ok(meta) = entry.metadata() {
                        bytes += meta.len();
                        count += 1;
                    }
                }
            }
        }
        (bytes, count)
    }

    /// Delete all on-disk terrain tiles and clear the in-memory caches. Returns files removed.
    pub fn clear_cache(&self) -> usize {
        let mut removed = 0usize;
        if let Ok(entries) = std::fs::read_dir(&self.cache_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("tif")
                    && std::fs::remove_file(&path).is_ok()
                {
                    removed += 1;
                }
            }
        }
        // Drop decoded tiles + the known-missing set so cleared tiles re-download on demand.
        if let Ok(mut st) = self.state.lock() {
            st.tiles.clear();
            st.missing.clear();
            st.order.clear();
        }
        removed
    }

    /// Elevation (metres ≈ MSL) at each lat/lon, in input order. A batched
    /// `elevation()` so a caller resolving many points (e.g. every mission
    /// waypoint) pays a single IPC round-trip instead of one per point. Loads
    /// stay serialized via `elevation()`'s `load_lock`, so points sharing a tile
    /// reuse the same load rather than racing duplicate 42 MB downloads.
    pub async fn elevations(&self, points: &[(f64, f64)]) -> Vec<Option<f32>> {
        let mut out = Vec::with_capacity(points.len());
        for &(lat, lon) in points {
            out.push(self.elevation(lat, lon).await);
        }
        out
    }

    /// Sample terrain along a polyline of waypoints at `spacing_m` ground spacing.
    /// Returns cumulative-distance samples (including the exact waypoint positions).
    pub async fn profile(&self, points: &[(f64, f64)], spacing_m: f64) -> Vec<ProfileSample> {
        let mut out: Vec<ProfileSample> = Vec::new();
        if points.is_empty() {
            return out;
        }
        let spacing = spacing_m.max(1.0);
        let mut cumulative = 0.0_f64;

        // first point
        out.push(ProfileSample {
            dist_m: 0.0,
            lat: points[0].0,
            lon: points[0].1,
            elev_m: self.elevation(points[0].0, points[0].1).await,
        });

        for w in points.windows(2) {
            let (lat0, lon0) = w[0];
            let (lat1, lon1) = w[1];
            let seg = haversine_m(lat0, lon0, lat1, lon1);
            let steps = (seg / spacing).floor() as usize;
            for s in 1..=steps {
                let t = (s as f64 * spacing) / seg;
                if t >= 1.0 {
                    break;
                }
                let lat = lat0 + (lat1 - lat0) * t;
                let lon = lon0 + (lon1 - lon0) * t;
                out.push(ProfileSample {
                    dist_m: cumulative + s as f64 * spacing,
                    lat,
                    lon,
                    elev_m: self.elevation(lat, lon).await,
                });
            }
            cumulative += seg;
            out.push(ProfileSample {
                dist_m: cumulative,
                lat: lat1,
                lon: lon1,
                elev_m: self.elevation(lat1, lon1).await,
            });
        }
        out
    }

    /// Sample terrain over a forward fan (polar grid) for the terrain-radar widget.
    /// `heading_deg` is true bearing; the fan spans ±`half_angle_deg`. Each cell is
    /// sampled at its angular + radial centre. Row-major `[ang_cell][rad_cell]`.
    pub async fn fan(
        &self,
        lat: f64,
        lon: f64,
        heading_deg: f64,
        half_angle_deg: f64,
        range_m: f64,
        ang_cells: usize,
        rad_cells: usize,
    ) -> TerrainFan {
        let ang = ang_cells.max(1);
        let rad = rad_cells.max(1);
        let range = range_m.max(1.0);
        let mut elev = Vec::with_capacity(ang * rad);
        for a in 0..ang {
            // angular centre of this cell, 0..1 across the fan (left → right)
            let frac = (a as f64 + 0.5) / ang as f64;
            let bearing = heading_deg - half_angle_deg + 2.0 * half_angle_deg * frac;
            for b in 0..rad {
                let dist = range * (b as f64 + 0.5) / rad as f64;
                let (clat, clon) = dest_point(lat, lon, bearing, dist);
                elev.push(self.elevation(clat, clon).await);
            }
        }
        TerrainFan { ang_cells: ang, rad_cells: rad, range_m: range, elev }
    }

    /// Fetch (disk-cached) + decode a tile. None if the tile doesn't exist.
    /// The CPU-bound decode runs on the blocking thread pool so it never stalls
    /// the async runtime's worker threads.
    async fn load_tile(&self, key: &str) -> Option<Arc<TileData>> {
        let bytes = self.tile_bytes(key).await?;
        let key_owned = key.to_string();
        let decoded = tauri::async_runtime::spawn_blocking(move || decode_tile(&bytes)).await;
        match decoded {
            Ok(Ok(t)) => Some(Arc::new(t)),
            Ok(Err(e)) => {
                eprintln!("[terrain] decode failed for {key_owned}: {e}");
                None
            }
            Err(e) => {
                eprintln!("[terrain] decode task failed for {key_owned}: {e}");
                None
            }
        }
    }

    /// Raw .tif bytes for a tile: disk cache → else HTTP GET (and cache to disk).
    /// Blocking disk I/O (≈42 MB) is offloaded to the blocking thread pool.
    async fn tile_bytes(&self, key: &str) -> Option<Vec<u8>> {
        let path = self.cache_dir.join(format!("{key}.tif"));

        let read_path = path.clone();
        if let Ok(Ok(b)) =
            tauri::async_runtime::spawn_blocking(move || std::fs::read(&read_path)).await
        {
            return Some(b);
        }

        let url = format!("{BASE_URL}/{key}/{key}.tif");
        eprintln!("[terrain] fetching {url}");
        let resp = self.client.get(&url).send().await.ok()?;
        if !resp.status().is_success() {
            // 403/404 → tile doesn't exist (ocean)
            return None;
        }
        let bytes = resp.bytes().await.ok()?.to_vec();

        let write_path = path.clone();
        let write_bytes = bytes.clone();
        tauri::async_runtime::spawn_blocking(move || {
            std::fs::write(&write_path, &write_bytes).ok();
        })
        .await
        .ok();

        Some(bytes)
    }
}

impl Default for TerrainProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Copernicus GLO-30 tile name for the 1° cell containing lat/lon.
/// e.g. (47.5, 11.2) → "Copernicus_DSM_COG_10_N47_00_E011_00_DEM"
fn tile_name(lat: f64, lon: f64) -> String {
    let lat_i = lat.floor() as i32;
    let lon_i = lon.floor() as i32;
    let ns = if lat_i >= 0 { 'N' } else { 'S' };
    let ew = if lon_i >= 0 { 'E' } else { 'W' };
    format!(
        "Copernicus_DSM_COG_10_{}{:02}_00_{}{:03}_00_DEM",
        ns,
        lat_i.unsigned_abs(),
        ew,
        lon_i.unsigned_abs()
    )
}

/// Decode a GeoTIFF DEM tile into a Float32 grid + geographic transform.
fn decode_tile(bytes: &[u8]) -> Result<TileData, String> {
    let mut dec = Decoder::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let (width, height) = dec.dimensions().map_err(|e| e.to_string())?;

    // GeoTIFF georeferencing tags (non-baseline → read by numeric id):
    //   33550 ModelPixelScaleTag  = [scaleX, scaleY, scaleZ]
    //   33922 ModelTiepointTag     = [i, j, k, x, y, z]  (pixel i,j → geo x,y)
    let scale = dec
        .find_tag(Tag::ModelPixelScaleTag)
        .map_err(|e| e.to_string())?
        .ok_or("missing ModelPixelScaleTag")?
        .into_f64_vec()
        .map_err(|e| e.to_string())?;
    let tie = dec
        .find_tag(Tag::ModelTiepointTag)
        .map_err(|e| e.to_string())?
        .ok_or("missing ModelTiepointTag")?
        .into_f64_vec()
        .map_err(|e| e.to_string())?;

    if scale.len() < 2 || tie.len() < 6 {
        return Err("malformed geo tags".into());
    }
    let px_lon = scale[0].abs();
    let px_lat = scale[1].abs();
    // pixel (tie[0],tie[1]) maps to (tie[3],tie[4]); GLO-30 has i=j=0 → top-left
    let origin_lon = tie[3] - tie[0] * px_lon;
    let origin_lat = tie[4] + tie[1] * px_lat;

    let grid = match dec.read_image().map_err(|e| e.to_string())? {
        DecodingResult::F32(v) => v,
        DecodingResult::F64(v) => v.into_iter().map(|x| x as f32).collect(),
        DecodingResult::I16(v) => v.into_iter().map(|x| x as f32).collect(),
        DecodingResult::U16(v) => v.into_iter().map(|x| x as f32).collect(),
        _ => return Err("unexpected DEM sample format".into()),
    };

    let (w, h) = (width as usize, height as usize);
    if grid.len() != w * h {
        return Err(format!("grid size {} != {}×{}", grid.len(), w, h));
    }

    Ok(TileData {
        width: w,
        height: h,
        grid,
        origin_lon,
        origin_lat,
        px_lon,
        px_lat,
    })
}

/// Destination point from (lat, lon) along `bearing_deg` (true) at `dist_m`,
/// great-circle (matches the frontend's `destPoint`).
fn dest_point(lat: f64, lon: f64, bearing_deg: f64, dist_m: f64) -> (f64, f64) {
    const R: f64 = 6_371_000.0;
    let br = bearing_deg.to_radians();
    let p1 = lat.to_radians();
    let l1 = lon.to_radians();
    let dr = dist_m / R;
    let p2 = (p1.sin() * dr.cos() + p1.cos() * dr.sin() * br.cos()).asin();
    let l2 = l1 + (br.sin() * dr.sin() * p1.cos()).atan2(dr.cos() - p1.sin() * p2.sin());
    (p2.to_degrees(), l2.to_degrees())
}

/// Great-circle distance in metres.
fn haversine_m(lat0: f64, lon0: f64, lat1: f64, lon1: f64) -> f64 {
    const R: f64 = 6_371_000.0;
    let (p0, p1) = (lat0.to_radians(), lat1.to_radians());
    let dlat = (lat1 - lat0).to_radians();
    let dlon = (lon1 - lon0).to_radians();
    let a = (dlat / 2.0).sin().powi(2) + p0.cos() * p1.cos() * (dlon / 2.0).sin().powi(2);
    2.0 * R * a.sqrt().asin()
}

/// Terrain tile cache directory. Mirrors the DB path policy: portable marker
/// next to the exe → `<exe>/data/terrain`, else platform AppData.
fn resolve_terrain_cache_dir() -> PathBuf {
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
    {
        if exe_dir.join(".portable").exists() {
            return exe_dir.join("data").join("terrain");
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return PathBuf::from(appdata).join("kite-gc").join("terrain");
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("kite-gc")
                .join("terrain");
        }
    }
    PathBuf::from("terrain")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_naming() {
        assert_eq!(tile_name(47.5, 11.2), "Copernicus_DSM_COG_10_N47_00_E011_00_DEM");
        assert_eq!(tile_name(-0.5, -4.3), "Copernicus_DSM_COG_10_S01_00_W005_00_DEM");
        assert_eq!(tile_name(0.0, 0.0), "Copernicus_DSM_COG_10_N00_00_E000_00_DEM");
    }

    #[test]
    fn haversine_sanity() {
        // ~111 km per degree of latitude
        let d = haversine_m(48.0, 11.0, 49.0, 11.0);
        assert!((d - 111_195.0).abs() < 500.0, "got {d}");
    }

    // Network spike: fetch + decode a real Copernicus GLO-30 tile and sample a
    // known point. Validates DEFLATE + floating-point predictor (PREDICTOR=3)
    // decoding end-to-end. Run with: cargo test -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn decode_real_tile_zugspitze() {
        let p = TerrainProvider::new();
        // Zugspitze summit ≈ 47.421°N, 10.985°E, ~2962 m
        let e = p.elevation(47.421, 10.985).await;
        println!("Zugspitze elevation sample: {e:?}");
        let v = e.expect("expected an elevation sample");
        assert!((2000.0..3200.0).contains(&v), "implausible elevation {v}");
    }
}
