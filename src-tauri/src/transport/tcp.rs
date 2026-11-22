// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// TCP Transport
// Connects to a flight controller via TCP socket (e.g. Wi-Fi bridges, SITL).
// Implements ByteTransport for protocol-agnostic byte-level I/O.

use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use super::{is_no_network_path, ByteTransport, TransportError};

/// TCP connection timeout
const CONNECT_TIMEOUT_MS: u64 = 5000;
/// Read timeout for individual read calls. Kept short: the MAVLink handler loop interleaves reads
/// with outgoing writes (a queued command — e.g. a mission item to upload — is only serviced once the
/// current blocking read returns), so this value bounds the latency of every GCS→FC message. A long
/// timeout stalled mission upload (each item delayed up to a second → ArduPilot cancels the transfer
/// with MAV_MISSION_OPERATION_CANCELLED); 50 ms makes uploads snappy without busy-spinning (the read
/// still sleeps when no data is available).
const READ_TIMEOUT_MS: u64 = 50;

/// Reconnect pacing after a lost stream. An attempt runs inside `read_bytes` and blocks it for up to
/// `attempt`, so that one stays short.
#[derive(Clone, Copy)]
struct Reconnect {
    first: Duration,
    cap: Duration,
    attempt: Duration,
}

const RECONNECT: Reconnect = Reconnect {
    first: Duration::from_millis(500),
    cap: Duration::from_secs(5),
    attempt: Duration::from_secs(1),
};

/// A TCP link to a flight controller. A lost stream is re-established underneath (the layers above see
/// a stall). `Disconnected` only when the OS reports no network path, when the stream ends before it
/// delivered anything (a handshake must fail fast), or without `reconnect`.
pub struct TcpTransport {
    address: String,
    target: SocketAddr,
    /// `None` while the link is down and being re-established.
    stream: Option<TcpStream>,
    read_timeout: Duration,
    timing: Reconnect,
    backoff: Duration,
    next_attempt: Instant,
    /// A reconnected stream that has not delivered a byte yet: losing it again keeps the backoff growing
    /// and logs at debug, so a server that accepts and drops at once cannot flood the log.
    unproven: bool,
    restarted: bool,
    /// False for callers that reopen the link themselves and show its state (radar sources).
    reconnect: bool,
    received: bool,
    ended: bool,
}

fn configure(stream: &TcpStream, read_timeout: Duration) -> Result<(), String> {
    stream
        .set_read_timeout(Some(read_timeout))
        .map_err(|e| format!("Failed to set read timeout: {}", e))?;
    stream
        .set_nodelay(true)
        .map_err(|e| format!("Failed to set TCP_NODELAY: {}", e))
}

impl TcpTransport {
    /// Connect to a flight controller via TCP
    pub fn connect(host: &str, port: u16) -> Result<Self, String> {
        Self::connect_with(host, port, RECONNECT)
    }

    /// Like `connect`, but a lost stream ends the link.
    pub fn connect_once(host: &str, port: u16) -> Result<Self, String> {
        let mut link = Self::connect_with(host, port, RECONNECT)?;
        link.reconnect = false;
        Ok(link)
    }

    fn connect_with(host: &str, port: u16, timing: Reconnect) -> Result<Self, String> {
        let addr = format!("{}:{}", host, port);
        let target: SocketAddr = addr.parse().map_err(|e| format!("Invalid address '{}': {}", addr, e))?;
        let stream = TcpStream::connect_timeout(&target, Duration::from_millis(CONNECT_TIMEOUT_MS))
            .map_err(|e| format!("TCP connect to {} failed: {}", addr, e))?;
        let read_timeout = Duration::from_millis(READ_TIMEOUT_MS);
        configure(&stream, read_timeout)?;

        Ok(Self {
            address: addr,
            target,
            stream: Some(stream),
            read_timeout,
            timing,
            backoff: timing.first,
            next_attempt: Instant::now(),
            unproven: false,
            restarted: false,
            reconnect: true,
            received: false,
            ended: false,
        })
    }

    /// Drop the stream after a loss. Reported to the caller as a stall, or as `Disconnected` when the
    /// link ends (see the struct).
    fn lost(&mut self, reason: &str, no_path: bool) -> Result<(), TransportError> {
        self.stream = None;
        if no_path {
            return Err(self.give_up(&format!("no network path ({})", reason)));
        }
        if !self.reconnect || !self.received {
            return Err(self.give_up(reason));
        }
        if self.unproven {
            self.backoff = (self.backoff * 2).min(self.timing.cap);
            log::debug!("TCP link to {} lost again ({}) — retry in {:?}", self.address, reason, self.backoff);
        } else {
            log::warn!("TCP link to {} lost ({}) — reconnecting", self.address, reason);
        }
        self.next_attempt = Instant::now() + self.backoff;
        Ok(())
    }

    fn give_up(&mut self, reason: &str) -> TransportError {
        self.stream = None;
        if !self.ended {
            self.ended = true;
            log::warn!("TCP link to {}: {} — closing the link", self.address, reason);
        }
        TransportError::Disconnected
    }

    fn try_reconnect(&mut self) {
        let failure = match TcpStream::connect_timeout(&self.target, self.timing.attempt) {
            Ok(stream) => match configure(&stream, self.read_timeout) {
                Ok(()) => {
                    log::info!("TCP link to {} re-established", self.address);
                    self.stream = Some(stream);
                    self.unproven = true;
                    self.restarted = true;
                    return;
                }
                Err(e) => e,
            },
            Err(e) if is_no_network_path(&e) => {
                self.give_up(&format!("no network path (reconnect: {})", e));
                return;
            }
            Err(e) => e.to_string(),
        };
        self.backoff = (self.backoff * 2).min(self.timing.cap);
        self.next_attempt = Instant::now() + self.backoff;
        log::debug!("TCP reconnect to {} failed: {} — retry in {:?}", self.address, failure, self.backoff);
    }
}

impl ByteTransport for TcpTransport {
    fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        if self.ended {
            return Err(TransportError::Disconnected);
        }
        if self.stream.is_none() {
            let start = Instant::now();
            if start >= self.next_attempt {
                self.try_reconnect();
            }
            if self.ended {
                return Err(TransportError::Disconnected);
            }
            if self.stream.is_none() {
                // Pace the caller like a read that timed out — it loops on `Ok(0)` without waiting.
                std::thread::sleep(self.read_timeout.saturating_sub(start.elapsed()));
                return Ok(0);
            }
        }
        let Some(stream) = self.stream.as_mut() else { return Ok(0) };
        match stream.read(buf) {
            Ok(0) => self.lost("closed by the remote end", false).map(|()| 0),
            Ok(n) => {
                self.received = true;
                if self.unproven {
                    self.unproven = false;
                    self.backoff = self.timing.first;
                }
                Ok(n)
            }
            Err(e) if matches!(e.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock | ErrorKind::Interrupted) => Ok(0),
            Err(e) => self.lost(&e.to_string(), is_no_network_path(&e)).map(|()| 0),
        }
    }

    /// While the link is down the data is dropped, like on any stalled link.
    fn write_bytes(&mut self, data: &[u8]) -> Result<(), TransportError> {
        if self.ended {
            return Err(TransportError::Disconnected);
        }
        let Some(stream) = self.stream.as_mut() else { return Ok(()) };
        match stream.write_all(data).and_then(|()| stream.flush()) {
            Ok(()) => Ok(()),
            Err(e) => self.lost(&format!("write: {}", e), is_no_network_path(&e)),
        }
    }

    fn set_read_timeout(&mut self, timeout: Duration) {
        self.read_timeout = timeout;
        if let Some(stream) = &self.stream {
            let _ = stream.set_read_timeout(Some(timeout));
        }
    }

    fn take_stream_restart(&mut self) -> bool {
        std::mem::take(&mut self.restarted)
    }

    fn description(&self) -> String {
        format!("TCP({})", self.address)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::mpsc;

    const FAST: Reconnect = Reconnect {
        first: Duration::from_millis(20),
        cap: Duration::from_millis(100),
        attempt: Duration::from_millis(500),
    };

    #[test]
    fn a_closed_stream_is_a_stall_and_reconnects_underneath() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut link = TcpTransport::connect_with("127.0.0.1", port, FAST).unwrap();

        let (done_tx, done_rx) = mpsc::channel::<()>();
        let server = std::thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            first.write_all(b"hi").unwrap();
            drop(first);
            let (mut second, _) = listener.accept().unwrap();
            second.write_all(b"after").unwrap();
            let _ = done_rx.recv_timeout(Duration::from_secs(3));
        });

        let mut buf = [0u8; 16];
        let mut got = Vec::new();
        let mut saw_down = false;
        let mut restarted = false;
        let deadline = Instant::now() + Duration::from_secs(2);
        while got.len() < 7 && Instant::now() < deadline {
            let n = link.read_bytes(&mut buf).expect("a lost stream must read as a stall");
            got.extend_from_slice(&buf[..n]);
            saw_down |= link.stream.is_none();
            restarted |= link.take_stream_restart();
            link.write_bytes(b"ping").expect("writes while down are dropped, not failed");
        }
        let _ = done_tx.send(());
        server.join().unwrap();

        assert!(saw_down, "the closed stream was never noticed");
        assert!(restarted, "the reconnect did not report a stream restart");
        assert_eq!(got, b"hiafter");
        assert!(!link.take_stream_restart(), "the restart is reported once");
    }

    /// Closes each accepted stream, the first one after sending `hello`.
    fn closing_server(hello: &'static [u8]) -> (u16, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.write_all(hello).unwrap();
        });
        (port, server)
    }

    fn read_until_error(link: &mut TcpTransport) -> (Vec<u8>, bool) {
        let mut buf = [0u8; 16];
        let mut got = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            match link.read_bytes(&mut buf) {
                Ok(n) => got.extend_from_slice(&buf[..n]),
                Err(_) => return (got, true),
            }
        }
        (got, false)
    }

    #[test]
    fn a_stream_closed_before_any_data_ends_the_link() {
        let (port, server) = closing_server(b"");
        let mut link = TcpTransport::connect_with("127.0.0.1", port, FAST).unwrap();
        assert_eq!(read_until_error(&mut link), (Vec::new(), true));
        server.join().unwrap();
    }

    #[test]
    fn without_reconnect_a_lost_stream_ends_the_link() {
        let (port, server) = closing_server(b"hi");
        let mut link = TcpTransport::connect_with("127.0.0.1", port, FAST).unwrap();
        link.reconnect = false;
        assert_eq!(read_until_error(&mut link), (b"hi".to_vec(), true));
        server.join().unwrap();
    }

    #[test]
    fn no_network_path_ends_the_link_for_good() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut link = TcpTransport::connect_with("127.0.0.1", port, FAST).unwrap();
        assert!(matches!(link.lost("test", true), Err(TransportError::Disconnected)));
        let mut buf = [0u8; 4];
        for _ in 0..3 {
            assert!(matches!(link.read_bytes(&mut buf), Err(TransportError::Disconnected)));
            assert!(matches!(link.write_bytes(b"x"), Err(TransportError::Disconnected)));
        }
    }
}
