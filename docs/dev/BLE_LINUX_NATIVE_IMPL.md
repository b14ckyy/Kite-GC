# Implementation brief — native BlueZ BLE data path on Linux (`bluer` / AcquireNotify)

> **Audience:** an AI coding agent working **on the Debian build machine** (can compile Linux + has the
> BLE hardware). This file is a throwaway implementation brief — delete it before merging the branch.
> **Branch:** `feat/ble-linux-native` (already created off `master`). Do all work here. **Do NOT touch
> `master`** — it runs fine on Debian today; this is an experiment to fix Ubuntu/BlueZ instability.
> Follow the repo `CLAUDE.md` (Rust logging convention, no mass refactors, etc.).

## 1. Problem & goal

On some Linux/BlueZ stacks (reported on **Ubuntu**; stable on **Debian 13**) a BLE serial link to an
INAV FC (Matek mLRS / **HC-04BLE, CC2541**, mLRS 1.4) connects fine, then the data stream **stalls or
drops after ~10–30 s**. The reporter is an MWPTools user — **the same machine/link/plane is stable in
MWPTools** — so this is **our BLE implementation**, not the hardware or BlueZ itself.

**Root cause (confirmed from a debug log):** we use the `btleplug` crate, whose Linux backend receives
GATT notifications via **`StartNotify` + per-notification D-Bus signals** (callback path). On this stack
that path silently stalls; on a CC2541 the *write* then returns `Not connected`. (We already merged a
fix on `master` that at least *detects* this and tears down cleanly instead of zombie-ing — see
`git log`. This brief is about making the link **actually stable**, not just failing cleanly.)

**Goal:** replace the **FC BLE connection's data path on Linux** with the approach MWPTools uses —
talk to BlueZ over D-Bus directly and use **`AcquireNotify` / `AcquireWrite`**, which hand back a raw
**file-descriptor (SEQPACKET socket) + negotiated MTU**. Read/write that FD like a serial socket. This
is BlueZ's high-throughput, low-latency path; it gives clean EOF on disconnect and no signal-delivery
backpressure. In Rust the idiomatic binding for this is the **`bluer`** crate (`Characteristic::notify_io()`
/ `write_io()` return tokio `AsyncRead`/`AsyncWrite` over those FDs).

### MWPTools reference (for behaviour parity)
`src/common/bluez.vala`, `src/common/ble-helper.vala` on github.com/stronnag/mwptools (mirror) /
codeberg.org/stronnag/mwptools:
- D-Bus native to `org.bluez`.
- Disconnect via the device `Connected` property (`PropertiesChanged` / `InterfacesRemoved`).
- **`get_fd_for_characteristic(...)` calls `AcquireNotify` (rx) and `AcquireWrite` (tx)** → FD + MTU;
  reads/writes the FD. Uses `min(rxmtu, txmtu)`.

## 2. Scope (keep it minimal & focused)

**In scope:** ONLY `connect_ble()` (the FC connection data path) on **Linux**. Implement it with `bluer`.

**Out of scope (leave on `btleplug`, all platforms, unchanged):**
- `scan_ble_devices()` and `run_scan_session()` (device discovery works fine).
- `connect_ble_listen()` (passive radio-telemetry GATT explorer — separate feature, not in question).
- Windows/macOS `connect_ble()` — stays `btleplug`.

Rationale: the instability is specifically in the FC notification/write data path. Keeping scan on
`btleplug` means **the device-id string the frontend stores still comes from `btleplug`**, so the new
`bluer` `connect_ble` must accept that id (see §5). A full bluer migration of scan+listen is an optional
later step *if* this experiment proves the AcquireNotify path fixes stability.

**The `BleTransport` struct and its `ByteTransport` impl do NOT change** — they're just channels
(`read_rx` / `write_tx` / `stop_tx`), backend-agnostic. Both backends construct the same struct.

## 3. Cargo.toml changes (`src-tauri/Cargo.toml`)

Add `bluer` to the **Linux** target deps (leave `btleplug` exactly as is — still used for scan/listen):

```toml
[target.'cfg(target_os = "linux")'.dependencies]
# ... existing entries (evdev, libc, webkit2gtk) ...
bluer = { version = "0.17", features = ["bluetoothd"] }   # native BlueZ GATT (AcquireNotify/AcquireWrite)
```

Add the `io-util` feature to the shared `tokio` dependency (the FD reader/writer are tokio AsyncRead/Write):

```toml
# in [dependencies]:
tokio = { version = "1", features = ["time", "sync", "macros", "io-util"] }
```

> Pin the exact `bluer` version the agent resolves (`cargo update -p bluer` then record it). **Verify the
> API names below against `cargo doc -p bluer` / docs.rs for that version** — this brief was written
> without a Linux compiler, so method names / return shapes (esp. `Option` wrapping) may need small
> adjustments. `bluer` requires a running `bluetoothd` (it does on the target).

## 4. `transport/ble.rs` changes

Currently `connect_ble()` (≈ lines 259–434) is the `btleplug` implementation for all platforms. Gate it
to non-Linux and add the Linux module + re-export. Near the top of `ble.rs`:

```rust
// Linux: the FC BLE data path uses bluer (native BlueZ AcquireNotify/AcquireWrite) instead of btleplug,
// which stalls on some BlueZ stacks (see docs/dev/BLE_LINUX_NATIVE_IMPL.md). Scan + listen stay btleplug.
#[cfg(target_os = "linux")]
mod ble_bluer;
#[cfg(target_os = "linux")]
pub use ble_bluer::connect_ble;
```

Change the existing function signature line:

```rust
pub async fn connect_ble(device_id: &str) -> Result<BleTransport, String> {
```
to
```rust
#[cfg(not(target_os = "linux"))]
pub async fn connect_ble(device_id: &str) -> Result<BleTransport, String> {
```

Leave `scan_ble_devices`, `run_scan_session`, `connect_ble_listen`, `BleDeviceInfo`, `BleDeviceProfile`,
`known_profiles`, `BleTransport`, the `ByteTransport` impl and `Drop` **unchanged** (all platforms).

> `ble_bluer` is a **child module** of `ble`, so it can construct `BleTransport` using its private fields
> (`device_name`, `profile_name`, `write_tx`, `read_rx`, `stop_tx`, `read_buffer`) — same as the existing
> btleplug `connect_ble` does. Reuse `super::known_profiles()` for the UUIDs.

## 5. New file `transport/ble_bluer.rs` (reference implementation)

Reference code — **verify against the pinned `bluer` API and adjust as the compiler dictates.** Behaviour
to preserve: same `BleTransport` channel contract; profile/characteristic matching by the same UUIDs;
robust disconnect detection (read EOF, write error, `Connected=false`); MTU-chunked writes.

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Linux BLE FC data path via bluer (native BlueZ). Uses AcquireNotify/AcquireWrite (FD sockets) for a
//! stable, low-latency serial-over-BLE link — the approach MWPTools uses. Scan + listen stay on btleplug
//! (transport/ble.rs); only the FC connection is bluer here. See docs/dev/BLE_LINUX_NATIVE_IMPL.md.

use std::str::FromStr;
use std::sync::mpsc;
use std::time::Duration;

use bluer::{Address, Session, DeviceEvent, DeviceProperty};
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
        let _discovery = adapter.discover_devices().await.map_err(|e| format!("BLE discover: {e}"))?;
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
        let Some(profile) = profiles.iter().find(|p| p.service_uuid == suuid) else { continue };
        let (mut rc, mut wc) = (None, None);
        for ch in service.characteristics().await.map_err(|e| format!("chars: {e}"))? {
            let cu = ch.uuid().await.map_err(|e| format!("chr uuid: {e}"))?;
            if cu == profile.read_characteristic { rc = Some(ch.clone()); }
            if cu == profile.write_characteristic { wc = Some(ch.clone()); }
        }
        if let (Some(r), Some(w)) = (rc, wc) { found = Some((profile.clone(), r, w)); break; }
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
                // Outgoing frames, chunked to the negotiated MTU (one ATT packet per write).
                Some(data) = write_rx.recv() => {
                    let mut failed = false;
                    for chunk in data.chunks(write_mtu) {
                        if let Err(e) = writer.write_all(chunk).await {
                            log::warn!("BLE(bluer): write failed: {e} — link lost");
                            failed = true; break;
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
    Ok(BleTransport { device_name, profile_name, write_tx, read_rx, stop_tx, read_buffer: Vec::new() })
}
```

### Notes / likely adjustment points (verify on Linux)
- `notify_io()` / `write_io()` return types and `mtu()` — confirm names; some bluer versions name them
  differently or require `bluer::gatt::remote::CharacteristicReader/Writer` imports.
- `device.events()` item type / `DeviceProperty::Connected` variant — confirm exact path.
- `Device::name()/is_connected()/services()` return shapes (`Result<Option<..>>` vs `Result<..>`).
- `Characteristic::clone()` availability (handles are cheap clones in bluer; if not `Clone`, restructure
  to avoid cloning).
- SEQPACKET write semantics: if `write_all` per chunk misbehaves, use a single `writer.write(chunk)` and
  assert it wrote the whole packet.

## 6. Verification (on the Debian/Ubuntu machine)

1. `cargo check` and `cargo clippy` clean. `cargo test --no-run` (per CLAUDE.md, also compile test targets).
2. Build the app; **on the Ubuntu machine that reproduced the bug**, connect to the FC over BLE.
3. **Expected:** link stays up well beyond the previous 10–30 s — stable like MWPTools. Telemetry keeps
   flowing; no `Link stalled` / `notify EOF` / `write failed` spam.
4. Pull a person out of range and back: with this change, an *actual* range loss now produces a clean
   `BLE(bluer): notify EOF — link lost` and disconnect (correct), not a silent stall.
5. Cross-check it still builds + runs on Windows (CI / the Windows dev box) — the Linux code is
   `#[cfg(target_os = "linux")]` so Windows uses btleplug unchanged.

## 7. Acceptance criteria
- A BLE FC link on the previously-failing Ubuntu setup is **stable for minutes** (no 10–30 s drop).
- Disconnect is still detected cleanly (so the future auto-reconnect can build on it).
- Windows/macOS BLE behaviour unchanged (btleplug).
- No change to `scan`, `run_scan_session`, `connect_ble_listen`, or the `BleTransport`/`ByteTransport`
  contract.

## 8. If it works / if it doesn't
- **Works:** keep the branch; we then decide whether to also migrate scan + listen to bluer (so Linux
  drops the btleplug dependency entirely) and remove this brief. Update CHANGELOG (Fixed). Then revisit
  the deferred **auto-reconnect** (`active/AUTO_RECONNECT.md`) as a *comfort* feature for genuine range
  loss — explicitly NOT to mask link instability.
- **Doesn't:** the fallback is a full native BlueZ path or the `mwp-ble-bridge` sidecar approach (see
  `future/BLE_LINUX_NATIVE.md` in the private dev-docs). Keep `master` as the shipping branch meanwhile.

## 9. Guardrails
- Work only on `feat/ble-linux-native`; never push to `master` from this task.
- Keep new log lines at sensible levels (CLAUDE.md): the link-lost cases at `warn` (tester-visible at the
  default level), the connect milestone at `info`.
- Delete this file (`docs/dev/BLE_LINUX_NATIVE_IMPL.md`) before the branch is merged.
