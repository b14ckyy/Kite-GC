// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! RTSP control protocol + the two data transports.
//!
//! Control: OPTIONS → DESCRIBE → SETUP → PLAY over a persistent TCP connection, with
//! Basic/Digest authentication and periodic keepalive (GET_PARAMETER when the server
//! advertises it, OPTIONS otherwise).
//!
//! Data: **RTP/UDP** preferred (client_port pair, NAT pinhole punch, minimal RTCP receiver
//! reports) — over the lossy, high-latency links Kite flies on, TCP retransmits stack
//! delay exactly when the link degrades. **TCP-interleaved** serves NAT-hostile paths and
//! is the automatic fallback when no RTP arrives within `first_packet_timeout` after PLAY
//! (or the server refuses the UDP SETUP outright).

use std::io::{ErrorKind, Read, Write};
use std::net::{IpAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use md5::{Digest, Md5};

use super::h264::H264Depacketizer;
use super::h265::H265Depacketizer;
use super::mjpeg::MjpegDepacketizer;
use super::rtp::{self, Reorderer, RtpPacket, RtpStats};
use super::sdp::{self, MediaSection, Sdp};

// ─── Public surface ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RtspTransport {
    /// UDP first, silent TCP retry when no RTP arrives (the default).
    Auto,
    Udp,
    Tcp,
}

#[derive(Debug, Clone)]
pub struct RtspConfig {
    pub url: String,
    pub transport: RtspTransport,
    pub connect_timeout: Duration,
    /// UDP → TCP fallback trigger: no RTP within this window after PLAY.
    pub first_packet_timeout: Duration,
    pub user_agent: String,
    /// Codecs the caller can consume — stream selection refuses sources offering none of
    /// them (e.g. the MJPEG-only bridge on platforms without a native decode sink).
    pub accept: Vec<VideoCodec>,
    /// Optional live-counter sink, published ~4×/s from the receive loop — the Debug
    /// Monitor's data source. `run_rtsp`'s final `RtspStats` stays the authoritative
    /// end-of-stream summary.
    pub live: Option<std::sync::Arc<LiveRtspStats>>,
    /// H.265: after lost data hold pictures until the next IRAP — for a stateless hardware
    /// decoder that hangs on a missing reference (Pi 5). Concealing decoders leave it off.
    pub resync_on_loss: bool,
}

impl Default for RtspConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            transport: RtspTransport::Auto,
            connect_timeout: Duration::from_secs(5),
            first_packet_timeout: Duration::from_secs(2),
            user_agent: "Kite-GC".into(),
            accept: vec![VideoCodec::Mjpeg, VideoCodec::H264, VideoCodec::H265],
            live: None,
            resync_on_loss: false,
        }
    }
}

/// Live counters of a running stream (atomics — read from any thread without touching the
/// receive loop). Absolute totals since stream start; rate math is the consumer's job.
#[derive(Debug, Default)]
pub struct LiveRtspStats {
    /// 0 = not streaming yet, 1 = UDP, 2 = TCP-interleaved.
    pub transport: std::sync::atomic::AtomicU8,
    pub rtp_received: std::sync::atomic::AtomicU64,
    pub rtp_lost: std::sync::atomic::AtomicU64,
    pub rtp_reordered: std::sync::atomic::AtomicU64,
    pub rtp_late: std::sync::atomic::AtomicU64,
    /// Complete frames/AUs delivered to the consumer.
    pub frames: std::sync::atomic::AtomicU64,
    /// Damaged frames/AUs dropped whole (loss policy).
    pub frames_dropped: std::sync::atomic::AtomicU64,
    /// Raw received bytes (RTP payloads incl. headers) — the link-bitrate proxy.
    pub bytes: std::sync::atomic::AtomicU64,
}

fn publish_live(live: &LiveRtspStats, transport: u8, rtp: &RtpStats, depack: &Depack, bytes: u64) {
    use std::sync::atomic::Ordering::Relaxed;
    let (frames, dropped) = depack.totals();
    live.transport.store(transport, Relaxed);
    live.rtp_received.store(rtp.received, Relaxed);
    live.rtp_lost.store(rtp.lost, Relaxed);
    live.rtp_reordered.store(rtp.reordered, Relaxed);
    live.rtp_late.store(rtp.late_dropped, Relaxed);
    live.frames.store(frames, Relaxed);
    live.frames_dropped.store(dropped, Relaxed);
    live.bytes.store(bytes, Relaxed);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodec {
    Mjpeg,
    H264,
    H265,
}

#[derive(Debug)]
pub struct VideoFrame {
    pub codec: VideoCodec,
    /// MJPEG: one complete JFIF image. H264/H265: one Annex-B access unit.
    pub data: Vec<u8>,
    pub rtp_timestamp: u32,
}

#[derive(Debug, Clone)]
pub struct RtspStats {
    pub transport: RtspTransport,
    pub rtp: RtpStats,
    pub frames: u64,
    pub dropped_frames: u64,
}

/// Connect, negotiate and stream until `stop` flips; frames arrive via `on_frame`.
/// Blocking — run it on a dedicated thread.
pub fn run_rtsp(
    config: &RtspConfig,
    stop: &AtomicBool,
    on_frame: &mut dyn FnMut(VideoFrame),
) -> Result<RtspStats, String> {
    let attempt = |udp: bool, stop: &AtomicBool, on_frame: &mut dyn FnMut(VideoFrame)| {
        run_once(config, udp, stop, on_frame)
    };
    match config.transport {
        RtspTransport::Tcp => attempt(false, stop, on_frame).map_err(RunErr::into_message),
        RtspTransport::Udp => attempt(true, stop, on_frame).map_err(RunErr::into_message),
        RtspTransport::Auto => match attempt(true, stop, on_frame) {
            Err(RunErr::NoRtp) => {
                log::warn!(
                    "RTSP: no RTP over UDP within {:?} — falling back to TCP-interleaved",
                    config.first_packet_timeout
                );
                attempt(false, stop, on_frame).map_err(RunErr::into_message)
            }
            other => other.map_err(RunErr::into_message),
        },
    }
}

// ─── Internal error type (drives the Auto fallback) ─────────────────────────

enum RunErr {
    /// UDP produced no RTP (or the server refused the UDP SETUP) — retry over TCP.
    NoRtp,
    Msg(String),
}

impl RunErr {
    fn into_message(self) -> String {
        match self {
            RunErr::NoRtp => "No RTP data arrived over UDP — try the TCP transport".into(),
            RunErr::Msg(m) => m,
        }
    }
}

fn msg<E: std::fmt::Display>(context: &str) -> impl FnOnce(E) -> RunErr + '_ {
    move |e| RunErr::Msg(format!("{context}: {e}"))
}

// ─── URL ─────────────────────────────────────────────────────────────────────

struct Target {
    host: String,
    port: u16,
    /// Credential-free request URL (safe to log).
    request_url: String,
    user: Option<String>,
    pass: Option<String>,
}

fn parse_url(url: &str) -> Result<Target, String> {
    let rest = url
        .strip_prefix("rtsp://")
        .ok_or("RTSP URL must start with rtsp://")?;
    let (authority, path) = match rest.split_once('/') {
        Some((a, p)) => (a, format!("/{p}")),
        None => (rest, "/".to_string()),
    };
    let (creds, hostport) = match authority.rsplit_once('@') {
        Some((c, h)) => (Some(c), h),
        None => (None, authority),
    };
    let (user, pass) = match creds {
        Some(c) => match c.split_once(':') {
            Some((u, p)) => (Some(u.to_string()), Some(p.to_string())),
            None => (Some(c.to_string()), None),
        },
        None => (None, None),
    };
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty() => {
            (h.to_string(), p.parse::<u16>().map_err(|_| "Invalid port")?)
        }
        _ => (hostport.to_string(), 554),
    };
    if host.is_empty() {
        return Err("RTSP URL has no host".into());
    }
    let request_url = format!("rtsp://{host}:{port}{path}");
    Ok(Target { host, port, request_url, user, pass })
}

// ─── Control connection ──────────────────────────────────────────────────────

enum Auth {
    Basic,
    Digest { realm: String, nonce: String, qop_auth: bool, nc: u32 },
}

struct Response {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Response {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

struct Control {
    stream: TcpStream,
    cseq: u32,
    session: Option<String>,
    session_timeout: Duration,
    auth: Option<Auth>,
    user: Option<String>,
    pass: Option<String>,
    user_agent: String,
    /// Bytes read past the last parsed message (interleaved data mixes into this).
    pending: Vec<u8>,
}

fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

fn is_timeout(e: &std::io::Error) -> bool {
    matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
}

impl Control {
    /// Grow `pending` to at least `n` bytes, or fail at `deadline`.
    fn fill(&mut self, n: usize, deadline: Instant) -> Result<(), String> {
        let mut chunk = [0u8; 4096];
        while self.pending.len() < n {
            if Instant::now() > deadline {
                return Err("RTSP control read timeout".into());
            }
            match self.stream.read(&mut chunk) {
                Ok(0) => return Err("RTSP connection closed by server".into()),
                Ok(k) => self.pending.extend_from_slice(&chunk[..k]),
                Err(e) if is_timeout(&e) => continue,
                Err(e) => return Err(format!("RTSP control read error: {e}")),
            }
        }
        Ok(())
    }

    /// Read one RTSP response, skipping any interleaved `$`-frames in front of it.
    fn read_response(&mut self, deadline: Instant) -> Result<Response, String> {
        loop {
            self.fill(1, deadline)?;
            if self.pending[0] == b'$' {
                self.fill(4, deadline)?;
                let len = u16::from_be_bytes([self.pending[2], self.pending[3]]) as usize;
                self.fill(4 + len, deadline)?;
                self.pending.drain(..4 + len);
                continue;
            }
            let head_end = loop {
                if let Some(pos) = find_subslice(&self.pending, b"\r\n\r\n") {
                    break pos;
                }
                let cur = self.pending.len();
                self.fill(cur + 1, deadline)?;
            };
            let head = String::from_utf8_lossy(&self.pending[..head_end]).to_string();
            let mut lines = head.lines();
            let status = lines
                .next()
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|s| s.parse::<u16>().ok())
                .unwrap_or(0);
            let mut headers = Vec::new();
            for l in lines {
                if let Some((k, v)) = l.split_once(':') {
                    headers.push((k.trim().to_string(), v.trim().to_string()));
                }
            }
            let clen: usize = headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                .and_then(|(_, v)| v.parse().ok())
                .unwrap_or(0);
            self.fill(head_end + 4 + clen, deadline)?;
            let body = self.pending[head_end + 4..head_end + 4 + clen].to_vec();
            self.pending.drain(..head_end + 4 + clen);
            return Ok(Response { status, headers, body });
        }
    }

    fn write_request(&mut self, method: &str, url: &str, extra: &[(&str, String)]) -> Result<(), String> {
        self.cseq += 1;
        let mut m = format!(
            "{method} {url} RTSP/1.0\r\nCSeq: {}\r\nUser-Agent: {}\r\n",
            self.cseq, self.user_agent
        );
        if let Some(s) = &self.session {
            m.push_str(&format!("Session: {s}\r\n"));
        }
        if let Some(h) = self.auth_header(method, url) {
            m.push_str(&h);
        }
        for (k, v) in extra {
            m.push_str(&format!("{k}: {v}\r\n"));
        }
        m.push_str("\r\n");
        self.stream
            .write_all(m.as_bytes())
            .map_err(|e| format!("RTSP send error: {e}"))
    }

    /// Send a request and wait for its response; a 401 challenge is answered once.
    fn request(&mut self, method: &str, url: &str, extra: &[(&str, String)]) -> Result<Response, String> {
        for attempt in 0..2 {
            self.write_request(method, url, extra)?;
            let resp = self.read_response(Instant::now() + Duration::from_secs(8))?;
            if resp.status == 401 && attempt == 0 && self.user.is_some() {
                self.take_auth_challenge(&resp)?;
                continue;
            }
            return Ok(resp);
        }
        Err("RTSP authentication failed (401 after credentials)".into())
    }

    /// Fire-and-tolerate keepalive: the reply is consumed with a short grace, or — in
    /// TCP-interleaved mode — by the data loop's inline-response handling.
    fn keepalive(&mut self, has_get_parameter: bool, url: &str, consume_reply: bool) {
        let method = if has_get_parameter { "GET_PARAMETER" } else { "OPTIONS" };
        if self.write_request(method, url, &[]).is_err() {
            return;
        }
        if consume_reply {
            let _ = self.read_response(Instant::now() + Duration::from_millis(400));
        }
    }

    /// Write-only TEARDOWN — waiting for the reply of a dying session is wasted time.
    fn teardown(&mut self, url: &str) {
        let _ = self.write_request("TEARDOWN", url, &[]);
    }

    fn adopt_session(&mut self, resp: &Response) {
        if let Some(s) = resp.header("Session") {
            let mut parts = s.split(';');
            if let Some(id) = parts.next() {
                self.session = Some(id.trim().to_string());
            }
            for p in parts {
                if let Some(t) = p.trim().strip_prefix("timeout=") {
                    if let Ok(secs) = t.trim().parse::<u64>() {
                        self.session_timeout = Duration::from_secs(secs.max(5));
                    }
                }
            }
        }
    }

    fn take_auth_challenge(&mut self, resp: &Response) -> Result<(), String> {
        let mut basic = false;
        for (k, v) in &resp.headers {
            if !k.eq_ignore_ascii_case("www-authenticate") {
                continue;
            }
            let lower = v.to_ascii_lowercase();
            if lower.starts_with("digest") {
                let params = parse_auth_params(&v[6..]);
                let realm = params_get(&params, "realm");
                let nonce = params_get(&params, "nonce");
                let qop_auth = params_get(&params, "qop")
                    .split(',')
                    .any(|t| t.trim() == "auth");
                self.auth = Some(Auth::Digest { realm, nonce, qop_auth, nc: 0 });
                return Ok(());
            }
            if lower.starts_with("basic") {
                basic = true;
            }
        }
        if basic {
            self.auth = Some(Auth::Basic);
            return Ok(());
        }
        Err("Server requires an unsupported authentication scheme".into())
    }

    fn auth_header(&mut self, method: &str, url: &str) -> Option<String> {
        let user = self.user.clone()?;
        let pass = self.pass.clone().unwrap_or_default();
        match &mut self.auth {
            None => None,
            Some(Auth::Basic) => Some(format!(
                "Authorization: Basic {}\r\n",
                base64(format!("{user}:{pass}").as_bytes())
            )),
            Some(Auth::Digest { realm, nonce, qop_auth, nc }) => {
                *nc += 1;
                let cnonce = format!(
                    "{:016x}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos() as u64)
                        .unwrap_or(0x4B69_7465)
                );
                let response =
                    digest_response(&user, &pass, realm, nonce, method, url, *qop_auth, *nc, &cnonce);
                let mut h = format!(
                    "Authorization: Digest username=\"{user}\", realm=\"{realm}\", \
                     nonce=\"{nonce}\", uri=\"{url}\", response=\"{response}\""
                );
                if *qop_auth {
                    h.push_str(&format!(", qop=auth, nc={:08x}, cnonce=\"{cnonce}\"", *nc));
                }
                h.push_str("\r\n");
                Some(h)
            }
        }
    }
}

/// `key="value"` / `key=value` pairs of an auth challenge.
fn parse_auth_params(s: &str) -> Vec<(String, String)> {
    s.split(',')
        .filter_map(|part| {
            let (k, v) = part.split_once('=')?;
            Some((
                k.trim().to_ascii_lowercase(),
                v.trim().trim_matches('"').to_string(),
            ))
        })
        .collect()
}

fn params_get(params: &[(String, String)], key: &str) -> String {
    params
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
        .unwrap_or_default()
}

fn md5_hex(data: &str) -> String {
    let mut h = Md5::new();
    h.update(data.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// RFC 2617 Digest response (MD5; with and without `qop=auth`).
#[allow(clippy::too_many_arguments)]
fn digest_response(
    user: &str,
    pass: &str,
    realm: &str,
    nonce: &str,
    method: &str,
    uri: &str,
    qop_auth: bool,
    nc: u32,
    cnonce: &str,
) -> String {
    let ha1 = md5_hex(&format!("{user}:{realm}:{pass}"));
    let ha2 = md5_hex(&format!("{method}:{uri}"));
    if qop_auth {
        md5_hex(&format!("{ha1}:{nonce}:{nc:08x}:{cnonce}:auth:{ha2}"))
    } else {
        md5_hex(&format!("{ha1}:{nonce}:{ha2}"))
    }
}

fn base64(data: &[u8]) -> String {
    const TBL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(TBL[(n >> 18) as usize & 63] as char);
        out.push(TBL[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TBL[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TBL[n as usize & 63] as char } else { '=' });
    }
    out
}

// ─── Stream selection ────────────────────────────────────────────────────────

fn pick_video<'a>(
    sdp_doc: &'a Sdp,
    accept: &[VideoCodec],
) -> Result<(&'a MediaSection, VideoCodec, u8), String> {
    let mut offered: Vec<String> = Vec::new();
    for m in sdp_doc.media.iter().filter(|m| m.kind == "video") {
        for &pt in &m.payload_types {
            let enc = m
                .encoding_of(pt)
                .map(str::to_string)
                .unwrap_or_else(|| if pt == 26 { "JPEG".into() } else { format!("PT{pt}") });
            let codec = match enc.as_str() {
                "JPEG" => Some(VideoCodec::Mjpeg),
                "H264" => Some(VideoCodec::H264),
                "H265" | "HEVC" => Some(VideoCodec::H265),
                _ => None,
            };
            match codec {
                Some(c) if accept.contains(&c) => return Ok((m, c, pt)),
                _ => offered.push(enc),
            }
        }
    }
    if offered.is_empty() {
        Err("No video track in the RTSP stream description".into())
    } else {
        Err(format!(
            "No supported video track for this path — the stream offers {}",
            offered.join("/")
        ))
    }
}

/// Resolve an SDP control URL against the DESCRIBE base (RFC 7826 §C.1.1).
fn join_control(base: &str, control: Option<&str>) -> String {
    match control {
        None | Some("*") => base.to_string(),
        Some(c) if c.starts_with("rtsp://") || c.starts_with("rtsps://") => c.to_string(),
        Some(c) => format!("{}/{}", base.trim_end_matches('/'), c),
    }
}

// ─── Transport setup ─────────────────────────────────────────────────────────

/// Bind an RTP/RTCP socket pair on an even/odd port couple.
fn bind_udp_pair() -> Result<(UdpSocket, UdpSocket, u16), String> {
    for _ in 0..16 {
        let probe = UdpSocket::bind(("0.0.0.0", 0)).map_err(|e| format!("UDP bind: {e}"))?;
        let base = probe.local_addr().map_err(|e| e.to_string())?.port() & !1;
        drop(probe);
        if base == 0 {
            continue;
        }
        let rtp = match UdpSocket::bind(("0.0.0.0", base)) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let rtcp = match UdpSocket::bind(("0.0.0.0", base + 1)) {
            Ok(s) => s,
            Err(_) => continue,
        };
        return Ok((rtp, rtcp, base));
    }
    Err("Could not bind an RTP/RTCP UDP port pair".into())
}

/// server_port + source address from a SETUP response's Transport header.
fn parse_transport(resp: &Response) -> (Option<u16>, Option<IpAddr>) {
    let mut server_port = None;
    let mut source = None;
    if let Some(t) = resp.header("Transport") {
        for token in t.split(';') {
            let token = token.trim();
            if let Some(v) = token.strip_prefix("server_port=") {
                server_port = v.split('-').next().and_then(|p| p.parse().ok());
            } else if let Some(v) = token.strip_prefix("source=") {
                source = v.trim().parse().ok();
            }
        }
    }
    (server_port, source)
}

/// Minimal RTCP receiver report (RC=0) — session keepalive + the RTCP-side NAT pinhole.
fn minimal_rr() -> [u8; 8] {
    let ssrc = 0x4B69_7465u32.to_be_bytes(); // "Kite"
    [0x80, 201, 0x00, 0x01, ssrc[0], ssrc[1], ssrc[2], ssrc[3]]
}

// ─── Session ─────────────────────────────────────────────────────────────────

fn run_once(
    cfg: &RtspConfig,
    use_udp: bool,
    stop: &AtomicBool,
    on_frame: &mut dyn FnMut(VideoFrame),
) -> Result<RtspStats, RunErr> {
    let target = parse_url(&cfg.url).map_err(RunErr::Msg)?;
    let addr = (target.host.as_str(), target.port)
        .to_socket_addrs()
        .map_err(msg("RTSP host lookup failed"))?
        .next()
        .ok_or_else(|| RunErr::Msg("RTSP host resolved to no address".into()))?;

    let stream =
        TcpStream::connect_timeout(&addr, cfg.connect_timeout).map_err(msg("RTSP connect failed"))?;
    stream.set_nodelay(true).ok();
    stream.set_read_timeout(Some(Duration::from_millis(500))).ok();

    let mut ctl = Control {
        stream,
        cseq: 0,
        session: None,
        session_timeout: Duration::from_secs(60),
        auth: None,
        user: target.user.clone(),
        pass: target.pass.clone(),
        user_agent: cfg.user_agent.clone(),
        pending: Vec::new(),
    };

    // OPTIONS — best effort; only used to pick the keepalive method.
    let has_get_parameter = ctl
        .request("OPTIONS", &target.request_url, &[])
        .ok()
        .and_then(|r| r.header("Public").map(|p| p.contains("GET_PARAMETER")))
        .unwrap_or(false);

    let resp = ctl
        .request("DESCRIBE", &target.request_url, &[("Accept", "application/sdp".into())])
        .map_err(RunErr::Msg)?;
    if resp.status != 200 {
        return Err(RunErr::Msg(format!("DESCRIBE failed: status {}", resp.status)));
    }
    let base = resp
        .header("Content-Base")
        .or_else(|| resp.header("Content-Location"))
        .unwrap_or(&target.request_url)
        .to_string();
    let sdp_doc = sdp::parse(&String::from_utf8_lossy(&resp.body));
    let (media, codec, pt) = pick_video(&sdp_doc, &cfg.accept).map_err(RunErr::Msg)?;
    let mut depack = Depack::new(codec, media.fmtp_of(pt), cfg.resync_on_loss);
    let setup_url = join_control(&base, media.control.as_deref());
    let aggregate_url = join_control(&base, sdp_doc.session_control.as_deref());
    log::info!(
        "RTSP: {:?} video (PT {}) at {} — negotiating {}",
        codec,
        pt,
        target.request_url,
        if use_udp { "RTP/UDP" } else { "TCP-interleaved" }
    );

    if use_udp {
        let (rtp_sock, rtcp_sock, rtp_port) = bind_udp_pair().map_err(RunErr::Msg)?;
        let resp = ctl
            .request(
                "SETUP",
                &setup_url,
                &[(
                    "Transport",
                    format!("RTP/AVP;unicast;client_port={}-{}", rtp_port, rtp_port + 1),
                )],
            )
            .map_err(RunErr::Msg)?;
        if resp.status == 461 {
            // Unsupported Transport — let Auto retry as TCP-interleaved.
            return Err(RunErr::NoRtp);
        }
        if resp.status != 200 {
            return Err(RunErr::Msg(format!("SETUP failed: status {}", resp.status)));
        }
        ctl.adopt_session(&resp);
        let (server_rtp_port, source_ip) = parse_transport(&resp);
        let server_ip = source_ip.unwrap_or_else(|| addr.ip());

        let resp = ctl
            .request("PLAY", &aggregate_url, &[("Range", "npt=0.000-".into())])
            .map_err(RunErr::Msg)?;
        if resp.status != 200 {
            return Err(RunErr::Msg(format!("PLAY failed: status {}", resp.status)));
        }

        // NAT pinhole punch toward the announced server ports.
        if let Some(sp) = server_rtp_port {
            for _ in 0..2 {
                let _ = rtp_sock.send_to(&[0u8, 0u8], (server_ip, sp));
                let _ = rtcp_sock.send_to(&minimal_rr(), (server_ip, sp.saturating_add(1)));
            }
        }

        let result = udp_loop(
            cfg,
            &mut ctl,
            has_get_parameter,
            &aggregate_url,
            &rtp_sock,
            &rtcp_sock,
            server_rtp_port.map(|p| (server_ip, p)),
            &mut depack,
            stop,
            on_frame,
        );
        ctl.teardown(&aggregate_url);
        result
    } else {
        let resp = ctl
            .request(
                "SETUP",
                &setup_url,
                &[("Transport", "RTP/AVP/TCP;unicast;interleaved=0-1".into())],
            )
            .map_err(RunErr::Msg)?;
        if resp.status != 200 {
            return Err(RunErr::Msg(format!("SETUP (TCP) failed: status {}", resp.status)));
        }
        ctl.adopt_session(&resp);
        let resp = ctl
            .request("PLAY", &aggregate_url, &[("Range", "npt=0.000-".into())])
            .map_err(RunErr::Msg)?;
        if resp.status != 200 {
            return Err(RunErr::Msg(format!("PLAY failed: status {}", resp.status)));
        }
        let result =
            tcp_loop(cfg, &mut ctl, has_get_parameter, &aggregate_url, &mut depack, stop, on_frame);
        ctl.teardown(&aggregate_url);
        result
    }
}

/// Codec-dispatching facade over the three per-codec depacketizers.
enum Depack {
    Mjpeg(MjpegDepacketizer),
    H264(H264Depacketizer),
    H265(H265Depacketizer),
}

impl Depack {
    fn new(codec: VideoCodec, fmtp: Option<&str>, resync_on_loss: bool) -> Self {
        match codec {
            VideoCodec::Mjpeg => Depack::Mjpeg(MjpegDepacketizer::new()),
            VideoCodec::H264 => Depack::H264(H264Depacketizer::new(fmtp)),
            VideoCodec::H265 => Depack::H265(H265Depacketizer::new(fmtp).with_resync(resync_on_loss)),
        }
    }

    fn push(&mut self, pkt: &RtpPacket, on_frame: &mut dyn FnMut(VideoFrame)) {
        match self {
            Depack::Mjpeg(d) => {
                if let Some(jpeg) = d.push(pkt) {
                    on_frame(VideoFrame {
                        codec: VideoCodec::Mjpeg,
                        data: jpeg,
                        rtp_timestamp: pkt.timestamp,
                    });
                }
            }
            Depack::H264(d) => {
                for au in d.push(pkt) {
                    on_frame(VideoFrame {
                        codec: VideoCodec::H264,
                        data: au,
                        rtp_timestamp: pkt.timestamp,
                    });
                }
            }
            Depack::H265(d) => {
                for au in d.push(pkt) {
                    on_frame(VideoFrame {
                        codec: VideoCodec::H265,
                        data: au,
                        rtp_timestamp: pkt.timestamp,
                    });
                }
            }
        }
    }

    /// (complete frames/AUs delivered, damaged ones dropped)
    fn totals(&self) -> (u64, u64) {
        match self {
            Depack::Mjpeg(d) => (d.frames, d.dropped_frames),
            Depack::H264(d) => (d.aus, d.damaged_aus),
            Depack::H265(d) => (d.aus, d.damaged_aus),
        }
    }
}

/// Deliver in-order packets through the depacketizer to the frame callback.
fn deliver(ordered: &mut Vec<RtpPacket>, depack: &mut Depack, on_frame: &mut dyn FnMut(VideoFrame)) {
    for p in ordered.drain(..) {
        depack.push(&p, on_frame);
    }
}

#[allow(clippy::too_many_arguments)]
fn udp_loop(
    cfg: &RtspConfig,
    ctl: &mut Control,
    has_get_parameter: bool,
    keepalive_url: &str,
    rtp_sock: &UdpSocket,
    rtcp_sock: &UdpSocket,
    server: Option<(IpAddr, u16)>,
    depack: &mut Depack,
    stop: &AtomicBool,
    on_frame: &mut dyn FnMut(VideoFrame),
) -> Result<RtspStats, RunErr> {
    rtp_sock
        .set_read_timeout(Some(Duration::from_millis(100)))
        .ok();
    rtcp_sock.set_nonblocking(true).ok();
    // The control socket is idle during UDP play — drain keepalive replies quickly.
    ctl.stream.set_read_timeout(Some(Duration::from_millis(1))).ok();

    let mut reorder = Reorderer::new();
    let mut ordered: Vec<RtpPacket> = Vec::new();
    let mut buf = vec![0u8; 65536];
    let started = Instant::now();
    let mut got_first = false;
    let keepalive_every = (ctl.session_timeout / 2).max(Duration::from_secs(5));
    let mut last_keepalive = Instant::now();
    let mut last_rr = Instant::now();
    let mut bytes: u64 = 0;
    let mut last_live = Instant::now();

    while !stop.load(Ordering::Relaxed) {
        match rtp_sock.recv(&mut buf) {
            Ok(n) => {
                if !got_first {
                    got_first = true;
                    log::info!("RTSP: first RTP packet after {} ms (UDP)", started.elapsed().as_millis());
                }
                bytes += n as u64;
                if let Some(pkt) = rtp::parse(&buf[..n]) {
                    reorder.push(pkt, &mut ordered);
                    deliver(&mut ordered, depack, on_frame);
                }
            }
            Err(e) if is_timeout(&e) => {
                if !got_first && started.elapsed() > cfg.first_packet_timeout {
                    return Err(RunErr::NoRtp);
                }
            }
            Err(e) => return Err(RunErr::Msg(format!("RTP receive error: {e}"))),
        }

        if let Some(live) = &cfg.live {
            if last_live.elapsed() >= Duration::from_millis(250) {
                last_live = Instant::now();
                publish_live(live, 1, &reorder.stats, depack, bytes);
            }
        }

        while rtcp_sock.recv(&mut buf).is_ok() {} // sender reports — contents unused

        if let Some((ip, rtp_port)) = server {
            if last_rr.elapsed() >= Duration::from_secs(5) {
                last_rr = Instant::now();
                let _ = rtcp_sock.send_to(&minimal_rr(), (ip, rtp_port.saturating_add(1)));
            }
        }
        if last_keepalive.elapsed() >= keepalive_every {
            last_keepalive = Instant::now();
            ctl.keepalive(has_get_parameter, keepalive_url, true);
        }
    }

    let (frames, dropped_frames) = depack.totals();
    Ok(RtspStats {
        transport: RtspTransport::Udp,
        rtp: reorder.stats.clone(),
        frames,
        dropped_frames,
    })
}

fn tcp_loop(
    cfg: &RtspConfig,
    ctl: &mut Control,
    has_get_parameter: bool,
    keepalive_url: &str,
    depack: &mut Depack,
    stop: &AtomicBool,
    on_frame: &mut dyn FnMut(VideoFrame),
) -> Result<RtspStats, RunErr> {
    ctl.stream.set_read_timeout(Some(Duration::from_millis(100))).ok();

    let mut reorder = Reorderer::new();
    let mut ordered: Vec<RtpPacket> = Vec::new();
    let mut chunk = [0u8; 8192];
    let started = Instant::now();
    let mut last_data = Instant::now();
    let mut got_first = false;
    let keepalive_every = (ctl.session_timeout / 2).max(Duration::from_secs(5));
    let mut last_keepalive = Instant::now();
    let mut bytes: u64 = 0;
    let mut last_live = Instant::now();

    while !stop.load(Ordering::Relaxed) {
        match ctl.stream.read(&mut chunk) {
            Ok(0) => return Err(RunErr::Msg("RTSP connection closed by server".into())),
            Ok(n) => {
                ctl.pending.extend_from_slice(&chunk[..n]);
                last_data = Instant::now();
            }
            Err(e) if is_timeout(&e) => {}
            Err(e) => return Err(RunErr::Msg(format!("RTSP read error: {e}"))),
        }

        if let Some(live) = &cfg.live {
            if last_live.elapsed() >= Duration::from_millis(250) {
                last_live = Instant::now();
                publish_live(live, 2, &reorder.stats, depack, bytes);
            }
        }

        // Consume every complete message in the buffer: `$`-framed RTP/RTCP, or inline
        // RTSP responses (keepalive replies) which are parsed and discarded.
        loop {
            if ctl.pending.is_empty() {
                break;
            }
            if ctl.pending[0] == b'$' {
                if ctl.pending.len() < 4 {
                    break;
                }
                let len = u16::from_be_bytes([ctl.pending[2], ctl.pending[3]]) as usize;
                if ctl.pending.len() < 4 + len {
                    break;
                }
                if ctl.pending[1] == 0 {
                    bytes += len as u64;
                    if let Some(pkt) = rtp::parse(&ctl.pending[4..4 + len]) {
                        if !got_first {
                            got_first = true;
                            log::info!(
                                "RTSP: first RTP packet after {} ms (TCP)",
                                started.elapsed().as_millis()
                            );
                        }
                        reorder.push(pkt, &mut ordered);
                    }
                }
                ctl.pending.drain(..4 + len);
                deliver(&mut ordered, depack, on_frame);
                continue;
            }
            match find_subslice(&ctl.pending, b"\r\n\r\n") {
                Some(pos) => {
                    let head = String::from_utf8_lossy(&ctl.pending[..pos]).to_string();
                    let clen: usize = head
                        .lines()
                        .find_map(|l| {
                            let (k, v) = l.split_once(':')?;
                            k.trim()
                                .eq_ignore_ascii_case("content-length")
                                .then(|| v.trim().parse().ok())?
                        })
                        .unwrap_or(0);
                    if ctl.pending.len() < pos + 4 + clen {
                        break;
                    }
                    ctl.pending.drain(..pos + 4 + clen);
                }
                None => break,
            }
        }

        if !got_first && started.elapsed() > cfg.first_packet_timeout.max(Duration::from_secs(5)) {
            return Err(RunErr::Msg("No RTP data over TCP-interleaved within 5 s".into()));
        }
        if last_data.elapsed() > Duration::from_secs(10) {
            return Err(RunErr::Msg("RTSP TCP stream stalled (10 s without data)".into()));
        }
        if last_keepalive.elapsed() >= keepalive_every {
            last_keepalive = Instant::now();
            ctl.keepalive(has_get_parameter, keepalive_url, false); // loop eats the reply
        }
    }

    let (frames, dropped_frames) = depack.totals();
    Ok(RtspStats {
        transport: RtspTransport::Tcp,
        rtp: reorder.stats.clone(),
        frames,
        dropped_frames,
    })
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_urls() {
        let t = parse_url("rtsp://cam.local/stream/main").unwrap();
        assert_eq!(t.host, "cam.local");
        assert_eq!(t.port, 554);
        assert_eq!(t.request_url, "rtsp://cam.local:554/stream/main");
        assert!(t.user.is_none());

        let t = parse_url("rtsp://user:pw@10.0.0.2:8554/live?x=1").unwrap();
        assert_eq!(t.host, "10.0.0.2");
        assert_eq!(t.port, 8554);
        assert_eq!(t.user.as_deref(), Some("user"));
        assert_eq!(t.pass.as_deref(), Some("pw"));
        assert_eq!(t.request_url, "rtsp://10.0.0.2:8554/live?x=1");

        let t = parse_url("rtsp://host").unwrap();
        assert_eq!(t.request_url, "rtsp://host:554/");
    }

    /// The RFC 2617 §3.5 example — validates the whole MD5/Digest pipeline.
    #[test]
    fn digest_matches_the_rfc_example() {
        let r = digest_response(
            "Mufasa",
            "Circle Of Life",
            "testrealm@host.com",
            "dcd98b7102dd2f0e8b11d0f600bfb0c093",
            "GET",
            "/dir/index.html",
            true,
            1,
            "0a4f113b",
        );
        assert_eq!(r, "6629fae49393a05397450978507c4ef1");
    }

    #[test]
    fn base64_matches_the_rfc_example() {
        assert_eq!(base64(b"Aladdin:open sesame"), "QWxhZGRpbjpvcGVuIHNlc2FtZQ==");
        assert_eq!(base64(b"a"), "YQ==");
        assert_eq!(base64(b"ab"), "YWI=");
    }

    #[test]
    fn joins_control_urls() {
        let base = "rtsp://h:554/path/";
        assert_eq!(join_control(base, None), base);
        assert_eq!(join_control(base, Some("*")), base);
        assert_eq!(join_control(base, Some("streamid=0")), "rtsp://h:554/path/streamid=0");
        assert_eq!(
            join_control(base, Some("rtsp://h:554/other")),
            "rtsp://h:554/other"
        );
    }

    #[test]
    fn picks_mjpeg_and_reports_unsupported() {
        let all = [VideoCodec::Mjpeg, VideoCodec::H264, VideoCodec::H265];
        let s = sdp::parse("m=video 0 RTP/AVP 26\r\na=control:t1\r\n");
        let (_, codec, pt) = pick_video(&s, &all).unwrap();
        assert_eq!(codec, VideoCodec::Mjpeg);
        assert_eq!(pt, 26);

        let s = sdp::parse("m=video 0 RTP/AVP 96\r\na=rtpmap:96 H264/90000\r\n");
        let (_, codec, _) = pick_video(&s, &all).unwrap();
        assert_eq!(codec, VideoCodec::H264, "H264 picked when accepted");
        let err = pick_video(&s, &[VideoCodec::Mjpeg]).unwrap_err();
        assert!(err.contains("H264"), "{err}");
    }

    /// Live smoke test against a real server (UAV-Link, MediaMTX, an IP cam):
    /// `KITE_RTSP_URL=rtsp://... cargo test streams_from_a_real_server -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn streams_from_a_real_server_when_provided() {
        let Ok(url) = std::env::var("KITE_RTSP_URL") else {
            eprintln!("KITE_RTSP_URL not set — skipping");
            return;
        };
        let transport = match std::env::var("KITE_RTSP_TRANSPORT").as_deref() {
            Ok("tcp") => RtspTransport::Tcp,
            Ok("udp") => RtspTransport::Udp,
            _ => RtspTransport::Auto,
        };
        let cfg = RtspConfig { url, transport, ..Default::default() };
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        {
            let stop = stop.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(6));
                stop.store(true, Ordering::Relaxed);
            });
        }
        let mut frames = 0u64;
        let mut bytes = 0usize;
        let stats = run_rtsp(&cfg, &stop, &mut |f| {
            frames += 1;
            bytes += f.data.len();
        })
        .expect("stream");
        eprintln!(
            "transport={:?} frames={frames} bytes={bytes} rtp: recv={} lost={} reordered={} late={} frames_dropped={}",
            stats.transport,
            stats.rtp.received,
            stats.rtp.lost,
            stats.rtp.reordered,
            stats.rtp.late_dropped,
            stats.dropped_frames,
        );
        assert!(frames > 0, "no MJPEG frames received");
    }

    /// Codec-agnostic live capture: writes every received frame/AU concatenated to
    /// KITE_RTSP_DUMP — an H264/H265 dump is a decodable Annex-B stream, validated with
    /// `ffmpeg -f h264 -i <dump> -f null NUL`.
    #[test]
    #[ignore]
    fn dumps_access_units_from_a_real_server() {
        let Ok(url) = std::env::var("KITE_RTSP_URL") else {
            eprintln!("KITE_RTSP_URL not set — skipping");
            return;
        };
        let transport = match std::env::var("KITE_RTSP_TRANSPORT").as_deref() {
            Ok("tcp") => RtspTransport::Tcp,
            Ok("udp") => RtspTransport::Udp,
            _ => RtspTransport::Auto,
        };
        let cfg = RtspConfig { url, transport, ..Default::default() };
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        {
            let stop = stop.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(8));
                stop.store(true, Ordering::Relaxed);
            });
        }
        let mut frames = 0u64;
        let mut codec = None;
        let mut dump: Vec<u8> = Vec::new();
        let stats = run_rtsp(&cfg, &stop, &mut |f| {
            frames += 1;
            codec = Some(f.codec);
            dump.extend_from_slice(&f.data);
        })
        .expect("stream");
        eprintln!(
            "codec={codec:?} frames={frames} bytes={} transport={:?} rtp: recv={} lost={} frames_dropped={}",
            dump.len(),
            stats.transport,
            stats.rtp.received,
            stats.rtp.lost,
            stats.dropped_frames,
        );
        if let Ok(path) = std::env::var("KITE_RTSP_DUMP") {
            std::fs::write(&path, &dump).expect("write dump");
            eprintln!("dumped to {path}");
        }
        assert!(frames > 0, "no frames received");
    }
}
