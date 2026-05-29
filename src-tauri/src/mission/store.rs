// Mission Store — in-memory mission state, exposed as Tauri managed state.
// Handles mission download/upload via the scheduler, and XML file I/O.

use std::sync::Mutex;

use super::types::{Mission, MissionInfo, Waypoint, WpAction, WP_FLAG_LAST, WP_FLAG_NORMAL};
use super::codec;

/// Thread-safe mission store managed by Tauri
pub struct MissionStore {
    inner: Mutex<Mission>,
}

impl MissionStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Mission::new()),
        }
    }

    /// Get a snapshot of the current mission
    pub fn snapshot(&self) -> Mission {
        self.inner.lock().unwrap().clone()
    }

    /// Replace the entire mission (e.g. after download from FC or file load)
    pub fn set(&self, mission: Mission) {
        *self.inner.lock().unwrap() = mission;
    }

    /// Clear the mission
    pub fn clear(&self) {
        self.inner.lock().unwrap().clear();
    }

    /// Add a waypoint
    pub fn push(&self, wp: Waypoint) {
        self.inner.lock().unwrap().push(wp);
    }

    /// Remove waypoint at index
    pub fn remove(&self, index: usize) {
        self.inner.lock().unwrap().remove(index);
    }

    /// Insert waypoint at index
    pub fn insert(&self, index: usize, wp: Waypoint) {
        self.inner.lock().unwrap().insert(index, wp);
    }

    /// Update waypoint at index
    pub fn update(&self, index: usize, wp: Waypoint) {
        self.inner.lock().unwrap().update(index, wp);
    }

    /// Reorder waypoint
    pub fn reorder(&self, from: usize, to: usize) {
        self.inner.lock().unwrap().reorder(from, to);
    }

    /// Set mission info (from MSP_WP_GETINFO)
    pub fn set_info(&self, info: MissionInfo) {
        self.inner.lock().unwrap().info = info;
    }

    /// Mark mission as clean (after successful upload or save)
    pub fn mark_clean(&self) {
        self.inner.lock().unwrap().dirty = false;
    }
}

// ── XML Mission File I/O ────────────────────────────────────────────
// MW XML format — interoperable with INAV Configurator, mwp, ezgui.

/// Serialize mission to MW XML format string
pub fn mission_to_xml(mission: &Mission) -> String {
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<mission>\n");
    xml.push_str("  <version value=\"2.3-pre8\"/>\n");

    // Planned home/launch point — mwp-compatible <mwp> meta (x=lon, y=lat).
    // Other tools (INAV Configurator) ignore this element and read only <missionitem>.
    if let Some(h) = &mission.home {
        xml.push_str(&format!(
            "  <mwp save-date=\"{}\" zoom=\"14\" cx=\"{:.7}\" cy=\"{:.7}\" generator=\"Kite-GC\" home-x=\"{:.7}\" home-y=\"{:.7}\"/>\n",
            chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S"),
            h.lon, h.lat, h.lon, h.lat,
        ));
    }

    for wp in &mission.waypoints {
        xml.push_str(&format!(
            "  <missionitem no=\"{}\" action=\"{}\" lat=\"{:.7}\" lon=\"{:.7}\" alt=\"{}\" parameter1=\"{}\" parameter2=\"{}\" parameter3=\"{}\" flag=\"{}\"/>\n",
            wp.number,
            wp.action.xml_name(),
            wp.lat as f64 / 1e7,
            wp.lon as f64 / 1e7,
            wp.altitude / 100, // stored as cm, saved as m in XML
            wp.p1,
            wp.p2,
            wp.p3,
            wp.flag as u16,
        ));
    }

    xml.push_str("</mission>\n");
    xml
}

/// Parse MW XML format string into a Mission
pub fn mission_from_xml(xml_str: &str) -> Result<Mission, String> {
    let mut mission = Mission::new();
    mission.dirty = false;

    // Simple XML parser — we don't pull in a full XML crate for this flat format.
    // Matches <missionitem ... /> elements and extracts attributes.
    for line in xml_str.lines() {
        let trimmed = line.trim();
        // Planned home/launch point from the mwp-compatible <mwp home-x/home-y> meta
        if trimmed.starts_with("<mwp ") {
            let (mut hx, mut hy): (Option<f64>, Option<f64>) = (None, None);
            for attr in parse_xml_attrs(trimmed) {
                match attr.0.as_str() {
                    "home-x" => hx = attr.1.parse().ok(),
                    "home-y" => hy = attr.1.parse().ok(),
                    _ => {}
                }
            }
            if let (Some(x), Some(y)) = (hx, hy) {
                mission.home = Some(super::types::HomePt { lat: y, lon: x });
            }
            continue;
        }
        if !trimmed.starts_with("<missionitem ") {
            continue;
        }

        let mut number: u8 = 0;
        let mut action = WpAction::Waypoint;
        let mut lat: i32 = 0;
        let mut lon: i32 = 0;
        let mut altitude: i32 = 0;
        let mut p1: i16 = 0;
        let mut p2: i16 = 0;
        let mut p3: i16 = 0;
        let mut flag: u8 = WP_FLAG_NORMAL;

        for attr in parse_xml_attrs(trimmed) {
            match attr.0.as_str() {
                "no" => number = attr.1.parse().unwrap_or(0),
                "action" => {
                    action = WpAction::from_xml_name(&attr.1)
                        .ok_or_else(|| format!("Unknown action: {}", attr.1))?;
                }
                "lat" => {
                    let v: f64 = attr.1.parse().map_err(|e| format!("Invalid lat: {}", e))?;
                    lat = (v * 1e7).round() as i32;
                }
                "lon" => {
                    let v: f64 = attr.1.parse().map_err(|e| format!("Invalid lon: {}", e))?;
                    lon = (v * 1e7).round() as i32;
                }
                "alt" => {
                    let v: i32 = attr.1.parse().map_err(|e| format!("Invalid alt: {}", e))?;
                    altitude = v * 100; // XML stores metres, we store cm
                }
                "parameter1" => p1 = attr.1.parse().unwrap_or(0),
                "parameter2" => p2 = attr.1.parse().unwrap_or(0),
                "parameter3" => p3 = attr.1.parse().unwrap_or(0),
                "flag" => flag = attr.1.parse().unwrap_or(0),
                _ => {}
            }
        }

        mission.waypoints.push(Waypoint {
            number,
            action,
            lat,
            lon,
            altitude,
            p1,
            p2,
            p3,
            flag,
            // .mission only encodes REL/AMSL (p3 bit0); derive the GCS alt mode.
            alt_mode: if (p3 as u16) & super::types::P3_ALT_TYPE != 0 {
                super::types::ALT_MODE_AMSL
            } else {
                super::types::ALT_MODE_REL
            },
        });
    }

    // If no explicit flag was set, mark the last WP
    if let Some(last) = mission.waypoints.last_mut() {
        if last.flag == WP_FLAG_NORMAL {
            last.flag = WP_FLAG_LAST;
        }
    }

    Ok(mission)
}

/// Extract key="value" pairs from an XML element string
fn parse_xml_attrs(element: &str) -> Vec<(String, String)> {
    let mut attrs = Vec::new();
    let mut rest = element;

    loop {
        // Find next key="value"
        let eq_pos = match rest.find('=') {
            Some(p) => p,
            None => break,
        };

        // Extract key (word before =)
        let key_start = rest[..eq_pos]
            .rfind(|c: char| c.is_whitespace())
            .map(|p| p + 1)
            .unwrap_or(0);
        let key = rest[key_start..eq_pos].trim().to_string();

        // Extract value (between quotes after =)
        let after_eq = &rest[eq_pos + 1..];
        let quote_char = match after_eq.chars().find(|&c| c == '"' || c == '\'') {
            Some(c) => c,
            None => break,
        };
        let val_start = match after_eq.find(quote_char) {
            Some(p) => p + 1,
            None => break,
        };
        let val_end = match after_eq[val_start..].find(quote_char) {
            Some(p) => val_start + p,
            None => break,
        };
        let value = after_eq[val_start..val_end].to_string();

        attrs.push((key, value));
        rest = &after_eq[val_end + 1..];
    }

    attrs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_roundtrip() {
        let mut mission = Mission::new();
        mission.push(Waypoint::new(0, WpAction::Waypoint, 540353000, -45170000, 3500));
        mission.waypoints[0].p1 = 200;
        mission.push(Waypoint::new(0, WpAction::PosholdTime, 540360000, -45180000, 5000));
        mission.waypoints[1].p1 = 30; // 30 seconds
        mission.push(Waypoint::new(0, WpAction::Rth, 0, 0, 0));
        mission.waypoints[2].p1 = 1; // land

        let xml = mission_to_xml(&mission);
        assert!(xml.contains("WAYPOINT"));
        assert!(xml.contains("POSHOLD_TIME"));
        assert!(xml.contains("RTH"));

        let parsed = mission_from_xml(&xml).unwrap();
        assert_eq!(parsed.waypoints.len(), 3);
        assert_eq!(parsed.waypoints[0].action, WpAction::Waypoint);
        assert_eq!(parsed.waypoints[0].p1, 200);
        assert_eq!(parsed.waypoints[1].action, WpAction::PosholdTime);
        assert_eq!(parsed.waypoints[1].p1, 30);
        assert_eq!(parsed.waypoints[2].action, WpAction::Rth);
        // lat/lon roundtrip: within 1e-7 degree precision
        assert!((parsed.waypoints[0].lat - 540353000).abs() <= 1);
    }

    #[test]
    fn parse_xml_attrs_basic() {
        let elem = r#"<missionitem no="1" action="WAYPOINT" lat="54.0353000" lon="-4.5170000"/>"#;
        let attrs = parse_xml_attrs(elem);
        assert!(attrs.iter().any(|(k, v)| k == "no" && v == "1"));
        assert!(attrs.iter().any(|(k, v)| k == "action" && v == "WAYPOINT"));
    }

    #[test]
    fn empty_mission_xml() {
        let mission = Mission::new();
        let xml = mission_to_xml(&mission);
        assert!(xml.contains("<mission>"));
        assert!(xml.contains("</mission>"));

        let parsed = mission_from_xml(&xml).unwrap();
        assert!(parsed.waypoints.is_empty());
    }
}
