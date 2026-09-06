// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Minimal HTTP/1.1 GET server for the snapshot routes — hand-rolled on `std::net`, the way
//! `video/mjpeg_server.rs` does it, so no HTTP crate enters the tree for two routes.
//!
//! Routes (anything else → 404):
//!   GET /api/v1/telemetry → the newest frame as `application/json` (503 before the first one)
//!   GET /api/v1/health    → `{ ok, schema, connected, protocol, clients, rateHz }`
//!
//! One thread per connection, `Connection: close`, bounded read + write timeouts so a client that
//! connects and goes silent never holds anything. CORS is wide open — a read-only local API that a
//! browser dashboard on the same machine should be able to poll.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const READ_TIMEOUT: Duration = Duration::from_secs(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// What the routes read — filled by the ticker, shared with every connection thread.
#[derive(Default)]
pub struct Shared {
    /// Newest frame, already serialized. `None` until the ticker produced one.
    pub last_frame: Option<String>,
    /// Health fields the ticker keeps current.
    pub health: String,
}

pub struct HttpServer {
    addr: String,
    running: Arc<AtomicBool>,
}

impl HttpServer {
    pub fn open(bind_addr: &str, port: u16, shared: Arc<Mutex<Shared>>) -> Result<Self, String> {
        let addr = format!("{bind_addr}:{port}");
        let listener = TcpListener::bind(&addr).map_err(|e| format!("bind {addr}: {e}"))?;
        listener.set_nonblocking(true).map_err(|e| format!("set_nonblocking: {e}"))?;
        let running = Arc::new(AtomicBool::new(true));
        let r = running.clone();
        thread::spawn(move || {
            while r.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let shared = shared.clone();
                        thread::spawn(move || handle(stream, &shared));
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(50)),
                    Err(e) => {
                        log::warn!("[telemetry-api http] accept error: {e}");
                        thread::sleep(Duration::from_millis(200));
                    }
                }
            }
        });
        Ok(Self { addr, running })
    }

    pub fn addr(&self) -> &str {
        &self.addr
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

fn handle(mut stream: TcpStream, shared: &Mutex<Shared>) {
    let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
    let _ = stream.set_nonblocking(false);

    // Request line + headers (discarded): we only route on method + path.
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    let mut header = String::new();
    while reader.read_line(&mut header).map(|n| n > 0).unwrap_or(false) {
        if header == "\r\n" || header == "\n" {
            break;
        }
        header.clear();
    }

    let (status, body): (&str, String) = match route(&request_line) {
        Route::Telemetry => match shared.lock().unwrap().last_frame.clone() {
            Some(f) => ("200 OK", f),
            None => ("503 Service Unavailable", "{\"error\":\"no telemetry frame yet\"}".into()),
        },
        Route::Health => ("200 OK", shared.lock().unwrap().health.clone()),
        Route::NotFound => ("404 Not Found", "{\"error\":\"not found\"}".into()),
        Route::BadMethod => ("405 Method Not Allowed", "{\"error\":\"GET only\"}".into()),
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

enum Route {
    Telemetry,
    Health,
    NotFound,
    BadMethod,
}

/// Route on the request line only. The query string is ignored so `?cachebust=…` style calls work.
fn route(request_line: &str) -> Route {
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");
    if method != "GET" {
        return Route::BadMethod;
    }
    let path = target.split('?').next().unwrap_or("");
    match path.trim_end_matches('/') {
        "/api/v1/telemetry" => Route::Telemetry,
        "/api/v1/health" => Route::Health,
        _ => Route::NotFound,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn get(port: u16, path: &str) -> String {
        let mut sock = TcpStream::connect(("127.0.0.1", port)).unwrap();
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        sock.write_all(format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes()).unwrap();
        let mut out = String::new();
        sock.read_to_string(&mut out).unwrap();
        out
    }

    #[test]
    fn routes_snapshot_health_and_404() {
        let probe = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let shared = Arc::new(Mutex::new(Shared { last_frame: None, health: "{\"ok\":true}".into() }));
        let _srv = HttpServer::open("127.0.0.1", port, shared.clone()).unwrap();
        thread::sleep(Duration::from_millis(50));

        let r = get(port, "/api/v1/telemetry");
        assert!(r.starts_with("HTTP/1.1 503"), "{r}");
        shared.lock().unwrap().last_frame = Some("{\"seq\":7}".into());
        let r = get(port, "/api/v1/telemetry?x=1");
        assert!(r.starts_with("HTTP/1.1 200") && r.ends_with("{\"seq\":7}"), "{r}");
        let r = get(port, "/api/v1/health");
        assert!(r.contains("Access-Control-Allow-Origin: *") && r.ends_with("{\"ok\":true}"), "{r}");
        let r = get(port, "/nope");
        assert!(r.starts_with("HTTP/1.1 404"), "{r}");
    }
}
