// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! RTSP → fragmented-MP4 loopback bridge (see docs/active/RTSP_VIDEO.md). One feed at a time: ffmpeg
//! re-muxes the RTSP stream to fragmented MP4 on stdout, and a tiny loopback HTTP server streams those
//! bytes to a plain `<video src="http://127.0.0.1:PORT/stream">` in the webview (which can't speak
//! RTSP natively). A fresh ffmpeg is spawned per HTTP connection so a reconnecting `<video>` always
//! receives the fMP4 init segment (ftyp+moov) first.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// Managed Tauri state: the current RTSP feed (at most one).
#[derive(Default)]
pub struct VideoBridge {
    inner: Mutex<Option<Active>>,
}

struct Active {
    port: u16,
    stop: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
}

impl VideoBridge {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start (or restart) the bridge for `url` with the given transport ("tcp" | "udp"). Returns the
    /// loopback stream URL for a `<video src>`.
    pub fn start(&self, url: String, transport: String) -> Result<String, String> {
        self.stop(); // tear down any previous feed first

        let ffmpeg = super::ffmpeg::find_ffmpeg()
            .ok_or("ffmpeg not found — download it in Settings or place it next to the app / on PATH.")?;
        // "auto" lets ffmpeg negotiate the transport (UDP→TCP fallback, like VLC) — most compatible
        // with quirky servers (e.g. obs-rtspserver, which 461s a forced SETUP). "tcp"/"udp" force it.
        let transport = match transport.as_str() {
            "udp" => "udp",
            "tcp" => "tcp",
            _ => "auto",
        }
        .to_string();

        let listener = TcpListener::bind(("127.0.0.1", 0))
            .map_err(|e| format!("Cannot bind loopback video server: {e}"))?;
        let port = listener
            .local_addr()
            .map_err(|e| format!("Loopback addr error: {e}"))?
            .port();
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("Loopback nonblocking error: {e}"))?;

        let stop = Arc::new(AtomicBool::new(false));
        let child: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
        eprintln!("RTSP bridge starting on 127.0.0.1:{port} ({transport})");

        {
            let stop = stop.clone();
            let child = child.clone();
            thread::spawn(move || serve_loop(listener, &ffmpeg, &url, &transport, &stop, &child));
        }

        *self.inner.lock().unwrap() = Some(Active { port, stop, child });
        Ok(format!("http://127.0.0.1:{port}/stream"))
    }

    /// Stop the current feed (kills ffmpeg + the accept loop). Idempotent.
    pub fn stop(&self) {
        if let Some(active) = self.inner.lock().unwrap().take() {
            active.stop.store(true, Ordering::SeqCst);
            kill_child(&active.child);
            eprintln!("RTSP bridge stopped (was on :{}).", active.port);
        }
    }
}

/// Accept loop (non-blocking poll so it can observe the stop flag). One connection at a time.
fn serve_loop(
    listener: TcpListener,
    ffmpeg: &Path,
    url: &str,
    transport: &str,
    stop: &Arc<AtomicBool>,
    child: &Arc<Mutex<Option<Child>>>,
) {
    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        match listener.accept() {
            Ok((sock, _)) => {
                let _ = sock.set_nonblocking(false);
                handle_conn(sock, ffmpeg, url, transport, stop, child);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                eprintln!("RTSP bridge accept error: {e}");
                break;
            }
        }
    }
    kill_child(child);
}

/// Serve one HTTP connection: spawn a fresh ffmpeg and pipe its fMP4 stdout to the socket.
fn handle_conn(
    mut sock: TcpStream,
    ffmpeg: &Path,
    url: &str,
    transport: &str,
    stop: &Arc<AtomicBool>,
    child: &Arc<Mutex<Option<Child>>>,
) {
    let _ = drain_request(&mut sock); // we ignore the request (single endpoint, no range support)

    // Fresh ffmpeg → this connection gets the fMP4 init segment. Replace any previous one.
    kill_child(child);
    let mut cmd = Command::new(ffmpeg);
    let mut args: Vec<&str> = Vec::new();
    if transport != "auto" {
        // Forced transport. Omitted in "auto" so ffmpeg negotiates (UDP→TCP) like VLC.
        args.extend_from_slice(&["-rtsp_transport", transport]);
    }
    args.extend_from_slice(&[
        "-fflags", "nobuffer+flush_packets",
        "-flags", "low_delay",
        "-avioflags", "direct",
        "-probesize", "100000",   // small probe → fast start (don't over-analyze a live feed)
        "-analyzeduration", "0",
        "-i", url,
        "-an",            // no audio (v1)
        "-c:v", "copy",   // no transcode (H.264 happy path; HEVC fallback is P3)
        "-f", "mp4",
        "-movflags", "frag_keyframe+empty_moov+default_base_moof",
        "-flush_packets", "1",    // push each packet to stdout immediately (no output buffering)
        "pipe:1",
    ]);
    cmd.args(&args);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    eprintln!("RTSP bridge: client connected, spawning ffmpeg for {url} ({transport})");
    let mut proc = match cmd.spawn() {
        Ok(p) => p,
        Err(e) => {
            let _ = write_http_error(&mut sock, &format!("ffmpeg spawn failed: {e}"));
            return;
        }
    };
    // Drain ffmpeg's stderr to the app log (so connection/codec failures are visible).
    if let Some(err) = proc.stderr.take() {
        thread::spawn(move || {
            use std::io::BufRead;
            let reader = std::io::BufReader::new(err);
            for line in reader.lines().map_while(Result::ok) {
                eprintln!("ffmpeg: {line}");
            }
        });
    }
    let mut out = match proc.stdout.take() {
        Some(o) => o,
        None => {
            let _ = proc.kill();
            return;
        }
    };
    *child.lock().unwrap() = Some(proc);

    let mut total: u64 = 0;

    // Streaming response: no Content-Length (live), CORS * so the webview origin may fetch loopback.
    let headers = "HTTP/1.1 200 OK\r\n\
        Content-Type: video/mp4\r\n\
        Cache-Control: no-cache, no-store\r\n\
        Access-Control-Allow-Origin: *\r\n\
        Connection: close\r\n\r\n";
    if sock.write_all(headers.as_bytes()).is_err() {
        kill_child(child);
        return;
    }

    let mut buf = [0u8; 32 * 1024];
    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        match out.read(&mut buf) {
            Ok(0) => break,                                       // ffmpeg exited
            Ok(n) => {
                if sock.write_all(&buf[..n]).is_err() {
                    break; // client (the <video>) went away
                }
                total += n as u64;
            }
            Err(_) => break,
        }
    }
    eprintln!("RTSP bridge: connection ended after {total} bytes streamed");
    kill_child(child); // feed ended → drop ffmpeg; a reconnect restarts it
}

/// Read + discard the HTTP request headers (bounded + timed out so a silent client can't hang us).
fn drain_request(sock: &mut TcpStream) -> std::io::Result<()> {
    sock.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut req = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        let n = sock.read(&mut buf)?;
        if n == 0 {
            break;
        }
        req.extend_from_slice(&buf[..n]);
        if req.windows(4).any(|w| w == b"\r\n\r\n") || req.len() > 8192 {
            break;
        }
    }
    sock.set_read_timeout(None)?;
    let text = String::from_utf8_lossy(&req);
    let first = text.lines().next().unwrap_or("");
    let range = text.lines().find(|l| l.to_ascii_lowercase().starts_with("range:")).unwrap_or("(no range)");
    eprintln!("RTSP bridge: HTTP request: {first} | {range}");
    Ok(())
}

fn write_http_error(sock: &mut TcpStream, msg: &str) -> std::io::Result<()> {
    let resp = format!(
        "HTTP/1.1 500 Internal Server Error\r\n\
         Content-Type: text/plain\r\n\
         Access-Control-Allow-Origin: *\r\n\
         Connection: close\r\n\r\n{msg}"
    );
    sock.write_all(resp.as_bytes())
}

fn kill_child(child: &Arc<Mutex<Option<Child>>>) {
    if let Some(mut c) = child.lock().unwrap().take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}
