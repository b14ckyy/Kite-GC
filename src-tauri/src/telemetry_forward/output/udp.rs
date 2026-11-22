// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! UDP output sink — fire-and-forget datagrams to a configured `host:port` (e.g. a GCS listening on a
//! UDP port, or a broadcast address). One datagram per frame set.

use std::net::UdpSocket;

use super::OutputSink;

pub struct UdpSink {
    socket: UdpSocket,
    target: String,
    local_port: Option<u16>,
}

impl UdpSink {
    pub fn open(host: &str, port: u16) -> Result<Self, String> {
        let socket = UdpSocket::bind("0.0.0.0:0").map_err(|e| format!("UDP relay socket failed: {e}"))?;
        // Allow broadcast targets (e.g. 255.255.255.255 / subnet broadcast) — harmless for unicast.
        let _ = socket.set_broadcast(true);
        let target = format!("{host}:{port}");
        let local_port = socket.local_addr().ok().map(|a| a.port());
        if let Some(p) = local_port {
            crate::transport::udp::register_own_sender(p);
        }
        Ok(Self { socket, target, local_port })
    }
}

impl Drop for UdpSink {
    fn drop(&mut self) {
        if let Some(p) = self.local_port {
            crate::transport::udp::forget_own_sender(p);
        }
    }
}

impl OutputSink for UdpSink {
    fn write(&mut self, data: &[u8]) -> Result<(), String> {
        self.socket
            .send_to(data, &self.target)
            .map_err(|e| format!("UDP relay send to {} failed: {e}", self.target))?;
        Ok(())
    }

    fn description(&self) -> String {
        format!("UDP({})", self.target)
    }
}
