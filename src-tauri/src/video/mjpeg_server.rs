// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Embedded MJPEG-over-HTTP server for native capture devices (V4L2 / DirectShow / AVFoundation).
//!
//! Spawns `ffmpeg -f <demuxer> … -f mpjpeg -` and **broadcasts** its stdout to every connected HTTP
//! client as a `multipart/x-mixed-replace` stream on a local port. Multiple sinks (panel preview,
//! floating window, dock widget) each open their own `<img>` on the same URL — one ffmpeg
//! decode/transcode fans out to all of them. The per-OS input + codec handling lives in
//! `video/native.rs`.
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

/// Drop a client that can't accept data within this window — prevents one stalled `<img>` (e.g. an
/// occluded/throttled view) from blocking the shared broadcast (and back-pressuring ffmpeg).
const CLIENT_WRITE_TIMEOUT: Duration = Duration::from_secs(2);

/// How long `start()` waits for the capture's first bytes before declaring it failed. Generous enough
/// for an HDMI dongle negotiating a mode (1–2 s is normal), short enough that a rejected mode reports
/// back while the user is still looking at "Starting…".
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(6);

/// The multipart HTTP response preamble sent once per client before the frame stream.
const HTTP_HEADERS: &[u8] = b"HTTP/1.1 200 OK\r\n\
    Content-Type: multipart/x-mixed-replace; boundary=ffmpeg\r\n\
    Cache-Control: no-cache\r\n\
    Connection: close\r\n\
    \r\n";

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

    /// Start the MJPEG server. Spawns ffmpeg to capture from a native device per `spec` and output
    /// MJPEG (stream-copied when the input is already MJPEG, transcoded otherwise), then broadcasts its
    /// stdout to all connected HTTP clients.
    ///
    /// Returns the port **only once the capture has actually delivered its first bytes** (up to
    /// `FIRST_FRAME_TIMEOUT`); otherwise everything is torn down again and the error carries ffmpeg's
    /// own first stderr line. Blocking by design — call it from an async command.
    pub fn start(&self, spec: &super::native::CaptureSpec) -> Result<u16, String> {
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

        // Build: [loglevel] + per-OS input (native) + output codec + mpjpeg mux (flushing per packet).
        // `-fflags nobuffer` (an input option, so it precedes the demuxer/-i) cuts input-side buffering
        // for lower latency and tighter pacing on live capture.
        let mut args: Vec<String> = vec![
            "-loglevel".into(),
            "error".into(),
            "-fflags".into(),
            "nobuffer".into(),
        ];
        args.extend(super::native::input_args(spec));
        if super::native::needs_transcode(&spec.codec) {
            // Raw / H.264 / auto → re-encode to MJPEG for the multipart sink.
            args.extend(["-c:v".into(), "mjpeg".into(), "-q:v".into(), "5".into()]);
        } else {
            // The camera already emits MJPEG: pass the packets straight through. No decode, no
            // encode, no colour conversion — cheaper than any hardware transcode could ever be, so
            // there is deliberately nothing to accelerate here.
            args.extend(["-c".into(), "copy".into()]);
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
        let clients: Arc<Mutex<Vec<TcpStream>>> = Arc::new(Mutex::new(Vec::new()));
        // First-frame signal + the first ffmpeg error line, so a failed capture can be reported to the
        // caller instead of only reaching the log.
        let (first_tx, first_rx) = std::sync::mpsc::channel();
        let first_err: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

        let accept = {
            let clients = clients.clone();
            let shutdown = shutdown.clone();
            thread::spawn(move || accept_loop(listener, clients, shutdown))
        };
        let reader = {
            let shutdown = shutdown.clone();
            thread::spawn(move || broadcast_loop(stdout, clients, shutdown, Some(first_tx)))
        };
        let stderr_thread = {
            let first_err = first_err.clone();
            thread::spawn(move || log_ffmpeg_stderr(stderr, first_err))
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
            let detail = first_err.lock().ok().and_then(|g| g.clone());
            let msg = match detail {
                Some(line) => format!("{reason}: {line}"),
                None => reason,
            };
            log::warn!("[video] native capture failed to start — {msg}");
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
                        "no video from the capture device within {} s",
                        FIRST_FRAME_TIMEOUT.as_secs()
                    ));
                }
            }
        }
    }
}

/// Accept clients and register them for broadcast. Each gets the multipart preamble, `TCP_NODELAY`
/// (no Nagle bunching on localhost), and blocking writes. Exits when `shutdown` is set.
fn accept_loop(listener: TcpListener, clients: Arc<Mutex<Vec<TcpStream>>>, shutdown: Arc<AtomicBool>) {
    loop {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_nodelay(true);
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_write_timeout(Some(CLIENT_WRITE_TIMEOUT));
                if stream.write_all(HTTP_HEADERS).is_ok() && stream.flush().is_ok() {
                    clients.lock().unwrap().push(stream);
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
/// (disconnected `<img>`). Always drains stdout even with no clients so ffmpeg never blocks on a full
/// pipe. Exits on EOF (ffmpeg died) or shutdown.
fn broadcast_loop(
    mut stdout: impl Read,
    clients: Arc<Mutex<Vec<TcpStream>>>,
    shutdown: Arc<AtomicBool>,
    mut first: Option<Sender<()>>,
) {
    let mut buf = [0u8; 65536];
    loop {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }
        let n = match stdout.read(&mut buf) {
            Ok(0) => {
                // stdout EOF = ffmpeg exited. If we didn't ask it to stop, that's the "goes black"
                // failure — surface it (the stderr logger prints ffmpeg's reason just above).
                if !shutdown.load(Ordering::SeqCst) {
                    log::warn!("[video] native MJPEG source ended unexpectedly (ffmpeg exited)");
                }
                break;
            }
            Ok(n) => n,
            Err(e) => {
                if !shutdown.load(Ordering::SeqCst) {
                    log::warn!("[video] native MJPEG read error: {e}");
                }
                break;
            }
        };
        // Signal the successful start exactly once, then drop the sender so `start()` learns about a
        // later EOF through the closed channel.
        if let Some(tx) = first.take() {
            let _ = tx.send(());
        }
        let mut list = clients.lock().unwrap();
        if list.is_empty() {
            continue; // stdout already drained above
        }
        list.retain_mut(|c| c.write_all(&buf[..n]).is_ok());
    }
}

/// Forward ffmpeg's stderr to the log. With `-loglevel error` these are genuine errors (device lost,
/// corrupt frame, codec failure) → tester-relevant, so they go at the default-visible `warn` level.
/// The **first** line is also recorded in `first_err` so a start-up failure can be reported to the UI
/// with ffmpeg's own wording (e.g. "Selected video size is not supported by the device").
fn log_ffmpeg_stderr(stderr: Option<ChildStderr>, first_err: Arc<Mutex<Option<String>>>) {
    let Some(stderr) = stderr else { return };
    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
        let line = line.trim();
        if !line.is_empty() {
            if let Ok(mut slot) = first_err.lock() {
                slot.get_or_insert_with(|| line.to_string());
            }
            log::warn!("[video][ffmpeg] {line}");
        }
    }
}
