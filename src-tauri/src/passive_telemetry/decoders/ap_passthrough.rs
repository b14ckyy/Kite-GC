// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// ArduPilot Passthrough ("Yaapu") decoder — the AP-specific telemetry layer carried over either FrSky
// S.Port (DIY appIDs 0x5000–0x500D) or CRSF (AP_CUSTOM_TELEM frame 0x80/0x7F). Both carriers transport
// the SAME bit-packed `{appID:u16, data:u32}` packets, so this one decoder is fed from both (see
// docs/active/RADIO_TELEMETRY.md "ArduPilot Passthrough").
//
// It is a level-2 sub-decoder: the FrSky/CRSF host decoder parses the carrier framing and routes the
// passthrough packets here, while continuing to decode the standard sensors/frames itself. This decoder
// therefore emits ONLY the AP-unique data that the native path can't provide — flight mode (real AP
// modes via classify_ardupilot), armed, EKF health, and status-text messages — leaving GPS / battery /
// attitude to the native decoder (no overlap, no field fights).
//
// v1 scope: AP_STATUS (0x5001), TEXT (0x5000 S.Port chunked + CRSF 0xF1 status-text sub-frame), PARAM
// (0x5007 → vehicle variant, bonus), WAYPOINT (0x500D number). The metric fields that use ArduPilot's
// `prep_number` compact encoding (HDOP, home distance, speeds, rangefinder, terrain, …) are a v2.

use tauri::{AppHandle, Emitter};

use crate::flightlog::recorder::FlightRecorderHandle;
use crate::flightmode::{classify_ardupilot, FlightModeState};
use crate::scheduler::telemetry::StatusData;

/// INAV/ArduPilot arming_flags bit 2 = ARMED (what the recorder + frontend look for).
const ARMED_FLAG: u32 = 0x04;

// ── Passthrough packet appIDs ────────────────────────────────────────────────
const ID_TEXT: u16 = 0x5000;
const ID_AP_STATUS: u16 = 0x5001;
const ID_PARAM: u16 = 0x5007;
const ID_WAYPOINT: u16 = 0x500D;

/// Best-effort map of the PARAM frame-type value (≈ MAV_TYPE) to the variant string `classify_ardupilot`
/// expects ("plane" → plane table, else copter). Bonus only — verify against a real AP source.
fn variant_from_frame_type(mav_type: u32) -> &'static str {
    match mav_type {
        1 => "plane",                         // FIXED_WING
        19..=25 => "plane", // VTOL variants
        _ => "copter",                        // quad/hexa/octo/heli/tri/... (rover unhandled → copter)
    }
}

#[derive(Default)]
struct State {
    armed: bool,
    custom_mode: u32,
    variant: String,
    ekf_status: u8, // 0 unknown, 1 OK, 3 bad (matches telemetry-ekf-status scale)
    seen_status: bool,

    wp_number: u16,
    seen_wp: bool,
}

pub struct ApPassthroughDecoder {
    state: State,
    /// Accumulates S.Port TEXT (0x5000) chars until a NUL terminates the message.
    text_buf: Vec<u8>,
    /// Completed (severity, message) status-texts awaiting emission in `publish`.
    pending_text: Vec<(u8, String)>,
}

impl ApPassthroughDecoder {
    pub fn new() -> Self {
        Self {
            state: State::default(),
            text_buf: Vec::with_capacity(64),
            pending_text: Vec::new(),
        }
    }

    /// Apply one passthrough packet (`appID`, 32-bit `data`) — from S.Port directly, or unpacked from a
    /// CRSF AP_CUSTOM_TELEM frame.
    pub fn apply_packet(&mut self, appid: u16, data: u32) {
        match appid {
            ID_AP_STATUS => {
                // [4:0] control_mode = (custom_mode+1)&0x1F; [8] armed; [11:10] EKF failsafe (bad).
                let control_mode = data & 0x1F;
                self.state.custom_mode = (control_mode + 0x1F) & 0x1F; // = (control_mode - 1) mod 32
                self.state.armed = (data >> 8) & 0x1 != 0;
                let ekf_bad = (data >> 10) & 0x3 != 0;
                self.state.ekf_status = if ekf_bad { 3 } else { 1 };
                self.state.seen_status = true;
            }
            ID_TEXT => self.apply_text(data),
            ID_PARAM => {
                // [31:24] param id, [23:0] value. id 1 = frame type (≈ MAV_TYPE).
                let id = (data >> 24) & 0xFF;
                if id == 1 {
                    self.state.variant = variant_from_frame_type(data & 0x00FF_FFFF).to_string();
                }
            }
            ID_WAYPOINT => {
                self.state.wp_number = (data & 0x7FF) as u16;
                self.state.seen_wp = true;
            }
            _ => {} // GPS/BATT/ATTITUDE/HOME/VEL_YAW/… handled natively or deferred to v2
        }
    }

    /// Status-text chunk (0x5000): 4 chars, first char in the MSB. A NUL byte ends the message; on that
    /// final chunk the 3-bit severity sits in bits 7/15/23 (the spare LSBs of the zero bytes).
    fn apply_text(&mut self, data: u32) {
        let mut ended = false;
        for shift in [24u32, 16, 8, 0] {
            let c = ((data >> shift) & 0xFF) as u8;
            if c == 0 {
                ended = true;
                break;
            }
            self.text_buf.push(c);
            if self.text_buf.len() >= 200 {
                ended = true;
                break;
            }
        }
        if ended {
            let severity = (((data >> 7) & 0x1) | (((data >> 15) & 0x1) << 1) | (((data >> 23) & 0x1) << 2)) as u8;
            if !self.text_buf.is_empty() {
                let msg = String::from_utf8_lossy(&self.text_buf).into_owned();
                self.pending_text.push((severity, msg));
            }
            self.text_buf.clear();
        }
    }

    /// Unpack a CRSF AP_CUSTOM_TELEM frame payload (`type` byte already stripped). `payload[0]` is the
    /// sub-type: single (0xF0) = one {appid:u16, data:u32}; multi (0xF2) = size + size×{appid,data};
    /// status-text (0xF1) = severity + NUL-terminated text. Layout verified against AP_CRSF_Telem.h.
    pub fn apply_crsf_custom(&mut self, payload: &[u8]) {
        if payload.is_empty() {
            return;
        }
        match payload[0] {
            0xF0 => {
                if payload.len() >= 7 {
                    let appid = u16::from_le_bytes([payload[1], payload[2]]);
                    let data = u32::from_le_bytes([payload[3], payload[4], payload[5], payload[6]]);
                    self.apply_packet(appid, data);
                }
            }
            0xF2 => {
                let count = payload[1] as usize;
                let mut off = 2;
                for _ in 0..count {
                    if off + 6 > payload.len() {
                        break;
                    }
                    let appid = u16::from_le_bytes([payload[off], payload[off + 1]]);
                    let data = u32::from_le_bytes([
                        payload[off + 2],
                        payload[off + 3],
                        payload[off + 4],
                        payload[off + 5],
                    ]);
                    self.apply_packet(appid, data);
                    off += 6;
                }
            }
            0xF1 => {
                // Dedicated status-text sub-frame: sub_type, severity, NUL-terminated text.
                if payload.len() >= 2 {
                    let severity = payload[1];
                    let end = payload[2..]
                        .iter()
                        .position(|&b| b == 0)
                        .map(|p| 2 + p)
                        .unwrap_or(payload.len());
                    let msg = String::from_utf8_lossy(&payload[2..end]).into_owned();
                    if !msg.is_empty() {
                        self.pending_text.push((severity, msg));
                    }
                }
            }
            _ => {}
        }
    }

    /// Emit the AP-unique telemetry: status (armed), flight mode (real AP modes), EKF health, status-text
    /// + waypoint number. GPS/battery/attitude are NOT emitted here — the native host decoder owns those.
    pub fn publish(&mut self, app: &AppHandle, recorder: Option<&FlightRecorderHandle>) {
        let s = &self.state;

        if s.seen_status {
            let status = StatusData {
                arming_flags: if s.armed { ARMED_FLAG } else { 0 },
                flight_mode_flags: 0,
                cpu_load: 0,
                sensor_status: 0,
                msp_rc_override: false,
            };
            let variant = if s.variant.is_empty() { "copter" } else { s.variant.as_str() };
            let fm: FlightModeState = classify_ardupilot(s.custom_mode, variant);
            let _ = app.emit("telemetry-status", &status);
            let _ = app.emit("telemetry-flightmode", &fm);
            // EKF health for the header indicator (passthrough has only a bad/ok flag, no variances).
            let _ = app.emit(
                "telemetry-ekf-status",
                serde_json::json!({ "status": s.ekf_status, "max_variance": 0.0, "flags": 0 }),
            );
            if let Some(rec) = recorder {
                if let Ok(mut r) = rec.lock() {
                    r.on_status(&status);
                    r.on_flightmode(&fm);
                }
            }
        }

        if s.seen_wp {
            let _ = app.emit(
                "telemetry-nav-status",
                serde_json::json!({ "active_wp_number": s.wp_number, "nav_state": 0 }),
            );
        }

        // Status-text messages reuse the MAVLink statustext sink (severity + text; same MAV_SEVERITY scale).
        for (severity, text) in self.pending_text.drain(..) {
            let _ = app.emit(
                "mavlink-statustext",
                serde_json::json!({ "severity": severity, "text": text }),
            );
        }
    }
}
