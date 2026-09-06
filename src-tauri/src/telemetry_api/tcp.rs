// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! NDJSON stream server — Kite listens, consumers connect and receive one JSON frame per line.
//!
//! Same shape as the relay's `TcpSink` (accept loop on its own thread, broadcast to every client,
//! bounded writes that drop a client which stopped reading), plus two things the API needs: a `hello`
//! line sent to each client right after accept, and a bind address that defaults to loopback.

use std::io::{ErrorKind, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// Per-client write budget. A frame is a couple of kB at ≤ 10 Hz, so a healthy client drains it at
/// once; only a client that stopped reading fills the socket buffer. The ticker writes every client
/// sequentially, so an unbounded write would stall the whole API behind one dead consumer.
const WRITE_TIMEOUT: Duration = Duration::from_millis(200);

pub struct TcpServer {
    addr: String,
    clients: Arc<Mutex<Vec<TcpStream>>>,
    running: Arc<AtomicBool>,
}

impl TcpServer {
    /// Bind `bind_addr:port` and start accepting. `hello` is written to every client on connect.
    pub fn open(bind_addr: &str, port: u16, hello: String) -> Result<Self, String> {
        let addr = format!("{bind_addr}:{port}");
        let listener = TcpListener::bind(&addr).map_err(|e| format!("bind {addr}: {e}"))?;
        listener.set_nonblocking(true).map_err(|e| format!("set_nonblocking: {e}"))?;

        let clients: Arc<Mutex<Vec<TcpStream>>> = Arc::new(Mutex::new(Vec::new()));
        let running = Arc::new(AtomicBool::new(true));
        let c = clients.clone();
        let r = running.clone();
        thread::spawn(move || {
            while r.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, peer)) => {
                        let _ = stream.set_nodelay(true);
                        // Windows hands the accepted socket the listener's non-blocking mode; blocking +
                        // the write budget is the same behaviour on every platform.
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
                        if let Err(e) = stream.write_all(hello.as_bytes()) {
                            log::warn!("[telemetry-api tcp] {peer}: hello failed, dropped: {e}");
                            continue;
                        }
                        log::info!("[telemetry-api tcp] client connected: {peer}");
                        c.lock().unwrap().push(stream);
                    }
                    Err(ref e) if e.kind() == ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(50)),
                    Err(e) => {
                        log::warn!("[telemetry-api tcp] accept error: {e}");
                        thread::sleep(Duration::from_millis(200));
                    }
                }
            }
        });
        Ok(Self { addr, clients, running })
    }

    /// Broadcast one line to every client; a client that does not take it within budget is dropped.
    pub fn broadcast(&self, line: &[u8]) {
        self.clients.lock().unwrap().retain_mut(|s| {
            if let Err(e) = s.write_all(line) {
                let peer = s.peer_addr().map(|p| p.to_string()).unwrap_or_else(|_| "unknown".into());
                match e.kind() {
                    ErrorKind::WouldBlock | ErrorKind::TimedOut => log::warn!(
                        "[telemetry-api tcp] client {peer} did not read within {} ms — dropped",
                        WRITE_TIMEOUT.as_millis()
                    ),
                    _ => log::info!("[telemetry-api tcp] client {peer} gone: {e}"),
                }
                return false;
            }
            true
        });
    }

    pub fn client_count(&self) -> usize {
        self.clients.lock().unwrap().len()
    }

    pub fn addr(&self) -> &str {
        &self.addr
    }
}

impl Drop for TcpServer {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        // Dropping the streams closes every client; the accept thread exits on its next poll.
        self.clients.lock().unwrap().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};

    #[test]
    fn client_gets_hello_then_frames() {
        // Port 0 = any free port; read it back from the bound address.
        let probe = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let srv = TcpServer::open("127.0.0.1", port, "{\"hello\":true}\n".into()).unwrap();
        let sock = TcpStream::connect(("127.0.0.1", port)).unwrap();
        sock.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut rd = BufReader::new(sock);
        let mut line = String::new();
        rd.read_line(&mut line).unwrap();
        assert_eq!(line, "{\"hello\":true}\n");
        // Wait for the accept thread to register the client, then broadcast.
        for _ in 0..50 {
            if srv.client_count() == 1 { break; }
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(srv.client_count(), 1);
        srv.broadcast(b"{\"seq\":1}\n");
        line.clear();
        rd.read_line(&mut line).unwrap();
        assert_eq!(line, "{\"seq\":1}\n");
    }
}
