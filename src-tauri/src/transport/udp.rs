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
use std::net::{IpAddr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::{ByteTransport, TransportError};

/// Read timeout for individual recv calls. Short on purpose — bounds the latency the MAVLink handler
/// loop adds to outgoing commands (it services a queued write only once the current blocking recv
/// returns). See the TCP transport for the full rationale.
const READ_TIMEOUT_MS: u64 = 50;

/// Local ports the open UDP links listen on, and the source ports of Kite's own relay sockets. A relay
/// aimed at a listening port of this machine would be read back as vehicle telemetry and relayed again.
static LISTENING: Mutex<Vec<u16>> = Mutex::new(Vec::new());
static OWN_SENDERS: Mutex<Vec<u16>> = Mutex::new(Vec::new());

fn forget(list: &Mutex<Vec<u16>>, port: u16) {
    if let Ok(mut l) = list.lock() {
        if let Some(i) = l.iter().position(|p| *p == port) {
            l.remove(i);
        }
    }
}

/// A relay's UDP socket registers its source port; the links drop what it sends.
pub fn register_own_sender(port: u16) {
    if let Ok(mut l) = OWN_SENDERS.lock() {
        l.push(port);
    }
}

pub fn forget_own_sender(port: u16) {
    forget(&OWN_SENDERS, port);
}

/// An address of this machine: loopback, unspecified, or one a socket can be bound to.
fn is_this_machine(ip: IpAddr) -> bool {
    ip.is_loopback() || ip.is_unspecified() || UdpSocket::bind((ip, 0)).is_ok()
}

/// True when `host:port` is a UDP link Kite itself listens on (this machine, or the limited broadcast).
pub fn targets_own_listener(host: &str, port: u16) -> bool {
    if !LISTENING.lock().map(|l| l.contains(&port)).unwrap_or(false) {
        return false;
    }
    let Ok(addrs) = (host, port).to_socket_addrs() else { return false };
    addrs
        .map(|a| a.ip())
        .any(|ip| matches!(ip, IpAddr::V4(v4) if v4.is_broadcast()) || is_this_machine(ip))
}

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
    /// The local port as registered in `LISTENING`.
    local_port: u16,
    /// Every source that has sent us data recently, with its last-seen time (peer learning). Outgoing
    /// frames go to ALL of them: several vehicles pushing to our listening port each speak from their
    /// own socket (multi-vehicle SITL / several telemetry bridges), and a command for sysid 3 sent
    /// only to "whoever spoke last" would mostly land on the wrong aircraft and be ignored. Vehicles
    /// drop frames not addressed to them, so the fan-out is harmless; peers silent for PEER_TTL age out.
    peers: HashMap<SocketAddr, Instant>,
    socket: UdpSocket,
    /// The first ICMP "port unreachable" of the session is logged at warn (a tester's log must show why
    /// nothing arrives); every further one goes to debug so a dead peer doesn't flood the log.
    peer_unreachable_logged: bool,
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

        let local_port = socket.local_addr().map(|a| a.port()).unwrap_or(port);
        if let Ok(mut l) = LISTENING.lock() {
            l.push(local_port);
        }

        Ok(Self {
            configured: addr,
            bind_note,
            peer,
            local_port,
            peers: HashMap::new(),
            socket,
            peer_unreachable_logged: false,
        })
    }

    /// True for the errors Windows reports on a UDP socket after an ICMP "port unreachable" came back
    /// for an earlier datagram — on the next `recv_from` (`WSAECONNRESET`) or `send_to`. UDP is
    /// connectionless: the peer being gone is a stall (ADR-042), not a transport loss — the link stays
    /// up, the stall watchdog reports it, and the peer may come back (a restarted SITL or bridge).
    fn is_peer_unreachable(e: &std::io::Error) -> bool {
        matches!(
            e.kind(),
            std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::ConnectionRefused
                | std::io::ErrorKind::ConnectionAborted
        )
    }

    fn note_peer_unreachable(&mut self, op: &str, peer: SocketAddr, e: &std::io::Error) {
        if !self.peer_unreachable_logged {
            self.peer_unreachable_logged = true;
            log::warn!(
                "UDP peer {} is not reachable ({} — {}); the link stays open, nothing arrives until the peer is back",
                peer, op, e
            );
        } else {
            log::debug!("UDP {}: {} — peer still unreachable, ignoring", op, e);
        }
    }
}

impl Drop for UdpTransport {
    fn drop(&mut self) {
        forget(&LISTENING, self.local_port);
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
                // Our own echo: with the default target 127.0.0.1:<our port> the first GCS HEARTBEAT goes
                // to ourselves and comes straight back. Learning that "peer" would loop every later frame
                // back into our own parser for the rest of the session — drop it unseen.
                if src.ip().is_loopback() && Some(src.port()) == self.socket.local_addr().ok().map(|a| a.port()) {
                    return Ok(0);
                }
                // A relay of our own that reaches this port (a subnet broadcast, a second address of this
                // machine): relaying it again would never end.
                let own_sender = OWN_SENDERS.lock().map(|l| l.contains(&src.port())).unwrap_or(false);
                if own_sender && is_this_machine(src.ip()) {
                    return Ok(0);
                }
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
            Err(ref e) if Self::is_peer_unreachable(e) => {
                // The ICMP error carries no source: with one learned peer that is the one that went away,
                // otherwise the best we can name is the configured target.
                let peer = if self.peers.len() == 1 { self.peers.keys().copied().next() } else { None };
                self.note_peer_unreachable("recv", peer.unwrap_or(self.peer), e);
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
        let targets: Vec<SocketAddr> = if self.peers.is_empty() {
            vec![self.peer]
        } else {
            self.peers.keys().copied().collect()
        };
        // A datagram is delivered when at least one target took it. A peer that is gone (ICMP "port
        // unreachable" surfacing on the send side, see `is_peer_unreachable`) is not a failure: the
        // datagram is simply lost, like on any quiet link, and the peer ages out on its own. A fatal
        // `Io` here would make the MSP scheduler tear the link down (`mark_lost`) and the MAVLink
        // handler warn once per heartbeat / RC frame. Only a real socket error on EVERY target is
        // reported — with several peers, one dead one must not hide that the others were reached.
        let mut delivered = 0usize;
        let mut first_err = None;
        for addr in targets {
            match self.socket.send_to(data, addr) {
                Ok(_) => delivered += 1,
                Err(ref e) if Self::is_peer_unreachable(e) => self.note_peer_unreachable("send", addr, e),
                Err(e) => {
                    log::debug!("UDP send to {} failed: {}", addr, e);
                    first_err.get_or_insert_with(|| TransportError::Io(format!("UDP send to {} failed: {}", addr, e)));
                }
            }
        }
        match first_err {
            Some(e) if delivered == 0 => Err(e),
            _ => Ok(()),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn free_port() -> u16 {
        UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
    }

    #[test]
    fn a_relay_aimed_at_the_link_port_is_recognised() {
        let port = free_port();
        assert!(!targets_own_listener("127.0.0.1", port));
        let link = UdpTransport::connect("127.0.0.1", port).unwrap();
        assert!(targets_own_listener("127.0.0.1", port));
        assert!(targets_own_listener("localhost", port));
        assert!(targets_own_listener("255.255.255.255", port));
        // Another machine may listen on the same port number: that is a normal relay target.
        assert!(!targets_own_listener("192.0.2.1", port));
        assert!(!targets_own_listener("127.0.0.1", port.wrapping_add(1)));
        drop(link);
        assert!(!targets_own_listener("127.0.0.1", port));
    }

    #[test]
    fn datagrams_of_an_own_relay_socket_are_dropped() {
        let port = free_port();
        let mut link = UdpTransport::connect("127.0.0.1", port).unwrap();
        let mut buf = [0u8; 64];

        let relay = UdpSocket::bind("127.0.0.1:0").unwrap();
        let relay_port = relay.local_addr().unwrap().port();
        register_own_sender(relay_port);
        relay.send_to(b"relayed", ("127.0.0.1", port)).unwrap();
        assert_eq!(link.read_bytes(&mut buf).unwrap(), 0);
        assert!(link.peers.is_empty());

        let vehicle = UdpSocket::bind("127.0.0.1:0").unwrap();
        vehicle.send_to(b"vehicle", ("127.0.0.1", port)).unwrap();
        assert_eq!(link.read_bytes(&mut buf).unwrap(), 7);
        assert_eq!(link.peers.len(), 1);

        forget_own_sender(relay_port);
        relay.send_to(b"relayed", ("127.0.0.1", port)).unwrap();
        assert_eq!(link.read_bytes(&mut buf).unwrap(), 7);
    }
}
