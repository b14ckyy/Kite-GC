// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Programmatic PIN pairing for BLE modules that send a security request on connect and drop the link
// when it stays unanswered (older FrSky modules). They do not keep the bond across a disconnect, so
// every connection pairs anew and the bond is removed again afterwards.

use std::sync::Mutex;

/// Fixed passkey of the FrSky Bluetooth modules.
pub const FRSKY_PIN: &str = "000000";

static NEEDS_PIN: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Mark a device (btleplug peripheral id) as one that must be paired before every connection.
pub fn flag(device_id: &str) {
    if let Ok(mut list) = NEEDS_PIN.lock() {
        if !list.iter().any(|d| d == device_id) {
            list.push(device_id.to_string());
        }
    }
}

pub fn flagged(device_id: &str) -> bool {
    NEEDS_PIN
        .lock()
        .map(|l| l.iter().any(|d| d == device_id))
        .unwrap_or(false)
}

#[cfg(target_os = "windows")]
mod imp {
    use windows::core::HSTRING;
    use windows::Devices::Bluetooth::BluetoothLEDevice;
    use windows::Devices::Enumeration::{
        DeviceInformationCustomPairing, DevicePairingKinds, DevicePairingRequestedEventArgs,
        DevicePairingResultStatus,
    };
    use windows::Foundation::TypedEventHandler;

    /// Keeps the paired link open; a stale bond makes the module drop every later connection.
    pub struct Bond {
        device: BluetoothLEDevice,
    }

    impl Bond {
        pub async fn release(self) {
            let _ = tokio::task::spawn_blocking(move || {
                if let Err(e) = unpair(&self.device) {
                    log::warn!("[ble-pair] unpair failed: {e}");
                }
            })
            .await;
        }
    }

    fn unpair(device: &BluetoothLEDevice) -> windows::core::Result<()> {
        let pairing = device.DeviceInformation()?.Pairing()?;
        if pairing.IsPaired()? {
            pairing.UnpairAsync()?.get()?;
        }
        Ok(())
    }

    fn pair_blocking(
        address: u64,
        pin: &str,
    ) -> windows::core::Result<(BluetoothLEDevice, DevicePairingResultStatus)> {
        let stale = BluetoothLEDevice::FromBluetoothAddressAsync(address)?.get()?;
        unpair(&stale)?;
        drop(stale);
        // The pairing state is a snapshot of the device object: take a new one after the unpair.
        let device = BluetoothLEDevice::FromBluetoothAddressAsync(address)?.get()?;
        let custom = device.DeviceInformation()?.Pairing()?.Custom()?;
        let pin = HSTRING::from(pin);
        let token = custom.PairingRequested(&TypedEventHandler::<
            DeviceInformationCustomPairing,
            DevicePairingRequestedEventArgs,
        >::new(move |_, args| {
            if let Some(args) = args.as_ref() {
                if args.PairingKind()? == DevicePairingKinds::ProvidePin {
                    args.AcceptWithPin(&pin)?;
                }
            }
            Ok(())
        }))?;
        // PIN only: a module that pairs without one fails here and gets no bond.
        let result = custom.PairAsync(DevicePairingKinds::ProvidePin)?.get();
        let _ = custom.RemovePairingRequested(token);
        let status = result?.Status()?;
        Ok((device, status))
    }

    /// btleplug's peripheral id on Windows is the Bluetooth address (`AA:BB:CC:DD:EE:FF`).
    pub async fn pair(device_id: &str, pin: &str) -> Result<Bond, String> {
        let address = u64::from_str_radix(&device_id.replace(':', ""), 16)
            .map_err(|_| format!("'{device_id}' is not a Bluetooth address"))?;
        let pin = pin.to_string();
        let (device, status) = tokio::task::spawn_blocking(move || pair_blocking(address, &pin))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| format!("pairing failed: {e}"))?;
        if status == DevicePairingResultStatus::Paired
            || status == DevicePairingResultStatus::AlreadyPaired
        {
            Ok(Bond { device })
        } else {
            Err(format!("pairing refused (status {})", status.0))
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod imp {
    pub struct Bond;

    impl Bond {
        pub async fn release(self) {}
    }

    pub async fn pair(_device_id: &str, _pin: &str) -> Result<Bond, String> {
        Err("programmatic BLE pairing is only implemented on Windows".into())
    }
}

pub use imp::{pair, Bond};
