// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Protocol detector — a reference table of framing signatures that classifies the incoming passive
// telemetry stream and locks onto one protocol per session.
//
// IMPORTANT: the signatures below are **provisional heuristics**. The whole point of Phase B is to
// capture a real stream from EdgeTX/ETHOS and confirm what the radios actually emit (FrSkyX may well be
// a decoded plain-text variant with no 0x7E framing at all). The detector exists so the Debug Monitor
// can show a live "best guess"; the capture file is the authoritative source for building the real
// decoders. Counts are crude and only meant to surface which framing dominates.

use std::collections::VecDeque;

use crate::msp::codec::MspCodec;

/// Protocols we may eventually decode. FrSky is the one wired first.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Frsky,
    Crsf,
    Ltm,
    Mavlink,
}

impl Protocol {
    pub fn name(self) -> &'static str {
        match self {
            Protocol::Frsky => "FrSkyX/SmartPort",
            Protocol::Crsf => "CRSF",
            Protocol::Ltm => "LTM",
            Protocol::Mavlink => "MAVLink",
        }
    }
}

/// Registered protocols, in table order. FrSky first (the one we start with).
pub const REGISTERED: [Protocol; 4] = [
    Protocol::Frsky,
    Protocol::Crsf,
    Protocol::Ltm,
    Protocol::Mavlink,
];

/// Number of carry bytes kept between chunks so signatures spanning a chunk/notification boundary are
/// still counted (and not double-counted). Sized to hold a full CRSF frame (max 64 B) so CRC-validated
/// CRSF frames split across a BLE notification boundary are still recognized.
const CARRY: usize = 64;

/// Minimum plausible-frame hits before a protocol may lock, and the dominance it must show over the
/// runner-up (winner ≥ 2× the rest combined).
const LOCK_MIN_HITS: u32 = 8;

pub struct Detector {
    tail: VecDeque<u8>,
    hits: [u32; REGISTERED.len()],
    locked: Option<Protocol>,
    total_bytes: u64,
}

impl Detector {
    pub fn new() -> Self {
        Self {
            tail: VecDeque::with_capacity(CARRY),
            hits: [0; REGISTERED.len()],
            locked: None,
            total_bytes: 0,
        }
    }

    /// Feed a freshly-read chunk. Updates per-protocol hit counters and may lock a protocol.
    pub fn push(&mut self, data: &[u8]) {
        self.total_bytes += data.len() as u64;

        // Scan window = carried tail + new data. We only credit a signature whose **end** index lands
        // in the new region (>= carry_len), so markers already counted last time aren't recounted.
        let carry_len = self.tail.len();
        let mut scan = Vec::with_capacity(carry_len + data.len());
        scan.extend(self.tail.iter().copied());
        scan.extend_from_slice(data);

        self.hits[0] += scan_frsky(&scan, carry_len);
        self.hits[1] += scan_crsf(&scan, carry_len);
        self.hits[2] += scan_ltm(&scan, carry_len);
        self.hits[3] += scan_mavlink(&scan, carry_len);

        // Keep the last CARRY bytes for the next boundary.
        self.tail.clear();
        let keep = scan.len().min(CARRY);
        for &b in &scan[scan.len() - keep..] {
            self.tail.push_back(b);
        }

        if self.locked.is_none() {
            if let Some((idx, &winner)) = self
                .hits
                .iter()
                .enumerate()
                .max_by_key(|(_, c)| **c)
            {
                let total: u32 = self.hits.iter().sum();
                let rest = total - winner;
                if winner >= LOCK_MIN_HITS && winner > rest * 2 {
                    self.locked = Some(REGISTERED[idx]);
                }
            }
        }
    }

    pub fn locked(&self) -> Option<Protocol> {
        self.locked
    }

    /// Best current guess (highest hit count), regardless of lock — or None if nothing matched yet.
    pub fn best_guess(&self) -> Option<Protocol> {
        let (idx, &c) = self.hits.iter().enumerate().max_by_key(|(_, c)| **c)?;
        if c == 0 {
            None
        } else {
            Some(REGISTERED[idx])
        }
    }

    /// (protocol-name, hit-count) pairs for the Debug Monitor.
    pub fn hit_table(&self) -> Vec<(&'static str, u32)> {
        REGISTERED
            .iter()
            .zip(self.hits.iter())
            .map(|(p, &c)| (p.name(), c))
            .collect()
    }

    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }
}

// ── Provisional signature scanners ───────────────────────────────────────────
// Each returns the number of plausible frame starts found whose marker ends at/after `carry_len`.

/// FrSky S.Port: 0x7E frame delimiter (byte-stuffed payloads escape literal 0x7E). Plain-text EdgeTX
/// variants would score ~0 here — itself a useful signal.
fn scan_frsky(scan: &[u8], carry_len: usize) -> u32 {
    let mut n = 0;
    for (i, &b) in scan.iter().enumerate() {
        if b == 0x7E && i >= carry_len {
            n += 1;
        }
    }
    n
}

/// CRSF: a full CRC-validated frame `<sync> <len> <type> <payload> <crc8>`. We require the CRC8/DVB-S2
/// (the same poly as MSP v2) over `type..payload` to match the trailing byte — a CRC-valid frame is
/// effectively false-positive-free, so CRSF locks cleanly even next to FrSky's raw 0x7E counting.
/// `sync` may be the FC (0xC8), radio TX (0xEA) or TX module (0xEE) address depending on how the radio
/// re-wraps the forwarded stream.
fn scan_crsf(scan: &[u8], carry_len: usize) -> u32 {
    let mut n = 0;
    let mut i = 0;
    while i + 1 < scan.len() {
        let sync = scan[i];
        let len = scan[i + 1] as usize;
        if matches!(sync, 0xC8 | 0xEA | 0xEE) && (2..=62).contains(&len) {
            let crc_idx = i + 1 + len; // sync, len, then `len` bytes (type..payload..crc)
            if crc_idx < scan.len() {
                let crc = MspCodec::crc8_dvb_s2(&scan[i + 2..crc_idx]); // over type..payload
                if crc == scan[crc_idx] {
                    if crc_idx >= carry_len {
                        n += 1;
                    }
                    i = crc_idx + 1; // consume the validated frame
                    continue;
                }
            }
        }
        i += 1;
    }
    n
}

/// LTM: full XOR-validated frame `$T<type><payload><crc>` (CRC = XOR of payload only). Validating the
/// checksum makes the "$T" + type-letter signature false-positive-free, on par with the CRSF matcher.
fn scan_ltm(scan: &[u8], carry_len: usize) -> u32 {
    use super::decoders::ltm::ltm_payload_len;
    let mut n = 0;
    let mut i = 0;
    while i + 2 < scan.len() {
        if scan[i] == b'$' && scan[i + 1] == b'T' {
            if let Some(plen) = ltm_payload_len(scan[i + 2]) {
                let crc_idx = i + 3 + plen;
                if crc_idx < scan.len() {
                    let crc = scan[i + 3..crc_idx].iter().fold(0u8, |c, &b| c ^ b);
                    if crc == scan[crc_idx] {
                        if crc_idx >= carry_len {
                            n += 1;
                        }
                        i = crc_idx + 1;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    n
}

/// MAVLink: v1 magic 0xFE or v2 magic 0xFD.
fn scan_mavlink(scan: &[u8], carry_len: usize) -> u32 {
    let mut n = 0;
    for (i, &b) in scan.iter().enumerate() {
        if (b == 0xFD || b == 0xFE) && i >= carry_len {
            n += 1;
        }
    }
    n
}
