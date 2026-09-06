// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! UDP subscription server — Kite listens on UDP 27300; a consumer subscribes by sending any datagram
//! there and keeps receiving frames as long as it sends one at least every `SUBSCRIPTION_TTL`.
//!
//! Same shape as the TCP stream from the client's side (the client initiates, Kite needs no target
//! configured, several clients at once), with UDP's fire-and-forget delivery. The first datagram from
//! a new address is answered with the `hello` record so the client can confirm the subscription and
//! the schema; a subscriber that goes quiet simply ages out — no explicit unsubscribe.

use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// A subscriber is dropped when it has not sent anything for this long. Consumers send a keepalive
/// every few seconds (an empty datagram is enough).
pub const SUBSCRIPTION_TTL: Duration = Duration::from_secs(10);

pub struct UdpServer {
    addr: String,
    socket: UdpSocket,
    clients: Arc<Mutex<HashMap<SocketAddr, Instant>>>,
    running: Arc<AtomicBool>,
}

impl UdpServer {
    /// Bind `bind_addr:port` and start the receive loop that registers subscribers.
    pub fn open(bind_addr: &str, port: u16, hello: String) -> Result<Self, String> {
        let addr = format!("{bind_addr}:{port}");
        let socket = UdpSocket::bind(&addr).map_err(|e| format!("bind {addr}: {e}"))?;
        let _ = socket.set_broadcast(true);
        let rx = socket.try_clone().map_err(|e| format!("socket clone: {e}"))?;
        rx.set_read_timeout(Some(Duration::from_millis(200))).map_err(|e| format!("read timeout: {e}"))?;

        let clients: Arc<Mutex<HashMap<SocketAddr, Instant>>> = Arc::new(Mutex::new(HashMap::new()));
        let running = Arc::new(AtomicBool::new(true));
        let c = clients.clone();
        let r = running.clone();
        thread::spawn(move || {
            let mut buf = [0u8; 1500];
            while r.load(Ordering::Relaxed) {
                match rx.recv_from(&mut buf) {
                    Ok((_, peer)) => {
                        let is_new = c.lock().unwrap().insert(peer, Instant::now()).is_none();
                        if is_new {
                            log::info!("[telemetry-api udp] subscriber: {peer}");
                            let _ = rx.send_to(hello.as_bytes(), peer);
                        }
                    }
                    // Timeout (Windows: TimedOut, Unix: WouldBlock) → just poll `running` again.
                    Err(ref e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                    // Windows reports an ICMP port-unreachable of a previous send as a recv error.
                    Err(ref e) if e.kind() == std::io::ErrorKind::ConnectionReset => {}
                    Err(e) => {
                        log::warn!("[telemetry-api udp] recv error: {e}");
                        thread::sleep(Duration::from_millis(200));
                    }
                }
            }
        });
        Ok(Self { addr, socket, clients, running })
    }

    /// One datagram per live subscriber; subscribers past the TTL are forgotten first.
    pub fn broadcast(&self, data: &[u8]) {
        let now = Instant::now();
        let mut clients = self.clients.lock().unwrap();
        clients.retain(|peer, last| {
            let alive = now.duration_since(*last) < SUBSCRIPTION_TTL;
            if !alive {
                log::info!("[telemetry-api udp] subscriber {peer} timed out");
            }
            alive
        });
        for peer in clients.keys() {
            let _ = self.socket.send_to(data, peer);
        }
    }

    pub fn client_count(&self) -> usize {
        let now = Instant::now();
        self.clients.lock().unwrap().values().filter(|t| now.duration_since(**t) < SUBSCRIPTION_TTL).count()
    }

    pub fn addr(&self) -> &str {
        &self.addr
    }
}

impl Drop for UdpServer {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datagram_subscribes_and_gets_hello_then_frames() {
        let probe = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let srv = UdpServer::open("127.0.0.1", port, "{\"hello\":true}".into()).unwrap();
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        client.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        client.send_to(b"hi", ("127.0.0.1", port)).unwrap();
        let mut buf = [0u8; 256];
        let (n, _) = client.recv_from(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"{\"hello\":true}");
        for _ in 0..50 {
            if srv.client_count() == 1 { break; }
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(srv.client_count(), 1);
        srv.broadcast(b"{\"seq\":1}");
        let (n, _) = client.recv_from(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"{\"seq\":1}");
    }
}
