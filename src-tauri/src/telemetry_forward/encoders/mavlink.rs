// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! MAVLink encoder — re-encodes the unified telemetry into standard MAVLink v2 messages so any MAVLink
//! GCS (Mission Planner, QGroundControl, …) can consume the relayed stream. Uses the same `rust-mavlink`
//! crate + `serialize_v2` framing as the inbound MAVLink path.
//!
//! Per pacer tick we emit ATTITUDE / GPS_RAW_INT / GLOBAL_POSITION_INT / VFR_HUD / SYS_STATUS for the data
//! present in the cache, plus a HEARTBEAT throttled to ~1 Hz. We have no real vehicle type / custom flight
//! mode in the unified model, so HEARTBEAT advertises a generic type + armed flag (custom_mode = 0).

use std::time::Instant;

use ::mavlink::ardupilotmega::{
    MavAutopilot, MavMessage, MavModeFlag, MavState, MavSysStatusSensor, MavType, GpsFixType,
    ATTITUDE_DATA, GLOBAL_POSITION_INT_DATA, GPS_RAW_INT_DATA, HEARTBEAT_DATA, SYS_STATUS_DATA,
    VFR_HUD_DATA,
};
use ::mavlink::MavHeader;

use super::super::cache::TelemetryCache;
use super::Encoder;
use crate::mavlink_proto::codec::{serialize_v2, MavSequence};

const ARMED_FLAG: u32 = 0x04;
const HEARTBEAT_INTERVAL_MS: u128 = 1000;

pub struct MavlinkEncoder {
    header: MavHeader,
    seq: MavSequence,
    start: Instant,
    last_heartbeat: Option<Instant>,
}

impl MavlinkEncoder {
    pub fn new() -> Self {
        Self {
            header: MavHeader { system_id: 1, component_id: 1, sequence: 0 },
            seq: MavSequence::new(),
            start: Instant::now(),
            last_heartbeat: None,
        }
    }

    fn emit(&mut self, msg: &MavMessage, out: &mut Vec<u8>) {
        out.extend_from_slice(&serialize_v2(&self.header, msg, &mut self.seq));
    }
}

impl Encoder for MavlinkEncoder {
    fn frame_set(&mut self, cache: &TelemetryCache) -> Vec<u8> {
        let mut out = Vec::with_capacity(128);
        let boot_ms = self.start.elapsed().as_millis() as u32;
        let armed = cache.status.as_ref().map_or(false, |s| s.arming_flags & ARMED_FLAG != 0);

        // HEARTBEAT, throttled to ~1 Hz (the pacer ticks faster).
        let due = self.last_heartbeat.map_or(true, |t| t.elapsed().as_millis() >= HEARTBEAT_INTERVAL_MS);
        if due {
            self.last_heartbeat = Some(Instant::now());
            let msg = MavMessage::HEARTBEAT(HEARTBEAT_DATA {
                custom_mode: 0,
                mavtype: MavType::MAV_TYPE_GENERIC,
                autopilot: MavAutopilot::MAV_AUTOPILOT_GENERIC,
                base_mode: if armed { MavModeFlag::MAV_MODE_FLAG_SAFETY_ARMED } else { MavModeFlag::empty() },
                system_status: if armed { MavState::MAV_STATE_ACTIVE } else { MavState::MAV_STATE_STANDBY },
                mavlink_version: 3,
            });
            self.emit(&msg, &mut out);
        }

        if let Some(a) = cache.attitude.as_ref() {
            let msg = MavMessage::ATTITUDE(ATTITUDE_DATA {
                time_boot_ms: boot_ms,
                roll: a.roll.to_radians() as f32,
                // MAVLink: pitch positive = nose UP; our (INAV) pitch positive = nose DOWN → negate.
                pitch: (-a.pitch).to_radians() as f32,
                yaw: a.yaw.to_radians() as f32,
                ..Default::default()
            });
            self.emit(&msg, &mut out);
        }

        if let Some(g) = cache.gps.as_ref() {
            let fix = match g.fix_type {
                3 => GpsFixType::GPS_FIX_TYPE_3D_FIX,
                2 => GpsFixType::GPS_FIX_TYPE_2D_FIX,
                _ => GpsFixType::GPS_FIX_TYPE_NO_GPS,
            };
            let lat = (g.lat * 1e7).round() as i32;
            let lon = (g.lon * 1e7).round() as i32;
            let alt_mm = (g.alt_msl * 1000.0).round() as i32;
            self.emit(
                &MavMessage::GPS_RAW_INT(GPS_RAW_INT_DATA {
                    time_usec: self.start.elapsed().as_micros() as u64,
                    lat,
                    lon,
                    alt: alt_mm,
                    eph: u16::MAX,
                    epv: u16::MAX,
                    vel: (g.ground_speed * 100.0).round().clamp(0.0, 65535.0) as u16,
                    cog: (g.course * 100.0).round().rem_euclid(36000.0) as u16,
                    fix_type: fix,
                    satellites_visible: g.num_sat,
                    ..Default::default()
                }),
                &mut out,
            );

            // GLOBAL_POSITION_INT: NED velocity from groundspeed+course, vz from vario; heading from yaw.
            let cog_rad = g.course.to_radians();
            let vx = (g.ground_speed * cog_rad.cos() * 100.0).round() as i16;
            let vy = (g.ground_speed * cog_rad.sin() * 100.0).round() as i16;
            let vz = cache.altitude.as_ref().map_or(0, |al| (-al.vario * 100.0).round() as i16);
            let rel_alt = cache.altitude.as_ref().map_or(alt_mm, |al| (al.altitude * 1000.0).round() as i32);
            let hdg = cache.attitude.as_ref().map_or(u16::MAX, |a| (a.yaw * 100.0).round().rem_euclid(36000.0) as u16);
            self.emit(
                &MavMessage::GLOBAL_POSITION_INT(GLOBAL_POSITION_INT_DATA {
                    time_boot_ms: boot_ms,
                    lat,
                    lon,
                    alt: alt_mm,
                    relative_alt: rel_alt,
                    vx,
                    vy,
                    vz,
                    hdg,
                }),
                &mut out,
            );
        }

        // VFR_HUD when we have any of speed / heading / altitude.
        if cache.attitude.is_some() || cache.gps.is_some() || cache.altitude.is_some() {
            let gs = cache.gps.as_ref().map_or(0.0, |g| g.ground_speed) as f32;
            let aspd = cache.airspeed.as_ref().map_or(gs, |a| a.airspeed as f32);
            let alt = cache.gps.as_ref().map_or(0.0, |g| g.alt_msl) as f32;
            let climb = cache.altitude.as_ref().map_or(0.0, |a| a.vario) as f32;
            let hdg = cache.attitude.as_ref().map_or(0.0, |a| a.yaw).round() as i16;
            self.emit(
                &MavMessage::VFR_HUD(VFR_HUD_DATA {
                    airspeed: aspd,
                    groundspeed: gs,
                    heading: hdg,
                    throttle: 0,
                    alt,
                    climb,
                    ..Default::default()
                }),
                &mut out,
            );
        }

        if let Some(an) = cache.analog.as_ref() {
            // Advertise the basic sensors as present/healthy so the GCS HUD isn't all-red.
            let sensors = MavSysStatusSensor::MAV_SYS_STATUS_SENSOR_3D_GYRO
                | MavSysStatusSensor::MAV_SYS_STATUS_SENSOR_3D_ACCEL
                | MavSysStatusSensor::MAV_SYS_STATUS_SENSOR_3D_MAG
                | MavSysStatusSensor::MAV_SYS_STATUS_SENSOR_GPS;
            self.emit(
                &MavMessage::SYS_STATUS(SYS_STATUS_DATA {
                    onboard_control_sensors_present: sensors,
                    onboard_control_sensors_enabled: sensors,
                    onboard_control_sensors_health: sensors,
                    load: 0,
                    voltage_battery: (an.voltage * 1000.0).round().clamp(0.0, 65535.0) as u16,
                    current_battery: (an.current * 100.0).round().clamp(-32768.0, 32767.0) as i16,
                    battery_remaining: if an.battery_percentage > 0 { an.battery_percentage as i8 } else { -1 },
                    ..Default::default()
                }),
                &mut out,
            );
        }

        out
    }
}
