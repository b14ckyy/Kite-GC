// MSP Debug Statistics Tracker
// Only compiled in debug builds via #[cfg(debug_assertions)] in mod.rs.
// Tracks per-message request/response/timeout stats and emits periodic
// debug snapshots to the frontend via Tauri events.

use std::collections::HashMap;
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// Status of the last MSP interaction for a given code
#[derive(Clone, Copy)]
enum MspActivity {
    Idle,
    Request,
    Response,
    Timeout,
}

/// Per-MSP-code statistics
struct CodeStats {
    name: String,
    is_polling: bool,
    target_rate_hz: f64,
    request_count: u64,
    response_count: u64,
    timeout_count: u64,
    /// Activity since last emit (reset after each snapshot)
    cycle_status: MspActivity,
    // Actual rate measurement: count responses in a rolling 1s window
    rate_window_start: Instant,
    rate_window_count: u64,
    actual_rate_hz: f64,
}

/// Snapshot of a single MSP code's debug info (sent to frontend)
#[derive(Debug, Clone, Serialize)]
pub struct MspCodeDebug {
    pub code: u16,
    pub name: String,
    pub is_polling: bool,
    pub request_count: u64,
    pub response_count: u64,
    pub timeout_count: u64,
    /// "idle", "request", "response", "timeout"
    pub last_status: String,
    /// Configured target rate in Hz (0 for handshake/one-shot codes)
    pub target_rate_hz: f64,
    /// Measured actual rate in Hz over the last second
    pub actual_rate_hz: f64,
}

/// Full debug snapshot emitted as a Tauri event
#[derive(Debug, Clone, Serialize)]
pub struct DebugSnapshot {
    pub messages: Vec<MspCodeDebug>,
    pub msg_per_sec_tx: f64,
    pub msg_per_sec_rx: f64,
    pub bytes_per_sec_tx: u64,
    pub bytes_per_sec_rx: u64,
}

/// Tracks MSP communication statistics in the scheduler thread
pub struct DebugTracker {
    stats: HashMap<u16, CodeStats>,
    // Per-second sliding window
    window_start: Instant,
    window_bytes_tx: u64,
    window_bytes_rx: u64,
    window_msg_tx: u64,
    window_msg_rx: u64,
    // Results from last completed 1-second window
    last_bytes_per_sec_tx: u64,
    last_bytes_per_sec_rx: u64,
    last_msg_per_sec_tx: f64,
    last_msg_per_sec_rx: f64,
    // Emit throttle
    last_emit: Instant,
}

impl DebugTracker {
    /// Create a new tracker.
    /// `polling_codes`: (msp_code, target_rate_hz) pairs for actively polled messages.
    /// `handshake_codes`: codes that ran once during handshake (rate=0).
    pub fn new(polling_codes: &[(u16, f64)], handshake_codes: &[u16]) -> Self {
        let now = Instant::now();
        let mut stats = HashMap::new();

        // Register actively polled codes with their target rates
        for &(code, rate) in polling_codes {
            stats.insert(
                code,
                CodeStats {
                    name: msp_code_name(code),
                    is_polling: true,
                    target_rate_hz: rate,
                    request_count: 0,
                    response_count: 0,
                    timeout_count: 0,
                    cycle_status: MspActivity::Idle,
                    rate_window_start: now,
                    rate_window_count: 0,
                    actual_rate_hz: 0.0,
                },
            );
        }

        // Register handshake codes as inactive (already completed once)
        for &code in handshake_codes {
            if !stats.contains_key(&code) {
                stats.insert(
                    code,
                    CodeStats {
                        name: msp_code_name(code),
                        is_polling: false,
                        target_rate_hz: 0.0,
                        request_count: 1,
                        response_count: 1,
                        timeout_count: 0,
                        cycle_status: MspActivity::Idle,
                        rate_window_start: now,
                        rate_window_count: 0,
                        actual_rate_hz: 0.0,
                    },
                );
            }
        }
        Self {
            stats,
            window_start: now,
            window_bytes_tx: 0,
            window_bytes_rx: 0,
            window_msg_tx: 0,
            window_msg_rx: 0,
            last_bytes_per_sec_tx: 0,
            last_bytes_per_sec_rx: 0,
            last_msg_per_sec_tx: 0.0,
            last_msg_per_sec_rx: 0.0,
            last_emit: now,
        }
    }

    /// Record that an MSP request was sent
    pub fn on_request(&mut self, code: u16, frame_bytes: usize) {
        self.ensure_code(code);
        if let Some(s) = self.stats.get_mut(&code) {
            s.request_count += 1;
            // Only upgrade to Request if not already Response/Timeout in this cycle
            if matches!(s.cycle_status, MspActivity::Idle) {
                s.cycle_status = MspActivity::Request;
            }
        }
        self.window_bytes_tx += frame_bytes as u64;
        self.window_msg_tx += 1;
    }

    /// Record that an MSP response was received
    pub fn on_response(&mut self, code: u16, frame_bytes: usize) {
        self.ensure_code(code);
        if let Some(s) = self.stats.get_mut(&code) {
            s.response_count += 1;
            s.cycle_status = MspActivity::Response;

            // Per-code rate measurement: count in 1s window then roll over
            s.rate_window_count += 1;
            let elapsed = s.rate_window_start.elapsed().as_secs_f64();
            if elapsed >= 1.0 {
                s.actual_rate_hz = s.rate_window_count as f64 / elapsed;
                s.rate_window_count = 0;
                s.rate_window_start = Instant::now();
            }
        }
        self.window_bytes_rx += frame_bytes as u64;
        self.window_msg_rx += 1;
    }

    /// Record that an MSP request timed out
    pub fn on_timeout(&mut self, code: u16) {
        self.ensure_code(code);
        if let Some(s) = self.stats.get_mut(&code) {
            s.timeout_count += 1;
            s.cycle_status = MspActivity::Timeout;
        }
    }

    /// Emit debug stats to the frontend at ~60 Hz
    pub fn maybe_emit(&mut self, app_handle: &AppHandle) {
        if self.last_emit.elapsed().as_millis() < 16 {
            return;
        }

        // Roll over the 1-second window if elapsed
        let elapsed = self.window_start.elapsed().as_secs_f64();
        if elapsed >= 1.0 {
            self.last_bytes_per_sec_tx = (self.window_bytes_tx as f64 / elapsed) as u64;
            self.last_bytes_per_sec_rx = (self.window_bytes_rx as f64 / elapsed) as u64;
            self.last_msg_per_sec_tx = self.window_msg_tx as f64 / elapsed;
            self.last_msg_per_sec_rx = self.window_msg_rx as f64 / elapsed;
            self.window_bytes_tx = 0;
            self.window_bytes_rx = 0;
            self.window_msg_tx = 0;
            self.window_msg_rx = 0;
            self.window_start = Instant::now();
        }

        let mut messages: Vec<MspCodeDebug> = self
            .stats
            .iter_mut()
            .map(|(&code, s)| {
                let status_str = match s.cycle_status {
                    MspActivity::Idle => "idle",
                    MspActivity::Request => "request",
                    MspActivity::Response => "response",
                    MspActivity::Timeout => "timeout",
                };

                // Reset cycle status after reading
                let snapshot = MspCodeDebug {
                    code,
                    name: s.name.clone(),
                    is_polling: s.is_polling,
                    request_count: s.request_count,
                    response_count: s.response_count,
                    timeout_count: s.timeout_count,
                    last_status: status_str.into(),
                    target_rate_hz: (s.target_rate_hz * 10.0).round() / 10.0,
                    actual_rate_hz: (s.actual_rate_hz * 10.0).round() / 10.0,
                };
                s.cycle_status = MspActivity::Idle;
                snapshot
            })
            .collect();

        // Sort: polling codes first, then by code number
        messages.sort_by(|a, b| {
            b.is_polling
                .cmp(&a.is_polling)
                .then_with(|| a.code.cmp(&b.code))
        });

        let snapshot = DebugSnapshot {
            messages,
            msg_per_sec_tx: (self.last_msg_per_sec_tx * 10.0).round() / 10.0,
            msg_per_sec_rx: (self.last_msg_per_sec_rx * 10.0).round() / 10.0,
            bytes_per_sec_tx: self.last_bytes_per_sec_tx,
            bytes_per_sec_rx: self.last_bytes_per_sec_rx,
        };

        let _ = app_handle.emit("debug-msp-stats", &snapshot);
        self.last_emit = Instant::now();
    }

    fn ensure_code(&mut self, code: u16) {
        let now = Instant::now();
        self.stats.entry(code).or_insert_with(|| CodeStats {
            name: msp_code_name(code),
            is_polling: false,
            target_rate_hz: 0.0,
            request_count: 0,
            response_count: 0,
            timeout_count: 0,
            cycle_status: MspActivity::Idle,
            rate_window_start: now,
            rate_window_count: 0,
            actual_rate_hz: 0.0,
        });
    }
}

/// Map MSP code to human-readable name
fn msp_code_name(code: u16) -> String {
    match code {
        1 => "MSP_API_VERSION".into(),
        2 => "MSP_FC_VARIANT".into(),
        3 => "MSP_FC_VERSION".into(),
        4 => "MSP_BOARD_INFO".into(),
        5 => "MSP_BUILD_INFO".into(),
        10 => "MSP_NAME".into(),
        101 => "MSP_STATUS".into(),
        106 => "MSP_RAW_GPS".into(),
        108 => "MSP_ATTITUDE".into(),
        109 => "MSP_ALTITUDE".into(),
        110 => "MSP_ANALOG".into(),
        20 => "MSP_WP_GETINFO".into(),
        118 => "MSP_WP".into(),
        119 => "MSP_BOXIDS".into(),
        121 => "MSP_NAV_STATUS".into(),
        130 => "MSP_BATTERY_STATE".into(),
        150 => "MSP_STATUS_EX".into(),
        151 => "MSP_SENSOR_STATUS".into(),
        166 => "MSP_GPSSTATISTICS".into(),
        209 => "MSP_SET_WP".into(),
        0x2000 => "MSPV2_INAV_STATUS".into(),
        0x2002 => "MSPV2_INAV_ANALOG".into(),
        0x2003 => "MSPV2_INAV_MISC".into(),
        0x2009 => "MSPV2_INAV_AIR_SPEED".into(),
        0x2010 => "MSPV2_INAV_MIXER".into(),
        0x2090 => "MSP2_ADSB_VEHICLE_LIST".into(),
        _ => format!("MSP_0x{:04X}", code),
    }
}
