// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Linux BLE FC data path via bluer (native BlueZ). Uses AcquireNotify/AcquireWrite (FD sockets) for a
//! stable, low-latency serial-over-BLE link — the approach MWPTools uses. Scan + listen stay on btleplug
//! (transport/ble.rs); only the FC connection is bluer here. See docs/dev/BLE_LINUX_NATIVE_IMPL.md.
//!
//! Why: btleplug's Linux backend receives notifications via `StartNotify` + per-notification D-Bus
//! signals; on some BlueZ stacks (reported on Ubuntu) that path silently stalls after ~10-30 s and the
//! write then returns "Not connected" on a CC2541. AcquireNotify/AcquireWrite hand back a raw SEQPACKET
//! socket FD + negotiated MTU — BlueZ's high-throughput path, with clean EOF on disconnect and no
//! signal-delivery backpressure.

use std::str::FromStr;
use std::sync::mpsc;
use std::time::Duration;

use bluer::{Address, DeviceEvent, DeviceProperty, Session};
use futures::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::{known_profiles, BleTransport};

/// The frontend stores the btleplug device id (Linux format `hciX/dev_AA_BB_CC_DD_EE_FF`, or sometimes a
/// bare MAC). Extract a `bluer::Address`.
fn parse_address(device_id: &str) -> Result<Address, String> {
    let mac = if let Some(idx) = device_id.find("dev_") {
        device_id[idx + 4..].replace('_', ":")
    } else {
        device_id.replace('_', ":")
    };
    Address::from_str(mac.trim()).map_err(|_| format!("Cannot parse BLE address from '{device_id}'"))
}

pub async fn connect_ble(device_id: &str) -> Result<BleTransport, String> {
    let addr = parse_address(device_id)?;

    let session = Session::new().await.map_err(|e| format!("BlueZ session: {e}"))?;
    let adapter = session.default_adapter().await.map_err(|e| format!("No BLE adapter: {e}"))?;
    adapter.set_powered(true).await.map_err(|e| format!("Adapter power: {e}"))?;

    // Make sure BlueZ knows the device (it usually does after the btleplug scan). A short discovery is
    // cheap insurance; keep the stream alive only for the sleep, then drop it to stop discovery.
    {
        let _discovery = adapter
            .discover_devices()
            .await
            .map_err(|e| format!("BLE discover: {e}"))?;
        tokio::time::sleep(Duration::from_millis(2500)).await;
    }

    let device = adapter.device(addr).map_err(|e| format!("BLE device handle: {e}"))?;
    if !device.is_connected().await.unwrap_or(false) {
        device.connect().await.map_err(|e| format!("BLE connect failed: {e}"))?;
    }

    // Match a known serial profile + its read/write characteristics by UUID (same set as btleplug path).
    let profiles = known_profiles();
    let mut found = None;
    for service in device.services().await.map_err(|e| format!("BLE services: {e}"))? {
        let suuid = service.uuid().await.map_err(|e| format!("svc uuid: {e}"))?;
        let Some(profile) = profiles.iter().find(|p| p.service_uuid == suuid) else {
            continue;
        };
        let (mut rc, mut wc) = (None, None);
        for ch in service.characteristics().await.map_err(|e| format!("chars: {e}"))? {
            let cu = ch.uuid().await.map_err(|e| format!("chr uuid: {e}"))?;
            if cu == profile.read_characteristic {
                rc = Some(ch.clone());
            }
            if cu == profile.write_characteristic {
                wc = Some(ch.clone());
            }
        }
        if let (Some(r), Some(w)) = (rc, wc) {
            found = Some((profile.clone(), r, w));
            break;
        }
    }
    let (profile, read_char, write_char) =
        found.ok_or_else(|| "No matching BLE serial profile/characteristics".to_string())?;

    // THE key bit: FD-socket GATT (AcquireNotify / AcquireWrite), not signal callbacks.
    let reader = read_char.notify_io().await.map_err(|e| format!("AcquireNotify: {e}"))?;
    let writer = write_char.write_io().await.map_err(|e| format!("AcquireWrite: {e}"))?;
    let write_mtu = writer.mtu().max(20);

    let (write_tx, mut write_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    let (read_tx, read_rx) = mpsc::channel::<Vec<u8>>();
    let (stop_tx, mut stop_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

    let device_name = device.name().await.ok().flatten().unwrap_or_else(|| "Unknown".to_string());
    let profile_name = profile.name.to_string();
    // Same per-profile write throttle as the btleplug path: the CC2541/HM-10 UART bridge drops data on
    // back-to-back ATT writes (30 ms for that profile), independent of the FD socket's flow control.
    let write_delay = Duration::from_millis(profile.write_delay_ms);
    let mut dev_events = device.events().await.map_err(|e| format!("device events: {e}"))?;

    tauri::async_runtime::spawn(async move {
        let mut reader = reader;
        let mut writer = writer;
        let mut buf = vec![0u8; 512];
        loop {
            tokio::select! {
                // Incoming bytes from the notify FD. EOF (Ok(0)) / error == link lost — break so read_tx
                // drops and the sync read_bytes() returns Disconnected -> scheduler teardown.
                r = reader.read(&mut buf) => match r {
                    Ok(0) => { log::warn!("BLE(bluer): notify EOF — link lost"); break; }
                    Ok(n) => { let _ = read_tx.send(buf[..n].to_vec()); }
                    Err(e) => { log::warn!("BLE(bluer): notify read error: {e} — link lost"); break; }
                },
                // Outgoing frames, chunked to the negotiated MTU (one ATT packet per write), throttled
                // per profile (CC2541 needs spacing or its UART bridge drops bytes).
                Some(data) = write_rx.recv() => {
                    let mut failed = false;
                    for chunk in data.chunks(write_mtu) {
                        if let Err(e) = writer.write_all(chunk).await {
                            log::warn!("BLE(bluer): write failed: {e} — link lost");
                            failed = true;
                            break;
                        }
                        if !write_delay.is_zero() {
                            tokio::time::sleep(write_delay).await;
                        }
                    }
                    if failed { break; }
                }
                // BlueZ-level disconnect signal (most reliable detection).
                Some(ev) = dev_events.next() => {
                    if matches!(ev, DeviceEvent::PropertyChanged(DeviceProperty::Connected(false))) {
                        log::warn!("BLE(bluer): device disconnected (Connected=false)");
                        break;
                    }
                }
                _ = stop_rx.recv() => { log::info!("BLE(bluer) runtime stopping"); break; }
            }
        }
        let _ = device.disconnect().await; // best effort
    });

    log::info!("BLE connected (bluer): {device_name} [{profile_name}], write MTU {write_mtu}");
    Ok(BleTransport {
        device_name,
        profile_name,
        write_tx,
        read_rx,
        stop_tx,
        read_buffer: Vec::new(),
    })
}
