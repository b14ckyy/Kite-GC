// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! TCP server output sink — Kite hosts a TCP server; other GCS / monitoring apps connect *to* it and
//! receive the encoded telemetry stream. Frames are broadcast to all connected clients; dead clients are
//! dropped on the next write.

use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use super::OutputSink;

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
        // Broadcast to every client; retain only those that accepted the write.
        self.clients.lock().unwrap().retain_mut(|s| s.write_all(data).is_ok());
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
