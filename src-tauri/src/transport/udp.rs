// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// UDP Transport
// Connects to a flight controller via UDP socket (e.g. MAVLink radios, Wi-Fi telemetry).
// Note: UDP is connectionless — we bind locally and send/receive to/from a remote address.
// Implements ByteTransport for protocol-agnostic byte-level I/O.
//
// Peer-learning + fixed-port bind (vs. the naive bind(:0)+connect() client model):
//   Wi-Fi telemetry bridges (mavesp8266 / DroneBridge / typical ESP APs at 192.168.4.1) push
//   telemetry to a *fixed* port (usually 14550) — by unicast to a learned client, by broadcast, or
//   from a source port that differs from the one they listen on. An ephemeral local port + a
//   connect()-restricted peer (the old behaviour) silently drops all of that: we'd never receive a
//   single byte (handshake "read 0 bytes"). Mission Planner's default "UDP" mode is a *listener* on
//   14550 for exactly this reason.
//   So we (a) bind to the same port number the user targeted (fall back to ephemeral only if that
//   port is busy) so datagrams sent to that well-known port reach us, and (b) use recv_from/send_to
//   instead of connect(): we seed the peer with the configured host:port (so the initial GCS
//   HEARTBEAT goes out and wakes ArduPilot/the bridge) and then re-target to whatever source actually
//   sends us data. This covers listen-mode bridges, client-mode SITL, and broadcast setups alike.

use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::Duration;

use super::{ByteTransport, TransportError};

/// Read timeout for individual recv calls. Short on purpose — bounds the latency the MAVLink handler
/// loop adds to outgoing commands (it services a queued write only once the current blocking recv
/// returns). See the TCP transport for the full rationale.
const READ_TIMEOUT_MS: u64 = 50;

/// An active UDP transport to a flight controller
pub struct UdpTransport {
    /// Configured target (host:port) — initial send destination and the description label.
    configured: String,
    /// Current send target. Starts as `configured`, then re-targets to the source of received data
    /// (peer learning) so we reply to wherever the FC/bridge actually speaks from.
    peer: SocketAddr,
    socket: UdpSocket,
}

impl UdpTransport {
    /// Create a UDP transport targeting `host:port`.
    ///
    /// Binds the local socket to `0.0.0.0:port` so we receive datagrams the FC/bridge sends to that
    /// well-known port; falls back to an ephemeral port if it's already in use. Does not `connect()`
    /// — the peer is learned from incoming traffic (see module docs).
    pub fn connect(host: &str, port: u16) -> Result<Self, String> {
        let addr = format!("{}:{}", host, port);

        // Resolve the configured target so the first send (the GCS HEARTBEAT) has a destination.
        let peer = addr
            .to_socket_addrs()
            .map_err(|e| format!("UDP resolve {} failed: {}", addr, e))?
            .next()
            .ok_or_else(|| format!("UDP resolve {} returned no address", addr))?;

        // Prefer binding to the same port number the user targeted (listener-friendly). If that port
        // is busy, fall back to an ephemeral one so client-style links still work.
        let socket = match UdpSocket::bind(("0.0.0.0", port)) {
            Ok(s) => {
                log::info!("UDP bound to local port {} (listening for {})", port, addr);
                s
            }
            Err(e) => {
                log::warn!(
                    "UDP bind to local port {} failed ({}) — falling back to an ephemeral port",
                    port, e
                );
                UdpSocket::bind("0.0.0.0:0")
                    .map_err(|e| format!("UDP bind failed: {}", e))?
            }
        };

        socket
            .set_read_timeout(Some(Duration::from_millis(READ_TIMEOUT_MS)))
            .map_err(|e| format!("Failed to set read timeout: {}", e))?;

        Ok(Self {
            configured: addr,
            peer,
            socket,
        })
    }
}

impl ByteTransport for UdpTransport {
    fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        match self.socket.recv_from(buf) {
            Ok((n, src)) => {
                // Peer learning: re-target sends to wherever data actually arrives from. Ignore our
                // own loopback echoes (n == 0 never happens for a real datagram).
                if src != self.peer {
                    log::debug!("UDP peer learned: {} (was {})", src, self.peer);
                    self.peer = src;
                }
                Ok(n)
            }
            Err(ref e)
                if e.kind() == std::io::ErrorKind::TimedOut
                    || e.kind() == std::io::ErrorKind::WouldBlock =>
            {
                Ok(0)
            }
            Err(e) => Err(TransportError::from(e)),
        }
    }

    fn write_bytes(&mut self, data: &[u8]) -> Result<(), TransportError> {
        self.socket
            .send_to(data, self.peer)
            .map_err(|e| TransportError::Io(format!("UDP send to {} failed: {}", self.peer, e)))?;
        Ok(())
    }

    fn set_read_timeout(&mut self, timeout: Duration) {
        let _ = self.socket.set_read_timeout(Some(timeout));
    }

    fn description(&self) -> String {
        format!("UDP({})", self.configured)
    }
}
