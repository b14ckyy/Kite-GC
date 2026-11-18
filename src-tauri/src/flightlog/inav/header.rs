// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)
// Ported from Kite-Analyzer 8820a3a src-tauri/src/logs/inav/header.rs (2026-11-11)

//! The ASCII header of one INAV Blackbox log: `H key:value` lines up to the first frame marker.
//! Logic after blackbox-tools v9.0.0 (`1918a75`) `src/parser.c` `parseHeaderLine` (keys and
//! defaults) and the header state of `flightLogParse` (garbage before the first frame is skipped).

use std::collections::BTreeMap;

/// Field definitions of one frame type (`H Field <X> name|signed|predictor|encoding`). Shorter
/// `signed`/`predictor`/`encoding` lists are padded with 0, as the C parser's zeroed arrays.
#[derive(Clone, Default, Debug)]
pub struct FrameDef {
    pub names: Vec<String>,
    pub signed: Vec<bool>,
    pub predictor: Vec<i32>,
    pub encoding: Vec<i32>,
}

impl FrameDef {
    pub fn count(&self) -> usize {
        self.names.len()
    }

    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n == name)
    }

    fn pad(&mut self) {
        let n = self.names.len();
        self.signed.resize(n, false);
        self.predictor.resize(n, 0);
        self.encoding.resize(n, 0);
    }
}

#[derive(Clone, Debug)]
pub struct InavHeader {
    /// `Data version` (INAV writes 2); selects the TAG8_4S16 layout. 0 when absent.
    pub data_version: i32,
    /// `I interval` (≥ 1, default 32).
    pub i_interval: u32,
    /// `P interval` `num/denom` (default 1/1).
    pub p_interval_num: u32,
    pub p_interval_denom: u32,
    pub firmware_type: Option<String>,
    /// `Firmware revision` verbatim, e.g. `INAV 9.1.0 (d17f2b3e) MATEKF765`.
    pub firmware_revision: Option<String>,
    /// `(major, minor, patch)` parsed from an `INAV x.y.z` revision.
    pub firmware_version: Option<(u8, u8, u8)>,
    pub firmware_date: Option<String>,
    pub log_start_datetime: Option<String>,
    pub minthrottle: i64,
    pub maxthrottle: i64,
    /// `motorOutput` low/high (defaults follow `minthrottle`/`maxthrottle`, as in the C parser).
    pub motor_output: (i64, i64),
    pub vbatref: i64,
    /// `vbatcellvoltage` min / warning / max.
    pub vbat_cell_voltage: [i64; 3],
    /// `currentMeter` offset / scale.
    pub current_meter: (i64, i64),
    /// `gyro_scale` (hex-encoded float bits); INAV writes 1.0 — raw gyro is deg/s.
    pub gyro_scale: f32,
    pub acc_1g: u32,
    pub i: FrameDef,
    pub p: FrameDef,
    pub s: FrameDef,
    pub g: FrameDef,
    pub h: FrameDef,
    /// Every `H key:value` line verbatim (last occurrence wins), including the field definitions.
    pub settings: BTreeMap<String, String>,
    /// Offset of the first frame marker within the log's bytes.
    pub data_start: usize,
}

impl Default for InavHeader {
    /// `parser.c` `resetSysConfigToDefaults` + the frame-interval defaults of `flightLogParse`.
    fn default() -> Self {
        InavHeader {
            data_version: 0,
            i_interval: 32,
            p_interval_num: 1,
            p_interval_denom: 1,
            firmware_type: None,
            firmware_revision: None,
            firmware_version: None,
            firmware_date: None,
            log_start_datetime: None,
            minthrottle: 1150,
            maxthrottle: 1850,
            motor_output: (1150, 1850),
            vbatref: 4095,
            vbat_cell_voltage: [33, 35, 43],
            current_meter: (0, 400),
            gyro_scale: 1.0,
            acc_1g: 1,
            i: FrameDef::default(),
            p: FrameDef::default(),
            s: FrameDef::default(),
            g: FrameDef::default(),
            h: FrameDef::default(),
            settings: BTreeMap::new(),
            data_start: 0,
        }
    }
}

/// Frame markers that end the header (`parser.c` `frameTypes`).
pub fn is_frame_marker(b: u8) -> bool {
    matches!(b, b'I' | b'P' | b'G' | b'H' | b'E' | b'S')
}

/// Longest header line accepted. The C parser stops at 2048 bytes; INAV's `Field I name` line of a
/// fully featured target gets close to that, so this is only a guard against binary garbage.
const MAX_LINE: usize = 16 * 1024;

/// Parse the header of one log (`log` starts at its `H Product:` marker). Errors when no frame
/// follows the header or the `I` field list is missing.
pub fn parse_header(log: &[u8]) -> Result<InavHeader, String> {
    let mut h = InavHeader::default();
    let mut pos = 0usize;
    loop {
        let Some(&b) = log.get(pos) else {
            return Err("log contains no data frames (header only)".into());
        };
        pos += 1;
        if b == b'H' {
            pos = parse_line(log, pos, &mut h);
        } else if is_frame_marker(b) {
            h.data_start = pos - 1;
            break;
        }
        // Anything else is garbage before the first frame — skipped, as the C parser does.
    }
    if h.i.count() == 0 {
        return Err("log header has no main-frame field definitions (Field I name)".into());
    }
    h.p.names = h.i.names.clone();
    for def in [&mut h.i, &mut h.p, &mut h.s, &mut h.g, &mut h.h] {
        def.pad();
    }
    Ok(h)
}

/// One header line after its `H`; returns the position after the line. A line that is not
/// `H <key>:<value>\n` is ignored, consuming what was read (`parseHeaderLine`).
fn parse_line(log: &[u8], start: usize, h: &mut InavHeader) -> usize {
    if log.get(start) != Some(&b' ') {
        return start;
    }
    let line_start = start + 1;
    let mut i = line_start;
    let line_end = loop {
        match log.get(i) {
            None | Some(0) => return i.min(log.len()),
            Some(b'\n') => break i,
            Some(_) if i - line_start >= MAX_LINE => return i,
            Some(_) => i += 1,
        }
    };
    let line = &log[line_start..line_end];
    let next = line_end + 1;
    let Some(sep) = line.iter().position(|&c| c == b':') else {
        return next;
    };
    let key = String::from_utf8_lossy(&line[..sep]).into_owned();
    let mut value = String::from_utf8_lossy(&line[sep + 1..]).into_owned();
    if value.ends_with('\r') {
        value.pop();
    }
    apply_key(h, &key, &value);
    h.settings.insert(key, value);
    next
}

fn apply_key(h: &mut InavHeader, key: &str, value: &str) {
    if let Some(rest) = key.strip_prefix("Field ") {
        let mut chars = rest.chars();
        let (Some(frame), Some(' ')) = (chars.next(), chars.next()) else {
            return;
        };
        let attr = chars.as_str();
        let def = match frame {
            'I' => &mut h.i,
            'P' => &mut h.p,
            'S' => &mut h.s,
            'G' => &mut h.g,
            'H' => &mut h.h,
            _ => return,
        };
        match attr {
            "name" => def.names = value.split(',').map(str::to_owned).collect(),
            "signed" => def.signed = int_list(value).into_iter().map(|v| v != 0).collect(),
            "predictor" => def.predictor = int_list(value),
            "encoding" => def.encoding = int_list(value),
            _ => {}
        }
        // P frames are derived from I frames: they inherit the signedness (and, in `parse_header`,
        // the names).
        if frame == 'I' && attr == "signed" {
            h.p.signed = h.i.signed.clone();
        }
        return;
    }
    match key {
        "I interval" => h.i_interval = atoi(value).max(1) as u32,
        "P interval" => {
            if let Some((num, denom)) = value.split_once('/') {
                h.p_interval_num = atoi(num).max(0) as u32;
                h.p_interval_denom = atoi(denom).max(0) as u32;
            }
        }
        "Data version" => h.data_version = atoi(value),
        "Firmware type" => h.firmware_type = Some(value.to_owned()),
        "Firmware revision" => {
            h.firmware_revision = Some(value.to_owned());
            h.firmware_version = parse_inav_version(value);
        }
        "Firmware date" => h.firmware_date = Some(value.to_owned()),
        "Log start datetime" => h.log_start_datetime = Some(value.to_owned()),
        "minthrottle" => {
            h.minthrottle = atoi(value) as i64;
            h.motor_output.0 = h.minthrottle;
        }
        "maxthrottle" => {
            h.maxthrottle = atoi(value) as i64;
            h.motor_output.1 = h.maxthrottle;
        }
        "motorOutput" => {
            let v = int_list(value);
            h.motor_output = (
                v.first().copied().unwrap_or(0) as i64,
                v.get(1).copied().unwrap_or(0) as i64,
            );
        }
        "vbatref" => h.vbatref = atoi(value) as i64,
        "vbatcellvoltage" => {
            let v = int_list(value);
            for (k, slot) in h.vbat_cell_voltage.iter_mut().enumerate() {
                *slot = v.get(k).copied().unwrap_or(0) as i64;
            }
        }
        "currentMeter" => {
            let v = int_list(value);
            h.current_meter = (
                v.first().copied().unwrap_or(0) as i64,
                v.get(1).copied().unwrap_or(0) as i64,
            );
        }
        "gyro_scale" | "gyro.scale" => {
            let digits = value.trim();
            let digits = digits
                .strip_prefix("0x")
                .or_else(|| digits.strip_prefix("0X"))
                .unwrap_or(digits);
            if let Ok(bits) = u32::from_str_radix(digits, 16) {
                h.gyro_scale = f32::from_bits(bits);
            }
        }
        "acc_1G" => h.acc_1g = atoi(value).max(0) as u32,
        _ => {}
    }
}

/// `INAV 9.1.0 (d17f2b3e) MATEKF765` → `(9, 1, 0)`; missing minor/patch count as 0.
pub fn parse_inav_version(revision: &str) -> Option<(u8, u8, u8)> {
    let rest = revision.trim().strip_prefix("INAV")?.trim_start();
    let version = rest.split_whitespace().next()?;
    let mut parts = version.split('.').map(|p| {
        let digits: String = p.chars().take_while(char::is_ascii_digit).collect();
        digits.parse::<u8>().ok()
    });
    let major = parts.next()??;
    let minor = parts.next().flatten().unwrap_or(0);
    let patch = parts.next().flatten().unwrap_or(0);
    Some((major, minor, patch))
}

/// C `atoi`: leading whitespace, optional sign, digits; anything else ends the number; 0 if none.
fn atoi(s: &str) -> i32 {
    let s = s.trim_start();
    let (neg, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let mut v: i64 = 0;
    for c in digits.bytes().take_while(u8::is_ascii_digit) {
        v = (v * 10 + i64::from(c - b'0')).min(i64::from(i32::MAX) + 1);
    }
    let v = if neg { -v } else { v };
    v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// `parseCommaSeparatedIntegers`.
fn int_list(s: &str) -> Vec<i32> {
    if s.is_empty() {
        return Vec::new();
    }
    s.split(',').map(atoi).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "H Product:Blackbox flight data recorder by Nicholas Sherlock\n\
H Data version:2\n\
H I interval:32\n\
H Field I name:loopIteration,time,axisP[0],motor[0]\n\
H Field I signed:0,0,1,0\n\
H Field I predictor:0,0,0,4\n\
H Field I encoding:1,1,0,1\n\
H Field P predictor:6,2,1,5\n\
H Field P encoding:9,0,0,0\n\
H Field H name:GPS_home[0],GPS_home[1]\n\
H Field H signed:1,1\n\
H Field H predictor:0,0\n\
H Field H encoding:0,0\n\
H Field S name:flightModeFlags,rxUpdateRate\n\
H Field S signed:0,0\n\
H Field S predictor:0,1\n\
H Field S encoding:1,1\n\
H Firmware type:Cleanflight\n\
H Firmware revision:INAV 9.1.0 (d17f2b3e) MATEKF765\n\
H Firmware date:Aug 26 2026 10:33:45\n\
H Log start datetime:2041-06-28T03:04:00.000+02:00\n\
H P interval:1/5\n\
H minthrottle:1050\n\
H maxthrottle:2000\n\
H gyro_scale:0x3f800000\n\
H motorOutput:1050,2000\n\
H acc_1G:2048\n\
H vbatcellvoltage:33,35,42\n\
H vbatref:1260\n\
H currentMeter:0,250\n\
H rollPID:15,2,10,116\n";

    #[test]
    fn parses_hand_written_header() {
        let mut bytes = b"\x00junk".to_vec();
        let header_at = bytes.len();
        bytes.extend_from_slice(SAMPLE.as_bytes());
        let data_at = bytes.len();
        bytes.extend_from_slice(b"I\x01\x02");
        let h = parse_header(&bytes[header_at..]).unwrap();
        assert_eq!(h.data_start, data_at - header_at);
        assert_eq!(h.data_version, 2);
        assert_eq!(h.i_interval, 32);
        assert_eq!((h.p_interval_num, h.p_interval_denom), (1, 5));
        assert_eq!(h.firmware_version, Some((9, 1, 0)));
        assert_eq!(h.firmware_type.as_deref(), Some("Cleanflight"));
        assert_eq!(h.minthrottle, 1050);
        assert_eq!(h.motor_output, (1050, 2000));
        assert_eq!(h.vbatref, 1260);
        assert_eq!(h.vbat_cell_voltage, [33, 35, 42]);
        assert_eq!(h.current_meter, (0, 250));
        assert_eq!(h.gyro_scale, 1.0);
        assert_eq!(h.acc_1g, 2048);
        assert_eq!(h.i.names, ["loopIteration", "time", "axisP[0]", "motor[0]"]);
        assert_eq!(h.i.signed, [false, false, true, false]);
        assert_eq!(h.i.predictor, [0, 0, 0, 4]);
        // P inherits names and signedness from I.
        assert_eq!(h.p.names, h.i.names);
        assert_eq!(h.p.signed, h.i.signed);
        assert_eq!(h.p.predictor, [6, 2, 1, 5]);
        assert_eq!(h.p.encoding, [9, 0, 0, 0]);
        assert_eq!(h.s.names, ["flightModeFlags", "rxUpdateRate"]);
        assert_eq!(h.h.count(), 2);
        assert_eq!(h.g.count(), 0);
        // Every line lands in the settings map verbatim.
        assert_eq!(
            h.settings.get("rollPID").map(String::as_str),
            Some("15,2,10,116")
        );
        assert_eq!(
            h.settings.get("Product").map(String::as_str),
            Some("Blackbox flight data recorder by Nicholas Sherlock")
        );
        assert_eq!(
            h.settings.get("Field I predictor").map(String::as_str),
            Some("0,0,0,4")
        );
    }

    #[test]
    fn header_without_frames_or_fields_is_an_error() {
        assert!(parse_header(SAMPLE.as_bytes()).is_err());
        assert!(parse_header(b"H Data version:2\nI\x00").is_err());
    }

    #[test]
    fn versions() {
        assert_eq!(parse_inav_version("INAV 7.1.2 (abc) X"), Some((7, 1, 2)));
        assert_eq!(
            parse_inav_version("INAV 10.0.0-RC1 (abc) X"),
            Some((10, 0, 0))
        );
        assert_eq!(parse_inav_version("INAV 8"), Some((8, 0, 0)));
        assert_eq!(parse_inav_version("Betaflight 4.4.0"), None);
    }

    #[test]
    fn c_atoi() {
        assert_eq!(atoi(" 42abc"), 42);
        assert_eq!(atoi("-7"), -7);
        assert_eq!(atoi(""), 0);
        assert_eq!(int_list("1,,3"), [1, 0, 3]);
    }
}
