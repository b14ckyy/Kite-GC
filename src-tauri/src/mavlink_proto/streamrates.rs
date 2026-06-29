// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// MAVLink stream-rate configuration (ADR-043).
//
// ArduPilot is push-based: it streams per the port's SRn_* params as soon as it sees a GCS
// heartbeat. To get MSP-parity (two knobs: attitude + position rate, everything else 1 Hz) and to
// fit real RC links (ELRS/CRSF/mLRS ≈ 0.5–1.5 KB/s), we request explicit per-message rates via
// MAV_CMD_SET_MESSAGE_INTERVAL, and disable the high-rate "ballast" no widget consumes.
//
// SET_MESSAGE_INTERVAL is sticky FC-side per channel — it survives reconnects until the FC reboots.
// So the two modes are NOT "configure vs. do nothing":
//   • reduced  → apply_stream_rates(): wanted msgs at the two knobs, ballast disabled (-1)
//   • full     → reset_stream_rates(): every message we ever touch reset to its SRn default (0),
//                so a link previously narrowed by Kite returns to the operator's full FC stream.
// Applied ONCE on connect (after the handshake, before the handler thread starts), scoped FC-side to
// our channel — we never touch the persistent SRn_* parameters.

use ::mavlink::ardupilotmega::{MavCmd, MavMessage, COMMAND_LONG_DATA};

use crate::transport::ByteTransport;

use super::codec::{self, MavSequence};

// ── Managed message IDs ──────────────────────────────────────────────────────────────────────
const ID_SYS_STATUS: f32 = 1.0;
const ID_GPS_RAW_INT: f32 = 24.0;
const ID_GLOBAL_POSITION_INT: f32 = 33.0;
const ID_MISSION_CURRENT: f32 = 42.0;
const ID_RC_CHANNELS: f32 = 65.0;
const ID_VFR_HUD: f32 = 74.0;
const ID_ATTITUDE: f32 = 30.0;
const ID_BATTERY_STATUS: f32 = 147.0;
const ID_EKF_STATUS_REPORT: f32 = 193.0;
const ID_HOME_POSITION: f32 = 242.0;
const ID_WIND: f32 = 168.0;

// Rate kinds for the wanted messages (resolved against the two settings at apply time).
const R_ATTITUDE: u8 = 0;
const R_POSITION: u8 = 1;
const R_FIXED_1HZ: u8 = 2;
const R_HOME_LOW: u8 = 3;

/// Wanted messages: (message_id, rate_kind). Single source of truth — also drives the reset list.
const WANTED: &[(f32, u8)] = &[
    (ID_ATTITUDE, R_ATTITUDE),
    (ID_GLOBAL_POSITION_INT, R_POSITION),
    // GPS_RAW_INT now contributes only fix type / sat count / HDOP (slow-changing) — the position,
    // ground speed and course all come from the fused GLOBAL_POSITION_INT, so 1 Hz is plenty.
    (ID_GPS_RAW_INT, R_FIXED_1HZ),
    // VFR_HUD is consumed only for airspeed — gated on the airspeed module (see apply_stream_rates).
    (ID_VFR_HUD, R_POSITION),
    // WIND (ArduPilot EKF wind estimate) — gated on the wind module (see apply_stream_rates).
    (ID_WIND, R_FIXED_1HZ),
    (ID_SYS_STATUS, R_FIXED_1HZ),
    (ID_BATTERY_STATUS, R_FIXED_1HZ),
    (ID_RC_CHANNELS, R_FIXED_1HZ), // RSSI (link quality)
    (ID_MISSION_CURRENT, R_FIXED_1HZ),
    (ID_EKF_STATUS_REPORT, R_FIXED_1HZ),
    (ID_HOME_POSITION, R_HOME_LOW),
];

// ── Ballast: high-rate messages no widget consumes — disabled (interval = -1) ────────────────
const BALLAST_IDS: &[f32] = &[
    2.0,   // SYSTEM_TIME
    27.0,  // RAW_IMU
    29.0,  // SCALED_PRESSURE
    36.0,  // SERVO_OUTPUT_RAW
    // 65 (RC_CHANNELS) is now WANTED at 1 Hz — it is our RSSI source.
    116.0, // SCALED_IMU2
    125.0, // POWER_STATUS
    129.0, // SCALED_IMU3
    136.0, // TERRAIN_REPORT
    137.0, // SCALED_PRESSURE2
    143.0, // SCALED_PRESSURE3
    152.0, // MEMINFO
    163.0, // AHRS
    165.0, // HWSTATUS
    178.0, // AHRS2
    // 182 (AHRS3) intentionally omitted: obsolete in modern ArduPilot (no longer produced), so a
    // SET_MESSAGE_INTERVAL for it makes the FC emit "No ap_message for mavlink id (182)" on connect.
    // 193 (EKF_STATUS_REPORT) is now WANTED at 1 Hz — it drives the header EKF indicator.
    241.0, // VIBRATION
];

/// Resolve a rate kind to Hz against the two settings.
fn rate_hz(kind: u8, attitude_hz: f64, position_hz: f64) -> f32 {
    match kind {
        R_ATTITUDE => attitude_hz as f32,
        R_POSITION => position_hz as f32,
        R_HOME_LOW => 0.2,
        _ => 1.0, // R_FIXED_1HZ
    }
}

/// Convert a desired rate (Hz) to a SET_MESSAGE_INTERVAL interval in microseconds.
/// 0 / negative → -1 (disable the message).
fn hz_to_interval_us(hz: f32) -> f32 {
    if hz > 0.0 {
        1_000_000.0 / hz
    } else {
        -1.0
    }
}

/// Apply the reduced GCS-requested stream rates (default mode): wanted messages at the two knobs,
/// ballast disabled. Fire-and-forget over the (pre-handler) byte transport.
pub fn apply_stream_rates(
    transport: &mut dyn ByteTransport,
    fc_sysid: u8,
    attitude_hz: f64,
    position_hz: f64,
    airspeed_enabled: bool,
    wind_enabled: bool,
) {
    let header = codec::gcs_header();
    let mut seq = MavSequence::new();

    let mut sent = 0usize;
    for &(msg_id, kind) in WANTED {
        // VFR_HUD (airspeed) and WIND each cost an extra message and are gated on their module — sent as
        // -1 when off (not skipped) because SET_MESSAGE_INTERVAL is sticky FC-side across sessions.
        let interval = if (msg_id == ID_VFR_HUD && !airspeed_enabled)
            || (msg_id == ID_WIND && !wind_enabled)
        {
            -1.0
        } else {
            hz_to_interval_us(rate_hz(kind, attitude_hz, position_hz))
        };
        if send_set_interval(transport, &header, &mut seq, fc_sysid, msg_id, interval) {
            sent += 1;
        }
    }
    for &msg_id in BALLAST_IDS {
        if send_set_interval(transport, &header, &mut seq, fc_sysid, msg_id, -1.0) {
            sent += 1;
        }
    }

    eprintln!(
        "[MAVLINK-RATES] reduced stream config: attitude={:.0}Hz, position={:.0}Hz, airspeed={}, {} SET_MESSAGE_INTERVAL sent ({} disabled)",
        attitude_hz, position_hz, airspeed_enabled, sent, BALLAST_IDS.len()
    );
}

/// Reset every managed message back to its FC SRn default (interval = 0) — used for Full-telemetry
/// mode. Because SET_MESSAGE_INTERVAL is sticky on the FC, a link previously narrowed by Kite would
/// otherwise stay narrow until the FC reboots; this undoes it so the operator's full stream returns.
pub fn reset_stream_rates(transport: &mut dyn ByteTransport, fc_sysid: u8) {
    let header = codec::gcs_header();
    let mut seq = MavSequence::new();

    let mut sent = 0usize;
    for &(msg_id, _) in WANTED {
        if send_set_interval(transport, &header, &mut seq, fc_sysid, msg_id, 0.0) {
            sent += 1;
        }
    }
    for &msg_id in BALLAST_IDS {
        if send_set_interval(transport, &header, &mut seq, fc_sysid, msg_id, 0.0) {
            sent += 1;
        }
    }

    eprintln!(
        "[MAVLINK-RATES] Full MAVLink Telemetry — reset {} messages to FC default rates (interval=0)",
        sent
    );
}

/// Build + write one MAV_CMD_SET_MESSAGE_INTERVAL (511). Returns true on a successful write.
fn send_set_interval(
    transport: &mut dyn ByteTransport,
    header: &::mavlink::MavHeader,
    seq: &mut MavSequence,
    fc_sysid: u8,
    msg_id: f32,
    interval_us: f32,
) -> bool {
    let cmd = MavMessage::COMMAND_LONG(COMMAND_LONG_DATA {
        target_system: fc_sysid,
        target_component: 1, // MAV_COMP_ID_AUTOPILOT1
        command: MavCmd::MAV_CMD_SET_MESSAGE_INTERVAL,
        confirmation: 0,
        param1: msg_id,
        param2: interval_us,
        param3: 0.0,
        param4: 0.0,
        param5: 0.0,
        param6: 0.0,
        param7: 0.0,
    });
    let frame = codec::serialize_v2(header, &cmd, seq);
    match transport.write_bytes(&frame) {
        Ok(()) => true,
        Err(e) => {
            log::warn!("Failed to send SET_MESSAGE_INTERVAL for msg {}: {}", msg_id as u32, e);
            false
        }
    }
}
