// Flight Recording & Logbook Module
// Records telemetry data during flights (arm→disarm) into a SQLite database.
// Supports optional raw text logging, reverse geocoding, and weather metadata.

pub mod db;
pub mod ardupilot;
pub mod blackbox;
pub mod exchange;
pub mod geocode;
pub mod raw_logger;
pub mod recorder;
pub mod track_export;
pub mod types;
pub mod weather;
