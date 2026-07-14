// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! HTTP output sink — Kite hosts a small read-only HTTP server that serves the encoded telemetry as a
//! snapshot (pull) and as a live SSE stream (push). Pairs with the `json` encoder; external services
//! consume it without having to parse a binary FC protocol.
//!
//! Hand-rolled on `std::net::TcpListener`, like `video/mjpeg_server.rs` — `tokio` is compiled without
//! `net`/`rt-multi-thread` in the release build, so axum/hyper aren't available to us.
//!
//! Server-Sent Events rather than WebSocket: the stream is one-way and read-only, SSE is plain HTTP/1.1
//! text (no SHA-1 handshake, no frame masking), and it's consumable by `EventSource` in a browser and by
//! any HTTP client.
//!
//! Routes:
//!   GET /api/v1/telemetry  → 200 application/json, the most recent frame, then close
//!   GET /api/v1/stream     → 200 text/event-stream, frames pushed as they're encoded
//!   GET /api/v1/health     → 200 application/json, liveness + mission id
//!
//! Binds loopback by default. Unlike a tracker relay (which exists to reach the LAN), a telemetry API
//! shouldn't be silently readable by everyone on a field network — LAN exposure is an explicit opt-in.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use super::OutputSink;

/// Drop a client that can't accept a frame within this window, so one stalled consumer can't block the
/// broadcast — `write()` runs on the Tauri event-listener thread that drives every relay's dispatch.
const CLIENT_WRITE_TIMEOUT: Duration = Duration::from_secs(2);

/// Give up on a client that connects but never sends a request line.
const CLIENT_READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Cap the request we're willing to read. We only need the request line; anything larger is not a client
/// we want to serve.
const MAX_REQUEST_BYTES: usize = 2048;

/// Read-only API with no credentials, so a wildcard origin is safe and lets a browser-based consumer
/// fetch it directly.
const CORS: &str = "Access-Control-Allow-Origin: *\r\n";

pub struct HttpSink {
    addr: String,
    /// Most recently encoded frame, served by `GET /api/v1/telemetry`. `None` until the first frame.
    snapshot: Arc<Mutex<Option<Vec<u8>>>>,
    /// Clients currently subscribed to `GET /api/v1/stream`.
    clients: Arc<Mutex<Vec<TcpStream>>>,
    running: Arc<AtomicBool>,
}

impl HttpSink {
    /// Bind the API server. `lan` exposes it on `0.0.0.0` instead of loopback.
    pub fn open(port: u16, lan: bool, mission_id: String) -> Result<Self, String> {
        let host = if lan { "0.0.0.0" } else { "127.0.0.1" };
        let addr = format!("{host}:{port}");
        let listener =
            TcpListener::bind(&addr).map_err(|e| format!("HTTP relay bind {addr} failed: {e}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("HTTP relay set_nonblocking failed: {e}"))?;

        let snapshot: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));
        let clients: Arc<Mutex<Vec<TcpStream>>> = Arc::new(Mutex::new(Vec::new()));
        let running = Arc::new(AtomicBool::new(true));

        // Accept loop on a background thread (non-blocking poll so Drop can stop it promptly). Each
        // connection is served on its own thread so a client that connects and then dawdles can't hold up
        // the accept loop for its whole read timeout. Request volume here is a handful of clients, so a
        // thread per connection is cheap.
        let snap = snapshot.clone();
        let cl = clients.clone();
        let r = running.clone();
        thread::spawn(move || {
            while r.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, peer)) => {
                        let _ = stream.set_nodelay(true);
                        let snap = snap.clone();
                        let cl = cl.clone();
                        let mission_id = mission_id.clone();
                        thread::spawn(move || serve(stream, peer.to_string(), &snap, &cl, &mission_id));
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(50));
                    }
                    Err(e) => {
                        log::warn!("[RELAY http] accept error: {e}");
                        thread::sleep(Duration::from_millis(200));
                    }
                }
            }
        });

        Ok(Self { addr, snapshot, clients, running })
    }
}

impl OutputSink for HttpSink {
    /// Cache the frame for the snapshot route, then push it to every SSE subscriber. Clients that error
    /// or time out are dropped. Never fails: a relay with no consumers is idle, not broken.
    fn write(&mut self, data: &[u8]) -> Result<(), String> {
        *self.snapshot.lock().unwrap() = Some(data.to_vec());

        let mut clients = self.clients.lock().unwrap();
        if clients.is_empty() {
            return Ok(());
        }
        // SSE framing. The JSON encoder emits compact, newline-terminated JSON — one line, so it maps to
        // a single `data:` field. Strip its trailing newline; the blank line is the record terminator.
        let payload = data.strip_suffix(b"\n").unwrap_or(data);
        let mut frame = Vec::with_capacity(payload.len() + 8);
        frame.extend_from_slice(b"data: ");
        frame.extend_from_slice(payload);
        frame.extend_from_slice(b"\n\n");

        clients.retain_mut(|c| c.write_all(&frame).is_ok());
        Ok(())
    }

    fn description(&self) -> String {
        format!("HTTP({})", self.addr)
    }

    /// "Pending" while nobody is streaming — the server is up but nothing is being pushed.
    fn pending(&self) -> bool {
        self.clients.lock().unwrap().is_empty()
    }
}

impl Drop for HttpSink {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

/// Handle one accepted connection: parse the request line and either answer and close, or (for the
/// stream route) hand the socket to the broadcast list.
fn serve(
    mut stream: TcpStream,
    peer: String,
    snapshot: &Arc<Mutex<Option<Vec<u8>>>>,
    clients: &Arc<Mutex<Vec<TcpStream>>>,
    mission_id: &str,
) {
    // The listener is non-blocking, and accepted sockets inherit that on some platforms — force blocking
    // with explicit timeouts so a silent client can't wedge the accept loop.
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(CLIENT_READ_TIMEOUT));
    let _ = stream.set_write_timeout(Some(CLIENT_WRITE_TIMEOUT));

    let Some((method, path)) = read_request_line(&mut stream) else {
        return;
    };

    if method == "OPTIONS" {
        let _ = stream.write_all(
            format!("HTTP/1.1 204 No Content\r\n{CORS}Access-Control-Allow-Methods: GET, OPTIONS\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes(),
        );
        return;
    }
    if method != "GET" {
        respond(&mut stream, "405 Method Not Allowed", "application/json", br#"{"error":"method not allowed"}"#);
        return;
    }

    // Ignore any query string — none of the routes take parameters.
    let route = path.split('?').next().unwrap_or("");
    match route {
        "/api/v1/telemetry" => {
            // Copy the frame out and release the lock *before* writing: a response can block for up to
            // CLIENT_WRITE_TIMEOUT, and `write()` needs this same lock on the relay dispatch thread.
            // Holding it across the write would stall every relay behind one slow HTTP client.
            let frame = snapshot.lock().unwrap().clone();
            match frame {
                Some(f) => respond(&mut stream, "200 OK", "application/json", &f),
                // Server is up but no telemetry has arrived yet — a real state, not an error.
                None => respond(&mut stream, "503 Service Unavailable", "application/json", br#"{"error":"no telemetry yet"}"#),
            }
        }
        "/api/v1/health" => {
            // Same reasoning: snapshot the values, drop the locks, then write.
            let has_data = snapshot.lock().unwrap().is_some();
            let stream_clients = clients.lock().unwrap().len();
            let body = serde_json::json!({
                "ok": true,
                "schema": 1,
                "missionId": mission_id,
                "hasData": has_data,
                "streamClients": stream_clients,
            })
            .to_string();
            respond(&mut stream, "200 OK", "application/json", body.as_bytes());
        }
        "/api/v1/stream" => {
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n{CORS}Cache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n"
            );
            if stream.write_all(headers.as_bytes()).is_ok() && stream.flush().is_ok() {
                log::info!("[RELAY http] stream client connected: {peer}");
                clients.lock().unwrap().push(stream);
            }
        }
        _ => respond(&mut stream, "404 Not Found", "application/json", br#"{"error":"not found"}"#),
    }
}

/// Write a complete response and let the socket close on drop.
fn respond(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) {
    let headers = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\n{CORS}Cache-Control: no-cache\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(headers.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

/// Read until the end of the request headers (or the cap) and return the request line's method + path.
fn read_request_line(stream: &mut TcpStream) -> Option<(String, String)> {
    let mut buf = vec![0u8; MAX_REQUEST_BYTES];
    let mut len = 0;
    while len < buf.len() {
        match stream.read(&mut buf[len..]) {
            Ok(0) => break,
            Ok(n) => {
                len += n;
                if buf[..len].windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    if len == 0 {
        return None;
    }
    let head = String::from_utf8_lossy(&buf[..len]);
    let mut parts = head.lines().next()?.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    Some((method, path))
}
