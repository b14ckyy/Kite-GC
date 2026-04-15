// MSP Scheduler Module
// Dedicated thread that owns the serial connection and coordinates all MSP traffic.
// Telemetry slots are time-based, commands and bulk transfers interleave between polls.

mod telemetry;

#[cfg(debug_assertions)]
mod debug;

// In release builds, DebugTracker is a zero-sized no-op struct.
// All tracking calls are inlined away by the compiler.
#[cfg(not(debug_assertions))]
mod debug {
    pub struct DebugTracker;
    impl DebugTracker {
        pub fn new(_polling: &[(u16, f64)], _handshake: &[u16]) -> Self { Self }
        #[inline(always)]
        pub fn on_request(&mut self, _: u16, _: usize) {}
        #[inline(always)]
        pub fn on_response(&mut self, _: u16, _: usize) {}
        #[inline(always)]
        pub fn on_timeout(&mut self, _: u16) {}
        #[inline(always)]
        pub fn maybe_emit(&mut self, _: &tauri::AppHandle) {}
    }
}

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter};

use crate::transport::serial::SerialConnection;

pub use telemetry::{TelemetryConfig, TelemetryGroup};

/// Commands sent to the scheduler thread via channel
#[derive(Debug)]
pub enum SchedulerCommand {
    /// Stop the scheduler and return the serial connection
    Stop,
    /// Send a one-shot MSP request and return the response
    MspRequest {
        code: u16,
        payload: Vec<u8>,
        reply: mpsc::Sender<Result<Vec<u8>, String>>,
    },
}

/// A telemetry slot that tracks when it was last polled
struct TelemetrySlot {
    group: TelemetryGroup,
    codes: Vec<u16>,
    interval: Duration,
    last_poll: Option<Instant>,
    /// For groups with rotating secondary codes, track current index
    rotation_index: usize,
    /// Scheduling priority (higher = more important, polled first when overloaded)
    priority: u8,
}

impl TelemetrySlot {
    fn new(group: TelemetryGroup, codes: Vec<u16>, rate_hz: f64, priority: u8) -> Self {
        Self {
            group,
            codes,
            interval: Duration::from_secs_f64(1.0 / rate_hz),
            last_poll: None,
            rotation_index: 0,
            priority,
        }
    }

    /// How long this slot is overdue (None if not yet due)
    fn overdue(&self) -> Option<Duration> {
        match self.last_poll {
            None => Some(Duration::from_secs(999)), // never polled = maximally overdue
            Some(last) => {
                let elapsed = last.elapsed();
                if elapsed >= self.interval {
                    Some(elapsed - self.interval)
                } else {
                    None
                }
            }
        }
    }

    /// Time until next poll is due
    fn time_until_due(&self) -> Duration {
        match self.last_poll {
            None => Duration::ZERO,
            Some(last) => {
                let elapsed = last.elapsed();
                if elapsed >= self.interval {
                    Duration::ZERO
                } else {
                    self.interval - elapsed
                }
            }
        }
    }
}

/// Handle returned to the caller to interact with the running scheduler
pub struct SchedulerHandle {
    cmd_tx: mpsc::Sender<SchedulerCommand>,
    thread: Option<thread::JoinHandle<Option<SerialConnection>>>,
}

impl SchedulerHandle {
    /// Send a one-shot MSP command through the scheduler (blocks until response)
    pub fn msp_request(&self, code: u16, payload: &[u8]) -> Result<Vec<u8>, String> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.cmd_tx
            .send(SchedulerCommand::MspRequest {
                code,
                payload: payload.to_vec(),
                reply: reply_tx,
            })
            .map_err(|_| "Scheduler thread gone".to_string())?;
        reply_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| "Scheduler request timeout".to_string())?
    }

    /// Stop the scheduler and return the serial connection for cleanup
    pub fn stop(mut self) -> Option<SerialConnection> {
        let _ = self.cmd_tx.send(SchedulerCommand::Stop);
        self.thread
            .take()
            .and_then(|t| t.join().ok())
            .flatten()
    }
}

/// Start the MSP scheduler on a dedicated thread
pub fn start(
    serial: SerialConnection,
    config: TelemetryConfig,
    app_handle: AppHandle,
) -> SchedulerHandle {
    let (cmd_tx, cmd_rx) = mpsc::channel::<SchedulerCommand>();

    let thread = thread::spawn(move || {
        scheduler_loop(serial, config, app_handle, cmd_rx)
    });

    SchedulerHandle {
        cmd_tx,
        thread: Some(thread),
    }
}

/// Main scheduler loop — runs until Stop command received
fn scheduler_loop(
    mut serial: SerialConnection,
    config: TelemetryConfig,
    app_handle: AppHandle,
    cmd_rx: mpsc::Receiver<SchedulerCommand>,
) -> Option<SerialConnection> {
    let mut slots = build_slots(&config);

    // Debug tracker for MSP communication stats (no-op in release builds)
    let mut debug_tracker = {
        let polling_codes: Vec<(u16, f64)> = slots.iter()
            .flat_map(|s| {
                let rate = 1.0 / s.interval.as_secs_f64();
                s.codes.iter().map(move |&c| (c, rate))
            })
            .collect();
        debug::DebugTracker::new(
            &polling_codes,
            &[
                crate::msp::MSP_API_VERSION,
                crate::msp::MSP_FC_VARIANT,
                crate::msp::MSP_FC_VERSION,
                crate::msp::MSP_BOARD_INFO,
                crate::msp::MSPV2_INAV_MIXER,
            ],
        )
    };

    log::info!(
        "Scheduler started: {} telemetry slots (attitude={:.0}Hz, position={:.0}Hz, airspeed={})",
        slots.len(),
        config.attitude_rate_hz,
        config.position_rate_hz,
        if config.airspeed_enabled { "on" } else { "off" },
    );

    loop {
        // 1. Check for stop/commands (non-blocking)
        match cmd_rx.try_recv() {
            Ok(SchedulerCommand::Stop) => {
                log::info!("Scheduler stopping");
                return Some(serial);
            }
            Ok(SchedulerCommand::MspRequest {
                code,
                payload,
                reply,
            }) => {
                let result = serial
                    .msp_request(code, &payload)
                    .map(|msg| msg.payload);
                let _ = reply.send(result);
                continue; // re-check commands before polling
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                log::warn!("Scheduler command channel disconnected");
                return Some(serial);
            }
        }

        // 2. Find most overdue telemetry slot (priority-based adaptive degradation:
        //    when multiple slots are overdue, highest priority wins.
        //    This naturally degrades low-priority groups first — GPS before Attitude.)
        let most_overdue = slots
            .iter()
            .enumerate()
            .filter_map(|(i, slot)| slot.overdue().map(|d| (i, slot.priority, d)))
            .max_by(|(_, p1, d1), (_, p2, d2)| {
                p1.cmp(p2).then_with(|| d1.cmp(d2))
            })
            .map(|(i, _, _)| i);

        if let Some(idx) = most_overdue {
            poll_slot(&mut serial, &mut slots[idx], &app_handle, &mut debug_tracker);
            debug_tracker.maybe_emit(&app_handle);
            continue;
        }

        // 3. Nothing overdue — check for commands with short timeout
        let min_wait = slots
            .iter()
            .map(|s| s.time_until_due())
            .min()
            .unwrap_or(Duration::from_millis(100));

        // Wait for a command or until next slot is due
        let wait = min_wait.min(Duration::from_millis(50));
        match cmd_rx.recv_timeout(wait) {
            Ok(SchedulerCommand::Stop) => {
                log::info!("Scheduler stopping");
                return Some(serial);
            }
            Ok(SchedulerCommand::MspRequest {
                code,
                payload,
                reply,
            }) => {
                let result = serial
                    .msp_request(code, &payload)
                    .map(|msg| msg.payload);
                let _ = reply.send(result);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                log::warn!("Scheduler command channel disconnected");
                return Some(serial);
            }
        }
    }
}

/// Build telemetry slots based on config
///
/// Priority levels (higher = polled first when bandwidth is limited):
///   5 = Attitude (most important for real-time display)
///   4 = Status (arming/safety critical)
///   3 = Analog (battery monitoring)
///   2 = PositionPrimary / GPS
///   1 = PositionSecondary (ALT, optional Airspeed)
fn build_slots(config: &TelemetryConfig) -> Vec<TelemetrySlot> {
    use crate::msp::*;

    let mut secondary_codes = vec![MSP_ALTITUDE];
    if config.airspeed_enabled {
        secondary_codes.push(MSPV2_INAV_AIR_SPEED);
    }

    vec![
        TelemetrySlot::new(
            TelemetryGroup::Attitude,
            vec![MSP_ATTITUDE],
            config.attitude_rate_hz,
            5,
        ),
        TelemetrySlot::new(
            TelemetryGroup::Analog,
            vec![MSPV2_INAV_ANALOG],
            1.0,
            3,
        ),
        TelemetrySlot::new(
            TelemetryGroup::PositionPrimary,
            vec![MSP_RAW_GPS],
            config.position_rate_hz,
            2,
        ),
        TelemetrySlot::new(
            TelemetryGroup::PositionSecondary,
            secondary_codes,
            config.position_rate_hz,
            1,
        ),
        TelemetrySlot::new(
            TelemetryGroup::Status,
            vec![MSPV2_INAV_STATUS, MSP_SENSOR_STATUS],
            1.0,
            4,
        ),
    ]
}

/// Poll a single telemetry slot and emit events
fn poll_slot(
    serial: &mut SerialConnection,
    slot: &mut TelemetrySlot,
    app_handle: &AppHandle,
    tracker: &mut debug::DebugTracker,
) {
    let codes_to_poll: Vec<u16> = match slot.group {
        TelemetryGroup::PositionSecondary => {
            // Rotating: pick one secondary code per cycle
            let code = slot.codes[slot.rotation_index % slot.codes.len()];
            slot.rotation_index = slot.rotation_index.wrapping_add(1);
            vec![code]
        }
        _ => slot.codes.clone(),
    };

    for code in codes_to_poll {
        tracker.on_request(code, 9); // MSP V2 request frame = 9 bytes (empty payload)
        match serial.msp_request(code, &[]) {
            Ok(msg) => {
                tracker.on_response(code, 9 + msg.payload.len());
                let event_name = telemetry::event_name_for_code(code);
                let payload = telemetry::decode_telemetry(code, &msg.payload);
                if let Err(e) = app_handle.emit(&event_name, &payload) {
                    log::warn!("Failed to emit {}: {}", event_name, e);
                }
            }
            Err(e) => {
                tracker.on_timeout(code);
                log::debug!("Telemetry poll 0x{:04X} failed: {}", code, e);
            }
        }
    }

    slot.last_poll = Some(Instant::now());
}
