// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

use tauri::State;

use std::sync::atomic::Ordering;

use crate::weather::service::{WeatherService, WindAloftState};

/// A weather widget came on screen (or the last one left). Returns what the service holds: its
/// event only announces changes.
#[tauri::command]
pub fn weather_watch(service: State<WeatherService>, on: bool) -> WindAloftState {
    service.set_watched(on)
}

/// Position to ask for while no vehicle reports a fix: FC home or the launch marker; none = clear.
#[tauri::command]
pub fn weather_set_reference(service: State<WeatherService>, lat: Option<f64>, lon: Option<f64>) {
    service.set_reference(lat.zip(lon));
}

/// The two Data settings for lookups nobody asked for; pushed at start and on every change.
#[tauri::command]
pub fn flightlog_set_lookups(weather: bool, location: bool) {
    crate::weather::RECORD_WITH_FLIGHTS.store(weather, Ordering::Relaxed);
    crate::flightlog::geocode::AUTO_FLIGHT_LOCATION.store(location, Ordering::Relaxed);
}
