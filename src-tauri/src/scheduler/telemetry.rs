// Telemetry decoding and configuration
// Decodes raw MSP payloads into structured telemetry events.

use serde::{Deserialize, Serialize};

use crate::msp::*;

use crate::flightlog::recorder::FlightRecorder;

/// Telemetry poll group identifiers
#[derive(Debug, Clone, PartialEq)]
pub enum TelemetryGroup {
    Attitude,
    Analog,
    PositionPrimary,
    PositionSecondary,
    Status,
}

/// User-configurable telemetry polling rates
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryConfig {
    /// Attitude poll rate in Hz (1.0–5.0), default 5.0
    pub attitude_rate_hz: f64,
    /// Position primary poll rate in Hz (1.0–5.0), default 2.0
    pub position_rate_hz: f64,
    /// Enable airspeed polling (requires pitot sensor)
    pub airspeed_enabled: bool,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            attitude_rate_hz: 5.0,
            position_rate_hz: 2.0,
            airspeed_enabled: false,
        }
    }
}

// ── Telemetry event payloads ─────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct AttitudeData {
    pub roll: f64,  // degrees, ±180
    pub pitch: f64, // degrees, ±90
    pub yaw: i16,   // heading 0–360
}

#[derive(Debug, Clone, Serialize)]
pub struct GpsData {
    pub fix_type: u8,
    pub num_sat: u8,
    pub lat: f64,         // decimal degrees
    pub lon: f64,         // decimal degrees
    pub alt_msl: f64,     // meters
    pub ground_speed: f64, // cm/s → m/s
    pub course: f64,      // degrees * 10 → degrees
}

#[derive(Debug, Clone, Serialize)]
pub struct AltitudeData {
    pub altitude: f64, // cm → meters
    pub vario: f64,    // cm/s → m/s
}

#[derive(Debug, Clone, Serialize)]
pub struct AnalogData {
    pub voltage: f64,     // V (0.01V units → V)
    pub mah_drawn: u32,
    pub rssi: u16,
    pub current: f64,     // A (0.01A units → A)
    pub power: f64,       // W (0.01W units → W)
    pub battery_percentage: u8,
    pub cell_count: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusData {
    pub arming_flags: u32,
    pub flight_mode_flags: Vec<u32>,
    pub cpu_load: u16,
    pub sensor_status: u16,
}

#[derive(Debug, Clone, Serialize)]
pub struct SensorStatusData {
    pub gyro: u8,        // hardwareSensorStatus_e: 0=NONE, 1=OK, 2=UNAVAILABLE, 3=UNHEALTHY
    pub acc: u8,
    pub mag: u8,
    pub baro: u8,
    pub gps: u8,
    pub rangefinder: u8,
    pub pitot: u8,
    pub opflow: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct AirspeedData {
    pub airspeed: f64, // cm/s → m/s
}

/// Generic telemetry payload wrapper for Tauri events
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum TelemetryPayload {
    Attitude(AttitudeData),
    Gps(GpsData),
    Altitude(AltitudeData),
    Analog(AnalogData),
    Status(StatusData),
    SensorStatus(SensorStatusData),
    Airspeed(AirspeedData),
}

/// Map MSP code to Tauri event name
pub fn event_name_for_code(code: u16) -> String {
    match code {
        MSP_ATTITUDE => "telemetry-attitude".into(),
        MSP_RAW_GPS => "telemetry-gps".into(),
        MSP_ALTITUDE => "telemetry-altitude".into(),
        MSPV2_INAV_ANALOG => "telemetry-analog".into(),
        MSPV2_INAV_STATUS => "telemetry-status".into(),
        MSP_SENSOR_STATUS => "telemetry-sensor-status".into(),
        MSPV2_INAV_AIR_SPEED => "telemetry-airspeed".into(),
        _ => format!("telemetry-0x{:04X}", code),
    }
}

/// Decode a raw MSP payload into a structured telemetry event
pub fn decode_telemetry(code: u16, payload: &[u8]) -> TelemetryPayload {
    match code {
        MSP_ATTITUDE => decode_attitude(payload),
        MSP_RAW_GPS => decode_gps(payload),
        MSP_ALTITUDE => decode_altitude(payload),
        MSPV2_INAV_ANALOG => decode_analog(payload),
        MSPV2_INAV_STATUS => decode_status(payload),
        MSP_SENSOR_STATUS => decode_sensor_status(payload),
        MSPV2_INAV_AIR_SPEED => decode_airspeed(payload),
        _ => {
            log::warn!("No decoder for MSP 0x{:04X}", code);
            TelemetryPayload::Attitude(AttitudeData {
                roll: 0.0,
                pitch: 0.0,
                yaw: 0,
            })
        }
    }
}

// ── Decoders ─────────────────────────────────────────────────────────

fn read_i16(buf: &[u8], offset: usize) -> i16 {
    if offset + 1 < buf.len() {
        i16::from_le_bytes([buf[offset], buf[offset + 1]])
    } else {
        0
    }
}

fn read_u16(buf: &[u8], offset: usize) -> u16 {
    if offset + 1 < buf.len() {
        u16::from_le_bytes([buf[offset], buf[offset + 1]])
    } else {
        0
    }
}

fn read_i32(buf: &[u8], offset: usize) -> i32 {
    if offset + 3 < buf.len() {
        i32::from_le_bytes([buf[offset], buf[offset + 1], buf[offset + 2], buf[offset + 3]])
    } else {
        0
    }
}

fn read_u32(buf: &[u8], offset: usize) -> u32 {
    if offset + 3 < buf.len() {
        u32::from_le_bytes([buf[offset], buf[offset + 1], buf[offset + 2], buf[offset + 3]])
    } else {
        0
    }
}

/// MSP_ATTITUDE (108): [roll:i16, pitch:i16, yaw:i16] — tenths of degrees
fn decode_attitude(payload: &[u8]) -> TelemetryPayload {
    TelemetryPayload::Attitude(AttitudeData {
        roll: read_i16(payload, 0) as f64 / 10.0,
        pitch: read_i16(payload, 2) as f64 / 10.0,
        yaw: read_i16(payload, 4),
    })
}

/// MSP_RAW_GPS (106): [fixType:u8, numSat:u8, lat:i32, lon:i32, alt:i16, speed:u16, cog:u16]
fn decode_gps(payload: &[u8]) -> TelemetryPayload {
    let fix_type = if !payload.is_empty() { payload[0] } else { 0 };
    let num_sat = if payload.len() > 1 { payload[1] } else { 0 };

    TelemetryPayload::Gps(GpsData {
        fix_type,
        num_sat,
        lat: read_i32(payload, 2) as f64 / 1e7,
        lon: read_i32(payload, 6) as f64 / 1e7,
        alt_msl: read_i16(payload, 10) as f64, // meters
        ground_speed: read_u16(payload, 12) as f64 / 100.0, // cm/s → m/s
        course: read_u16(payload, 14) as f64 / 10.0, // decidegrees → degrees
    })
}

/// MSP_ALTITUDE (109): [altitude:i32, vario:i16] — cm, cm/s
fn decode_altitude(payload: &[u8]) -> TelemetryPayload {
    TelemetryPayload::Altitude(AltitudeData {
        altitude: read_i32(payload, 0) as f64 / 100.0, // cm → m
        vario: read_i16(payload, 4) as f64 / 100.0,    // cm/s → m/s
    })
}

/// MSPV2_INAV_ANALOG (0x2002): Extended battery/power data
/// Layout: [batteryFlags:u8, vbat:u16, amperage:i16, powerDraw:u32,
///          mAhDrawn:u32, mWhDrawn:u32, remainingCapacity:u32,
///          percentageRemaining:u8, rssi:u16]
fn decode_analog(payload: &[u8]) -> TelemetryPayload {
    let battery_flags = if !payload.is_empty() { payload[0] } else { 0 };
    let cell_count = (battery_flags >> 4) & 0x0F;

    TelemetryPayload::Analog(AnalogData {
        voltage: read_u16(payload, 1) as f64 / 100.0,     // 0.01V → V
        current: read_i16(payload, 3) as f64 / 100.0,     // 0.01A → A
        power: read_u32(payload, 5) as f64 / 100.0,       // 0.01W → W
        mah_drawn: read_u32(payload, 9),                    // mAh
        battery_percentage: if payload.len() > 21 { payload[21] } else { 0 },
        rssi: read_u16(payload, 22),
        cell_count,
    })
}

/// MSPV2_INAV_STATUS (0x2000): comprehensive FC status
/// Layout: [cycleTime:u16, i2cErrors:u16, sensorStatus:u16, cpuLoad:u16,
///          profileAndBattProfile:u8, armingFlags:u32, activeModes:..., mixerProfile:u8]
fn decode_status(payload: &[u8]) -> TelemetryPayload {
    let sensor_status = read_u16(payload, 4);
    let cpu_load = read_u16(payload, 6);
    let arming_flags = read_u32(payload, 9);

    TelemetryPayload::Status(StatusData {
        arming_flags,
        flight_mode_flags: vec![], // TODO: parse full flight mode bitmask
        cpu_load,
        sensor_status,
    })
}

/// MSP_SENSOR_STATUS (151): per-sensor hardware health
/// Layout: [overallHealth:u8, gyro:u8, acc:u8, mag:u8, baro:u8,
///          gps:u8, rangefinder:u8, pitot:u8, opflow:u8]
/// Values: 0=NONE, 1=OK, 2=UNAVAILABLE, 3=UNHEALTHY
fn decode_sensor_status(payload: &[u8]) -> TelemetryPayload {
    TelemetryPayload::SensorStatus(SensorStatusData {
        gyro: if payload.len() > 1 { payload[1] } else { 0 },
        acc: if payload.len() > 2 { payload[2] } else { 0 },
        mag: if payload.len() > 3 { payload[3] } else { 0 },
        baro: if payload.len() > 4 { payload[4] } else { 0 },
        gps: if payload.len() > 5 { payload[5] } else { 0 },
        rangefinder: if payload.len() > 6 { payload[6] } else { 0 },
        pitot: if payload.len() > 7 { payload[7] } else { 0 },
        opflow: if payload.len() > 8 { payload[8] } else { 0 },
    })
}

/// MSPV2_INAV_AIR_SPEED (0x2009): [airspeed:i32] — cm/s
fn decode_airspeed(payload: &[u8]) -> TelemetryPayload {
    TelemetryPayload::Airspeed(AirspeedData {
        airspeed: read_i32(payload, 0) as f64 / 100.0, // cm/s → m/s
    })
}

/// Feed decoded telemetry data to the flight recorder.
/// Decodes the raw payload again (cheap) to pass typed data to the recorder.
pub fn feed_recorder(code: u16, payload: &[u8], recorder: &mut FlightRecorder) {
    match code {
        MSP_ATTITUDE => {
            let data = AttitudeData {
                roll: read_i16(payload, 0) as f64 / 10.0,
                pitch: read_i16(payload, 2) as f64 / 10.0,
                yaw: read_i16(payload, 4),
            };
            recorder.on_attitude(&data);
        }
        MSP_RAW_GPS => {
            let data = GpsData {
                fix_type: if !payload.is_empty() { payload[0] } else { 0 },
                num_sat: if payload.len() > 1 { payload[1] } else { 0 },
                lat: read_i32(payload, 2) as f64 / 1e7,
                lon: read_i32(payload, 6) as f64 / 1e7,
                alt_msl: read_i16(payload, 10) as f64,
                ground_speed: read_u16(payload, 12) as f64 / 100.0,
                course: read_u16(payload, 14) as f64 / 10.0,
            };
            recorder.on_gps(&data);
        }
        MSP_ALTITUDE => {
            let data = AltitudeData {
                altitude: read_i32(payload, 0) as f64 / 100.0,
                vario: read_i16(payload, 4) as f64 / 100.0,
            };
            recorder.on_altitude(&data);
        }
        MSPV2_INAV_ANALOG => {
            let data = AnalogData {
                voltage: read_u16(payload, 1) as f64 / 100.0,
                current: read_i16(payload, 3) as f64 / 100.0,
                power: read_u32(payload, 5) as f64 / 100.0,
                mah_drawn: read_u32(payload, 9),
                battery_percentage: if payload.len() > 21 { payload[21] } else { 0 },
                rssi: read_u16(payload, 22),
                cell_count: if !payload.is_empty() { (payload[0] >> 4) & 0x0F } else { 0 },
            };
            recorder.on_analog(&data);
        }
        MSPV2_INAV_STATUS => {
            let data = StatusData {
                arming_flags: read_u32(payload, 9),
                flight_mode_flags: vec![],
                cpu_load: read_u16(payload, 6),
                sensor_status: read_u16(payload, 4),
            };
            recorder.on_status(&data);
        }
        MSPV2_INAV_AIR_SPEED => {
            let data = AirspeedData {
                airspeed: read_i32(payload, 0) as f64 / 100.0,
            };
            recorder.on_airspeed(&data);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_attitude() {
        // roll=45.0° (450), pitch=-10.5° (-105), yaw=270
        let payload = [
            0xC2, 0x01, // 450 → 45.0°
            0x97, 0xFF, // -105 → -10.5°
            0x0E, 0x01, // 270
        ];
        match decode_telemetry(MSP_ATTITUDE, &payload) {
            TelemetryPayload::Attitude(a) => {
                assert!((a.roll - 45.0).abs() < 0.01);
                assert!((a.pitch - (-10.5)).abs() < 0.01);
                assert_eq!(a.yaw, 270);
            }
            _ => panic!("Expected Attitude"),
        }
    }

    #[test]
    fn test_decode_gps() {
        // fixType=2, numSat=12, lat=51.505*1e7, lon=-0.09*1e7, alt=100m, speed=500cm/s, cog=1800
        let mut payload = vec![2u8, 12];
        payload.extend_from_slice(&515050000i32.to_le_bytes()); // lat
        payload.extend_from_slice(&(-900000i32).to_le_bytes()); // lon
        payload.extend_from_slice(&100i16.to_le_bytes());       // alt
        payload.extend_from_slice(&500u16.to_le_bytes());       // speed
        payload.extend_from_slice(&1800u16.to_le_bytes());      // cog

        match decode_telemetry(MSP_RAW_GPS, &payload) {
            TelemetryPayload::Gps(g) => {
                assert_eq!(g.fix_type, 2);
                assert_eq!(g.num_sat, 12);
                assert!((g.lat - 51.505).abs() < 0.001);
                assert!((g.lon - (-0.09)).abs() < 0.001);
                assert_eq!(g.alt_msl as i16, 100);
                assert!((g.ground_speed - 5.0).abs() < 0.01);
                assert!((g.course - 180.0).abs() < 0.01);
            }
            _ => panic!("Expected Gps"),
        }
    }

    #[test]
    fn test_decode_altitude() {
        // altitude=15000cm (150m), vario=250cm/s (2.5m/s)
        let mut payload = Vec::new();
        payload.extend_from_slice(&15000i32.to_le_bytes());
        payload.extend_from_slice(&250i16.to_le_bytes());

        match decode_telemetry(MSP_ALTITUDE, &payload) {
            TelemetryPayload::Altitude(a) => {
                assert!((a.altitude - 150.0).abs() < 0.01);
                assert!((a.vario - 2.5).abs() < 0.01);
            }
            _ => panic!("Expected Altitude"),
        }
    }

    #[test]
    fn test_event_names() {
        assert_eq!(event_name_for_code(MSP_ATTITUDE), "telemetry-attitude");
        assert_eq!(event_name_for_code(MSP_RAW_GPS), "telemetry-gps");
        assert_eq!(event_name_for_code(MSPV2_INAV_ANALOG), "telemetry-analog");
        assert_eq!(event_name_for_code(0xFFFF), "telemetry-0xFFFF");
    }

    #[test]
    fn test_default_config() {
        let config = TelemetryConfig::default();
        assert_eq!(config.attitude_rate_hz, 5.0);
        assert_eq!(config.position_rate_hz, 2.0);
        assert!(!config.airspeed_enabled);
    }

    #[test]
    fn test_decode_analog_v2() {
        // MSP2_INAV_ANALOG (0x2002) — correct format with batteryFlags at offset 0
        // batteryFlags=0x31 (cellCount=3 in bits 4-7, state in bits 2-3)
        // vbat=1164 (11.64V), amperage=1994 (19.94A), powerDraw=23200 (232.00W)
        // mAhDrawn=500, mWhDrawn=5820, remainingCapacity=4500
        // percentageRemaining=90, rssi=512
        let mut payload = vec![0x31u8]; // batteryFlags: 3 cells, state bits
        payload.extend_from_slice(&1164u16.to_le_bytes());    // vbat = 11.64V
        payload.extend_from_slice(&1994i16.to_le_bytes());    // amperage = 19.94A
        payload.extend_from_slice(&23200u32.to_le_bytes());   // powerDraw = 232.00W
        payload.extend_from_slice(&500u32.to_le_bytes());     // mAhDrawn
        payload.extend_from_slice(&5820u32.to_le_bytes());    // mWhDrawn
        payload.extend_from_slice(&4500u32.to_le_bytes());    // remainingCapacity
        payload.push(90);                                       // percentageRemaining
        payload.extend_from_slice(&512u16.to_le_bytes());     // rssi

        match decode_telemetry(MSPV2_INAV_ANALOG, &payload) {
            TelemetryPayload::Analog(a) => {
                assert!((a.voltage - 11.64).abs() < 0.01, "voltage: {}", a.voltage);
                assert!((a.current - 19.94).abs() < 0.01, "current: {}", a.current);
                assert!((a.power - 232.0).abs() < 0.01, "power: {}", a.power);
                assert_eq!(a.mah_drawn, 500);
                assert_eq!(a.battery_percentage, 90);
                assert_eq!(a.rssi, 512);
                assert_eq!(a.cell_count, 3);
            }
            _ => panic!("Expected Analog"),
        }
    }

    #[test]
    fn test_decode_status_v2() {
        // MSP2_INAV_STATUS (0x2000) — correct offsets
        let mut payload = Vec::new();
        payload.extend_from_slice(&500u16.to_le_bytes());    // cycleTime
        payload.extend_from_slice(&0u16.to_le_bytes());      // i2cErrors
        payload.extend_from_slice(&0x000Fu16.to_le_bytes()); // sensorStatus (ACC+BARO+MAG+GPS)
        payload.extend_from_slice(&25u16.to_le_bytes());     // cpuLoad
        payload.push(0x00);                                    // profileAndBattProfile
        payload.extend_from_slice(&4u32.to_le_bytes());      // armingFlags (bit 2 = ARMED)

        match decode_telemetry(MSPV2_INAV_STATUS, &payload) {
            TelemetryPayload::Status(s) => {
                assert_eq!(s.cpu_load, 25);
                assert_eq!(s.arming_flags, 4); // ARMED
                assert_eq!(s.sensor_status, 0x000F);
            }
            _ => panic!("Expected Status"),
        }
    }

    #[test]
    fn test_decode_sensor_status() {
        // MSP_SENSOR_STATUS (151)
        let payload = [
            1, // overallHealth = OK
            1, // gyro = OK
            1, // acc = OK
            1, // mag = OK
            1, // baro = OK
            1, // gps = OK
            0, // rangefinder = NONE
            0, // pitot = NONE
            0, // opflow = NONE
        ];
        match decode_telemetry(MSP_SENSOR_STATUS, &payload) {
            TelemetryPayload::SensorStatus(s) => {
                assert_eq!(s.gyro, 1);
                assert_eq!(s.acc, 1);
                assert_eq!(s.mag, 1);
                assert_eq!(s.baro, 1);
                assert_eq!(s.gps, 1);
                assert_eq!(s.rangefinder, 0);
                assert_eq!(s.pitot, 0);
                assert_eq!(s.opflow, 0);
            }
            _ => panic!("Expected SensorStatus"),
        }
    }
}
