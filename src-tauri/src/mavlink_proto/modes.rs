// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! The FC's own flight-mode list — MAVLink AVAILABLE_MODES (#435), ArduPilot 4.7.0 and later.
//!
//! The control panel's mode tables are fixed per vehicle type: modes compiled into the firmware but
//! missing from a table are never offered, and modes a table lists may be absent from the connected
//! build. ArduPilot 4.7+ reports which modes it has; the panel uses that list when it exists and keeps
//! the static tables otherwise.
//!
//! - **The request is the feature check.** No version gate: one `MAV_CMD_REQUEST_MESSAGE` per index,
//!   and a vehicle that has never answered and leaves [`MAX_UNANSWERED`] requests in a row unanswered is
//!   not asked again (unless it comes back after a link stall — [`ModeEnumerator::restart`]). Once it
//!   has answered, a lossy link gets [`MAX_UNANSWERED_AFTER_REPLY`] tries in a row before the read
//!   stops. ArduPilot 4.1 – 4.6 answers with COMMAND_ACK FAILED and nothing else. The ACK
//!   decides nothing: other 512 requests (AUTOPILOT_VERSION, HOME_POSITION) share the command id, so a
//!   FAILED 512 can't be attributed.
//! - **Index by index** (param2 = i from 1): the "all" request (param2 = 0) pushes one message and leaves
//!   the rest to the SRn_PARAMS stream, whose default rate is 0 on Copter and Sub. One request in flight.
//! - **Peripheral-proof.** The request is addressed to the vehicle's autopilot component (never
//!   target_component 0, which ArduPilot forwards to every component behind it), and only replies from
//!   that component are accepted ([`from_autopilot`]); everything is validated before it is stored.
//! - **AVAILABLE_MODES_MONITOR** carries a sequence number the FC bumps when its mode set changes (4.7.x
//!   streams it every 5 s after the first request): a different `seq` re-reads the list, an unchanged
//!   one does nothing. A complete list is emitted once per read; a partial list never.
//!
//! Pure state machine with injected time: the handler loop asks [`ModeEnumerator::poll`] whether to send
//! a request and feeds replies to [`ModeEnumerator::on_mode`] / [`ModeEnumerator::on_monitor`], and
//! calls [`ModeEnumerator::restart`] when the vehicle is heard again after a stall.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use ::mavlink::ardupilotmega::{MavCmd, MavMessage, AVAILABLE_MODES_DATA, COMMAND_LONG_DATA};
use ::mavlink::MavHeader;
use serde::Serialize;

/// AVAILABLE_MODES message id (common #435).
const MSG_ID_AVAILABLE_MODES: f32 = 435.0;
/// Pause between the vehicle context becoming usable and the first request — lets the connect burst
/// (stream-rate commands, parameter reads, identity request) through first.
pub const START_DELAY: Duration = Duration::from_secs(2);
/// No reply within this window → the request counts as unanswered and is sent again. Generous for slow
/// radio links.
pub const REPLY_TIMEOUT: Duration = Duration::from_millis(1500);
/// Consecutive unanswered requests after which a vehicle that has NEVER answered keeps the static tables
/// — the feature check: ArduPilot 4.1 – 4.6 does not send AVAILABLE_MODES at all.
pub const MAX_UNANSWERED: u8 = 3;
/// Consecutive unanswered requests after which a vehicle that HAS answered before is no longer asked
/// (≈ 30 s at [`REPLY_TIMEOUT`]): a lossy or slow link keeps reading, a dead FC is not asked forever.
pub const MAX_UNANSWERED_AFTER_REPLY: u8 = 20;
/// MAV_MODE_PROPERTY_NOT_USER_SELECTABLE — the FC refuses this mode from a GCS (or it is FC-entered).
pub const PROPERTY_NOT_USER_SELECTABLE: u32 = 2;

/// One FC-reported mode, as the frontend receives it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FcMode {
    /// ArduPilot: the flat mode number (the value MAV_CMD_DO_SET_MODE takes).
    pub custom_mode: u32,
    /// MAV_MODE_PROPERTY bitmask (1 advanced, 2 not user-selectable, 4 auto mode).
    pub properties: u32,
    /// The firmware's own name ("Altitude Hold", "Smart RTL", a Lua mode's name, …).
    pub name: String,
}

impl FcMode {
    fn from_msg(m: &AVAILABLE_MODES_DATA) -> Self {
        let raw: &[u8] = &m.mode_name[..];
        let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        FcMode {
            custom_mode: m.custom_mode,
            properties: m.properties.bits(),
            name: String::from_utf8_lossy(&raw[..end]).trim().to_string(),
        }
    }
}

/// Payload of the `available-modes` event (the vehicle's emitter stamps `vehicleId` / `linkId`).
#[derive(Serialize)]
pub struct AvailableModesEvent<'a> {
    pub modes: &'a [FcMode],
}

/// Whether a frame comes from the vehicle's autopilot component. AVAILABLE_MODES / _MONITOR from any
/// other system or component (gimbal, camera, companion, radio, a second vehicle) is ignored.
pub fn from_autopilot(header: &MavHeader, vehicle_sysid: u8, autopilot_compid: u8) -> bool {
    header.system_id == vehicle_sysid && header.component_id == autopilot_compid
}

/// `MAV_CMD_REQUEST_MESSAGE` for one AVAILABLE_MODES index, addressed to the vehicle's autopilot.
pub fn request_msg(sysid: u8, compid: u8, index: u8) -> MavMessage {
    MavMessage::COMMAND_LONG(COMMAND_LONG_DATA {
        target_system: sysid,
        target_component: compid,
        command: MavCmd::MAV_CMD_REQUEST_MESSAGE,
        confirmation: 0,
        param1: MSG_ID_AVAILABLE_MODES,
        param2: index as f32,
        param3: 0.0,
        param4: 0.0,
        param5: 0.0,
        param6: 0.0,
        param7: 0.0,
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    /// Not asking yet (start delay); replies that arrive anyway are stored.
    Waiting { until: Instant },
    /// Asking index by index.
    Collecting,
    /// The list is complete and was handed out; a MONITOR `seq` change or [`ModeEnumerator::restart`]
    /// reopens it.
    Complete,
    /// Unanswered too often — no more requests until [`ModeEnumerator::restart`] (the vehicle came back
    /// after a link stall).
    GaveUp,
}

/// Per-vehicle AVAILABLE_MODES reader (see the module doc).
pub struct ModeEnumerator {
    /// For log lines only.
    sysid: u8,
    phase: Phase,
    /// `number_modes` of the replies being collected.
    total: Option<u8>,
    modes: BTreeMap<u8, FcMode>,
    /// The request in flight: (index, sent at).
    in_flight: Option<(u8, Instant)>,
    unanswered: u8,
    /// A valid reply was seen in this vehicle context — the FC has the feature, so a run of unanswered
    /// requests is link loss and gets the larger bound.
    answered: bool,
    monitor_seq: Option<u8>,
    /// The last complete list (re-announced after a frontend reload).
    last_complete: Option<Vec<FcMode>>,
}

impl ModeEnumerator {
    /// A reader that sends its first request [`START_DELAY`] after `now`.
    pub fn new(sysid: u8, now: Instant) -> Self {
        ModeEnumerator {
            sysid,
            phase: Phase::Waiting { until: now + START_DELAY },
            total: None,
            modes: BTreeMap::new(),
            in_flight: None,
            unanswered: 0,
            answered: false,
            monitor_seq: None,
            last_complete: None,
        }
    }

    /// The index to request now, if a request is due. At most one request is in flight: the next goes
    /// out when the previous one is answered or has timed out.
    pub fn poll(&mut self, now: Instant) -> Option<u8> {
        match self.phase {
            Phase::Waiting { until } if now < until => return None,
            Phase::Waiting { .. } => self.phase = Phase::Collecting,
            Phase::Collecting => {}
            Phase::Complete | Phase::GaveUp => return None,
        }
        if let Some((index, sent)) = self.in_flight {
            if !self.modes.contains_key(&index) {
                if now.duration_since(sent) < REPLY_TIMEOUT {
                    return None;
                }
                self.unanswered = self.unanswered.saturating_add(1);
                let limit = if self.answered { MAX_UNANSWERED_AFTER_REPLY } else { MAX_UNANSWERED };
                log::debug!(
                    "AVAILABLE_MODES: sysid={} index {} unanswered ({}/{})",
                    self.sysid, index, self.unanswered, limit,
                );
                if self.unanswered >= limit {
                    self.phase = Phase::GaveUp;
                    self.in_flight = None;
                    let received = format!(
                        "{} of {} received",
                        self.modes.len(),
                        self.total.map_or("?".to_string(), |n| n.to_string()),
                    );
                    if self.answered {
                        log::info!(
                            "AVAILABLE_MODES: sysid={} stopped answering ({} requests in a row, {}) — {}",
                            self.sysid, limit, received,
                            if self.last_complete.is_some() { "keeping the previous list" } else { "static mode table" },
                        );
                    } else {
                        log::info!(
                            "AVAILABLE_MODES: sysid={} no answer after {} requests — static mode table",
                            self.sysid, limit,
                        );
                    }
                    return None;
                }
            }
        }
        let total = self.total.unwrap_or(1);
        let next = (1..=total).find(|i| !self.modes.contains_key(i)).unwrap_or(1);
        self.in_flight = Some((next, now));
        log::debug!("AVAILABLE_MODES: sysid={} requesting index {}/{}", self.sysid, next, total);
        Some(next)
    }

    /// Feed one AVAILABLE_MODES (already checked with [`from_autopilot`]). Returns the full list, in
    /// index order, the moment it becomes complete — once per read.
    pub fn on_mode(&mut self, m: &AVAILABLE_MODES_DATA) -> Option<Vec<FcMode>> {
        if !matches!(self.phase, Phase::Waiting { .. } | Phase::Collecting) {
            return None;
        }
        let (n, index) = (m.number_modes, m.mode_index);
        if n == 0 || index == 0 || index > n {
            log::debug!("AVAILABLE_MODES: sysid={} dropped invalid index {}/{}", self.sysid, index, n);
            return None;
        }
        self.unanswered = 0;
        self.answered = true;
        if self.total != Some(n) {
            if let Some(prev) = self.total {
                log::debug!("AVAILABLE_MODES: sysid={} mode count changed {} → {} — collecting again", self.sysid, prev, n);
            }
            self.modes.clear();
            self.total = Some(n);
            // A request for an index the FC no longer has would never be answered.
            if self.in_flight.is_some_and(|(i, _)| i > n) {
                self.in_flight = None;
            }
        }
        let mode = FcMode::from_msg(m);
        log::debug!(
            "AVAILABLE_MODES: sysid={} {}/{} = {} \"{}\" (properties {})",
            self.sysid, index, n, mode.custom_mode, mode.name, mode.properties,
        );
        self.modes.insert(index, mode);
        if self.modes.len() < n as usize {
            return None;
        }
        self.phase = Phase::Complete;
        self.in_flight = None;
        let list: Vec<FcMode> = self.modes.values().cloned().collect();
        let selectable = list.iter().filter(|m| m.properties & PROPERTY_NOT_USER_SELECTABLE == 0).count();
        log::info!("AVAILABLE_MODES: sysid={} FC reports {} modes ({} selectable)", self.sysid, list.len(), selectable);
        self.last_complete = Some(list.clone());
        Some(list)
    }

    /// Feed one AVAILABLE_MODES_MONITOR (already checked with [`from_autopilot`]). A `seq` different from
    /// the last one seen means the FC's mode set changed → read the list again (the previous complete list
    /// stays valid until the new one is complete). Returns whether a new read started.
    pub fn on_monitor(&mut self, seq: u8) -> bool {
        if self.phase == Phase::GaveUp {
            return false;
        }
        let changed = self.monitor_seq.is_some_and(|prev| prev != seq);
        self.monitor_seq = Some(seq);
        if !changed {
            return false;
        }
        log::info!("AVAILABLE_MODES: sysid={} mode set changed (monitor seq {}) — reading the list again", self.sysid, seq);
        self.modes.clear();
        self.total = None;
        self.unanswered = 0;
        // A request still in flight stays the one in flight (its reply is stored under the new count).
        match self.phase {
            Phase::Waiting { .. } => {}
            _ => self.phase = Phase::Collecting,
        }
        true
    }

    /// Read the list again from scratch after [`START_DELAY`] — the vehicle came back after a link stall,
    /// possibly rebooted with a different mode set (Q_ENABLE, Lua modes, GCSBLOCK) and no MONITOR to say
    /// so. Also revives a reader that gave up. No-op (returns false) while a read is already running. The
    /// previous complete list stays valid until the new one is complete; the MONITOR `seq` is forgotten
    /// (a rebooted FC starts it again at 0).
    pub fn restart(&mut self, now: Instant) -> bool {
        if matches!(self.phase, Phase::Waiting { .. } | Phase::Collecting) {
            return false;
        }
        self.phase = Phase::Waiting { until: now + START_DELAY };
        self.modes.clear();
        self.total = None;
        self.in_flight = None;
        self.unanswered = 0;
        self.monitor_seq = None;
        true
    }

    /// The last complete list, if any.
    pub fn last_complete(&self) -> Option<&[FcMode]> {
        self.last_complete.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::mavlink::ardupilotmega::{MavModeProperty, MavStandardMode};

    fn mode(n: u8, index: u8, custom: u32, name: &str) -> AVAILABLE_MODES_DATA {
        AVAILABLE_MODES_DATA {
            custom_mode: custom,
            properties: MavModeProperty::empty(),
            number_modes: n,
            mode_index: index,
            standard_mode: MavStandardMode::DEFAULT,
            mode_name: name.into(),
        }
    }

    fn header(sysid: u8, compid: u8) -> MavHeader {
        MavHeader { system_id: sysid, component_id: compid, sequence: 0 }
    }

    /// Past the start delay, with one request (index 1) sent.
    fn started(t0: Instant) -> ModeEnumerator {
        let mut e = ModeEnumerator::new(1, t0);
        assert_eq!(e.poll(t0 + START_DELAY), Some(1));
        e
    }

    #[test]
    fn waits_for_the_start_delay_then_asks_index_one() {
        let t0 = Instant::now();
        let mut e = ModeEnumerator::new(1, t0);
        assert_eq!(e.poll(t0), None);
        assert_eq!(e.poll(t0 + START_DELAY - Duration::from_millis(1)), None);
        assert_eq!(e.poll(t0 + START_DELAY), Some(1));
        // One in flight: nothing more until it is answered or times out.
        assert_eq!(e.poll(t0 + START_DELAY + Duration::from_millis(10)), None);
    }

    #[test]
    fn walks_the_indices_and_emits_the_complete_list_once() {
        let t0 = Instant::now();
        let mut e = started(t0);
        let t = t0 + START_DELAY;
        assert_eq!(e.on_mode(&mode(3, 1, 0, "Stabilize")), None);
        assert_eq!(e.poll(t + Duration::from_millis(50)), Some(2));
        assert_eq!(e.on_mode(&mode(3, 2, 5, "Loiter")), None);
        assert_eq!(e.poll(t + Duration::from_millis(100)), Some(3));
        let list = e.on_mode(&mode(3, 3, 4, "Guided")).expect("complete");
        assert_eq!(list.iter().map(|m| m.custom_mode).collect::<Vec<_>>(), vec![0, 5, 4]);
        assert_eq!(list[1].name, "Loiter");
        // Complete: no more requests, a duplicate is not emitted again.
        assert_eq!(e.poll(t + Duration::from_secs(10)), None);
        assert_eq!(e.on_mode(&mode(3, 3, 4, "Guided")), None);
        assert_eq!(e.last_complete().map(|l| l.len()), Some(3));
    }

    #[test]
    fn gives_up_after_three_unanswered_requests_when_never_answered() {
        let t0 = Instant::now();
        let mut e = started(t0);
        let mut t = t0 + START_DELAY;
        // Two retries of index 1 …
        t += REPLY_TIMEOUT;
        assert_eq!(e.poll(t), Some(1));
        t += REPLY_TIMEOUT;
        assert_eq!(e.poll(t), Some(1));
        // … the third timeout ends it.
        t += REPLY_TIMEOUT;
        assert_eq!(e.poll(t), None);
        assert_eq!(e.poll(t + Duration::from_secs(60)), None);
        // No late reply or MONITOR revives it (only `restart`, see below).
        assert_eq!(e.on_mode(&mode(1, 1, 0, "Stabilize")), None);
        assert!(!e.on_monitor(0));
        assert!(!e.on_monitor(1));
        assert_eq!(e.poll(t + Duration::from_secs(120)), None);
    }

    #[test]
    fn a_reply_resets_the_unanswered_counter() {
        let t0 = Instant::now();
        let mut e = started(t0);
        let mut t = t0 + START_DELAY;
        t += REPLY_TIMEOUT;
        assert_eq!(e.poll(t), Some(1)); // 1 unanswered
        t += REPLY_TIMEOUT;
        assert_eq!(e.poll(t), Some(1)); // 2 unanswered
        assert_eq!(e.on_mode(&mode(4, 1, 0, "Stabilize")), None); // reset
        assert_eq!(e.poll(t), Some(2));
        t += REPLY_TIMEOUT;
        assert_eq!(e.poll(t), Some(2)); // 1 unanswered
        t += REPLY_TIMEOUT;
        assert_eq!(e.poll(t), Some(2)); // 2 unanswered — still asking
        assert_eq!(e.on_mode(&mode(4, 2, 1, "Acro")), None);
        assert_eq!(e.poll(t), Some(3));
    }

    #[test]
    fn invalid_indices_are_dropped_and_do_not_count_as_replies() {
        let t0 = Instant::now();
        let mut e = started(t0);
        let t = t0 + START_DELAY;
        assert_eq!(e.on_mode(&mode(0, 0, 0, "x")), None);
        assert_eq!(e.on_mode(&mode(0, 1, 0, "x")), None);
        assert_eq!(e.on_mode(&mode(3, 0, 0, "x")), None);
        assert_eq!(e.on_mode(&mode(3, 4, 0, "x")), None);
        assert_eq!(e.on_mode(&mode(3, 255, 0, "x")), None);
        // Nothing stored, nothing counted as an answer: index 1 is still pending and times out normally.
        assert_eq!(e.poll(t + Duration::from_millis(100)), None);
        let mut tt = t + REPLY_TIMEOUT;
        assert_eq!(e.poll(tt), Some(1));
        tt += REPLY_TIMEOUT;
        assert_eq!(e.poll(tt), Some(1));
        tt += REPLY_TIMEOUT;
        assert_eq!(e.poll(tt), None, "garbage never resets the counter");
    }

    #[test]
    fn duplicates_and_unrequested_indices_are_stored_once() {
        let t0 = Instant::now();
        let mut e = started(t0);
        let t = t0 + START_DELAY;
        assert_eq!(e.on_mode(&mode(3, 1, 0, "Stabilize")), None);
        assert_eq!(e.on_mode(&mode(3, 1, 0, "Stabilize")), None);
        // Index 3 pushed without being asked for — kept.
        assert_eq!(e.on_mode(&mode(3, 3, 9, "Land")), None);
        // Only index 2 is still missing.
        assert_eq!(e.poll(t + Duration::from_millis(10)), Some(2));
        let list = e.on_mode(&mode(3, 2, 6, "RTL")).expect("complete");
        assert_eq!(list.len(), 3);
        assert_eq!(list.iter().map(|m| m.custom_mode).collect::<Vec<_>>(), vec![0, 6, 9]);
    }

    #[test]
    fn a_changed_mode_count_restarts_the_collection() {
        let t0 = Instant::now();
        let mut e = started(t0);
        let t = t0 + START_DELAY;
        assert_eq!(e.on_mode(&mode(3, 1, 0, "Stabilize")), None);
        assert_eq!(e.poll(t), Some(2));
        assert_eq!(e.on_mode(&mode(3, 2, 5, "Loiter")), None);
        assert_eq!(e.poll(t), Some(3));
        // The FC's set changed: index 3 arrives with a count of 4 → the old indices are void.
        assert_eq!(e.on_mode(&mode(4, 3, 7, "Circle")), None);
        assert_eq!(e.poll(t), Some(1));
        assert_eq!(e.on_mode(&mode(4, 1, 0, "Stabilize")), None);
        assert_eq!(e.poll(t), Some(2));
        assert_eq!(e.on_mode(&mode(4, 2, 5, "Loiter")), None);
        assert_eq!(e.poll(t), Some(4));
        let list = e.on_mode(&mode(4, 4, 4, "Guided")).expect("complete");
        assert_eq!(list.iter().map(|m| m.custom_mode).collect::<Vec<_>>(), vec![0, 5, 7, 4]);
    }

    #[test]
    fn a_shrinking_count_drops_a_request_the_fc_can_no_longer_answer() {
        let t0 = Instant::now();
        let mut e = started(t0);
        let t = t0 + START_DELAY;
        assert_eq!(e.on_mode(&mode(5, 1, 0, "Stabilize")), None);
        assert_eq!(e.poll(t), Some(2));
        assert_eq!(e.on_mode(&mode(5, 2, 1, "Acro")), None);
        assert_eq!(e.poll(t), Some(3));
        assert_eq!(e.on_mode(&mode(5, 3, 2, "AltHold")), None);
        assert_eq!(e.poll(t), Some(4));
        // Index 4 in flight, then the FC reports only 2 modes.
        assert_eq!(e.on_mode(&mode(2, 1, 0, "Stabilize")), None);
        // Not waiting for index 4's timeout: index 2 goes out now.
        assert_eq!(e.poll(t + Duration::from_millis(1)), Some(2));
    }

    #[test]
    fn a_late_reply_after_a_retry_is_no_problem() {
        let t0 = Instant::now();
        let mut e = started(t0);
        let mut t = t0 + START_DELAY;
        t += REPLY_TIMEOUT;
        assert_eq!(e.poll(t), Some(1)); // retry of index 1
        // The first request's reply arrives late, then the retry's reply.
        assert_eq!(e.on_mode(&mode(2, 1, 0, "Stabilize")), None);
        assert_eq!(e.poll(t + Duration::from_millis(5)), Some(2));
        assert_eq!(e.on_mode(&mode(2, 1, 0, "Stabilize")), None); // duplicate of the retry
        // Index 2 is still the one in flight — no second request for it.
        assert_eq!(e.poll(t + Duration::from_millis(10)), None);
        let list = e.on_mode(&mode(2, 2, 6, "RTL")).expect("complete");
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn monitor_seq_change_reads_again_unchanged_seq_does_nothing() {
        let t0 = Instant::now();
        let mut e = started(t0);
        let t = t0 + START_DELAY;
        assert!(e.on_mode(&mode(1, 1, 0, "Stabilize")).is_some());
        // 4.7.x streams the monitor every 5 s: the first seq is just remembered, repeats do nothing.
        assert!(!e.on_monitor(0));
        assert!(!e.on_monitor(0));
        assert_eq!(e.poll(t + Duration::from_secs(5)), None);
        // seq bumped → read again from index 1; the old complete list stays available meanwhile.
        assert!(e.on_monitor(1));
        assert_eq!(e.last_complete().map(|l| l.len()), Some(1));
        assert_eq!(e.poll(t + Duration::from_secs(10)), Some(1));
        assert_eq!(e.on_mode(&mode(2, 1, 0, "Stabilize")), None);
        assert_eq!(e.poll(t + Duration::from_secs(10)), Some(2));
        let list = e.on_mode(&mode(2, 2, 23, "Follow")).expect("complete again");
        assert_eq!(list.len(), 2);
        assert_eq!(e.last_complete().map(|l| l.len()), Some(2));
    }

    #[test]
    fn replies_before_the_first_request_are_kept() {
        let t0 = Instant::now();
        let mut e = ModeEnumerator::new(1, t0);
        assert_eq!(e.on_mode(&mode(2, 2, 6, "RTL")), None);
        assert_eq!(e.poll(t0 + START_DELAY), Some(1));
        assert!(e.on_mode(&mode(2, 1, 0, "Stabilize")).is_some());
    }

    #[test]
    fn only_the_vehicles_autopilot_component_is_accepted() {
        // Vehicle sysid 1, autopilot compid 1.
        assert!(from_autopilot(&header(1, 1), 1, 1));
        // Wrong component on the vehicle's system (gimbal 154, camera 100, companion 191, radio 68).
        for comp in [154, 100, 191, 68, 0] {
            assert!(!from_autopilot(&header(1, comp), 1, 1), "compid {comp}");
        }
        // Wrong system (another vehicle or a peripheral with its own sysid), even as compid 1.
        assert!(!from_autopilot(&header(2, 1), 1, 1));
        assert!(!from_autopilot(&header(255, 1), 1, 1));
        // A vehicle whose autopilot heartbeats from another compid is matched on that compid.
        assert!(from_autopilot(&header(3, 2), 3, 2));
        assert!(!from_autopilot(&header(3, 1), 3, 2));
    }

    #[test]
    fn request_is_addressed_to_the_autopilot_never_broadcast() {
        let MavMessage::COMMAND_LONG(c) = request_msg(7, 1, 4) else { panic!("COMMAND_LONG") };
        assert_eq!(c.target_system, 7);
        assert_eq!(c.target_component, 1);
        assert_eq!(c.command, MavCmd::MAV_CMD_REQUEST_MESSAGE);
        assert_eq!(c.param1, 435.0);
        assert_eq!(c.param2, 4.0);
    }

    #[test]
    fn fc_name_is_cut_at_the_terminator() {
        let mut m = mode(1, 1, 2, "Altitude Hold");
        assert_eq!(FcMode::from_msg(&m).name, "Altitude Hold");
        m.mode_name = [b'A'; 35].into(); // no terminator, full length
        assert_eq!(FcMode::from_msg(&m).name.len(), 35);
        m.properties = MavModeProperty::MAV_MODE_PROPERTY_NOT_USER_SELECTABLE;
        assert_eq!(FcMode::from_msg(&m).properties, PROPERTY_NOT_USER_SELECTABLE);
    }

    /// Timeouts in a row from `t` on: each `poll` must re-request (returns the index asked).
    fn time_out(e: &mut ModeEnumerator, t: &mut Instant, times: u8) -> Vec<Option<u8>> {
        (0..times).map(|_| { *t += REPLY_TIMEOUT; e.poll(*t) }).collect()
    }

    #[test]
    fn once_answered_a_lossy_link_gets_the_larger_bound() {
        let t0 = Instant::now();
        let mut e = started(t0);
        let mut t = t0 + START_DELAY;
        assert_eq!(e.on_mode(&mode(5, 1, 0, "Stabilize")), None);
        assert_eq!(e.poll(t), Some(2));
        // 19 unanswered in a row: still asking for index 2 …
        let asked = time_out(&mut e, &mut t, MAX_UNANSWERED_AFTER_REPLY - 1);
        assert!(asked.iter().all(|a| *a == Some(2)), "{asked:?}");
        // … the 20th ends it.
        assert_eq!(time_out(&mut e, &mut t, 1), vec![None]);
        assert_eq!(e.poll(t + Duration::from_secs(60)), None);
    }

    #[test]
    fn answered_once_then_lossy_still_completes() {
        let t0 = Instant::now();
        let mut e = started(t0);
        let mut t = t0 + START_DELAY;
        assert_eq!(e.on_mode(&mode(4, 1, 0, "Stabilize")), None);
        assert_eq!(e.poll(t), Some(2));
        assert!(time_out(&mut e, &mut t, 10).iter().all(|a| *a == Some(2)));
        assert_eq!(e.on_mode(&mode(4, 2, 1, "Acro")), None);
        assert_eq!(e.poll(t), Some(3));
        assert!(time_out(&mut e, &mut t, 15).iter().all(|a| *a == Some(3)));
        assert_eq!(e.on_mode(&mode(4, 3, 2, "AltHold")), None);
        assert_eq!(e.poll(t), Some(4));
        let list = e.on_mode(&mode(4, 4, 4, "Guided")).expect("complete despite 25 losses");
        assert_eq!(list.len(), 4);
    }

    #[test]
    fn restart_rereads_after_the_start_delay_and_keeps_the_old_list() {
        let t0 = Instant::now();
        let mut e = started(t0);
        let t = t0 + START_DELAY;
        assert!(e.on_mode(&mode(1, 1, 0, "Stabilize")).is_some());
        assert!(!e.on_monitor(3)); // seq 3 remembered
        // The vehicle comes back after a stall (maybe rebooted).
        let back = t + Duration::from_secs(30);
        assert!(e.restart(back));
        assert_eq!(e.last_complete().map(|l| l.len()), Some(1), "old list valid meanwhile");
        assert_eq!(e.poll(back), None, "start delay first");
        assert_eq!(e.poll(back + START_DELAY), Some(1));
        // The rebooted FC's MONITOR starts at 0 again: remembered, not a second re-read.
        assert!(!e.on_monitor(0));
        assert_eq!(e.on_mode(&mode(2, 1, 0, "Stabilize")), None);
        assert_eq!(e.poll(back + START_DELAY), Some(2));
        let list = e.on_mode(&mode(2, 2, 19, "QLoiter")).expect("complete again");
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn restart_is_a_no_op_while_a_read_is_running() {
        let t0 = Instant::now();
        let mut e = ModeEnumerator::new(1, t0);
        assert!(!e.restart(t0), "waiting");
        assert_eq!(e.poll(t0 + START_DELAY), Some(1));
        assert!(!e.restart(t0 + START_DELAY), "collecting");
        // Still the same request in flight — no second one.
        assert_eq!(e.poll(t0 + START_DELAY + Duration::from_millis(10)), None);
    }

    #[test]
    fn restart_revives_a_reader_that_gave_up() {
        let t0 = Instant::now();
        let mut e = started(t0);
        let mut t = t0 + START_DELAY;
        assert_eq!(time_out(&mut e, &mut t, MAX_UNANSWERED), vec![Some(1), Some(1), None]);
        assert!(e.restart(t));
        assert_eq!(e.poll(t + START_DELAY), Some(1));
        assert!(e.on_mode(&mode(1, 1, 0, "Stabilize")).is_some());
    }

    #[test]
    fn monitor_seq_change_while_waiting_keeps_the_start_delay() {
        let t0 = Instant::now();
        let mut e = ModeEnumerator::new(1, t0);
        assert_eq!(e.on_mode(&mode(3, 2, 5, "Loiter")), None);
        assert!(!e.on_monitor(0));
        assert!(e.on_monitor(1));
        // No request before the start delay, and the early reply was discarded with the old set.
        assert_eq!(e.poll(t0 + Duration::from_millis(500)), None);
        assert_eq!(e.poll(t0 + START_DELAY), Some(1));
        assert_eq!(e.on_mode(&mode(2, 1, 0, "Stabilize")), None);
        assert_eq!(e.poll(t0 + START_DELAY), Some(2));
    }

    #[test]
    fn a_count_change_after_complete_is_ignored() {
        let t0 = Instant::now();
        let mut e = started(t0);
        let t = t0 + START_DELAY;
        assert!(e.on_mode(&mode(1, 1, 0, "Stabilize")).is_some());
        // A stray reply with another count after the list is complete: no new read, no emit.
        assert_eq!(e.on_mode(&mode(3, 1, 0, "Stabilize")), None);
        assert_eq!(e.poll(t + Duration::from_secs(5)), None);
        assert_eq!(e.last_complete().map(|l| l.len()), Some(1));
    }

    /// Opt-in, against a live ArduPilot 4.7+ SITL: `KITE_SITL_TCP=127.0.0.1:5760 cargo test --lib
    /// sitl_reads_the_mode_list -- --ignored --nocapture`. Drives this reader with Kite's own parser and
    /// encoder over TCP, the way the handler loop does (the loop itself needs a Tauri app handle).
    #[test]
    #[ignore]
    fn sitl_reads_the_mode_list() {
        use std::io::{Read, Write};
        let Ok(addr) = std::env::var("KITE_SITL_TCP") else {
            eprintln!("KITE_SITL_TCP not set — skipped");
            return;
        };
        let mut tcp = std::net::TcpStream::connect(&addr).expect("connect to SITL");
        tcp.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
        let mut parser = crate::mavlink_proto::parser::MavParser::new();
        let mut seq = crate::mavlink_proto::codec::MavSequence::new();
        let gcs = crate::mavlink_proto::codec::gcs_header();
        let (mut vehicle, mut reader, mut list) = (None::<(u8, u8)>, None::<ModeEnumerator>, None);
        let mut buf = [0u8; 1024];
        let deadline = Instant::now() + Duration::from_secs(60);
        while list.is_none() && Instant::now() < deadline {
            if let (Some((sys, comp)), Some(r)) = (vehicle, reader.as_mut()) {
                if let Some(i) = r.poll(Instant::now()) {
                    tcp.write_all(&crate::mavlink_proto::codec::serialize_v2(&gcs, &request_msg(sys, comp, i), &mut seq)).unwrap();
                }
            }
            let n = match tcp.read(&mut buf) { Ok(n) => n, Err(_) => continue };
            for p in parser.parse_all(&buf[..n]) {
                let crate::mavlink_proto::parser::Parsed::Msg(f) = p else { continue };
                match (&f.message, vehicle) {
                    (MavMessage::HEARTBEAT(_), None) if f.header.component_id == 1 => {
                        vehicle = Some((f.header.system_id, 1));
                        reader = Some(ModeEnumerator::new(f.header.system_id, Instant::now()));
                    }
                    (MavMessage::AVAILABLE_MODES(m), Some((sys, comp))) if from_autopilot(&f.header, sys, comp) => {
                        list = reader.as_mut().and_then(|r| r.on_mode(m));
                    }
                    _ => {}
                }
            }
        }
        let list = list.expect("complete list within 60 s");
        for m in &list {
            eprintln!("{:3} {:<16} properties {}", m.custom_mode, m.name, m.properties);
        }
        eprintln!("SITL reports {} modes", list.len());
        assert!(!list.is_empty());
    }
}
