// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! In-process RTSP → MJPEG bridge — the P1 stage of the native video path
//! (Dev-Docs active/MOBILE_RTSP.md): Kite's own RTSP client (`video::rtsp`) reads the
//! source (UDP first, automatic TCP fallback) and its MJPEG frames are broadcast on the
//! same local multipart HTTP port the ffmpeg image path serves. No MediaMTX, no ffmpeg —
//! and byte-compatible part framing, so every sink, the off-thread reader and the
//! reconnect wiring work unchanged.
//!
//! Reuses `mjpeg_server`'s accept/broadcast machinery through a channel-backed `Read`
//! adapter instead of an ffmpeg stdout — the broadcast loop neither knows nor cares where
//! the parts come from.

use std::io::Read;
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::mjpeg_server::{accept_loop, broadcast_loop, Client, EndedHook};
use super::rtsp::{run_rtsp, RtspConfig, RtspTransport, VideoCodec};

/// How long `start()` waits for the first frame: RTSP negotiation (incl. a possible 2 s
/// UDP→TCP fallback) plus the first JPEG.
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(12);

/// One JPEG as an mpjpeg part — byte-compatible with ffmpeg's muxer output (`--ffmpeg`
/// boundary, `Content-length` on every part), which is what the broadcast framing and all
/// sinks already parse.
fn mpjpeg_part(jpeg: &[u8], first: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(jpeg.len() + 96);
    if !first {
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"--ffmpeg\r\nContent-type: image/jpeg\r\n");
    out.extend_from_slice(format!("Content-length: {}\r\n\r\n", jpeg.len()).as_bytes());
    out.extend_from_slice(jpeg);
    out
}

/// `Read` over the frame channel, so the RTSP client can stand where ffmpeg's stdout does.
/// EOF when the sender is dropped — i.e. when the RTSP thread ends, for whatever reason.
struct ChannelRead {
    rx: Receiver<Vec<u8>>,
    buf: Vec<u8>,
    pos: usize,
}

impl Read for ChannelRead {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.buf.len() {
            match self.rx.recv() {
                Ok(b) => {
                    self.buf = b;
                    self.pos = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let n = out.len().min(self.buf.len() - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

#[derive(Default)]
pub struct NativeRtsp {
    inner: Mutex<Option<Running>>,
}

struct Running {
    stop: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    rtsp: JoinHandle<()>,
    broadcast: JoinHandle<()>,
    accept: JoinHandle<()>,
}

/// Tear the three threads down in the one order that cannot deadlock and cannot fire a
/// spurious "source died" event: `shutdown` first (so the broadcast's EOF reads as "asked
/// to stop", not "died"), then `stop` (ends the RTSP thread, which drops the frame sender
/// and unblocks the broadcast's channel read), then the joins.
fn teardown(r: Running) {
    r.shutdown.store(true, Ordering::SeqCst);
    r.stop.store(true, Ordering::SeqCst);
    let _ = r.rtsp.join();
    let _ = r.broadcast.join();
    let _ = r.accept.join();
}

impl NativeRtsp {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start the native RTSP client on `url` (`transport`: udp | tcp | anything → auto) and
    /// broadcast its MJPEG frames on a local multipart HTTP port. Returns the port once the
    /// FIRST frame arrived; a failure (unreachable, auth, no MJPEG track) returns the
    /// client's own error message. `on_ended` fires when a live feed dies — never on stop.
    pub fn start(&self, on_ended: EndedHook, url: &str, transport: &str) -> Result<u16, String> {
        self.stop();

        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("bind: {e}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("set_nonblocking: {e}"))?;
        let port = listener.local_addr().map_err(|e| format!("addr: {e}"))?.port();

        let cfg = RtspConfig {
            url: url.to_string(),
            transport: match transport {
                "udp" => RtspTransport::Udp,
                "tcp" => RtspTransport::Tcp,
                _ => RtspTransport::Auto,
            },
            // MJPEG only until the native decode sinks land — an H264/H265 source fails
            // stream selection with a message naming what it offers instead.
            accept: vec![VideoCodec::Mjpeg],
            ..Default::default()
        };

        let stop = Arc::new(AtomicBool::new(false));
        let shutdown = Arc::new(AtomicBool::new(false));
        let clients: Arc<Mutex<Vec<Client>>> = Arc::new(Mutex::new(Vec::new()));
        let (frame_tx, frame_rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let (first_tx, first_rx) = std::sync::mpsc::channel::<()>();
        // The RTSP thread's error, for a start that fails before the first frame.
        let error_slot: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

        let accept = {
            let clients = clients.clone();
            let shutdown = shutdown.clone();
            thread::spawn(move || accept_loop(listener, clients, shutdown))
        };
        let broadcast = {
            let shutdown = shutdown.clone();
            let reader = ChannelRead { rx: frame_rx, buf: Vec::new(), pos: 0 };
            thread::spawn(move || broadcast_loop(reader, clients, shutdown, Some(first_tx), on_ended))
        };
        let rtsp = {
            let stop = stop.clone();
            let error_slot = error_slot.clone();
            thread::spawn(move || {
                let mut first = true;
                let result = run_rtsp(&cfg, &stop, &mut |frame| {
                    let part = mpjpeg_part(&frame.data, first);
                    first = false;
                    let _ = frame_tx.send(part);
                });
                match result {
                    Ok(stats) => log::info!(
                        "[video] native RTSP client ended: {:?}, {} frames ({} dropped), rtp recv={} lost={} reordered={} late={}",
                        stats.transport,
                        stats.frames,
                        stats.dropped_frames,
                        stats.rtp.received,
                        stats.rtp.lost,
                        stats.rtp.reordered,
                        stats.rtp.late_dropped,
                    ),
                    Err(e) => {
                        log::warn!("[video] native RTSP client failed: {e}");
                        if let Ok(mut slot) = error_slot.lock() {
                            slot.get_or_insert(e);
                        }
                    }
                }
                // `frame_tx` (moved into the closure) drops here → the broadcast sees EOF.
            })
        };

        // Report success only once the source delivered — mirrors `MjpegServer::start`.
        let deadline = Instant::now() + FIRST_FRAME_TIMEOUT;
        let failure = loop {
            match first_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(()) => break None,
                Err(RecvTimeoutError::Disconnected) => {
                    break Some("the stream ended before delivering a frame".to_string());
                }
                Err(RecvTimeoutError::Timeout) => {
                    if let Some(e) = error_slot.lock().ok().and_then(|s| s.clone()) {
                        break Some(e);
                    }
                    if Instant::now() >= deadline {
                        break Some(format!(
                            "no video from the source within {} s",
                            FIRST_FRAME_TIMEOUT.as_secs()
                        ));
                    }
                }
            }
        };
        if let Some(mut msg) = failure {
            teardown(Running { stop, shutdown, rtsp, broadcast, accept });
            // The RTSP thread may have written the real reason while we were giving up.
            if let Some(e) = error_slot.lock().ok().and_then(|s| s.clone()) {
                msg = e;
            }
            log::warn!("[video] native RTSP client failed to start — {msg}");
            return Err(msg);
        }

        log::info!("[video] native RTSP client live on 127.0.0.1:{port}");
        self.inner
            .lock()
            .unwrap()
            .replace(Running { stop, shutdown, rtsp, broadcast, accept });
        Ok(port)
    }

    /// Stop the client and the broadcast if running. Idempotent; never fires `on_ended`.
    pub fn stop(&self) {
        let taken = self.inner.lock().unwrap().take();
        if let Some(r) = taken {
            teardown(r);
            log::info!("[video] native RTSP client stopped");
        }
    }
}

impl Drop for NativeRtsp {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    /// End-to-end backend slice against a real RTSP source (tools/rtsp_test_server.py, a
    /// UAV-Link, an IP cam): client → depacketizer → broadcast → HTTP multipart out.
    /// `KITE_RTSP_URL=rtsp://... cargo test serves_multipart -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn serves_multipart_from_a_real_rtsp_source() {
        let Ok(url) = std::env::var("KITE_RTSP_URL") else {
            eprintln!("KITE_RTSP_URL not set — skipping");
            return;
        };
        let server = NativeRtsp::new();
        let port = server.start(Arc::new(|| {}), &url, "auto").expect("start");

        let mut sock = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
        sock.write_all(b"GET / HTTP/1.1\r\n\r\n").unwrap();
        sock.set_read_timeout(Some(Duration::from_secs(5))).ok();
        let mut buf = vec![0u8; 512 * 1024];
        let mut got = 0usize;
        while got < buf.len() {
            match sock.read(&mut buf[got..]) {
                Ok(0) => break,
                Ok(n) => {
                    got += n;
                    if got > 200_000 {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        server.stop();

        let data = &buf[..got];
        let parts = data.windows(8).filter(|w| *w == b"--ffmpeg").count();
        eprintln!("read {got} bytes, {parts} multipart parts");
        assert!(parts >= 2, "expected several multipart JPEG parts, got {parts}");
        assert!(
            data.windows(2).any(|w| w == [0xFF, 0xD8]),
            "expected a JPEG SOI marker in the stream"
        );
    }
}
