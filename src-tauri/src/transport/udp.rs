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

use std::collections::HashMap;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};

use super::{ByteTransport, TransportError};

/// Read timeout for individual recv calls. Short on purpose — bounds the latency the MAVLink handler
/// loop adds to outgoing commands (it services a queued write only once the current blocking recv
/// returns). See the TCP transport for the full rationale.
const READ_TIMEOUT_MS: u64 = 50;

/// An active UDP transport to a flight controller
pub struct UdpTransport {
    /// Configured target (host:port) — initial send destination and the description label.
    configured: String,
    /// Set when the well-known local port was busy: what we bound instead and why it matters. Surfaced
    /// with a handshake failure so "no HEARTBEAT" points at the other listener, not at the vehicle.
    bind_note: Option<String>,
    /// Configured send target — where the initial GCS HEARTBEAT goes, and the fallback while no peer
    /// has spoken yet.
    peer: SocketAddr,
    /// Every source that has sent us data recently, with its last-seen time (peer learning). Outgoing
    /// frames go to ALL of them: several vehicles pushing to our listening port each speak from their
    /// own socket (multi-vehicle SITL / several telemetry bridges), and a command for sysid 3 sent
    /// only to "whoever spoke last" would mostly land on the wrong aircraft and be ignored. Vehicles
    /// drop frames not addressed to them, so the fan-out is harmless; peers silent for PEER_TTL age out.
    peers: HashMap<SocketAddr, Instant>,
    socket: UdpSocket,
}

/// A learned peer that has been silent this long is dropped from the send fan-out.
const PEER_TTL: Duration = Duration::from_secs(10);

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
        // is busy (another ground station listening there — Mission Planner on 14550/14560 is the
        // typical case), fall back to a STABLE alternate port derived from the target, and only then
        // to an ephemeral one. Stable matters: relays such as Mission Planner's UDP server remember
        // the client address they stream to, and keep streaming to a dead port for a while after we
        // close it. Coming back on the same port makes a reconnect pick the stream up immediately
        // instead of timing out until the relay forgets us.
        let mut bind_note = None;
        let socket = match UdpSocket::bind(("0.0.0.0", port)) {
            Ok(s) => {
                log::info!("UDP bound to local port {} (listening for {})", port, addr);
                s
            }
            Err(e) => {
                let alt = stable_fallback_port(port);
                let (s, bound) = match UdpSocket::bind(("0.0.0.0", alt)) {
                    Ok(s) => (s, alt.to_string()),
                    Err(_) => (
                        UdpSocket::bind("0.0.0.0:0").map_err(|e| format!("UDP bind failed: {}", e))?,
                        "an ephemeral port".to_string(),
                    ),
                };
                log::warn!(
                    "UDP bind to local port {} failed ({}) — bound {} instead; datagrams pushed to {} reach the other listener, not Kite",
                    port, e, bound, port
                );
                bind_note = Some(format!(
                    "Local UDP port {port} is already in use by another program (another ground station listening there?), so Kite listened on {bound} instead. Vehicles that push telemetry to {port} reach that program, not Kite — close its UDP connection, or point the vehicles (or its MAVLink mirror) at a port Kite can own."
                ));
                s
            }
        };

        socket
            .set_read_timeout(Some(Duration::from_millis(READ_TIMEOUT_MS)))
            .map_err(|e| format!("Failed to set read timeout: {}", e))?;

        Ok(Self {
            configured: addr,
            bind_note,
            peer,
            peers: HashMap::new(),
            socket,
        })
    }
}

/// Alternate local port when the well-known one is taken: the target port + 10000 (14550 → 24550,
/// 14560 → 24560), kept inside the registered/dynamic range. Deterministic across reconnects.
fn stable_fallback_port(port: u16) -> u16 {
    let alt = port as u32 + 10_000;
    if alt <= u16::MAX as u32 { alt as u16 } else { port.wrapping_sub(10_000).max(1024) }
}

impl ByteTransport for UdpTransport {
    fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        match self.socket.recv_from(buf) {
            Ok((n, src)) => {
                // Peer learning: remember every source that talks to us (n == 0 never happens for a
                // real datagram). Sends fan out to all of them — see `peers`.
                if self.peers.insert(src, Instant::now()).is_none() {
                    log::debug!("UDP peer learned: {} ({} peer(s) now)", src, self.peers.len());
                }
                Ok(n)
            }
            Err(ref e)
                if e.kind() == std::io::ErrorKind::TimedOut
                    || e.kind() == std::io::ErrorKind::WouldBlock =>
            {
                Ok(0)
            }
            // UDP is connectionless: on Windows a datagram we sent to a peer that has since closed its
            // socket comes back as an ICMP "port unreachable", which the NEXT recv_from reports as
            // ConnectionReset. With several learned peers (vehicles, another GCS) that happens whenever
            // one of them goes away — it must not take the whole link down. The peer ages out on its own.
            Err(ref e)
                if e.kind() == std::io::ErrorKind::ConnectionReset
                    || e.kind() == std::io::ErrorKind::ConnectionRefused
                    || e.kind() == std::io::ErrorKind::ConnectionAborted =>
            {
                log::debug!("UDP recv: {} — a peer went away, ignoring", e);
                Ok(0)
            }
            Err(e) => Err(TransportError::from(e)),
        }
    }

    fn write_bytes(&mut self, data: &[u8]) -> Result<(), TransportError> {
        // Age out silent peers, then fan out to every live one; with none learned yet, the configured
        // target gets it (this is how the first GCS HEARTBEAT wakes a client-mode FC / bridge).
        let now = Instant::now();
        self.peers.retain(|_, seen| now.duration_since(*seen) < PEER_TTL);
        if self.peers.is_empty() {
            return self.socket
                .send_to(data, self.peer)
                .map(|_| ())
                .map_err(|e| TransportError::Io(format!("UDP send to {} failed: {}", self.peer, e)));
        }
        let mut first_err = None;
        for addr in self.peers.keys() {
            if let Err(e) = self.socket.send_to(data, addr) {
                first_err.get_or_insert_with(|| TransportError::Io(format!("UDP send to {} failed: {}", addr, e)));
            }
        }
        match first_err {
            Some(e) if self.peers.len() == 1 => Err(e),
            _ => Ok(()), // at least one peer took it (a dead peer ages out on its own)
        }
    }

    fn set_read_timeout(&mut self, timeout: Duration) {
        let _ = self.socket.set_read_timeout(Some(timeout));
    }

    fn diagnostic_note(&self) -> Option<String> {
        self.bind_note.clone()
    }

    fn description(&self) -> String {
        format!("UDP({})", self.configured)
    }
}
