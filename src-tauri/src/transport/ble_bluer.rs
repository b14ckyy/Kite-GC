// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Linux BLE FC data path via bluer (native BlueZ). Uses AcquireNotify/AcquireWrite (FD sockets) for a
//! stable, low-latency serial-over-BLE link — the approach MWPTools uses. Scan + listen stay on btleplug
//! (transport/ble.rs); only the FC connection is bluer here. See docs/dev/BLE_LINUX_NATIVE_IMPL.md.
//!
//! Transparent reconnect: when the BLE link drops (notify EOF, write error, or a BlueZ disconnect) the
//! transport task does NOT end — it keeps the read/write channels open and silently re-establishes the
//! GATT connection underneath. The scheduler therefore never sees a `Disconnected` (only a stall, like a
//! temporary RC-link loss), keeps polling, and resumes when the link returns — no teardown, no fresh FC
//! handshake, just a time gap in the data. Even a full TX/BLE-endpoint power-cycle is just a longer gap.
//! The task only ends (dropping the read channel → scheduler teardown) on an explicit stop (user
//! disconnect). Writes are capped at 20 bytes regardless of the negotiated MTU — CC2541/HM-10 firmware
//! only digests ~20-byte ATT writes (a larger write overflows its UART bridge); matches MWPTools and the
//! btleplug path (`BLE_WRITE_MTU`).

use std::str::FromStr;
use std::sync::mpsc;
use std::time::Duration;

use bluer::gatt::remote::{CharacteristicReader, CharacteristicWriter};
use bluer::{Adapter, Address, Device, DeviceEvent, DeviceProperty, Session};
use futures::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc::error::TryRecvError;

use super::{known_profiles, BleDeviceProfile, BleTransport};

/// CC2541/HM-10-class serial modules negotiate a large ATT MTU on some stacks (e.g. ~185 on Ubuntu
/// 26.04's newer BlueZ) but their firmware only digests ~20-byte ATT writes — a larger write overflows
/// the UART bridge and drops the link. Cap every write at 20 bytes regardless of the negotiated MTU.
const BLE_WRITE_CHUNK: usize = 20;

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

/// (Re-)establish the GATT serial link: a short discovery (so a re-advertising device is found again),
/// connect if needed, match a known serial profile + its read/write characteristics, and acquire the
/// notify/write FD sockets. Returns the live device handle, the FD reader/writer and the matched profile.
async fn establish(
    adapter: &Adapter,
    addr: Address,
    profiles: &[BleDeviceProfile],
) -> Result<(Device, CharacteristicReader, CharacteristicWriter, BleDeviceProfile), String> {
    // A short discovery is cheap insurance that BlueZ knows the device (esp. after it re-advertised
    // following a power-cycle); drop the stream right after to stop discovery.
    {
        let _discovery = adapter
            .discover_devices()
            .await
            .map_err(|e| format!("BLE discover: {e}"))?;
        tokio::time::sleep(Duration::from_millis(2000)).await;
    }

    let device = adapter.device(addr).map_err(|e| format!("BLE device handle: {e}"))?;
    if !device.is_connected().await.unwrap_or(false) {
        device.connect().await.map_err(|e| format!("BLE connect failed: {e}"))?;
    }

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
            // THE key bit: FD-socket GATT (AcquireNotify / AcquireWrite), not signal callbacks.
            let reader = r.notify_io().await.map_err(|e| format!("AcquireNotify: {e}"))?;
            let writer = w.write_io().await.map_err(|e| format!("AcquireWrite: {e}"))?;
            return Ok((device, reader, writer, profile.clone()));
        }
    }
    Err("No matching BLE serial profile/characteristics".to_string())
}

pub async fn connect_ble(device_id: &str) -> Result<BleTransport, String> {
    let addr = parse_address(device_id)?;

    let session = Session::new().await.map_err(|e| format!("BlueZ session: {e}"))?;
    let adapter = session.default_adapter().await.map_err(|e| format!("No BLE adapter: {e}"))?;
    adapter.set_powered(true).await.map_err(|e| format!("Adapter power: {e}"))?;

    let profiles = known_profiles();

    // First connection MUST succeed — that's what the caller (and the scheduler's one-time handshake)
    // see as "connected". A later drop is recovered transparently inside the task below.
    let (device, reader, writer, profile) = establish(&adapter, addr, &profiles).await?;
    let device_name = device.name().await.ok().flatten().unwrap_or_else(|| "Unknown".to_string());
    let profile_name = profile.name.to_string();
    let write_mtu = writer.mtu().max(20); // informational (writes are capped at BLE_WRITE_CHUNK)
    // Per-profile write throttle: the CC2541/HM-10 UART bridge drops data on back-to-back ATT writes.
    let write_delay = Duration::from_millis(profile.write_delay_ms);

    let (write_tx, mut write_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    let (read_tx, read_rx) = mpsc::channel::<Vec<u8>>();
    let (stop_tx, mut stop_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

    tauri::async_runtime::spawn(async move {
        // Keep the session alive for the whole task so reconnects can use the adapter.
        let _session = session;
        let mut buf = vec![0u8; 512];
        // The live link for this serve cycle; `None` means "reconnect first".
        let mut current: Option<(Device, CharacteristicReader, CharacteristicWriter)> =
            Some((device, reader, writer));

        loop {
            // ── Ensure we have a live link (reconnect with capped backoff on loss) ──
            let (device, mut reader, mut writer) = match current.take() {
                Some(c) => c,
                None => {
                    let mut backoff_ms = 1000u64;
                    loop {
                        // Stop requested (or the transport was dropped) → end the task.
                        match stop_rx.try_recv() {
                            Err(TryRecvError::Empty) => {}
                            _ => return,
                        }
                        match establish(&adapter, addr, &profiles).await {
                            Ok((d, r, w, _p)) => {
                                log::info!("BLE(bluer): reconnected");
                                break (d, r, w);
                            }
                            Err(e) => {
                                log::warn!("BLE(bluer): reconnect failed: {e} — retry in {backoff_ms}ms");
                                tokio::select! {
                                    _ = tokio::time::sleep(Duration::from_millis(backoff_ms)) => {}
                                    _ = stop_rx.recv() => return,
                                }
                                backoff_ms = (backoff_ms * 2).min(5000);
                            }
                        }
                    }
                }
            };

            // Discard any writes queued during the gap — the scheduler's request/response already moved
            // on, so replaying stale requests would only confuse response matching.
            while write_rx.try_recv().is_ok() {}

            let mut dev_events = match device.events().await {
                Ok(e) => e,
                Err(e) => {
                    log::warn!("BLE(bluer): device events: {e} — reconnecting");
                    let _ = device.disconnect().await;
                    current = None;
                    continue;
                }
            };

            // ── Serve until the link is lost (then reconnect) or stop is requested (then end) ──
            let mut link_lost = false;
            loop {
                tokio::select! {
                    // Incoming bytes from the notify FD. EOF (Ok(0)) / error == link lost → reconnect.
                    r = reader.read(&mut buf) => match r {
                        Ok(0) => { log::warn!("BLE(bluer): notify EOF — reconnecting"); link_lost = true; break; }
                        Ok(n) => { let _ = read_tx.send(buf[..n].to_vec()); }
                        Err(e) => { log::warn!("BLE(bluer): notify read error: {e} — reconnecting"); link_lost = true; break; }
                    },
                    // Outgoing frames, capped at 20-byte ATT writes, throttled per profile.
                    Some(data) = write_rx.recv() => {
                        let mut failed = false;
                        for chunk in data.chunks(BLE_WRITE_CHUNK) {
                            if let Err(e) = writer.write_all(chunk).await {
                                log::warn!("BLE(bluer): write failed: {e} — reconnecting");
                                failed = true;
                                break;
                            }
                            if !write_delay.is_zero() {
                                tokio::time::sleep(write_delay).await;
                            }
                        }
                        if failed { link_lost = true; break; }
                    }
                    // BlueZ-level disconnect signal.
                    Some(ev) = dev_events.next() => {
                        if matches!(ev, DeviceEvent::PropertyChanged(DeviceProperty::Connected(false))) {
                            log::warn!("BLE(bluer): device disconnected — reconnecting");
                            link_lost = true;
                            break;
                        }
                    }
                    // Explicit stop (user disconnect) → end the task, dropping read_tx so the scheduler
                    // tears down. This is the ONLY path that reports the link as gone.
                    _ = stop_rx.recv() => { log::info!("BLE(bluer) runtime stopping"); let _ = device.disconnect().await; return; }
                }
            }

            if link_lost {
                let _ = device.disconnect().await; // clean slate before reconnecting
                current = None;
            }
        }
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
