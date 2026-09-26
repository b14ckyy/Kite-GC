// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Runtime MAVLink parameter reads (geofence params: ArduPilot FENCE_* / PX4 GF_*). Mirrors the mission
// microprotocol's request/receiver pattern via `RegisterParamReceiver`. Writes reuse
// `control::set_param` (fire-and-forget PARAM_SET). See docs/active/GEOFENCE.md.

use std::collections::HashMap;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use ::mavlink::ardupilotmega::{MavMessage, MavParamType, PARAM_REQUEST_READ_DATA};

use super::handler::MavlinkCommand;

const PARAM_TIMEOUT: Duration = Duration::from_secs(3);

fn pack_param_id(name: &str) -> [u8; 16] {
    let mut id = [0u8; 16];
    let b = name.as_bytes();
    let n = b.len().min(16);
    id[..n].copy_from_slice(&b[..n]);
    id
}

fn send(cmd_tx: &mpsc::Sender<MavlinkCommand>, msg: MavMessage) -> Result<(), String> {
    let (reply_tx, reply_rx) = mpsc::channel();
    cmd_tx.send(MavlinkCommand::SendMessage { msg, reply: reply_tx })
        .map_err(|_| "MAVLink handler stopped".to_string())?;
    reply_rx.recv_timeout(Duration::from_secs(5))
        .map_err(|_| "MAVLink send timed out".to_string())?
}

pub fn is_integer_type(ty: MavParamType) -> bool {
    !matches!(ty, MavParamType::MAV_PARAM_TYPE_REAL32 | MavParamType::MAV_PARAM_TYPE_REAL64)
}

/// Numeric value of a PARAM_VALUE. Two wire conventions exist for integer-typed params: ArduPilot
/// puts the NUMBER in the float field (2 → 2.0), PX4 byte-casts the integer into it (2 → the float
/// with bit pattern 0x00000002, a denormal). Decode by declared type + plausibility: an integer type
/// whose float is not a plain integral number (denormal, NaN, fraction, absurd magnitude) is a
/// byte-cast — e.g. `COM_RC_IN_MODE` on PX4 read as 2.8e-45 before this and showed up as 0.
pub fn decode_param(value: f32, ty: MavParamType) -> f32 {
    if !is_integer_type(ty) {
        return value;
    }
    let plain_number = value.is_finite() && value.fract() == 0.0 && value.abs() < 1.0e9
        && (value == 0.0 || value.abs() >= 1.0e-30);
    if plain_number {
        return value;
    }
    let bits = value.to_bits();
    match ty {
        MavParamType::MAV_PARAM_TYPE_INT8 => (bits as u8 as i8) as f32,
        MavParamType::MAV_PARAM_TYPE_UINT8 => (bits as u8) as f32,
        MavParamType::MAV_PARAM_TYPE_INT16 => (bits as u16 as i16) as f32,
        MavParamType::MAV_PARAM_TYPE_UINT16 => (bits as u16) as f32,
        MavParamType::MAV_PARAM_TYPE_UINT32 => bits as f32,
        _ => (bits as i32) as f32,
    }
}

/// Encode `value` for a PARAM_SET in PX4's byte-cast convention (integer types only; reals pass).
pub fn encode_param_bytecast(value: f32, ty: MavParamType) -> f32 {
    if !is_integer_type(ty) {
        return value;
    }
    let i = value.round() as i64;
    let bits: u32 = match ty {
        MavParamType::MAV_PARAM_TYPE_INT8 => (i as i8) as u8 as u32,
        MavParamType::MAV_PARAM_TYPE_UINT8 => (i as u8) as u32,
        MavParamType::MAV_PARAM_TYPE_INT16 => (i as i16) as u16 as u32,
        MavParamType::MAV_PARAM_TYPE_UINT16 => (i as u16) as u32,
        _ => i as i32 as u32,
    };
    f32::from_bits(bits)
}

/// Read the given parameters by name; returns the ones the FC actually reports (missing names are
/// simply absent — e.g. FENCE_* on PX4 or GF_* on ArduPilot). Best-effort, used to populate the
/// geofence panel's core-param controls. Values are decoded numerically (see `decode_param`).
pub fn read_params(
    cmd_tx: &mpsc::Sender<MavlinkCommand>,
    fc_sysid: u8,
    names: &[&str],
) -> HashMap<String, f32> {
    read_params_typed(cmd_tx, fc_sysid, names).into_iter().map(|(k, (v, _))| (k, v)).collect()
}

/// Like `read_params`, keeping the declared type of each parameter (needed to write it back the way
/// the firmware expects).
pub fn read_params_typed(
    cmd_tx: &mpsc::Sender<MavlinkCommand>,
    fc_sysid: u8,
    names: &[&str],
) -> HashMap<String, (f32, MavParamType)> {
    let (tx, rx) = mpsc::channel();
    if cmd_tx.send(MavlinkCommand::RegisterParamReceiver(tx)).is_err() {
        return HashMap::new();
    }
    std::thread::sleep(Duration::from_millis(10)); // let the handler pick up the registration

    let mut out: HashMap<String, (f32, MavParamType)> = HashMap::new();
    for &name in names {
        if send(cmd_tx, MavMessage::PARAM_REQUEST_READ(PARAM_REQUEST_READ_DATA {
            param_index: -1,
            target_system: fc_sysid,
            target_component: 1, // MAV_COMP_ID_AUTOPILOT1
            param_id: pack_param_id(name).into(),
        })).is_err() {
            continue;
        }
        let deadline = Instant::now() + PARAM_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() { break; }
            match rx.recv_timeout(remaining) {
                Ok(MavMessage::PARAM_VALUE(pv)) => {
                    let end = pv.param_id.iter().position(|&c| c == 0).unwrap_or(pv.param_id.len());
                    let pname: String = pv.param_id[..end].iter().map(|&c| c as char).collect();
                    out.insert(pname.clone(), (decode_param(pv.param_value, pv.param_type), pv.param_type));
                    if pname == name { break; } // got the one we asked for
                }
                Ok(_) => continue,
                Err(_) => break,
            }
        }
    }
    let _ = cmd_tx.send(MavlinkCommand::UnregisterParamReceiver);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ardupilot_integer_params_pass_through() {
        assert_eq!(decode_param(2.0, MavParamType::MAV_PARAM_TYPE_INT32), 2.0);
        assert_eq!(decode_param(0.0, MavParamType::MAV_PARAM_TYPE_UINT8), 0.0);
        assert_eq!(decode_param(-1.0, MavParamType::MAV_PARAM_TYPE_INT16), -1.0);
    }

    #[test]
    fn px4_bytecast_integers_are_decoded() {
        // COM_RC_IN_MODE = 2 arrives as the float with bit pattern 2 (a denormal).
        assert_eq!(decode_param(f32::from_bits(2), MavParamType::MAV_PARAM_TYPE_INT32), 2.0);
        assert_eq!(decode_param(f32::from_bits(0xFFFF_FFFF), MavParamType::MAV_PARAM_TYPE_INT32), -1.0);
        assert_eq!(decode_param(f32::from_bits(200), MavParamType::MAV_PARAM_TYPE_UINT8), 200.0);
    }

    #[test]
    fn reals_are_never_touched() {
        assert_eq!(decode_param(1.5, MavParamType::MAV_PARAM_TYPE_REAL32), 1.5);
        assert_eq!(encode_param_bytecast(1.5, MavParamType::MAV_PARAM_TYPE_REAL32), 1.5);
    }

    #[test]
    fn encode_roundtrips_through_decode() {
        for (v, ty) in [(2.0, MavParamType::MAV_PARAM_TYPE_INT32), (-3.0, MavParamType::MAV_PARAM_TYPE_INT8), (60000.0, MavParamType::MAV_PARAM_TYPE_UINT16)] {
            assert_eq!(decode_param(encode_param_bytecast(v, ty), ty), v);
        }
    }
}
