// Weather data fetching via Open-Meteo (free, no API key required)
// Fetches current weather conditions at a given GPS position.

use serde::Deserialize;

const OPEN_METEO_URL: &str = "https://api.open-meteo.com/v1/forecast";

#[derive(Debug, Deserialize)]
struct OpenMeteoResponse {
    current_weather: Option<CurrentWeather>,
}

#[derive(Debug, Deserialize)]
struct CurrentWeather {
    temperature: f64,
    windspeed: f64,
    winddirection: f64,
    weathercode: i32,
}

/// Weather snapshot at a location
#[derive(Debug, Clone)]
pub struct WeatherData {
    pub temp_c: f64,
    pub wind_ms: f64,
    pub wind_deg: i32,
    pub description: String,
}

/// Fetch current weather for a GPS position.
/// Returns None on error — never blocks the recorder.
pub async fn fetch_weather(lat: f64, lon: f64) -> Option<WeatherData> {
    let client = reqwest::Client::builder()
        .user_agent("KiteGC/0.3.0")
        .build()
        .ok()?;

    let resp = client
        .get(OPEN_METEO_URL)
        .query(&[
            ("latitude", lat.to_string()),
            ("longitude", lon.to_string()),
            ("current_weather", "true".to_string()),
            ("windspeed_unit", "ms".to_string()),
        ])
        .send()
        .await
        .ok()?;

    let data: OpenMeteoResponse = resp.json().await.ok()?;
    let cw = data.current_weather?;

    Some(WeatherData {
        temp_c: cw.temperature,
        wind_ms: cw.windspeed,
        wind_deg: cw.winddirection as i32,
        description: wmo_code_to_description(cw.weathercode),
    })
}

/// Convert WMO weather interpretation code to a human-readable description
fn wmo_code_to_description(code: i32) -> String {
    match code {
        0 => "Clear sky",
        1 => "Mainly clear",
        2 => "Partly cloudy",
        3 => "Overcast",
        45 => "Fog",
        48 => "Depositing rime fog",
        51 => "Light drizzle",
        53 => "Moderate drizzle",
        55 => "Dense drizzle",
        56 => "Light freezing drizzle",
        57 => "Dense freezing drizzle",
        61 => "Slight rain",
        63 => "Moderate rain",
        65 => "Heavy rain",
        66 => "Light freezing rain",
        67 => "Heavy freezing rain",
        71 => "Slight snow fall",
        73 => "Moderate snow fall",
        75 => "Heavy snow fall",
        77 => "Snow grains",
        80 => "Slight rain showers",
        81 => "Moderate rain showers",
        82 => "Violent rain showers",
        85 => "Slight snow showers",
        86 => "Heavy snow showers",
        95 => "Thunderstorm",
        96 => "Thunderstorm with slight hail",
        99 => "Thunderstorm with heavy hail",
        _ => "Unknown",
    }
    .to_string()
}
