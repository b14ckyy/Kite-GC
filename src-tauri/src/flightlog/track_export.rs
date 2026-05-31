// Track export — KMZ, KML, GPX, CSV export for flight tracks

use std::io::Write;
use std::path::Path;

use chrono::Duration;

use super::types::{Flight, TelemetryRecord};

/// Get the best altitude for a record.
/// Preference: nav_alt_m (INAV fused, relative to home) → baro_alt_m (relative to home).
/// Returns altitude relative to home/launch point.
fn best_alt_relative(r: &TelemetryRecord) -> f64 {
    r.nav_alt_m.unwrap_or_else(|| r.baro_alt_m.unwrap_or(0.0))
}

/// Ramer-Douglas-Peucker track simplification in 3D.
/// Reduces GPS jitter (especially during hover) while preserving the flight path shape.
fn simplify_track_dp<'a>(track: &[&'a TelemetryRecord], epsilon: f64) -> Vec<&'a TelemetryRecord> {
    if track.len() <= 2 {
        return track.to_vec();
    }
    dp_recurse(track, epsilon)
}

fn dp_recurse<'a>(track: &[&'a TelemetryRecord], epsilon: f64) -> Vec<&'a TelemetryRecord> {
    if track.len() <= 2 {
        return track.to_vec();
    }
    let first = track.first().unwrap();
    let last = track.last().unwrap();

    let (lat0, lon0) = (first.lat.unwrap_or(0.0), first.lon.unwrap_or(0.0));
    let (lat1, lon1) = (last.lat.unwrap_or(0.0), last.lon.unwrap_or(0.0));
    let alt0 = best_alt_relative(first);
    let alt1 = best_alt_relative(last);

    let mut max_dist = 0.0f64;
    let mut max_idx = 0usize;

    for i in 1..track.len() - 1 {
        let r = track[i];
        let lat = r.lat.unwrap_or(0.0);
        let lon = r.lon.unwrap_or(0.0);
        let alt = best_alt_relative(r);
        let d = point_to_line_distance_3d(
            lat, lon, alt, lat0, lon0, alt0, lat1, lon1, alt1,
        );
        if d > max_dist {
            max_dist = d;
            max_idx = i;
        }
    }

    if max_dist > epsilon {
        let mut left = dp_recurse(&track[..=max_idx], epsilon);
        let right = dp_recurse(&track[max_idx..], epsilon);
        left.pop(); // remove duplicate at split point
        left.extend(right);
        left
    } else {
        vec![*first, *last]
    }
}

/// Perpendicular distance from point P to the line segment A→B in approximate meters.
/// Uses a local flat-earth approximation (good enough for short segments).
fn point_to_line_distance_3d(
    plat: f64, plon: f64, palt: f64,
    alat: f64, alon: f64, aalt: f64,
    blat: f64, blon: f64, balt: f64,
) -> f64 {
    // Convert to approximate local meters
    let lat_mid = (alat + blat) / 2.0;
    let m_per_deg_lat = 111_320.0;
    let m_per_deg_lon = 111_320.0 * lat_mid.to_radians().cos();

    let ax = 0.0;
    let ay = 0.0;
    let az = aalt;
    let bx = (blon - alon) * m_per_deg_lon;
    let by = (blat - alat) * m_per_deg_lat;
    let bz = balt;
    let px = (plon - alon) * m_per_deg_lon;
    let py = (plat - alat) * m_per_deg_lat;
    let pz = palt;

    // Vector AB and AP
    let abx = bx - ax;
    let aby = by - ay;
    let abz = bz - az;
    let apx = px - ax;
    let apy = py - ay;
    let apz = pz - az;

    let ab_len2 = abx * abx + aby * aby + abz * abz;
    if ab_len2 < 1e-12 {
        return (apx * apx + apy * apy + apz * apz).sqrt();
    }

    let t = (apx * abx + apy * aby + apz * abz) / ab_len2;
    let t = t.clamp(0.0, 1.0);
    let dx = apx - t * abx;
    let dy = apy - t * aby;
    let dz = apz - t * abz;
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// Check if a GPS coordinate is valid (not 0,0 / NaN / out of range).
fn is_valid_gps(lat: f64, lon: f64) -> bool {
    lat.is_finite()
        && lon.is_finite()
        && (-90.0..=90.0).contains(&lat)
        && (-180.0..=180.0).contains(&lon)
        && !(lat == 0.0 && lon == 0.0)
}

/// Filter track to only records with valid raw GPS coordinates.
fn filter_valid_gps(track: &[TelemetryRecord]) -> Vec<&TelemetryRecord> {
    track
        .iter()
        .filter(|r| {
            matches!((r.lat, r.lon), (Some(la), Some(lo)) if is_valid_gps(la, lo))
        })
        .collect()
}

/// Remove position spikes: points where the distance from the previous point
/// implies an unreasonable speed (> 150 m/s ≈ 540 km/h).
fn remove_spikes<'a>(track: &[&'a TelemetryRecord]) -> Vec<&'a TelemetryRecord> {
    if track.len() <= 2 {
        return track.to_vec();
    }
    let mut result = Vec::with_capacity(track.len());
    result.push(track[0]);

    for i in 1..track.len() {
        let prev = result.last().unwrap();
        let cur = track[i];
        let (lat0, lon0) = (prev.lat.unwrap_or(0.0), prev.lon.unwrap_or(0.0));
        let (lat1, lon1) = (cur.lat.unwrap_or(0.0), cur.lon.unwrap_or(0.0));
        let dt_s = (cur.timestamp_ms - prev.timestamp_ms).max(1) as f64 / 1000.0;
        let dist = haversine_m(lat0, lon0, lat1, lon1);
        let speed = dist / dt_s;
        // Skip points implying > 150 m/s (540 km/h) — clearly erroneous
        if speed <= 150.0 {
            result.push(cur);
        }
    }
    result
}

/// Simple haversine distance in meters.
fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let r = 6_371_000.0;
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (dlon / 2.0).sin().powi(2);
    r * 2.0 * a.sqrt().asin()
}

/// Export a flight track to the given path. Format is detected from the file extension.
pub fn export_track(
    flight: &Flight,
    track: &[TelemetryRecord],
    output_path: &Path,
) -> Result<(), String> {
    let valid_track = filter_valid_gps(track);
    if valid_track.is_empty() {
        return Err("No valid GPS data in this flight.".to_string());
    }

    // Remove position spikes (erroneous jumps)
    let clean_track = remove_spikes(&valid_track);

    // Douglas-Peucker simplification: epsilon in meters.
    // 0.5 m reduces jitter from GPS noise while preserving real maneuvers.
    let simplified = simplify_track_dp(&clean_track, 0.5);

    let ext = output_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "kmz" => export_kmz(flight, &simplified, output_path),
        "kml" => {
            let kml = build_kml(flight, &simplified);
            std::fs::write(output_path, kml)
                .map_err(|e| format!("Failed to write KML: {}", e))
        }
        "gpx" => {
            let gpx = build_gpx(flight, &simplified);
            std::fs::write(output_path, gpx)
                .map_err(|e| format!("Failed to write GPX: {}", e))
        }
        "csv" => export_csv(flight, &valid_track, output_path),
        _ => Err(format!("Unsupported export format: .{}", ext)),
    }
}

// ── KMZ (zipped KML) ───────────────────────────────────────────────

fn export_kmz(
    flight: &Flight,
    track: &[&TelemetryRecord],
    output_path: &Path,
) -> Result<(), String> {
    let kml = build_kml(flight, track);
    let file = std::fs::File::create(output_path)
        .map_err(|e| format!("Failed to create KMZ file: {}", e))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("doc.kml", options)
        .map_err(|e| format!("Failed to start KML entry in KMZ: {}", e))?;
    zip.write_all(kml.as_bytes())
        .map_err(|e| format!("Failed to write KML to KMZ: {}", e))?;
    zip.finish()
        .map_err(|e| format!("Failed to finalize KMZ: {}", e))?;
    Ok(())
}

// ── KML ─────────────────────────────────────────────────────────────

fn build_kml(flight: &Flight, track: &[&TelemetryRecord]) -> String {
    let name = xml_escape(&flight_name(flight));
    let desc = xml_escape(&flight_description(flight));

    let mut coords = String::new();
    for r in track.iter() {
        if let (Some(lon), Some(lat)) = (r.lon, r.lat) {
            let alt = best_alt_relative(r);
            if !coords.is_empty() {
                coords.push(' ');
            }
            coords.push_str(&format!("{:.7},{:.7},{:.1}", lon, lat, alt));
        }
    }

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<kml xmlns="http://www.opengis.net/kml/2.2">
  <Document>
    <name>{name}</name>
    <description>{desc}</description>
    <Style id="flightPath">
      <LineStyle>
        <color>ff0080ff</color>
        <width>3</width>
      </LineStyle>
    </Style>
    <Placemark>
      <name>Flight Track</name>
      <styleUrl>#flightPath</styleUrl>
      <LineString>
        <altitudeMode>relativeToGround</altitudeMode>
        <coordinates>{coords}</coordinates>
      </LineString>
    </Placemark>
  </Document>
</kml>
"#
    )
}

// ── GPX ─────────────────────────────────────────────────────────────

fn build_gpx(flight: &Flight, track: &[&TelemetryRecord]) -> String {
    let name = xml_escape(&flight_name(flight));
    let flight_start = flight.start_time;

    let mut trkpts = String::new();
    for r in track.iter() {
        if let (Some(lat), Some(lon)) = (r.lat, r.lon) {
            let time = flight_start + Duration::milliseconds(r.timestamp_ms);
            let time_str = time.format("%Y-%m-%dT%H:%M:%S%.3fZ");
            trkpts.push_str(&format!(
                "      <trkpt lat=\"{:.7}\" lon=\"{:.7}\">\n",
                lat, lon
            ));
            let alt = best_alt_relative(r);
            trkpts.push_str(&format!("        <ele>{:.1}</ele>\n", alt));
            trkpts.push_str(&format!("        <time>{}</time>\n", time_str));
            if let Some(spd) = r.speed_ms {
                trkpts.push_str(&format!(
                    "        <extensions><speed>{:.2}</speed></extensions>\n",
                    spd
                ));
            }
            trkpts.push_str("      </trkpt>\n");
        }
    }

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<gpx version="1.1" creator="KiteGC"
     xmlns="http://www.topografix.com/GPX/1/1">
  <metadata>
    <name>{name}</name>
    <time>{time}</time>
  </metadata>
  <trk>
    <name>{name}</name>
    <trkseg>
{trkpts}    </trkseg>
  </trk>
</gpx>
"#,
        time = flight_start.format("%Y-%m-%dT%H:%M:%S%.3fZ"),
    )
}

// ── CSV ─────────────────────────────────────────────────────────────

fn export_csv(
    flight: &Flight,
    track: &[&TelemetryRecord],
    output_path: &Path,
) -> Result<(), String> {
    let mut wtr = csv::Writer::from_path(output_path)
        .map_err(|e| format!("Failed to create CSV: {}", e))?;

    wtr.write_record([
        "timestamp_ms",
        "time_utc",
        "lat",
        "lon",
        "alt_m",
        "baro_alt_m",
        "nav_lat",
        "nav_lon",
        "nav_alt_m",
        "speed_ms",
        "heading",
        "vario_ms",
        "roll",
        "pitch",
        "yaw",
        "voltage",
        "current_a",
        "mah_drawn",
        "rssi",
        "link_quality",
        "fix_type",
        "num_sat",
        "gps_hdop",
    ])
    .map_err(|e| format!("CSV header error: {}", e))?;

    let flight_start = flight.start_time;

    for r in track {
        let time = flight_start + Duration::milliseconds(r.timestamp_ms);
        wtr.write_record(&[
            r.timestamp_ms.to_string(),
            time.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
            opt_f64(r.lat),
            opt_f64(r.lon),
            opt_f64(r.alt_m),
            opt_f64(r.baro_alt_m),
            opt_f64(r.nav_lat),
            opt_f64(r.nav_lon),
            opt_f64(r.nav_alt_m),
            opt_f64(r.speed_ms),
            r.heading.map_or(String::new(), |v| v.to_string()),
            opt_f64(r.vario_ms),
            opt_f64(r.roll),
            opt_f64(r.pitch),
            r.yaw.map_or(String::new(), |v| v.to_string()),
            opt_f64(r.voltage),
            opt_f64(r.current_a),
            r.mah_drawn.map_or(String::new(), |v| v.to_string()),
            r.rssi.map_or(String::new(), |v| v.to_string()),
            r.link_quality.map_or(String::new(), |v| v.to_string()),
            r.fix_type.map_or(String::new(), |v| v.to_string()),
            r.num_sat.map_or(String::new(), |v| v.to_string()),
            opt_f64(r.gps_hdop),
        ])
        .map_err(|e| format!("CSV write error: {}", e))?;
    }

    wtr.flush().map_err(|e| format!("CSV flush error: {}", e))?;
    Ok(())
}

// ── Helpers ─────────────────────────────────────────────────────────

fn flight_name(flight: &Flight) -> String {
    let date = flight.start_time.format("%Y-%m-%d %H:%M");
    let craft = if flight.craft_name.is_empty() {
        "Unknown"
    } else {
        &flight.craft_name
    };
    format!("{} — {}", craft, date)
}

fn flight_description(flight: &Flight) -> String {
    let mut parts = Vec::new();
    if let Some(ref loc) = flight.location_name {
        parts.push(format!("Location: {}", loc));
    }
    if let Some(dur) = flight.duration_sec {
        let m = dur / 60;
        let s = dur % 60;
        parts.push(format!("Duration: {}m {}s", m, s));
    }
    if let Some(alt) = flight.max_alt_m {
        parts.push(format!("Max Alt: {:.0} m", alt));
    }
    if let Some(spd) = flight.max_speed_ms {
        parts.push(format!("Max Speed: {:.1} m/s", spd));
    }
    if let Some(dist) = flight.total_distance_m {
        parts.push(format!("Total Distance: {:.0} m", dist));
    }
    parts.join("\n")
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn opt_f64(v: Option<f64>) -> String {
    v.map_or(String::new(), |f| format!("{:.6}", f))
}
