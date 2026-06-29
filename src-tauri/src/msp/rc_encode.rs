// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// RC-control message builders (docs/archive/MSP_RC_CONTROL.md §7). Pure byte-level encoders for the two
// INAV messages used to inject RC from the GCS — verified against the firmware decoders in
// `src/main/fc/fc_msp.c` (MSP_SET_RAW_RC / MSP2_INAV_SET_AUX_RC):
//
//   MSP_SET_RAW_RC (200): u16-LE per channel from CH1; trim to the highest configured channel, gaps = 0
//     (INAV ≥8.0 ignores a 0 channel; we also validate the FC override bitmask). Sent fire-and-forget.
//   MSP2_INAV_SET_AUX_RC (0x2230): a latched overlay for CH13..32 (start channel ≥ 12, 0-based). One
//     resolution per message — 2/4/16-bit — packed; value 0 = "no update" (skip). Sent WITH reply (we
//     re-send on missing ACK), only on change.
//
// This module is only the byte packing. Grouping channels into per-resolution messages + the send
// policy live in the streaming layer.

/// First AUX-controllable channel index (0-based) = CH13. CH1..12 are protected by INAV firmware.
pub const AUX_FIRST_CHANNEL: u8 = 12;
/// One past the last RC channel index (CH32).
pub const RC_CHANNEL_LIMIT: u8 = 32;

/// AUX_RC per-channel resolution. Values match INAV's `resolutionMode` (2-bit=0, 4-bit=1, 16-bit=3).
/// The firmware also defines 2/4/8-bit packed modes, but the GCS only emits the 16-bit mode
/// (full-resolution µs), so only that variant is modelled here.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AuxResolution {
    Bits16,
}

impl AuxResolution {
    fn mode(self) -> u8 {
        match self {
            AuxResolution::Bits16 => 3,
        }
    }
}

/// Encode an `MSP_SET_RAW_RC` payload: u16-LE per channel for CH1..=values.len(). The caller trims to
/// the highest configured channel and fills gaps with 0.
pub fn encode_raw_rc(values: &[u16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 2);
    for &v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Quantise a target µs to the raw value for a resolution. Input 0 = skip → 0 (no update). Matches the
/// inverse of INAV's decode scaling.
pub fn us_to_raw(us: u16, res: AuxResolution) -> u16 {
    if us == 0 {
        return 0; // skip
    }
    match res {
        // raw = µs (firmware constrains 750..2250).
        AuxResolution::Bits16 => us.clamp(750, 2250),
    }
}

/// Encode an `MSP2_INAV_SET_AUX_RC` payload for one consecutive run starting at `start_channel`
/// (0-based, ≥12) at a single resolution. `values[i]` is the target µs for channel `start_channel + i`;
/// 0 = skip. Returns `[defByte, packed…]`.
///
/// For sub-byte resolutions the firmware derives the channel count from the byte count × channels/byte,
/// so the padded block must not run past CH32 — the caller must align such groups (this errors if not).
pub fn encode_aux_rc(start_channel: u8, res: AuxResolution, values: &[u16]) -> Result<Vec<u8>, String> {
    if start_channel < AUX_FIRST_CHANNEL {
        return Err(format!("AUX start channel {start_channel} < {AUX_FIRST_CHANNEL} (CH13)"));
    }
    if values.is_empty() {
        return Err("AUX payload has no channels".into());
    }

    // Number of channel slots the FC will read back (one per value in 16-bit mode).
    let slots = values.len();
    if start_channel as usize + slots > RC_CHANNEL_LIMIT as usize {
        return Err(format!(
            "AUX run CH{}..+{} exceeds CH{}",
            start_channel + 1,
            slots,
            RC_CHANNEL_LIMIT
        ));
    }

    let raws: Vec<u16> = values.iter().map(|&v| us_to_raw(v, res)).collect();
    let mut out = vec![(start_channel << 3) | res.mode()];

    match res {
        AuxResolution::Bits16 => {
            for r in raws {
                out.extend_from_slice(&r.to_le_bytes());
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Replicas of the INAV decoders (fc_msp.c) for round-trip verification ──

    fn decode_raw_rc(payload: &[u8]) -> Vec<u16> {
        payload.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect()
    }

    /// Returns (start_channel, decoded values) — 0 means "skip / no update".
    fn decode_aux_rc(payload: &[u8]) -> (u8, Vec<u16>) {
        let def = payload[0];
        let start = def >> 3;
        let mode = def & 0x07;
        let data = &payload[1..];
        let mut out = Vec::new();
        match mode {
            3 => {
                for c in data.chunks_exact(2) {
                    let raw = u16::from_le_bytes([c[0], c[1]]);
                    out.push(if raw == 0 { 0 } else { raw.clamp(750, 2250) });
                }
            }
            0 | 1 => {
                let bits = if mode == 0 { 2 } else { 4 };
                let per_byte = 8 / bits;
                let mask = (1u16 << bits) - 1;
                for &byte in data {
                    for sub in (0..per_byte).rev() {
                        let raw = (byte as u16 >> (sub * bits)) & mask;
                        out.push(if raw == 0 {
                            0
                        } else if bits == 2 {
                            1000 + (raw - 1) * 500
                        } else {
                            1000 + ((raw as u32 - 1) * 1000 / 14) as u16
                        });
                    }
                }
            }
            _ => {}
        }
        (start, out)
    }

    #[test]
    fn raw_rc_le_bytes() {
        assert_eq!(encode_raw_rc(&[1000, 1500, 2000]), vec![0xE8, 0x03, 0xDC, 0x05, 0xD0, 0x07]);
        assert_eq!(decode_raw_rc(&encode_raw_rc(&[1000, 1234, 2000])), vec![1000, 1234, 2000]);
    }

    #[test]
    fn aux_16bit_roundtrip() {
        let p = encode_aux_rc(12, AuxResolution::Bits16, &[1000, 1500, 2000]).unwrap();
        assert_eq!(p[0], (12 << 3) | 3);
        assert_eq!(decode_aux_rc(&p).1, vec![1000, 1500, 2000]);
    }

    #[test]
    fn aux_rejects_protected_and_overflow() {
        assert!(encode_aux_rc(11, AuxResolution::Bits16, &[1500]).is_err()); // CH12 protected
        // CH31 (idx30), 3 channels at 16-bit → 30+3=33 > 32 → must error.
        assert!(encode_aux_rc(30, AuxResolution::Bits16, &[1500, 1500, 1500]).is_err());
        // Three channels at CH30 (idx29) fit (29+3=32).
        assert!(encode_aux_rc(29, AuxResolution::Bits16, &[1500, 1500, 1500]).is_ok());
    }
}
