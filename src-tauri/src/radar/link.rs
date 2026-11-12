// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Radar — the byte link a local radar source reads from: a serial port, a TCP connection or a UDP
// socket. One spec type + one opener shared by the ADS-B receiver and the FormationFlight module, so
// both accept every transport the same way. Platform capability stays a runtime property: on iOS a
// serial open fails cleanly (transport::serial stand-in), TCP / UDP work everywhere.

use crate::transport::serial::SerialConnection;
use crate::transport::tcp::TcpTransport;
use crate::transport::udp::UdpTransport;
use crate::transport::ByteTransport;

/// The resolved link of one local radar source. Built only from a config that can actually be opened
/// (see `LinkSpec::from_config`), so a TCP / UDP spec always has a host and a non-zero port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkSpec {
    Serial { port: String, baud: u32 },
    Tcp { host: String, port: u16 },
    Udp { host: String, port: u16 },
}

impl LinkSpec {
    /// Turn the stored source fields into a link, or `None` when the source cannot be started: a serial
    /// source without a port, a TCP source with an empty host, an IP source with a port outside
    /// 1..=65535, or a transport this build does not know. A UDP source needs no host: Kite listens on
    /// the port and answers whoever sent (an empty host becomes `0.0.0.0`). `transport` empty ⇒ serial (configs stored before the field
    /// existed). `baud` is the effective baud (the caller applies its default for 0).
    pub fn from_config(transport: &str, port: &str, baud: u32, host: &str, net_port: u32) -> Option<Self> {
        let host = host.trim();
        let net_port = u16::try_from(net_port).ok().filter(|p| *p != 0);
        match transport {
            "" | "serial" => (!port.is_empty()).then(|| LinkSpec::Serial { port: port.to_string(), baud }),
            "tcp" => Some(LinkSpec::Tcp { host: (!host.is_empty()).then(|| host.to_string())?, port: net_port? }),
            "udp" => Some(LinkSpec::Udp {
                host: if host.is_empty() { "0.0.0.0".to_string() } else { host.to_string() },
                port: net_port?,
            }),
            _ => None,
        }
    }

    /// Short label for logs: the serial port + baud, or `tcp host:port` / `udp host:port` — nothing more.
    pub fn describe(&self) -> String {
        match self {
            LinkSpec::Serial { port, baud } => format!("{} @ {}", port, baud),
            LinkSpec::Tcp { host, port } => format!("tcp {}:{}", host, port),
            LinkSpec::Udp { host, port } => format!("udp {}:{}", host, port),
        }
    }

    /// Open the link. `raise_dtr_rts` applies to serial only: many USB-serial ADS-B receivers stream only
    /// once the host raises DTR/RTS, while toggling them resets an ESP32 (FormationFlight must not). A
    /// failure to set them is logged and the link is used anyway.
    ///
    /// UDP must own its configured local port. Both radar sources only ever answer or listen — the device
    /// pushes to that port — so `UdpTransport`'s fallback to another local port (meant for a main link
    /// that wakes its peer by sending first) would leave the source deaf for good, and a UDP link never
    /// errors out on its own to retry. A busy port is therefore a failed open: the caller's reconnect
    /// backoff retries until the port is free (e.g. once the previous worker of this very source has
    /// closed its socket after a reconfigure).
    pub fn open(&self, raise_dtr_rts: bool) -> Result<Box<dyn ByteTransport>, String> {
        match self {
            LinkSpec::Serial { port, baud } => {
                let mut conn = SerialConnection::open(port, *baud)?;
                if raise_dtr_rts {
                    if let Err(e) = conn.set_control_signals(true, true) {
                        log::warn!("[radar] {} DTR/RTS failed (continuing): {}", port, e);
                    }
                }
                Ok(Box::new(conn))
            }
            LinkSpec::Tcp { host, port } => Ok(Box::new(TcpTransport::connect(host, *port)?)),
            LinkSpec::Udp { host, port } => {
                // Probe first, so a busy port fails here quietly instead of through UdpTransport's
                // fallback path, which warns on every attempt (once per reconnect backoff otherwise).
                std::net::UdpSocket::bind(("0.0.0.0", *port))
                    .map_err(|e| format!("local UDP port {} is in use: {}", port, e))?;
                let udp = UdpTransport::connect(host, *port)?;
                // Taken between the probe and the real bind → UdpTransport fell back; still a failed open.
                if udp.diagnostic_note().is_some() {
                    return Err(format!("local UDP port {} is in use", port));
                }
                Ok(Box::new(udp))
            }
        }
    }
}

/// Logs failed opens so a tester's default-level log shows why a source stays dark, without one line per
/// reconnect attempt: the SECOND failure in a row goes to `warn`, every other one to `debug`. The first
/// one alone is not worth a warning — a reconfigure restarts the ADS-B sources while the previous worker
/// still holds the serial port / UDP socket for one read timeout, so the new worker's first open
/// routinely fails once and succeeds on the next attempt.
#[derive(Default)]
pub struct OpenFailureLog {
    failures: u32,
}

impl OpenFailureLog {
    pub fn failed(&mut self, source: &str, link: &LinkSpec, err: &str) {
        self.failures = self.failures.saturating_add(1);
        if self.failures == 2 {
            log::warn!("[radar] {} open {} failed (retrying): {}", source, link.describe(), err);
        } else {
            log::debug!("[radar] {} open {} failed: {}", source, link.describe(), err);
        }
    }

    pub fn opened(&mut self) {
        self.failures = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_transport_is_serial() {
        assert_eq!(
            LinkSpec::from_config("", "COM3", 57600, "", 0),
            Some(LinkSpec::Serial { port: "COM3".into(), baud: 57600 })
        );
    }

    #[test]
    fn serial_without_port_is_not_started() {
        assert_eq!(LinkSpec::from_config("serial", "", 57600, "10.0.0.1", 5760), None);
    }

    #[test]
    fn ip_links_need_host_and_port() {
        assert_eq!(
            LinkSpec::from_config("tcp", "COM3", 57600, " 192.168.4.1 ", 5760),
            Some(LinkSpec::Tcp { host: "192.168.4.1".into(), port: 5760 })
        );
        assert_eq!(
            LinkSpec::from_config("udp", "", 0, "192.168.4.1", 14550),
            Some(LinkSpec::Udp { host: "192.168.4.1".into(), port: 14550 })
        );
        assert_eq!(LinkSpec::from_config("tcp", "", 0, "", 5760), None);
        assert_eq!(
            LinkSpec::from_config("udp", "", 0, "  ", 14551),
            Some(LinkSpec::Udp { host: "0.0.0.0".into(), port: 14551 })
        );
        assert_eq!(LinkSpec::from_config("tcp", "", 0, "192.168.4.1", 0), None);
        assert_eq!(LinkSpec::from_config("udp", "", 0, "192.168.4.1", 70000), None);
    }

    #[test]
    fn unknown_transport_is_not_started() {
        assert_eq!(LinkSpec::from_config("ble", "COM3", 57600, "x", 1), None);
    }
}
