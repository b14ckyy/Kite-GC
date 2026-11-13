// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Pixel size of a tile image from its header — PNG, JPEG, WebP — without decoding it. Used once per
//! map at listing time to detect the map's tile size (see `probe_map`).

use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

/// How far into a JPEG the frame header is searched. APPn segments (EXIF, XMP, ICC) come before it and
/// are 64 KiB each at most, so a real tile has it within the first few hundred KiB.
const JPEG_SCAN_LIMIT: u64 = 1 << 20;

/// Pixel size (width, height) of the image file at `path`, `None` when it is no PNG / JPEG / WebP or
/// its header is unreadable.
pub fn read_image_size(path: &Path) -> Option<(u32, u32)> {
    let file = std::fs::File::open(path).ok()?;
    image_size(&mut io::BufReader::new(file))
}

/// Pixel size (width, height) of a PNG / JPEG / WebP image from its header.
pub fn image_size<R: Read + Seek>(r: &mut R) -> Option<(u32, u32)> {
    let mut buf = [0u8; 30];
    let n = read_up_to(r, &mut buf).ok()?;
    let head = &buf[..n];
    let size = if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        png_size(head)
    } else if head.starts_with(&[0xFF, 0xD8]) {
        r.seek(SeekFrom::Start(2)).ok()?;
        jpeg_size(r)
    } else if head.len() >= 12 && &head[0..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        webp_size(head)
    } else {
        None
    };
    size.filter(|&(w, h)| w > 0 && h > 0)
}

/// Normalise a tile's pixel size to 128 / 256 / 512: the nearest on a log scale (100 → 128, 300 → 256,
/// 400 → 512) — the same rule as the frontend's `normalizeTileSize`. 0 (unknown) → 256.
pub fn normalize_tile_size(px: u32) -> u16 {
    if px == 0 {
        return 256;
    }
    let exp = (px as f64).log2().round().clamp(7.0, 9.0) as u32;
    1 << exp
}

/// Fill `buf` as far as the reader has bytes; the count read.
fn read_up_to<R: Read>(r: &mut R, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

fn be16(b: &[u8]) -> u32 {
    u16::from_be_bytes([b[0], b[1]]) as u32
}

fn le16(b: &[u8]) -> u32 {
    u16::from_le_bytes([b[0], b[1]]) as u32
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// Signature (8), IHDR length (4), `IHDR` (4), width (4, BE), height (4, BE).
fn png_size(h: &[u8]) -> Option<(u32, u32)> {
    if h.len() < 24 || &h[12..16] != b"IHDR" {
        return None;
    }
    Some((be32(&h[16..20]), be32(&h[20..24])))
}

/// `RIFF` size `WEBP`, then the first chunk: its FourCC at 12, its payload from 20.
fn webp_size(h: &[u8]) -> Option<(u32, u32)> {
    if h.len() < 16 {
        return None;
    }
    match &h[12..16] {
        // Lossy: 3-byte frame tag, start code 9d 01 2a, then 14-bit width / height (top 2 bits = scale).
        b"VP8 " if h.len() >= 30 && h[23..26] == [0x9d, 0x01, 0x2a] => {
            Some((le16(&h[26..28]) & 0x3fff, le16(&h[28..30]) & 0x3fff))
        }
        // Lossless: signature 0x2f, then width-1 and height-1 as 14-bit fields (LSB first).
        b"VP8L" if h.len() >= 25 && h[20] == 0x2f => {
            let bits = u32::from_le_bytes([h[21], h[22], h[23], h[24]]);
            Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
        }
        // Extended: flags + 3 reserved bytes, then canvas width-1 and height-1, 24 bit LE each.
        b"VP8X" if h.len() >= 30 => Some((
            u32::from_le_bytes([h[24], h[25], h[26], 0]) + 1,
            u32::from_le_bytes([h[27], h[28], h[29], 0]) + 1,
        )),
        _ => None,
    }
}

/// Walk the JPEG segments (reader positioned after SOI) to the first frame header (SOFn) and read its
/// size. Segments before it (APPn incl. EXIF, DQT, DHT, COM …) are skipped by their length; stray bytes
/// between segments are tolerated as decoders do.
fn jpeg_size<R: Read + Seek>(r: &mut R) -> Option<(u32, u32)> {
    let mut pos: u64 = 2;
    let mut b = [0u8; 1];
    loop {
        // Next marker: skip to a 0xFF, then past fill bytes (0xFF) to the marker code.
        loop {
            if pos > JPEG_SCAN_LIMIT {
                return None;
            }
            r.read_exact(&mut b).ok()?;
            pos += 1;
            if b[0] == 0xFF {
                break;
            }
        }
        let marker = loop {
            if pos > JPEG_SCAN_LIMIT {
                return None;
            }
            r.read_exact(&mut b).ok()?;
            pos += 1;
            if b[0] != 0xFF {
                break b[0];
            }
        };
        match marker {
            // Standalone markers carry no length: TEM, RSTn, a repeated SOI; 0x00 is a stuffed byte.
            0x00 | 0x01 | 0xD0..=0xD8 => continue,
            // End of image / start of scan before any frame header: not a usable image.
            0xD9 | 0xDA => return None,
            _ => {}
        }
        let mut len = [0u8; 2];
        r.read_exact(&mut len).ok()?;
        pos += 2;
        let len = be16(&len) as u64;
        if len < 2 {
            return None;
        }
        // SOF0..SOF15 except DHT (C4), JPG (C8) and DAC (CC): precision (1), height (2), width (2).
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            if len < 7 {
                return None;
            }
            let mut sof = [0u8; 5];
            r.read_exact(&mut sof).ok()?;
            return Some((be16(&sof[3..5]), be16(&sof[1..3])));
        }
        r.seek(SeekFrom::Current(len as i64 - 2)).ok()?;
        pos += len - 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn size_of(bytes: &[u8]) -> Option<(u32, u32)> {
        image_size(&mut Cursor::new(bytes.to_vec()))
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
        v.extend_from_slice(&13u32.to_be_bytes());
        v.extend_from_slice(b"IHDR");
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&h.to_be_bytes());
        v.extend_from_slice(&[8, 2, 0, 0, 0]); // depth, colour type, compression, filter, interlace
        v.extend_from_slice(&[0, 0, 0, 0]); // CRC (not checked)
        v
    }

    /// A segment `FF <marker> <len> <payload>`.
    fn seg(marker: u8, payload: &[u8]) -> Vec<u8> {
        let mut v = vec![0xFF, marker];
        v.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        v.extend_from_slice(payload);
        v
    }

    fn sof(marker: u8, w: u16, h: u16) -> Vec<u8> {
        let mut p = vec![8];
        p.extend_from_slice(&h.to_be_bytes());
        p.extend_from_slice(&w.to_be_bytes());
        p.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        seg(marker, &p)
    }

    fn sos_and_eoi() -> Vec<u8> {
        let mut v = seg(0xDA, &[1, 1, 0, 0, 63, 0]);
        v.extend_from_slice(&[0x12, 0x34, 0xFF, 0x00, 0x56, 0xFF, 0xD9]);
        v
    }

    fn webp(chunk: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = b"RIFF".to_vec();
        v.extend_from_slice(&((4 + 8 + payload.len()) as u32).to_le_bytes());
        v.extend_from_slice(b"WEBP");
        v.extend_from_slice(chunk);
        v.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        v.extend_from_slice(payload);
        v
    }

    #[test]
    fn png_header() {
        assert_eq!(size_of(&png(256, 256)), Some((256, 256)));
        assert_eq!(size_of(&png(100, 120)), Some((100, 120)));
        // Truncated inside IHDR / zero size → unusable.
        assert_eq!(size_of(&png(256, 256)[..20]), None);
        assert_eq!(size_of(&png(0, 256)), None);
    }

    #[test]
    fn jpeg_baseline() {
        let mut v = vec![0xFF, 0xD8];
        v.extend(seg(0xE0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0"));
        v.extend(seg(0xDB, &[0u8; 65]));
        v.extend(sof(0xC0, 100, 100));
        v.extend(seg(0xC4, &[0u8; 30]));
        v.extend(sos_and_eoi());
        assert_eq!(size_of(&v), Some((100, 100)));
    }

    #[test]
    fn jpeg_progressive_behind_exif_and_icc() {
        // A large EXIF segment whose payload contains marker-like bytes (FF C0, FF D9) — they must be
        // skipped by the segment length, not parsed.
        let mut exif = b"Exif\0\0MM\0*".to_vec();
        for i in 0..6000u32 {
            exif.push((i % 251) as u8);
            if i % 997 == 0 {
                exif.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08, 0x00, 0x10, 0x00, 0x10]);
                exif.extend_from_slice(&[0xFF, 0xD9]);
            }
        }
        let mut v = vec![0xFF, 0xD8];
        v.extend(seg(0xE1, &exif));
        v.extend(seg(0xE2, &[0x11; 500])); // ICC profile
        v.extend(seg(0xFE, b"comment")); // COM
        // A DHT that looks like a frame header must not be taken for one.
        v.extend(seg(0xC4, &[8, 0x00, 0x20, 0x00, 0x20, 0, 0, 0]));
        v.extend_from_slice(&[0xFF, 0xFF, 0xFF]); // fill bytes before the next marker
        v.extend(sof(0xC2, 512, 256));
        v.extend(sos_and_eoi());
        assert_eq!(size_of(&v), Some((512, 256)));
    }

    #[test]
    fn jpeg_without_frame_header() {
        // Scan before any SOF, EOI right away, or truncated inside a segment → unusable.
        let mut v = vec![0xFF, 0xD8];
        v.extend(seg(0xDB, &[0u8; 65]));
        v.extend(sos_and_eoi());
        assert_eq!(size_of(&v), None);
        assert_eq!(size_of(&[0xFF, 0xD8, 0xFF, 0xD9]), None);
        let mut t = vec![0xFF, 0xD8];
        t.extend(seg(0xE1, &[0u8; 1000]));
        t.truncate(500);
        assert_eq!(size_of(&t), None);
        let mut s = vec![0xFF, 0xD8];
        s.extend(sof(0xC0, 100, 100));
        s.truncate(9); // cut inside the frame header
        assert_eq!(size_of(&s), None);
    }

    #[test]
    fn webp_lossy_lossless_extended() {
        // VP8: frame tag, start code, 14-bit sizes with the 2 scale bits set (must be masked off).
        let mut p = vec![0x50, 0x2a, 0x00, 0x9d, 0x01, 0x2a];
        p.extend_from_slice(&(512u16 | 0xC000).to_le_bytes());
        p.extend_from_slice(&(512u16 | 0x4000).to_le_bytes());
        p.extend_from_slice(&[0u8; 16]);
        assert_eq!(size_of(&webp(b"VP8 ", &p)), Some((512, 512)));
        // VP8L: 0x2f, (w-1) in bits 0..14, (h-1) in bits 14..28.
        let bits: u32 = 99 | (127 << 14) | (1 << 28);
        let mut p = vec![0x2f];
        p.extend_from_slice(&bits.to_le_bytes());
        p.extend_from_slice(&[0u8; 8]);
        assert_eq!(size_of(&webp(b"VP8L", &p)), Some((100, 128)));
        // VP8X: flags + reserved, canvas (w-1), (h-1) as 24-bit LE.
        let mut p = vec![0x10, 0, 0, 0];
        p.extend_from_slice(&(255u32).to_le_bytes()[..3]);
        p.extend_from_slice(&(70_000u32 - 1).to_le_bytes()[..3]);
        assert_eq!(size_of(&webp(b"VP8X", &p)), Some((256, 70_000)));
        // Bad start code / unknown chunk → unusable.
        let mut bad = vec![0x50, 0x2a, 0x00, 0x9d, 0x01, 0x2b];
        bad.extend_from_slice(&[0u8; 20]);
        assert_eq!(size_of(&webp(b"VP8 ", &bad)), None);
        assert_eq!(size_of(&webp(b"ALPH", &[0u8; 20])), None);
    }

    #[test]
    fn junk_is_no_image() {
        assert_eq!(size_of(&[]), None);
        assert_eq!(size_of(&[5, 1, 1]), None);
        assert_eq!(size_of(b"GIF89a\x10\x00\x10\x00"), None);
    }

    #[test]
    fn tile_sizes_normalise_on_a_log_scale() {
        for (px, want) in [
            (0, 256),
            (1, 128),
            (64, 128),
            (100, 128),
            (128, 128),
            (181, 128),
            (182, 256),
            (256, 256),
            (300, 256),
            (362, 256),
            (363, 512),
            (400, 512),
            (512, 512),
            (1024, 512),
        ] {
            assert_eq!(normalize_tile_size(px), want, "{px} px");
        }
    }
}
