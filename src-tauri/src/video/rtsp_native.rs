// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! In-process RTSP bridge — the native video path (Dev-Docs active/MOBILE_RTSP.md):
//! Kite's own RTSP client (`video::rtsp`) reads the source (UDP first, automatic TCP
//! fallback) and routes by the negotiated codec:
//!
//! * **MJPEG** (P1) — frames are broadcast on the same local multipart HTTP port the
//!   ffmpeg image path serves. No MediaMTX, no ffmpeg — and byte-compatible part framing,
//!   so every sink, the off-thread reader and the reconnect wiring work unchanged.
//! * **H264** (P2.1, Windows) — access units go straight into the Media Foundation decode
//!   sink (`video::win_sink`), which renders below the WebView in the hole the frontend
//!   cuts. No HTTP leg at all; the frontend drives the sink's rect/visibility.
//!
//! Reuses `mjpeg_server`'s accept/broadcast machinery through a channel-backed `Read`
//! adapter instead of an ffmpeg stdout — the broadcast loop neither knows nor cares where
//! the parts come from. In sink mode nothing is ever broadcast, but the machinery still
//! runs: the frame sender dropping at RTSP-thread end is what turns "the live feed died"
//! into the ended event, identically for both routes.

use std::io::Read;
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::mjpeg_server::{accept_loop, broadcast_loop, Client, EndedHook};
use super::rtsp::{run_rtsp, RtspConfig, RtspTransport, VideoCodec};
#[cfg(target_os = "windows")]
use super::win_sink::{SinkCodec, WinVideoSink};

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

/// What `start` brought up — decides how the frontend renders the feed.
pub enum Started {
    /// MJPEG broadcast on the local multipart port (the P1 image path).
    Mjpeg { port: u16 },
    /// H264/HEVC into the native decode sink (Windows) — no local HTTP leg; the frontend
    /// cuts the CSS hole and keeps the sink rect in sync. `codec` names what plays, for
    /// the panel's info line.
    Sink { codec: &'static str },
}

/// RTP 32-bit timestamp → monotonic 64-bit 90 kHz ticks for the sink's sample times.
#[cfg(target_os = "windows")]
#[derive(Default)]
struct SinkTs {
    unwrapped: u64,
    last: Option<u32>,
}

#[cfg(target_os = "windows")]
impl SinkTs {
    fn unwrap(&mut self, ts: u32) -> u64 {
        if let Some(prev) = self.last {
            self.unwrapped = self.unwrapped.wrapping_add(ts.wrapping_sub(prev) as u64);
        }
        self.last = Some(ts);
        self.unwrapped
    }
}

#[derive(Default)]
pub struct NativeRtsp {
    inner: Mutex<Option<Running>>,
    /// The active decode sink, when the stream selected that route. Lives on `self` (not
    /// in `Running`) so the rect/visibility commands can reach it without teardown
    /// plumbing; cleared together with the stream.
    #[cfg(target_os = "windows")]
    sink: Arc<Mutex<Option<WinVideoSink>>>,
    /// Which codec the active sink decodes ("H.264"/"H.265"), for the start verdict.
    #[cfg(target_os = "windows")]
    sink_codec: Arc<Mutex<Option<&'static str>>>,
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

    /// Start the native RTSP client on `url` (`transport`: udp | tcp | anything → auto).
    /// An MJPEG stream is broadcast on a local multipart HTTP port; an H264 stream goes
    /// into the native decode sink, created as a child of `parent_hwnd` (Windows only —
    /// `None` keeps the accept list MJPEG-only, so other platforms fail stream selection
    /// with a message naming what the source offers). Returns once the FIRST frame
    /// arrived; a failure (unreachable, auth, no usable track, sink init) returns the
    /// client's own error message. `on_ended` fires when a live feed dies — never on stop.
    pub fn start(
        &self,
        on_ended: EndedHook,
        url: &str,
        transport: &str,
        parent_hwnd: Option<isize>,
    ) -> Result<Started, String> {
        self.stop();

        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("bind: {e}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("set_nonblocking: {e}"))?;
        let port = listener.local_addr().map_err(|e| format!("addr: {e}"))?.port();

        #[cfg(target_os = "windows")]
        let accept = if parent_hwnd.is_some() {
            vec![VideoCodec::Mjpeg, VideoCodec::H264, VideoCodec::H265]
        } else {
            vec![VideoCodec::Mjpeg]
        };
        #[cfg(not(target_os = "windows"))]
        let accept = {
            let _ = parent_hwnd;
            vec![VideoCodec::Mjpeg]
        };

        let cfg = RtspConfig {
            url: url.to_string(),
            transport: match transport {
                "udp" => RtspTransport::Udp,
                "tcp" => RtspTransport::Tcp,
                _ => RtspTransport::Auto,
            },
            accept,
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
        // The MJPEG route's first-frame signal comes from the broadcast loop (first part
        // written); the sink route has no broadcast, so it signals through this clone the
        // moment the sink is up and fed.
        let sink_first = first_tx.clone();
        let broadcast = {
            let shutdown = shutdown.clone();
            let reader = ChannelRead { rx: frame_rx, buf: Vec::new(), pos: 0 };
            thread::spawn(move || broadcast_loop(reader, clients, shutdown, Some(first_tx), on_ended))
        };
        let rtsp = {
            let stop = stop.clone();
            let error_slot = error_slot.clone();
            #[cfg(target_os = "windows")]
            let sink_slot = self.sink.clone();
            #[cfg(target_os = "windows")]
            let sink_codec_slot = self.sink_codec.clone();
            thread::spawn(move || {
                let mut first = true;
                #[cfg(not(target_os = "windows"))]
                let _ = &sink_first;
                #[cfg(target_os = "windows")]
                let mut sink_ts: Option<SinkTs> = None;
                let stop_flag = stop.clone();
                let result = run_rtsp(&cfg, &stop, &mut |frame| match frame.codec {
                    VideoCodec::Mjpeg => {
                        let part = mpjpeg_part(&frame.data, first);
                        first = false;
                        let _ = frame_tx.send(part);
                    }
                    #[cfg(target_os = "windows")]
                    VideoCodec::H264 | VideoCodec::H265 => {
                        if sink_ts.is_none() {
                            // First AU decides the route: bring the decode sink up. It
                            // starts as a 1×1 child — invisible until the frontend cuts
                            // the CSS hole and pushes the real rect.
                            let Some(parent) = parent_hwnd else { return };
                            let (sink_codec, codec_name) = match frame.codec {
                                VideoCodec::H265 => (SinkCodec::H265, "H.265"),
                                _ => (SinkCodec::H264, "H.264"),
                            };
                            match WinVideoSink::start(parent, (0, 0, 1, 1), sink_codec) {
                                Ok(sink) => {
                                    *sink_slot.lock().unwrap() = Some(sink);
                                    *sink_codec_slot.lock().unwrap() = Some(codec_name);
                                    sink_ts = Some(SinkTs::default());
                                    let _ = sink_first.send(());
                                }
                                Err(e) => {
                                    if let Ok(mut slot) = error_slot.lock() {
                                        slot.get_or_insert(format!("native decode sink failed: {e}"));
                                    }
                                    stop_flag.store(true, Ordering::SeqCst);
                                    return;
                                }
                            }
                        }
                        let ts = sink_ts.as_mut().unwrap().unwrap(frame.rtp_timestamp);
                        if let Some(sink) = sink_slot.lock().unwrap().as_ref() {
                            sink.push(frame.data, ts);
                            if let Some(e) = sink.error() {
                                // Fatal decode error: end the stream — the sender drop
                                // below reads as "the feed died" and the frontend
                                // reconnects with a fresh sink.
                                if let Ok(mut slot) = error_slot.lock() {
                                    slot.get_or_insert(format!("native decode sink failed: {e}"));
                                }
                                stop_flag.store(true, Ordering::SeqCst);
                            }
                        }
                    }
                    // Not on the accept list — the client never selects such a track.
                    #[cfg(not(target_os = "windows"))]
                    _ => {}
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
            #[cfg(target_os = "windows")]
            drop(self.sink.lock().unwrap().take());
            // The RTSP thread may have written the real reason while we were giving up.
            if let Some(e) = error_slot.lock().ok().and_then(|s| s.clone()) {
                msg = e;
            }
            log::warn!("[video] native RTSP client failed to start — {msg}");
            return Err(msg);
        }

        let started = {
            #[cfg(target_os = "windows")]
            {
                if self.sink.lock().unwrap().is_some() {
                    Started::Sink {
                        codec: self.sink_codec.lock().unwrap().unwrap_or("H.264"),
                    }
                } else {
                    Started::Mjpeg { port }
                }
            }
            #[cfg(not(target_os = "windows"))]
            Started::Mjpeg { port }
        };
        match &started {
            Started::Mjpeg { .. } => {
                log::info!("[video] native RTSP client live on 127.0.0.1:{port}")
            }
            Started::Sink { codec } => {
                log::info!("[video] native RTSP client live on the {codec} decode sink")
            }
        }
        self.inner
            .lock()
            .unwrap()
            .replace(Running { stop, shutdown, rtsp, broadcast, accept });
        Ok(started)
    }

    /// Stop the client and the broadcast if running. Idempotent; never fires `on_ended`.
    pub fn stop(&self) {
        let taken = self.inner.lock().unwrap().take();
        if let Some(r) = taken {
            teardown(r);
            log::info!("[video] native RTSP client stopped");
        }
        // After the joins: the RTSP thread pushed into the sink, so it must be gone first.
        #[cfg(target_os = "windows")]
        {
            drop(self.sink.lock().unwrap().take());
            *self.sink_codec.lock().unwrap() = None;
        }
    }

    /// Forward the on-screen video rect (PHYSICAL px, main-window client coords) to the
    /// active decode sink. No-op without one (MJPEG route, non-Windows, stopped).
    pub fn sink_rect(&self, x: i32, y: i32, w: i32, h: i32) {
        #[cfg(target_os = "windows")]
        if let Some(s) = self.sink.lock().unwrap().as_ref() {
            s.set_rect(x, y, w, h);
        }
        #[cfg(not(target_os = "windows"))]
        let _ = (x, y, w, h);
    }

    /// Show/hide the decode sink's native layer (no DOM surface wants it right now).
    pub fn sink_visible(&self, visible: bool) {
        #[cfg(target_os = "windows")]
        if let Some(s) = self.sink.lock().unwrap().as_ref() {
            s.set_visible(visible);
        }
        #[cfg(not(target_os = "windows"))]
        let _ = visible;
    }

    /// `(frames_presented, picture_size, error)` of the active decode sink; `None` while
    /// no sink runs. The frontend polls this for aspect ratio, fps and stall detection.
    pub fn sink_stats(&self) -> Option<(u64, Option<(u32, u32)>, Option<String>)> {
        #[cfg(target_os = "windows")]
        {
            self.sink
                .lock()
                .unwrap()
                .as_ref()
                .map(|s| (s.frames_presented(), s.picture_size(), s.error()))
        }
        #[cfg(not(target_os = "windows"))]
        None
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
        let started = server.start(Arc::new(|| {}), &url, "auto", None).expect("start");
        let Started::Mjpeg { port } = started else {
            panic!("expected the MJPEG broadcast route (no parent hwnd was given)");
        };

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

    /// End-to-end backend slice of the H264 route (start tools/rtsp_test_server.py
    /// --codec h264 first): client → depacketizer → routing → MF decode sink, on a real
    /// host window standing in for the Tauri main window.
    /// `KITE_RTSP_URL=rtsp://... cargo test streams_h264_into -- --ignored --nocapture`
    #[cfg(target_os = "windows")]
    #[test]
    #[ignore]
    fn streams_h264_into_the_native_sink() {
        use windows::core::w;
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW,
            TranslateMessage, MSG, WINDOW_EX_STYLE, WNDCLASSW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
        };

        let Ok(url) = std::env::var("KITE_RTSP_URL") else {
            eprintln!("KITE_RTSP_URL not set — skipping");
            return;
        };

        unsafe extern "system" fn host_proc(
            hwnd: windows::Win32::Foundation::HWND,
            msg: u32,
            wp: windows::Win32::Foundation::WPARAM,
            lp: windows::Win32::Foundation::LPARAM,
        ) -> windows::Win32::Foundation::LRESULT {
            unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
        }

        let (hwnd_tx, hwnd_rx) = std::sync::mpsc::channel::<isize>();
        std::thread::spawn(move || unsafe {
            let class = w!("KiteNativeRtspBenchHost");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(host_proc),
                hInstance: GetModuleHandleW(None).unwrap().into(),
                lpszClassName: class,
                ..Default::default()
            };
            RegisterClassW(&wc);
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class,
                w!("Kite native RTSP bench"),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                120,
                120,
                740,
                520,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            let _ = hwnd_tx.send(hwnd.0 as isize);
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        });
        let host = hwnd_rx.recv_timeout(Duration::from_secs(5)).expect("host window");

        let server = NativeRtsp::new();
        let started = server
            .start(Arc::new(|| {}), &url, "auto", Some(host))
            .expect("start");
        assert!(
            matches!(started, Started::Sink { .. }),
            "expected the H264 decode-sink route for this source"
        );
        server.sink_rect(40, 40, 640, 400);
        std::thread::sleep(Duration::from_secs(6));
        let (presented, size, err) = server.sink_stats().expect("sink stats");
        eprintln!("presented={presented} size={size:?} err={err:?}");
        server.stop();
        assert!(server.sink_stats().is_none(), "sink must be gone after stop");
        assert!(err.is_none(), "sink error: {err:?}");
        assert!(presented > 50, "expected >50 presented frames, got {presented}");
    }
}
