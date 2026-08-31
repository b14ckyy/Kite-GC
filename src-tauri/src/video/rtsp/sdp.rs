// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Minimal SDP parsing for the RTSP client — only what stream selection needs: media
//! sections with their payload types, `a=rtpmap` encodings and `a=control` URLs.
//! (`a=fmtp` — H264 sprop parameter sets etc. — joins with the P2.1 depacketizers.)

#[derive(Debug, Clone, Default)]
pub struct MediaSection {
    /// "video" / "audio" / "application"
    pub kind: String,
    pub payload_types: Vec<u8>,
    /// Control URL, possibly relative to the DESCRIBE Content-Base.
    pub control: Option<String>,
    /// (payload type, encoding name UPPERCASED, clock rate)
    pub rtpmap: Vec<(u8, String, u32)>,
}

impl MediaSection {
    pub fn encoding_of(&self, pt: u8) -> Option<&str> {
        self.rtpmap
            .iter()
            .find(|(p, _, _)| *p == pt)
            .map(|(_, name, _)| name.as_str())
    }
}

#[derive(Debug, Clone, Default)]
pub struct Sdp {
    /// Session-level `a=control` (the aggregate control, often "*").
    pub session_control: Option<String>,
    pub media: Vec<MediaSection>,
}

pub fn parse(text: &str) -> Sdp {
    let mut sdp = Sdp::default();
    let mut current: Option<MediaSection> = None;

    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix("m=") {
            if let Some(m) = current.take() {
                sdp.media.push(m);
            }
            let mut parts = rest.split_whitespace();
            let kind = parts.next().unwrap_or("").to_string();
            let _port = parts.next();
            let _proto = parts.next();
            let payload_types = parts.filter_map(|p| p.parse().ok()).collect();
            current = Some(MediaSection { kind, payload_types, ..Default::default() });
        } else if let Some(rest) = line.strip_prefix("a=control:") {
            match current.as_mut() {
                Some(m) => m.control = Some(rest.trim().to_string()),
                None => sdp.session_control = Some(rest.trim().to_string()),
            }
        } else if let Some(rest) = line.strip_prefix("a=rtpmap:") {
            if let Some(m) = current.as_mut() {
                // "96 H264/90000" / "26 JPEG/90000"
                let mut parts = rest.split_whitespace();
                if let (Some(pt), Some(enc)) = (parts.next(), parts.next()) {
                    if let Ok(pt) = pt.parse::<u8>() {
                        let mut ec = enc.split('/');
                        let name = ec.next().unwrap_or("").to_ascii_uppercase();
                        let clock = ec.next().and_then(|c| c.parse().ok()).unwrap_or(90_000);
                        m.rtpmap.push((pt, name, clock));
                    }
                }
            }
        }
    }
    if let Some(m) = current.take() {
        sdp.media.push(m);
    }
    sdp
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A MediaMTX-style DESCRIBE body: session control + one video (H264) + one audio.
    #[test]
    fn parses_media_sections() {
        let text = "v=0\r\n\
                    o=- 0 0 IN IP4 0.0.0.0\r\n\
                    s=Stream\r\n\
                    a=control:*\r\n\
                    m=video 0 RTP/AVP 96\r\n\
                    a=rtpmap:96 H264/90000\r\n\
                    a=control:streamid=0\r\n\
                    m=audio 0 RTP/AVP 97\r\n\
                    a=rtpmap:97 mpeg4-generic/44100/2\r\n\
                    a=control:streamid=1\r\n";
        let sdp = parse(text);
        assert_eq!(sdp.session_control.as_deref(), Some("*"));
        assert_eq!(sdp.media.len(), 2);
        let v = &sdp.media[0];
        assert_eq!(v.kind, "video");
        assert_eq!(v.payload_types, vec![96]);
        assert_eq!(v.control.as_deref(), Some("streamid=0"));
        assert_eq!(v.encoding_of(96), Some("H264"));
        assert_eq!(sdp.media[1].encoding_of(97), Some("MPEG4-GENERIC"));
    }

    /// Static payload type 26 (JPEG) needs no rtpmap line at all.
    #[test]
    fn static_jpeg_payload_type_without_rtpmap() {
        let sdp = parse("m=video 0 RTP/AVP 26\r\na=control:track1\r\n");
        assert_eq!(sdp.media[0].payload_types, vec![26]);
        assert_eq!(sdp.media[0].encoding_of(26), None); // client maps PT 26 itself
    }
}
