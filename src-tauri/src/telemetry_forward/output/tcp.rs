// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! TCP server output sink — Kite hosts a TCP server; other GCS / monitoring apps connect *to* it and
//! receive the encoded telemetry stream. Frames are broadcast to all connected clients; dead clients are
//! dropped on the next write.
//!
//! Writes are **bounded** (see `WRITE_TIMEOUT`): every relay is written from the one dispatch thread, so
//! an unbounded write to a client that stopped reading would stall every other relay behind it.

use std::io::{ErrorKind, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use super::OutputSink;

/// Per-client write budget. A frame set is a few hundred bytes emitted at the pace of the attitude
/// update, so a healthy client drains it immediately — only one that has stopped reading (window
/// closed, process suspended) lets the socket buffer fill. Without a bound, `write_all` then blocks
/// forever and takes all relays with it, because `RelayHub::dispatch` writes them sequentially on a
/// single thread while holding the relay lock. Over budget the client is dropped: the timeout can
/// leave a half-written frame behind and a stream consumer cannot resynchronize from that anyway.
const WRITE_TIMEOUT: Duration = Duration::from_millis(200);

pub struct TcpSink {
    addr: String,
    clients: Arc<Mutex<Vec<TcpStream>>>,
    running: Arc<AtomicBool>,
}

impl TcpSink {
    /// Bind a TCP server on `0.0.0.0:<port>` (reachable on the LAN) and start accepting clients.
    pub fn open(port: u16) -> Result<Self, String> {
        let addr = format!("0.0.0.0:{port}");
        let listener = TcpListener::bind(&addr).map_err(|e| format!("TCP relay bind {addr} failed: {e}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("TCP relay set_nonblocking failed: {e}"))?;

        let clients: Arc<Mutex<Vec<TcpStream>>> = Arc::new(Mutex::new(Vec::new()));
        let running = Arc::new(AtomicBool::new(true));

        // Accept loop on a background thread (non-blocking poll so Drop can stop it promptly).
        let c = clients.clone();
        let r = running.clone();
        thread::spawn(move || {
            while r.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, peer)) => {
                        let _ = stream.set_nodelay(true);
                        // Windows hands the accepted socket the listener's non-blocking mode (Unix
                        // does not), which would make every full send buffer an instant WouldBlock —
                        // a client that pauses for a moment would be dropped with no grace at all.
                        // Blocking + the write budget below is the same behaviour on every platform.
                        let _ = stream.set_nonblocking(false);
                        if let Err(e) = stream.set_write_timeout(Some(WRITE_TIMEOUT)) {
                            log::warn!("[RELAY tcp] {peer}: write timeout could not be set ({e}) — writes stay unbounded");
                        }
                        log::info!("[RELAY tcp] client connected: {peer}");
                        c.lock().unwrap().push(stream);
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(50));
                    }
                    Err(e) => {
                        log::warn!("[RELAY tcp] accept error: {e}");
                        thread::sleep(Duration::from_millis(200));
                    }
                }
            }
        });

        Ok(Self { addr, clients, running })
    }
}

impl OutputSink for TcpSink {
    fn write(&mut self, data: &[u8]) -> Result<(), String> {
        // Broadcast to every client; retain only those that took the whole frame set within budget.
        self.clients.lock().unwrap().retain_mut(|s| {
            let result = s.write_all(data);
            if let Err(e) = result {
                let peer = s.peer_addr().map(|p| p.to_string()).unwrap_or_else(|_| "unknown".to_string());
                match e.kind() {
                    ErrorKind::WouldBlock | ErrorKind::TimedOut => log::warn!(
                        "[RELAY tcp] client {peer} did not read within {} ms — dropped",
                        WRITE_TIMEOUT.as_millis()
                    ),
                    _ => log::warn!("[RELAY tcp] client {peer} dropped: {e}"),
                }
                return false;
            }
            true
        });
        Ok(())
    }

    fn description(&self) -> String {
        format!("TCP({})", self.addr)
    }

    /// "Pending" while no client is connected — nothing is actually being sent yet.
    fn pending(&self) -> bool {
        self.clients.lock().unwrap().is_empty()
    }
}

impl Drop for TcpSink {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpStream;
    use std::time::Instant;

    /// Ask the OS for a free port, then release it — the sink binds it a moment later.
    fn free_port() -> u16 {
        let probe = TcpListener::bind("127.0.0.1:0").expect("probe bind");
        probe.local_addr().expect("probe addr").port()
    }

    /// A client that connects and never reads is the case that used to park the dispatch thread
    /// forever: once its receive buffer is full, an unbounded `write_all` blocks and every other
    /// relay waits behind it. With the write budget the sink drops that client and returns.
    #[test]
    fn a_client_that_never_reads_is_dropped_instead_of_blocking_the_sink() {
        let port = free_port();
        let mut sink = TcpSink::open(port).expect("sink bind");

        // Connect and then never read a byte.
        let client = TcpStream::connect(("127.0.0.1", port)).expect("client connect");

        // The accept loop runs on its own thread — wait for it to register the client.
        let deadline = Instant::now() + Duration::from_secs(5);
        while sink.pending() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!sink.pending(), "the sink never accepted the test client");

        // Push until the socket buffers are full and the budget trips. A healthy client would take
        // all of this without ever reaching the timeout.
        let payload = vec![0u8; 64 * 1024];
        let start = Instant::now();
        for _ in 0..256 {
            sink.write(&payload).expect("write never reports an error upward");
            if sink.pending() {
                break; // the client was dropped — what this test is about
            }
        }
        let elapsed = start.elapsed();

        assert!(sink.pending(), "the non-reading client was not dropped");
        assert!(
            elapsed < Duration::from_secs(20),
            "writing to a stalled client took {elapsed:?} — the budget did not bound it"
        );
        drop(client);
    }
}
