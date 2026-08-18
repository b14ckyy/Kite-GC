// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Embedded MJPEG-over-HTTP server. Serves two kinds of source:
//!
//! * **native capture devices** (V4L2 / DirectShow / AVFoundation) — the per-OS input + codec
//!   handling lives in `video/native.rs`.
//! * **RTSP streams whose picture reaches the screen as MJPEG** — either because the source already
//!   sends MJPEG, or because this WebView has no WebRTC and the H.264 has to be transcoded.
//!
//! Spawns `ffmpeg … -f mpjpeg -` and **broadcasts** its stdout to every connected HTTP client as a
//! `multipart/x-mixed-replace` stream on a local port. One ffmpeg decode/transcode fans out to all
//! sinks (panel preview, floating window, dock widget, full-screen swap).
//!
//! Sockets get `TCP_NODELAY` and ffmpeg flushes per packet, so localhost delivery isn't bunched by
//! Nagle/output buffering (which shows up as sporadic stutter, worse at 60 fps).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, ChildStderr, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
/// Called once when a running feed's source dies — see `commands::video::MJPEG_ENDED_EVENT`, which
/// is what it turns into.
///
/// A plain callback rather than an `AppHandle` **on purpose**: this is the only module here with
/// unit tests, so the linker pulls its object into the test binary — and with it everything it
/// references. Naming `Emitter::emit` dragged Tauri's window runtime in, whose ComCtl32 v6 entry
/// points (`SetWindowSubclass`, `TaskDialogIndirect`, …) only resolve for a binary carrying an
/// application manifest. Test binaries carry none, so `cargo test` died at load with
/// `STATUS_ENTRYPOINT_NOT_FOUND` on Windows before the harness ran a single test. Keeping Tauri out
/// of this module keeps the test binary linkable.
pub type EndedHook = Arc<dyn Fn() + Send + Sync>;

/// Give up on a client that has not accepted a single byte for this long. Not a stutter guard — the
/// broadcast never waits for anyone (see `Client::push`) — just the point at which a socket that
/// never drains again is considered gone.
const CLIENT_STALL_TIMEOUT: Duration = Duration::from_secs(10);

/// Safety valve: if this much data accumulates without yielding a complete part, the framing is not
/// what we expect and the bytes are forwarded as they are rather than held forever.
const MAX_UNFRAMED_BYTES: usize = 4 * 1024 * 1024;

/// How long `start()` waits for the capture's first bytes before declaring it failed. Generous enough
/// for an HDMI dongle negotiating a mode (1–2 s is normal), short enough that a rejected mode reports
/// back while the user is still looking at "Starting…".
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(6);

/// How long a client gets to send its request line before it is answered. Only our own WebView
/// connects here (loopback), so it always arrives at once; a connection that stays silent gets the
/// multipart answer, i.e. exactly what every client got before there was a choice.
const REQUEST_READ_TIMEOUT: Duration = Duration::from_millis(500);

/// ffmpeg's wording when the **hardware** decoder or encoder is what refused the stream.
///
/// Scanned across every stderr line, not just the first one, and that is the whole point: on a lossy
/// source the decoder's per-frame complaints ("negative number of zero coeffs at 42 39") reach us
/// first and would otherwise be reported as the cause. Measured on a Wi-Fi-fed 720p60 feed, where two
/// consecutive attempts at the same broken VAAPI path reported two different first lines — so the
/// caller's "was this the hardware?" verdict came out different each time.
const HW_FAILURE_MARKERS: [&str; 3] = [
    "Failed setup for format", // hwaccel decode init (VAAPI, V4L2)
    "hwaccel initialisation",
    "Could not open encoder", // mjpeg_vaapi / h264_v4l2m2m — checked only for a hardware attempt
];

/// Prefixed to a start failure the hardware caused, so the caller can tell a driver's verdict from an
/// unreachable source without parsing ffmpeg's prose. Kept in sync with `stores/video.ts`.
pub const HW_FAILURE_PREFIX: &str = "hardware transcode failed — ";

/// The multipart HTTP response preamble sent once per client before the frame stream.
/// `Access-Control-Allow-Origin` is required because the WebView reads this stream with `fetch` from
/// a worker (the off-thread MJPEG reader) — a cross-origin fetch without it is blocked. An `<img>`,
/// which is what the fallback path still uses, never needed the header.
const HTTP_HEADERS_MULTIPART: &[u8] = b"HTTP/1.1 200 OK\r\n\
    Content-Type: multipart/x-mixed-replace; boundary=ffmpeg\r\n\
    Access-Control-Allow-Origin: *\r\n\
    Cache-Control: no-cache\r\n\
    Connection: close\r\n\
    \r\n";

/// The same byte stream under a content type WebKit does not special-case, requested with `?raw=1`.
///
/// WebKit handles `multipart/x-mixed-replace` inside its resource loader and never exposes it to
/// `fetch`: measured on WebKitGTK 2.52.5 from a `tauri://localhost` page, the response headers arrive
/// (200, correct type) and the very first `reader.read()` fails with `Load failed`, zero bytes — main
/// thread and worker alike. The identical bytes as `application/octet-stream` stream perfectly. So
/// the off-thread reader asks for this variant and the `<img>` fallback keeps the multipart one,
/// which is the only type it can render. Requested on WebKit engines only, so WebView2 keeps the
/// exact response it has always had.
///
/// The body is byte-identical — same boundaries, same part headers. Dropping the `boundary`
/// parameter costs the reader nothing because ffmpeg's mpjpeg muxer writes a `Content-length` on
/// every part, which is what it frames on; the boundary is only its fallback for parts without one.
const HTTP_HEADERS_OCTET: &[u8] = b"HTTP/1.1 200 OK\r\n\
    Content-Type: application/octet-stream\r\n\
    Access-Control-Allow-Origin: *\r\n\
    Cache-Control: no-cache\r\n\
    Connection: close\r\n\
    \r\n";

/// What the server captures from.
pub enum MjpegSource<'a> {
    /// A local capture device, described by the mode the user picked.
    Device(&'a super::native::CaptureSpec),
    /// A network stream, read directly rather than through a republishing engine.
    ///
    /// The old go2rtc chain drove an `ffmpeg:` source by having ffmpeg publish **back into it** over
    /// RTSP/TCP, so a stream that was already MJPEG got packetised into RTP/JPEG (RFC 2435),
    /// reassembled and repacked as HTTP multipart. Measured over the same 120 s against a UAV-Link:
    /// the source had **zero** arrival gaps above 200 ms, the engine's output had **69**, each of
    /// them ~338 ms — the loopback TCP-publish stall, and the cause of the freezes testers reported
    /// for years. Reading the source once and broadcasting `-f mpjpeg` measures as clean as the
    /// source itself.
    Rtsp { url: &'a str, transcode: RtspTranscode },
}

/// How an RTSP source's video becomes the MJPEG the multipart sink needs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RtspTranscode {
    /// The source already sends MJPEG — pass its packets through untouched. No decode, no encode.
    Copy,
    /// Pi-class SoC: hardware H.264 decode, software MJPEG encode (no V4L2 M2M MJPEG encoder exists).
    V4l2m2m,
    /// Desktop GPU: both halves on the GPU, the frames never leaving it. Carries the render node.
    Vaapi(&'static str),
    Software,
}

impl RtspTranscode {
    /// The label the UI shows for the running pipeline — the pipeline that IS running, not what the
    /// host could do.
    pub fn label(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::V4l2m2m => "v4l2m2m",
            Self::Vaapi(_) => "vaapi",
            Self::Software => "software",
        }
    }
}

#[derive(Default)]
pub struct MjpegServer {
    inner: Mutex<Option<Running>>,
}

struct Running {
    ffmpeg: Child,
    shutdown: Arc<AtomicBool>,
    _accept: JoinHandle<()>,
    _reader: JoinHandle<()>,
    _stderr: JoinHandle<()>,
}

impl MjpegServer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start the MJPEG server. Spawns ffmpeg to read `source` and output MJPEG (stream-copied where
    /// the input already carries it, transcoded otherwise), then broadcasts its stdout to all
    /// connected HTTP clients.
    ///
    /// Returns the port **only once the source has actually delivered its first bytes** (up to
    /// `FIRST_FRAME_TIMEOUT`); otherwise everything is torn down again and the error carries ffmpeg's
    /// own first stderr line. Blocking by design — call it from an async command.
    ///
    /// `on_ended` fires only for a source that dies while live — never for a start that failed, which
    /// is reported through the return value instead.
    pub fn start(&self, on_ended: EndedHook, source: &MjpegSource) -> Result<u16, String> {
        self.stop();

        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("bind: {e}"))?;
        // Non-blocking so the accept loop can break out on shutdown.
        listener.set_nonblocking(true).map_err(|e| format!("set_nonblocking: {e}"))?;
        let port = listener.local_addr().map_err(|e| format!("addr: {e}"))?.port();

        // Resolve ffmpeg through the project's managed discovery (auto-download
        // on demand for Win/Linux, bundled on macOS). Falls back to "ffmpeg" if
        // not found (the error message will guide the user to install it).
        let ffmpeg_bin = super::ffmpeg::find_ffmpeg()
            .unwrap_or_else(|| std::path::PathBuf::from("ffmpeg"));

        // Build: [loglevel] + input (device or network) + output codec + mpjpeg mux (flushing per
        // packet). `-fflags nobuffer` and the hardware decoder selection are INPUT options, so they
        // precede the demuxer / `-i`.
        let mut args: Vec<String> = vec!["-loglevel".into(), "error".into()];
        match source {
            MjpegSource::Device(spec) => {
                args.extend(["-fflags".into(), "nobuffer".into()]);
                args.extend(super::native::input_args(spec));
                if super::native::needs_transcode(&spec.codec) {
                    // Raw / H.264 / auto → re-encode to MJPEG for the multipart sink.
                    args.extend(["-c:v".into(), "mjpeg".into(), "-q:v".into(), "5".into()]);
                } else {
                    // The camera already emits MJPEG: pass the packets straight through. No decode, no
                    // encode, no colour conversion — cheaper than any hardware transcode could ever be,
                    // so there is deliberately nothing to accelerate here.
                    args.extend(["-c".into(), "copy".into()]);
                }
            }
            MjpegSource::Rtsp { url, transcode } => {
                match transcode {
                    RtspTranscode::V4l2m2m => args.extend(["-c:v".into(), "h264_v4l2m2m".into()]),
                    RtspTranscode::Vaapi(node) => args.extend([
                        "-hwaccel".into(),
                        "vaapi".into(),
                        "-hwaccel_device".into(),
                        (*node).into(),
                        // Load-bearing: keeps decoded frames in GPU memory for the encoder below.
                        // Without it every frame is copied back to system memory and the chain ends
                        // up SLOWER than software.
                        "-hwaccel_output_format".into(),
                        "vaapi".into(),
                        // Without this the whole hardware path is unreachable for a large class of
                        // FPV sources. They encode **plain Baseline** (profile_idc 66 with
                        // constraint_set1 clear — the UAV-Link Pi among them), and Intel's iHD driver
                        // advertises only ConstrainedBaseline/Main/High, so ffmpeg refuses before it
                        // ever asks the GPU: "Codec h264 profile 66 not supported for hardware
                        // decode". The frames themselves are a subset of what the decoder does every
                        // day. Measured on that stream, 720p60, Debian 13 / Intel iGPU: 92 % of a
                        // core in software against 10.8 % here, both at 58 fps and no drops. If a
                        // driver really cannot decode it, the start fails the same way it does now
                        // and the caller demotes to the software transcode on the first strike.
                        "-hwaccel_flags".into(),
                        "allow_profile_mismatch".into(),
                    ]),
                    RtspTranscode::Copy | RtspTranscode::Software => {}
                }
                // Deliberately NO `-rtsp_transport`: forcing one is what stops a UDP-only server
                // (the UAV-Link class) from opening at all, while ffmpeg's own negotiation reads
                // both. `-timeout` is in microseconds and makes a dead source exit rather than hang,
                // which is what lets the frontend notice and reconnect.
                //
                // 10 s to match the WebRTC path's live-stall window (`RTSP_STALL_LIVE_MS`), so both
                // readers tolerate the same LTE radio hole. go2rtc used 5 s here; UDP fires blind, so
                // the longer window is the better trade — and a stream abandoned mid-flight is
                // reaped server-side after 60 s anyway, which bounds how many can pile up.
                args.extend([
                    "-fflags".into(),
                    "nobuffer".into(),
                    "-flags".into(),
                    "low_delay".into(),
                    "-timeout".into(),
                    "10000000".into(),
                    "-i".into(),
                    (*url).into(),
                    "-an".into(),
                ]);
                match transcode {
                    RtspTranscode::Copy => args.extend(["-c".into(), "copy".into()]),
                    // `-async_depth 1`: the VAAPI encoders pipeline 2 frames by default for
                    // throughput, which on a live feed is simply latency — we want the frame out,
                    // not the frame rate.
                    RtspTranscode::Vaapi(_) => args.extend([
                        "-c:v".into(),
                        "mjpeg_vaapi".into(),
                        "-async_depth".into(),
                        "1".into(),
                    ]),
                    RtspTranscode::V4l2m2m | RtspTranscode::Software => {
                        args.extend(["-c:v".into(), "mjpeg".into(), "-q:v".into(), "5".into()])
                    }
                }
            }
        }
        // Emit each packet immediately (no output buffering) → even, low-jitter frame delivery.
        args.extend(["-flush_packets".into(), "1".into(), "-f".into(), "mpjpeg".into(), "-".into()]);

        let mut cmd = Command::new(&ffmpeg_bin);
        crate::child_env::sanitize(&mut cmd);
        cmd.args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()) // capture ffmpeg errors for the log (tester diagnostics)
            .stdin(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW — don't flash a console
        }
        let mut ffmpeg = cmd
            .spawn()
            .map_err(|e| format!("ffmpeg spawn failed: {e}"))?;

        let stdout = ffmpeg.stdout.take().ok_or("no stdout")?;
        let stderr = ffmpeg.stderr.take();
        let shutdown = Arc::new(AtomicBool::new(false));
        let clients: Arc<Mutex<Vec<Client>>> = Arc::new(Mutex::new(Vec::new()));
        // First-frame signal + the first ffmpeg error line, so a failed capture can be reported to the
        // caller instead of only reaching the log. `hw_err` collects the same for the hardware
        // markers, whatever position they arrive in.
        let (first_tx, first_rx) = std::sync::mpsc::channel();
        let first_err: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let hw_err: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let hw_attempt = matches!(
            source,
            MjpegSource::Rtsp {
                transcode: RtspTranscode::Vaapi(_) | RtspTranscode::V4l2m2m,
                ..
            }
        );

        let accept = {
            let clients = clients.clone();
            let shutdown = shutdown.clone();
            thread::spawn(move || accept_loop(listener, clients, shutdown))
        };
        let reader = {
            let shutdown = shutdown.clone();
            thread::spawn(move || broadcast_loop(stdout, clients, shutdown, Some(first_tx), on_ended))
        };
        let stderr_thread = {
            let first_err = first_err.clone();
            let hw_err = hw_err.clone();
            thread::spawn(move || log_ffmpeg_stderr(stderr, first_err, hw_err))
        };

        // Don't report success until the capture actually delivers. Previously `start()` returned as
        // soon as the listener was bound, so a device that rejects the requested mode (AVFoundation and
        // DirectShow both abort hard) left the UI showing "live" over a black frame, with the reason
        // only in the log.
        if let Err(reason) = wait_for_first_frame(&mut ffmpeg, &first_rx) {
            shutdown.store(true, Ordering::SeqCst);
            let _ = ffmpeg.kill();
            let _ = ffmpeg.wait();
            let _ = reader.join();
            let _ = accept.join();
            let _ = stderr_thread.join(); // stderr is at EOF now → the error line is recorded
            // A hardware marker outranks the first line: it is the one that explains the failure,
            // where the first line may be a damaged macroblock the decoder shrugged off.
            let hw_detail = hw_err.lock().ok().and_then(|g| g.clone()).filter(|_| hw_attempt);
            let detail = hw_detail
                .clone()
                .or_else(|| first_err.lock().ok().and_then(|g| g.clone()));
            let msg = match detail {
                Some(line) => format!("{reason}: {line}"),
                None => reason,
            };
            let msg = match hw_detail {
                Some(_) => format!("{HW_FAILURE_PREFIX}{msg}"),
                None => msg,
            };
            let what = match source {
                MjpegSource::Device(_) => "native capture",
                MjpegSource::Rtsp { .. } => "the RTSP MJPEG reader",
            };
            log::warn!("[video] {what} failed to start — {msg}");
            return Err(msg);
        }

        self.inner.lock().unwrap().replace(Running {
            ffmpeg,
            shutdown,
            _accept: accept,
            _reader: reader,
            _stderr: stderr_thread,
        });
        Ok(port)
    }

    pub fn stop(&self) {
        let mut guard = self.inner.lock().unwrap();
        if let Some(mut r) = guard.take() {
            // Signal both threads, then kill ffmpeg — closing stdout unblocks the reader's read().
            r.shutdown.store(true, Ordering::SeqCst);
            let _ = r.ffmpeg.kill();
            let _ = r.ffmpeg.wait();
            let _ = r._reader.join();
            let _ = r._accept.join();
            let _ = r._stderr.join();
            log::info!("MJPEG server stopped");
        }
    }
}

impl Drop for MjpegServer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Block until the capture produced its first bytes, ffmpeg died, or the grace period ran out.
/// A dropped sender (channel `Disconnected`) means the broadcast loop hit stdout EOF — i.e. ffmpeg
/// exited before delivering anything, which is the common "device rejected the mode" case.
fn wait_for_first_frame(child: &mut Child, rx: &Receiver<()>) -> Result<(), String> {
    let deadline = Instant::now() + FIRST_FRAME_TIMEOUT;
    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(()) => return Ok(()),
            Err(RecvTimeoutError::Disconnected) => {
                return Err("the capture ended immediately".to_string());
            }
            Err(RecvTimeoutError::Timeout) => {
                if matches!(child.try_wait(), Ok(Some(_))) {
                    return Err("ffmpeg exited without delivering a frame".to_string());
                }
                if Instant::now() >= deadline {
                    return Err(format!(
                        "no video from the source within {} s",
                        FIRST_FRAME_TIMEOUT.as_secs()
                    ));
                }
            }
        }
    }
}

/// Length of the complete multipart part at the front of `buf`, or `None` while it is still
/// arriving.
///
/// A part is `[separator]--boundary\r\n<headers>\r\n\r\n<body>`, and ffmpeg's mpjpeg muxer states
/// the body length in every one of them. Whatever precedes the header block — the CRLF that closed
/// the previous body, the boundary line — belongs to this part, which is what makes a dropped frame
/// harmless: the next one a client receives still starts with its own separator and boundary.
fn frame_len(buf: &[u8]) -> Option<usize> {
    let head_end = buf.windows(4).position(|w| w == b"\r\n\r\n")? + 4;
    let head = std::str::from_utf8(&buf[..head_end]).ok()?;
    let value = head
        .lines()
        .find_map(|l| l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case("content-length")))
        .map(|(_, v)| v.trim())?;
    let body: usize = value.parse().ok()?;
    let total = head_end.checked_add(body)?;
    (buf.len() >= total).then_some(total)
}

/// Whether a request asked for the non-multipart variant (`?raw=1`) — see `HTTP_HEADERS_OCTET`.
/// Only the request line is looked at; the rest of the headers were never read and still aren't.
fn wants_raw(request: &str) -> bool {
    request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|target| target.split_once('?'))
        .is_some_and(|(_, query)| query.split('&').any(|p| p == "raw=1"))
}

/// Read a client's request line. A silent or unreadable client reads as "not raw", which is the
/// multipart answer this server has always given.
fn read_request(stream: &mut TcpStream) -> String {
    let _ = stream.set_read_timeout(Some(REQUEST_READ_TIMEOUT));
    // Read until the request LINE is complete, not just once. TCP is a stream: a client that builds
    // its request in pieces (`write!` issues one syscall per format fragment, and browsers are free
    // to split too) can have it arrive in several segments. A single read then saw `GET ` and judged
    // the target from that — answering the raw variant with the multipart response, which is the one
    // thing its reader cannot parse. It surfaced as a Linux-only test failure because on Windows the
    // fragments happened to coalesce; the bug was never platform-specific.
    let mut buf = [0u8; 512];
    let mut len = 0;
    while len < buf.len() {
        match stream.read(&mut buf[len..]) {
            Ok(0) => break, // client hung up
            Ok(n) => {
                len += n;
                if buf[..len].contains(&b'\n') {
                    break; // first line complete — everything we read is on it
                }
            }
            Err(_) => break, // timeout or error: judge on what arrived, which reads as "not raw"
        }
    }
    let _ = stream.set_read_timeout(None);
    String::from_utf8_lossy(&buf[..len]).into_owned()
}

/// One connected sink. Writes are non-blocking and never waited on: a client that cannot take a
/// frame right now loses **that frame**, and the broadcast moves on.
///
/// This is what keeps one slow consumer from stalling everyone. Measured against the real source
/// with a client that needs 25 ms per frame — the pace a WebKit JPEG decode actually manages — while
/// a second, always-fast client recorded arrivals: with the old blocking write the fast client fell
/// to 43 fps with 121 gaps over 50 ms and a *median* spacing of 0.5 ms, i.e. frames delivered in
/// bursts and nothing in between. Frame-wise and non-blocking, the same fast client holds 60.1 fps
/// with a 16.1 ms median and one gap — indistinguishable from having no slow client at all.
struct Client {
    sock: TcpStream,
    /// Remainder of a part the socket would not take in one go. While this is non-empty the client
    /// is behind, and new frames are dropped for it rather than queued.
    pending: Vec<u8>,
    /// When `pending` last stopped being empty, for `CLIENT_STALL_TIMEOUT`.
    behind_since: Option<Instant>,
    dropped: u64,
}

impl Client {
    /// Offer one complete part. Returns false when the socket is gone and the client should go.
    fn push(&mut self, frame: &[u8], now: Instant) -> bool {
        if !self.pending.is_empty() {
            // Finish what is owed before considering anything new.
            match self.sock.write(&self.pending) {
                Ok(0) => return false,
                Ok(n) => drop(self.pending.drain(..n)),
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => return false,
            }
            if !self.pending.is_empty() {
                self.dropped += 1;
                return now.duration_since(self.behind_since.unwrap_or(now)) < CLIENT_STALL_TIMEOUT;
            }
            self.behind_since = None;
        }
        match self.sock.write(frame) {
            Ok(0) => false,
            Ok(n) if n == frame.len() => true,
            Ok(n) => {
                self.pending.extend_from_slice(&frame[n..]);
                self.behind_since = Some(now);
                true
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                self.pending.extend_from_slice(frame);
                self.behind_since = Some(now);
                true
            }
            Err(_) => false,
        }
    }
}

/// Accept clients and register them for broadcast. Each gets the response preamble matching what it
/// asked for and `TCP_NODELAY` (no Nagle bunching on localhost); the socket then goes non-blocking
/// for the broadcast. Exits when `shutdown` is set.
fn accept_loop(listener: TcpListener, clients: Arc<Mutex<Vec<Client>>>, shutdown: Arc<AtomicBool>) {
    loop {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_nodelay(true);
                let _ = stream.set_nonblocking(false);
                let raw = wants_raw(&read_request(&mut stream));
                let headers = if raw { HTTP_HEADERS_OCTET } else { HTTP_HEADERS_MULTIPART };
                if stream.write_all(headers).is_ok()
                    && stream.flush().is_ok()
                    && stream.set_nonblocking(true).is_ok()
                {
                    clients.lock().unwrap().push(Client {
                        sock: stream,
                        pending: Vec::new(),
                        behind_since: None,
                        dropped: 0,
                    });
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(100));
            }
            Err(_) => break,
        }
    }
}

/// Read ffmpeg's stdout and write each chunk to every connected client, dropping clients that error
/// (a disconnected sink). Always drains stdout even with no clients so ffmpeg never blocks on a full
/// pipe. Exits on EOF (ffmpeg died) or shutdown, and **closes every client on the way out**: a
/// consumer whose socket stays open with no data has no way to tell a dead feed from a quiet one, so
/// leaving them connected left a permanently frozen picture instead of triggering a reconnect.
fn broadcast_loop(
    mut stdout: impl Read,
    clients: Arc<Mutex<Vec<Client>>>,
    shutdown: Arc<AtomicBool>,
    mut first: Option<Sender<()>>,
    on_ended: EndedHook,
) {
    let mut buf = [0u8; 65536];
    // Whole parts are assembled here before they go out, so a client that is behind loses a picture
    // rather than half of one — and so that reading from ffmpeg never waits for a socket.
    let mut acc: Vec<u8> = Vec::with_capacity(256 * 1024);
    let mut drop_report = Instant::now();
    // Set only for a feed that was actually running: a start attempt that never delivered is reported
    // to the caller by `start()` itself, and the copy-first probe in `video_rtsp_mjpeg_start` fails
    // exactly that way on every H.264 source — announcing a dead feed there would fire a reconnect
    // for a stream that never began.
    let mut ended_live = false;
    loop {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }
        let n = match stdout.read(&mut buf) {
            Ok(0) => {
                // stdout EOF = ffmpeg exited. If we didn't ask it to stop, that's the "goes black"
                // failure — surface it (the stderr logger prints ffmpeg's reason just above).
                if !shutdown.load(Ordering::SeqCst) {
                    log::warn!("[video] MJPEG source ended unexpectedly (ffmpeg exited)");
                    ended_live = first.is_none();
                }
                break;
            }
            Ok(n) => n,
            Err(e) => {
                if !shutdown.load(Ordering::SeqCst) {
                    log::warn!("[video] MJPEG read error: {e}");
                    ended_live = first.is_none();
                }
                break;
            }
        };
        // Signal the successful start exactly once, then drop the sender so `start()` learns about a
        // later EOF through the closed channel.
        if let Some(tx) = first.take() {
            let _ = tx.send(());
        }
        acc.extend_from_slice(&buf[..n]);
        // Cut every complete part out of the accumulator. A part carries its own leading separator,
        // so a client that misses one still receives the next with correct framing.
        let mut frames: Vec<&[u8]> = Vec::new();
        let mut cut = 0;
        while let Some(len) = frame_len(&acc[cut..]) {
            frames.push(&acc[cut..cut + len]);
            cut += len;
        }
        if frames.is_empty() {
            // Not a framing we recognise (or a part still arriving). Don't hold bytes forever.
            if acc.len() >= MAX_UNFRAMED_BYTES {
                log::warn!("[video] MJPEG stream carries no recognisable parts — forwarding raw");
                let now = Instant::now();
                let mut list = clients.lock().unwrap();
                let acc_ref = &acc;
                list.retain_mut(|c| c.push(acc_ref, now));
                drop(list);
                acc.clear();
            }
            continue;
        }
        let now = Instant::now();
        {
            let mut list = clients.lock().unwrap();
            if !list.is_empty() {
                for frame in &frames {
                    list.retain_mut(|c| c.push(frame, now));
                }
            }
            // Tell the log when a sink is losing pictures — it is the difference between "the source
            // is slow" and "this machine cannot keep up", and it is otherwise invisible.
            if now.duration_since(drop_report) >= Duration::from_secs(5) {
                let lost: u64 = list.iter().map(|c| c.dropped).sum();
                if lost > 0 {
                    log::debug!("[video] MJPEG broadcast: {lost} frames dropped for slow sinks so far");
                }
                drop_report = now;
            }
        }
        acc.drain(..cut);
    }
    // The source is gone. Stop accepting, and drop every client socket so the sinks see their stream
    // end and the frontend can reconnect instead of staring at a frozen frame.
    shutdown.store(true, Ordering::SeqCst);
    clients.lock().unwrap().clear();
    // Then say so explicitly. Closing the sockets is enough for the off-thread reader, which reads
    // the stream itself and sees it end — but not for the `<img>` fallback on WebKit, which stays
    // silent (see `commands::video::MJPEG_ENDED_EVENT`). The order matters: sockets first, so a
    // reader that notices by itself and the event agree on what happened.
    if ended_live {
        on_ended();
    }
}

/// Forward ffmpeg's stderr to the log. With `-loglevel error` these are genuine errors (device lost,
/// corrupt frame, codec failure) → tester-relevant, so they go at the default-visible `warn` level.
/// The **first** line is also recorded in `first_err` so a start-up failure can be reported to the UI
/// with ffmpeg's own wording (e.g. "Selected video size is not supported by the device"), and the
/// first line naming the hardware in `hw_err` — see `HW_FAILURE_MARKERS` for why that one cannot go
/// by position.
fn log_ffmpeg_stderr(
    stderr: Option<ChildStderr>,
    first_err: Arc<Mutex<Option<String>>>,
    hw_err: Arc<Mutex<Option<String>>>,
) {
    let Some(stderr) = stderr else { return };
    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
        let line = line.trim();
        if !line.is_empty() {
            if let Ok(mut slot) = first_err.lock() {
                slot.get_or_insert_with(|| line.to_string());
            }
            if HW_FAILURE_MARKERS.iter().any(|m| line.contains(m)) {
                if let Ok(mut slot) = hw_err.lock() {
                    slot.get_or_insert_with(|| line.to_string());
                }
            }
            log::warn!("[video][ffmpeg] {line}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run the real accept loop and return the preamble a client with this request target gets.
    fn preamble_for(target: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let clients: Arc<Mutex<Vec<Client>>> = Arc::new(Mutex::new(Vec::new()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let accept = {
            let clients = clients.clone();
            let shutdown = shutdown.clone();
            thread::spawn(move || accept_loop(listener, clients, shutdown))
        };

        let mut sock = TcpStream::connect(("127.0.0.1", port)).unwrap();
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        // Sent in two segments ON PURPOSE, with Nagle off so the first really goes out alone: a
        // request line arriving split is what broke the raw variant, and a single write would only
        // exercise that on whichever platform happens not to coalesce it — which is precisely how the
        // bug reached CI green on Windows and red on Linux.
        sock.set_nodelay(true).unwrap();
        let req = format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n");
        let (head, tail) = req.split_at(6); // mid-target, so a partial read cannot judge it
        sock.write_all(head.as_bytes()).unwrap();
        sock.flush().unwrap();
        thread::sleep(Duration::from_millis(20));
        sock.write_all(tail.as_bytes()).unwrap();
        let mut buf = [0u8; 512];
        let n = sock.read(&mut buf).unwrap();

        shutdown.store(true, Ordering::SeqCst);
        let _ = accept.join();
        String::from_utf8_lossy(&buf[..n]).into_owned()
    }

    /// The property the whole `?raw=1` design rests on: a client that doesn't ask for the variant
    /// gets byte-for-byte what this server always sent. That is every `<img>` sink on every platform,
    /// and the off-thread reader on WebView2, which reads multipart perfectly well.
    #[test]
    fn a_plain_request_still_gets_the_multipart_preamble() {
        assert_eq!(preamble_for("/"), String::from_utf8_lossy(HTTP_HEADERS_MULTIPART));
    }

    #[test]
    fn the_raw_variant_gets_the_non_multipart_preamble() {
        assert_eq!(preamble_for("/?raw=1"), String::from_utf8_lossy(HTTP_HEADERS_OCTET));
    }

    /// Exactly what ffmpeg's mpjpeg muxer emits, two parts back to back.
    fn mpjpeg(bodies: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        for (i, body) in bodies.iter().enumerate() {
            if i > 0 {
                out.extend_from_slice(b"\r\n");
            }
            out.extend_from_slice(b"--ffmpeg\r\nContent-type: image/jpeg\r\n");
            out.extend_from_slice(format!("Content-length: {}\r\n\r\n", body.len()).as_bytes());
            out.extend_from_slice(body);
        }
        out
    }

    #[test]
    fn parts_are_cut_whole_and_in_order() {
        let stream = mpjpeg(&[b"AAAA", b"BBBBBB"]);
        let first = frame_len(&stream).expect("first part is complete");
        assert!(stream[..first].ends_with(b"AAAA"));
        assert!(stream[..first].starts_with(b"--ffmpeg"));

        // The second part carries the CRLF that closed the first body, so a client that missed a
        // frame still receives correct framing.
        let rest = &stream[first..];
        let second = frame_len(rest).expect("second part is complete");
        assert_eq!(second, rest.len());
        assert!(rest.starts_with(b"\r\n--ffmpeg"));
        assert!(rest[..second].ends_with(b"BBBBBB"));
    }

    #[test]
    fn an_incomplete_part_waits() {
        let stream = mpjpeg(&[b"AAAA"]);
        // Body one byte short, and headers only.
        assert_eq!(frame_len(&stream[..stream.len() - 1]), None);
        assert_eq!(frame_len(b"--ffmpeg\r\nContent-length: 4\r\n"), None);
        // Header block present but no length to frame on → nothing is claimed.
        assert_eq!(frame_len(b"--ffmpeg\r\nContent-type: image/jpeg\r\n\r\nAAAA"), None);
    }

    /// A source that paces 16 KB parts at roughly 60 fps, standing in for ffmpeg's stdout.
    ///
    /// "Roughly" is deliberate: `thread::sleep` rounds up to the scheduler's tick, which on Windows
    /// is ~15.6 ms, so the same loop runs at half the rate there. Nothing in the test may depend on
    /// how long it takes — the sinks read until the stream ends, not until a clock runs out.
    struct FakeCapture {
        buf: Vec<u8>,
        next: Instant,
        left: usize,
    }

    impl Read for FakeCapture {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            if self.buf.is_empty() {
                if self.left == 0 {
                    return Ok(0);
                }
                let now = Instant::now();
                if now < self.next {
                    thread::sleep(self.next - now);
                }
                self.next += Duration::from_micros(16_666);
                self.left -= 1;
                self.buf = mpjpeg(&[&vec![0x5a; 30_000]]);
            }
            let n = out.len().min(self.buf.len());
            out[..n].copy_from_slice(&self.buf[..n]);
            self.buf.drain(..n);
            Ok(n)
        }
    }

    /// The property the whole rework exists for: one sink that cannot keep up must not slow the
    /// others down. With the previous blocking write this failed hard — measured against the real
    /// source, a fast client fell from 60 to 43 fps with a 0.5 ms median (i.e. bursts) as soon as a
    /// 25 ms-per-frame client was also connected.
    #[test]
    fn a_slow_sink_does_not_hold_up_a_fast_one() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let clients: Arc<Mutex<Vec<Client>>> = Arc::new(Mutex::new(Vec::new()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let accept = {
            let (c, s) = (clients.clone(), shutdown.clone());
            thread::spawn(move || accept_loop(listener, c, s))
        };

        // Both sinks read until the broadcast closes their socket, which it does when the source
        // ends. Nothing here waits on a clock: a wall-clock deadline made this fail on Windows for
        // the scheduler's timer granularity rather than for anything the server did. The long guards
        // below only stop a hang.
        let guard = || Instant::now() + Duration::from_secs(60);

        // A sink that reads a little and then sleeps — the pace a WebKit JPEG decode manages. It
        // stops with the broadcast rather than draining its receive buffer afterwards, which by then
        // holds megabytes it would take another ten seconds to sip through.
        let slow = {
            let stop = shutdown.clone();
            thread::spawn(move || {
                let mut sock = TcpStream::connect(("127.0.0.1", port)).unwrap();
                sock.write_all(b"GET / HTTP/1.1\r\n\r\n").unwrap();
                sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut sink = [0u8; 1024];
                let until = Instant::now() + Duration::from_secs(60);
                while Instant::now() < until && !stop.load(Ordering::SeqCst) {
                    match sock.read(&mut sink) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => thread::sleep(Duration::from_millis(25)),
                    }
                }
            })
        };
        // …and one that reads as fast as it can, counting the parts it receives.
        let fast = thread::spawn(move || {
            let mut sock = TcpStream::connect(("127.0.0.1", port)).unwrap();
            sock.write_all(b"GET / HTTP/1.1\r\n\r\n").unwrap();
            sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut got = 0usize;
            let mut sink = vec![0u8; 65536];
            let mut carry: Vec<u8> = Vec::new(); // a boundary may straddle two reads
            let until = guard();
            while Instant::now() < until {
                match sock.read(&mut sink) {
                    Ok(0) => break,
                    Ok(n) => {
                        carry.extend_from_slice(&sink[..n]);
                        got += carry.windows(8).filter(|w| *w == b"--ffmpeg").count();
                        let keep = carry.len().saturating_sub(7);
                        carry.drain(..keep);
                    }
                    Err(_) => break,
                }
            }
            got
        });
        // The counters live in the client list, which the broadcast empties on its way out, so they
        // are sampled while it runs.
        let seen_drops = Arc::new(AtomicBool::new(false));
        let sampler = {
            let (clients, stop, flag) = (clients.clone(), shutdown.clone(), seen_drops.clone());
            thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    if clients.lock().unwrap().iter().any(|c| c.dropped > 0) {
                        flag.store(true, Ordering::SeqCst);
                    }
                    thread::sleep(Duration::from_millis(20));
                }
            })
        };

        thread::sleep(Duration::from_millis(200)); // let both connect before the source starts

        let frames = 180;
        let source = FakeCapture { buf: Vec::new(), next: Instant::now(), left: frames };
        broadcast_loop(source, clients, shutdown.clone(), None, Arc::new(|| {}));

        let received = fast.join().unwrap();
        shutdown.store(true, Ordering::SeqCst);
        let _ = slow.join();
        let _ = sampler.join();
        let _ = accept.join();

        // Two halves, and both are needed. The first says a healthy sink still gets its pictures;
        // on its own it would also pass with a blocking write, which delivers everything eventually
        // — just late and in bursts, and "late" cannot be asserted here because the scheduler's timer
        // granularity varies by platform.
        assert!(
            received * 100 / frames >= 90,
            "the fast sink received only {received} of {frames} parts while a slow sink was attached"
        );
        // The second is the mechanism itself, and it is timing-free: a sink that cannot keep up must
        // LOSE frames. If nothing was ever dropped, the broadcast waited for it instead — which is
        // exactly the regression, and what dragged an always-fast client from 60 to 43 fps.
        assert!(
            seen_drops.load(Ordering::SeqCst),
            "no frame was ever dropped for the slow sink — the broadcast is waiting on it again"
        );
    }

    #[test]
    fn raw_variant_is_opt_in() {
        // What the off-thread reader sends on a WebKit engine.
        assert!(wants_raw("GET /?raw=1 HTTP/1.1\r\nHost: 127.0.0.1:9\r\n\r\n"));
        // What every other client sends — and what WebView2 keeps sending.
        assert!(!wants_raw("GET / HTTP/1.1\r\nHost: 127.0.0.1:9\r\n\r\n"));
        assert!(!wants_raw(""));
        // A value that merely contains the marker is not the marker.
        assert!(!wants_raw("GET /?src=raw=12 HTTP/1.1\r\n\r\n"));
        assert!(!wants_raw("GET /raw=1 HTTP/1.1\r\n\r\n"));
        // Order among parameters must not matter.
        assert!(wants_raw("GET /?x=1&raw=1 HTTP/1.1\r\n\r\n"));
    }
}
