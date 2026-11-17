// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Wind by height from the Open-Meteo forecast model (no key), for the weather widget.
// Fixed heights are above ground; pressure levels carry a geopotential height (MSL), made AGL with the answer's `elevation`.

use serde::Serialize;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};

const OPEN_METEO_URL: &str = "https://api.open-meteo.com/v1/forecast";

/// Heights above ground the model serves directly; a model without one answers `null` (180 m in GFS / ECMWF).
const FIXED_LEVELS_M: [u32; 4] = [10, 80, 120, 180];
/// Pressure levels asked for above them; the low ones lie below ground over high terrain.
const PRESSURE_LEVELS_HPA: [u32; 10] = [975, 950, 925, 900, 850, 800, 750, 700, 650, 600];
/// 15-minute steps per answer: now, the 90 minutes after it, and an hour of margin — the widget
/// reads 90 minutes ahead of the CURRENT step, which moves on while the answer ages.
const STEPS: usize = 11;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindLevel {
    pub height_agl_m: f64,
    pub speed_ms: f64,
    /// Direction the wind comes FROM, degrees.
    pub dir_deg: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindStep {
    /// Start of the 15-minute step, Unix seconds.
    pub time: i64,
    pub weather_code: Option<i32>,
    pub is_day: bool,
    /// Gusts at 10 m.
    pub gust_ms: Option<f64>,
    pub temp_c: Option<f64>,
    /// Ascending by height.
    pub levels: Vec<WindLevel>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindAloft {
    /// Terrain elevation at the requested point, metres MSL.
    pub elevation_m: f64,
    pub steps: Vec<WindStep>,
}

fn variables() -> String {
    let mut v: Vec<String> = vec!["weather_code".into(), "is_day".into(), "wind_gusts_10m".into(), "temperature_2m".into()];
    for h in FIXED_LEVELS_M {
        v.push(format!("wind_speed_{h}m"));
        v.push(format!("wind_direction_{h}m"));
    }
    for p in PRESSURE_LEVELS_HPA {
        v.push(format!("wind_speed_{p}hPa"));
        v.push(format!("wind_direction_{p}hPa"));
        v.push(format!("geopotential_height_{p}hPa"));
    }
    v.join(",")
}

pub async fn fetch_wind_aloft(lat: f64, lon: f64) -> Result<WindAloft, String> {
    let client = reqwest::Client::builder()
        .user_agent(crate::user_agent::user_agent("weather"))
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(OPEN_METEO_URL)
        .query(&[
            ("latitude", format!("{lat:.4}")),
            ("longitude", format!("{lon:.4}")),
            ("wind_speed_unit", "ms".to_string()),
            ("timeformat", "unixtime".to_string()),
            ("forecast_minutely_15", STEPS.to_string()),
            ("minutely_15", variables()),
        ])
        .send()
        .await
        // The error's URL carries the position.
        .map_err(|e| e.without_url().to_string())?;
    let status = resp.status();
    let parsed = if status.is_success() {
        resp.json::<Value>().await.ok().as_ref().and_then(parse)
    } else {
        None
    };
    match parsed {
        Some(w) => {
            REFUSED.store(false, Ordering::Relaxed);
            Ok(w)
        }
        None => {
            // The widget retries every minute: one line per outage.
            if !REFUSED.swap(true, Ordering::Relaxed) {
                log::warn!("weather: the service answered {status} without usable wind data");
            }
            Err(format!("weather service answered {status}"))
        }
    }
}

/// The service answered, but not with data (rate limit, rejected request, changed format).
static REFUSED: AtomicBool = AtomicBool::new(false);

fn parse(body: &Value) -> Option<WindAloft> {
    let elevation_m = body.get("elevation")?.as_f64()?;
    let series = body.get("minutely_15")?;
    let times = series.get("time")?.as_array()?;
    let at = |name: &str, i: usize| series.get(name).and_then(|a| a.get(i)).and_then(Value::as_f64);

    let mut steps = Vec::with_capacity(times.len());
    for (i, t) in times.iter().enumerate() {
        let mut levels = Vec::new();
        for h in FIXED_LEVELS_M {
            if let (Some(speed_ms), Some(dir_deg)) = (at(&format!("wind_speed_{h}m"), i), at(&format!("wind_direction_{h}m"), i)) {
                levels.push(WindLevel { height_agl_m: f64::from(h), speed_ms, dir_deg });
            }
        }
        let top_fixed = levels.last().map_or(0.0, |l| l.height_agl_m);
        let mut upper = Vec::new();
        for p in PRESSURE_LEVELS_HPA {
            if let (Some(speed_ms), Some(dir_deg), Some(gh)) = (
                at(&format!("wind_speed_{p}hPa"), i),
                at(&format!("wind_direction_{p}hPa"), i),
                at(&format!("geopotential_height_{p}hPa"), i),
            ) {
                let height_agl_m = gh - elevation_m;
                if height_agl_m > top_fixed {
                    upper.push(WindLevel { height_agl_m, speed_ms, dir_deg });
                }
            }
        }
        upper.sort_by(|a, b| a.height_agl_m.total_cmp(&b.height_agl_m));
        levels.extend(upper);
        steps.push(WindStep {
            time: t.as_i64()?,
            weather_code: at("weather_code", i).map(|c| c as i32),
            is_day: at("is_day", i).is_none_or(|d| d != 0.0),
            gust_ms: at("wind_gusts_10m", i),
            temp_c: at("temperature_2m", i),
            levels,
        });
    }
    if steps.first().is_none_or(|s| s.levels.is_empty()) {
        return None;
    }
    Some(WindAloft { elevation_m, steps })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_are_above_ground_and_ascending() {
        let body = serde_json::json!({
            "elevation": 400.0,
            "minutely_15": {
                "time": [1000, 1900],
                "weather_code": [3, 61], "is_day": [1, 0], "wind_gusts_10m": [9.5, null], "temperature_2m": [8.0, 7.5],
                "wind_speed_10m": [4.0, 4.1], "wind_direction_10m": [240, 241],
                "wind_speed_80m": [8.0, 8.1], "wind_direction_80m": [244, 245],
                "wind_speed_120m": [9.0, 9.1], "wind_direction_120m": [245, 246],
                "wind_speed_180m": [null, null], "wind_direction_180m": [null, null],
                "wind_speed_975hPa": [3.0, 3.0], "wind_direction_975hPa": [200, 200], "geopotential_height_975hPa": [205.0, 205.0],
                "wind_speed_925hPa": [13.0, 13.0], "wind_direction_925hPa": [263, 263], "geopotential_height_925hPa": [856.0, 856.0],
                "wind_speed_900hPa": [12.0, 12.0], "wind_direction_900hPa": [257, 257], "geopotential_height_900hPa": [635.0, 635.0]
            }
        });
        let w = parse(&body).unwrap();
        assert_eq!(w.steps.len(), 2);
        let h: Vec<f64> = w.steps[0].levels.iter().map(|l| l.height_agl_m).collect();
        // 180 m is null, 975 hPa lies below ground, the two upper levels arrive out of order.
        assert_eq!(h, vec![10.0, 80.0, 120.0, 235.0, 456.0]);
        assert!(w.steps[0].is_day && !w.steps[1].is_day);
        assert_eq!(w.steps[1].gust_ms, None);
    }
}
