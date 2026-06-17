// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Experimental MSP-over-SmartPort probe (dev-only feasibility test).
//!
//! Our passive telemetry path is otherwise strictly listen-only. This probe is the **one** deliberate
//! exception: it tests whether an EdgeTX/ETHOS radio's BLE bridge (here: FrSky X20RS) forwards an
//! inbound S.Port frame into the S.Port uplink to the flight controller. If it does, MSP-over-telemetry
//! becomes possible as a secondary protocol; if not, the idea is dead and we keep the path listen-only.
//!
//! Method (mirrors INAV `src/main/telemetry/msp_shared.c`, native MSPv1/v2 over S.Port, and the user's
//! own ETHOS reference in `INAVConfiguratorLite/src/platform/ethos/transport.lua`):
//!   • TX: pack an MSP request into a single S.Port uplink frame — physID `0x0D`, primID `0x30`,
//!     status byte `start|version|seq`, then the MSP-V1 body `[size,cmd]` + XOR checksum, zero-padded
//!     to the 6-byte chunk, wrapped as `7E <physID> <primID> <appId:2 LE> <value:4 LE> <crc>` with
//!     FrSky byte-stuffing.
//!   • RX: an MSP reply arrives as physID `0x1B` (S.Port) / `0x00` (FPort), primID `0x32`; the 6 data
//!     bytes are the MSP chunk (status byte + payload). We reassemble across chunks by sequence number.
//!
//! For the probe we send `MSP_API_VERSION` (cmd 1, V1, no payload) — the smallest possible round-trip,
//! whose reply (3 bytes) fits in a single chunk.

use serde::Serialize;

// ── S.Port MSP transport constants (see reference) ──────────────────────────
const SP_LOCAL_ID: u8 = 0x0D; // physID for outgoing MSP requests
const SP_REQ_FRAME: u8 = 0x30; // primID for MSP requests
const SP_REPLY_FRAME: u8 = 0x32; // primID for MSP replies
const SP_REMOTE_ID: u8 = 0x1B; // physID of replies from the FC (S.Port)
const FP_REMOTE_ID: u8 = 0x00; // physID of replies from the FC (FPort)

// MSP-over-transport chunk status-byte layout.
const MSP_STARTFLAG: u8 = 0x10; // bit 4 = start of message
const MSP_VER_SHIFT: u8 = 5; // bits 5-6 = MSP version
const MSP_SEQ_MASK: u8 = 0x0F; // bits 0-3 = sequence number
const MSP_V1: u8 = 1;

/// The command we probe with — smallest round-trip, reply fits one chunk.
pub const MSP_API_VERSION: u8 = 1;

/// Number of TX frame-format variants we sweep while hunting for one the radio's BLE bridge accepts.
const TX_VARIANTS: usize = 4;
/// How many sends to spend on each variant before advancing (so a reply reliably attributes to the
/// currently-active variant given sub-second reply latency).
const SENDS_PER_VARIANT: u32 = 4;

/// FrSky S.Port checksum over the bytes after the physID (primID + appID + value): running sum with
/// carry fold, final `0xFF - sum`.
fn sport_crc(bytes: &[u8]) -> u8 {
    let mut crc: u16 = 0;
    for &b in bytes {
        crc += b as u16;
        crc += crc >> 8;
        crc &= 0xFF;
    }
    (0xFF - crc) as u8
}

/// Append a byte to an outgoing frame with FrSky 0x7D-stuffing (0x7E/0x7D → 0x7D, byte^0x20).
fn push_stuffed(out: &mut Vec<u8>, b: u8) {
    if b == 0x7E || b == 0x7D {
        out.push(0x7D);
        out.push(b ^ 0x20);
    } else {
        out.push(b);
    }
}

/// Probe diagnostics surfaced in the Debug Monitor (telemetry tab).
#[derive(Clone, Serialize, Default)]
pub struct MspProbeStats {
    /// Probe request frames written to the transport.
    pub tx_count: u64,
    /// Inbound MSP-reply chunks (primID 0x32) seen — even partial / mismatched.
    pub rx_chunks: u64,
    /// Complete MSP replies reassembled (any command — includes the radio's own MSP polling that the
    /// BLE bridge mirrors to us).
    pub replies: u64,
    /// Complete replies whose command matches OUR probe request (`MSP_API_VERSION`) — the signal that
    /// our injected writes actually reach the FC, separate from passively-sniffed traffic.
    pub probe_replies: u64,
    /// Distinct reply command IDs seen, with counts, e.g. "1×3, 101×12, 106×8".
    pub cmds_seen: String,
    /// Command ID of the last complete reply, or -1 if none yet.
    pub last_reply_cmd: i32,
    /// Payload hex of the last complete reply (for inspection).
    pub last_reply_hex: String,
    /// The last TX frame we wrote, as hex (so the exact wire bytes are visible).
    pub last_tx_hex: String,
    /// Which frame-format variant the last TX used (e.g. "#0 7E+crc+stuff"). When a reply arrives this
    /// names the variant that worked.
    pub last_tx_variant: String,
    /// The most recent inbound 0x32 reply chunks (raw 6-byte hex, newest last), joined by " | ".
    /// Diagnostic: lets us see the actual status byte / header layout when reassembly fails.
    pub last_rx_hex: String,
    /// Human-readable parse of the most recent chunk (version, start/cont, seq, cmd, size, skip reason).
    pub last_rx_note: String,
}

/// Streaming MSP-over-S.Port probe: builds request frames and reassembles reply chunks. Keeps its own
/// 0x7E framing accumulator (independent of the sensor-data `FrskyDecoder`, which drops non-0x10 frames).
pub struct MspProbe {
    acc: Vec<u8>,
    // RX reassembly state.
    rx_buf: Vec<u8>,
    rx_size: usize,
    rx_cmd: u16,
    rx_started: bool,
    rx_remote_seq: u8,
    last_req_cmd: u16,
    /// Current TX frame-format variant being swept, and how many sends we've spent on it.
    tx_variant: usize,
    tx_sends_in_variant: u32,
    /// Once a reply confirms our request got through, stop sweeping and stick with this variant.
    frozen: bool,
    /// Rolling hex of the last few inbound 0x32 chunks (diagnostic).
    recent_rx: Vec<String>,
    /// Distinct reply command IDs → count, summarised into `stats.cmds_seen`.
    cmds: std::collections::BTreeMap<u16, u32>,
    stats: MspProbeStats,
}

impl MspProbe {
    pub fn new() -> Self {
        Self {
            acc: Vec::with_capacity(16),
            rx_buf: Vec::with_capacity(64),
            rx_size: 0,
            rx_cmd: 0,
            rx_started: false,
            rx_remote_seq: 0,
            last_req_cmd: 0,
            tx_variant: 0,
            tx_sends_in_variant: 0,
            frozen: false,
            recent_rx: Vec::with_capacity(8),
            cmds: std::collections::BTreeMap::new(),
            stats: MspProbeStats { last_reply_cmd: -1, ..Default::default() },
        }
    }

    pub fn stats(&self) -> &MspProbeStats {
        &self.stats
    }

    /// Build the S.Port frame for a V1 MSP request with no payload, in one of several wire-format
    /// variants we sweep while hunting for one the radio's BLE bridge accepts. Returns the bytes and a
    /// short variant label.
    fn build_request_variant(cmd: u8, variant: usize) -> (Vec<u8>, &'static str) {
        // MSP-V1 body after the status byte: size(0) + cmd, XOR checksum appended on the (only) chunk.
        let size = 0u8;
        let xor = size ^ cmd;
        let status = MSP_STARTFLAG | (MSP_V1 << MSP_VER_SHIFT);
        let chunk = [status, size, cmd, xor, 0u8, 0u8];
        // 8 frame bytes before the CRC: physID, primID, appID lo/hi, value 0..3.
        let raw = [
            SP_LOCAL_ID,
            SP_REQ_FRAME,
            chunk[0],
            chunk[1],
            chunk[2],
            chunk[3],
            chunk[4],
            chunk[5],
        ];
        // CRC covers primID..value (everything after the physID).
        let crc = sport_crc(&raw[1..]);
        let with_crc: Vec<u8> = raw.iter().copied().chain(std::iter::once(crc)).collect();

        let stuffed = |bytes: &[u8], lead_7e: bool| -> Vec<u8> {
            let mut o = if lead_7e { vec![0x7E] } else { Vec::new() };
            for &b in bytes {
                push_stuffed(&mut o, b);
            }
            o
        };

        match variant {
            // 0: full FrSky wire frame — leading 0x7E, S.Port CRC, byte-stuffed (the canonical form).
            0 => (stuffed(&with_crc, true), "7E+crc+stuff"),
            // 1: same but WITHOUT the leading 0x7E (some bridges add the delimiter themselves).
            1 => (stuffed(&with_crc, false), "no7E+crc+stuff"),
            // 2: leading 0x7E + CRC but NO byte-stuffing (bridge may stuff for us).
            2 => {
                let mut o = vec![0x7E];
                o.extend_from_slice(&with_crc);
                (o, "7E+crc,nostuff")
            }
            // 3: leading 0x7E, byte-stuffed, but NO S.Port CRC (bridge may append it).
            _ => (stuffed(&raw, true), "7E,nocrc+stuff"),
        }
    }

    /// Build the next probe frame and record it. Sweeps frame-format variants (~SENDS_PER_VARIANT sends
    /// each) until a reply confirms one works, then freezes on it. The caller writes the bytes out.
    pub fn next_tx(&mut self) -> Vec<u8> {
        let (frame, label) = Self::build_request_variant(MSP_API_VERSION, self.tx_variant);
        self.last_req_cmd = MSP_API_VERSION as u16;
        self.stats.tx_count += 1;
        self.stats.last_tx_hex = hex(&frame);
        self.stats.last_tx_variant = format!("#{} {}", self.tx_variant, label);
        if !self.frozen {
            self.tx_sends_in_variant += 1;
            if self.tx_sends_in_variant >= SENDS_PER_VARIANT {
                self.tx_variant = (self.tx_variant + 1) % TX_VARIANTS;
                self.tx_sends_in_variant = 0;
            }
        }
        frame
    }

    /// Feed inbound bytes; extract S.Port reply frames (primID 0x32) and reassemble MSP replies.
    pub fn push_bytes(&mut self, data: &[u8]) {
        for &b in data {
            if b == 0x7E {
                if !self.acc.is_empty() {
                    let frame = std::mem::take(&mut self.acc);
                    self.process(&frame);
                }
            } else {
                self.acc.push(b);
                if self.acc.len() > 32 {
                    self.acc.clear(); // framing lost — resync on next 0x7E
                }
            }
        }
    }

    fn process(&mut self, raw: &[u8]) {
        // Unstuff 0x7D xx → xx ^ 0x20.
        let mut f = Vec::with_capacity(9);
        let mut i = 0;
        while i < raw.len() {
            if raw[i] == 0x7D && i + 1 < raw.len() {
                f.push(raw[i + 1] ^ 0x20);
                i += 2;
            } else {
                f.push(raw[i]);
                i += 1;
            }
        }
        if f.len() != 9 || f[1] != SP_REPLY_FRAME {
            return; // not an MSP reply frame
        }
        if f[0] != SP_REMOTE_ID && f[0] != FP_REMOTE_ID {
            return; // reply not from the FC
        }
        self.stats.rx_chunks += 1;
        let chunk = [f[2], f[3], f[4], f[5], f[6], f[7]];
        // Diagnostic: keep the last few raw chunks visible (full frame incl. physID/primID/crc).
        if self.recent_rx.len() == 8 {
            self.recent_rx.remove(0);
        }
        self.recent_rx.push(hex(&f));
        self.stats.last_rx_hex = self.recent_rx.join(" | ");
        eprintln!("[MSP-PROBE] rx 0x32 chunk: {}", hex(&f));
        self.recv_chunk(&chunk);
    }

    fn recv_chunk(&mut self, chunk: &[u8; 6]) {
        let st = chunk[0];
        let ver = (st >> MSP_VER_SHIFT) & 0x03;
        let start = st & MSP_STARTFLAG != 0;
        let seq = st & MSP_SEQ_MASK;
        let mut idx = 1usize;

        if start {
            // Parse the MSP header (V1: size,cmd | V2: flags,cmd:2,size:2).
            let (cmd, size, hdr_adv) = if ver == 2 {
                if chunk.len() < idx + 5 {
                    return;
                }
                let cmd = chunk[idx + 1] as u16 | (chunk[idx + 2] as u16) << 8;
                let size = chunk[idx + 3] as usize | (chunk[idx + 4] as usize) << 8;
                (cmd, size, 5usize)
            } else {
                if chunk.len() < idx + 2 {
                    return;
                }
                let size = chunk[idx] as usize;
                let cmd = chunk[idx + 1] as u16;
                (cmd, size, 2usize)
            };
            self.stats.last_rx_note =
                format!("start v{} seq{} cmd{} size{}", ver, seq, cmd, size);
            // Accept ANY command — we want to see all MSP traffic on the link (the radio's own polling
            // as well as replies to our injected requests), then attribute them by command ID.
            self.rx_buf.clear();
            self.rx_size = size;
            self.rx_cmd = cmd;
            self.rx_started = true;
            idx += hdr_adv;
        } else if !self.rx_started {
            self.stats.last_rx_note = format!("cont v{} seq{} SKIP(no start)", ver, seq);
            return;
        } else if (self.rx_remote_seq.wrapping_add(1) & 0x0F) != seq {
            self.stats.last_rx_note = format!(
                "cont seq{} SKIP(seq-gap, want {})",
                seq,
                self.rx_remote_seq.wrapping_add(1) & 0x0F
            );
            self.rx_started = false; // sequence gap — drop the partial
            return;
        } else {
            self.stats.last_rx_note = format!("cont v{} seq{}", ver, seq);
        }

        while idx < chunk.len() && self.rx_buf.len() < self.rx_size {
            self.rx_buf.push(chunk[idx]);
            idx += 1;
        }

        if self.rx_buf.len() >= self.rx_size {
            self.rx_started = false;
            self.stats.replies += 1;
            if self.rx_cmd == self.last_req_cmd {
                self.stats.probe_replies += 1; // a reply to OUR injected request
                self.frozen = true; // lock onto the variant that worked
            }
            self.stats.last_reply_cmd = self.rx_cmd as i32;
            self.stats.last_reply_hex = hex(&self.rx_buf);
            *self.cmds.entry(self.rx_cmd).or_insert(0) += 1;
            self.stats.cmds_seen = self
                .cmds
                .iter()
                .map(|(c, n)| format!("{}×{}", c, n))
                .collect::<Vec<_>>()
                .join(", ");
            eprintln!(
                "[MSP-PROBE] reply cmd={} len={} payload={}",
                self.rx_cmd,
                self.rx_buf.len(),
                self.stats.last_reply_hex
            );
        } else {
            self.rx_remote_seq = seq;
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ")
}
