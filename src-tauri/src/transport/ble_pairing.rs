// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Programmatic PIN pairing for BLE modules that send a security request on connect and drop the link
// when it stays unanswered (older FrSky modules). They do not keep the bond across a disconnect, so
// every connection pairs anew. Windows pairs before the connect and removes the bond afterwards; on
// Linux the kernel pairs during the connect and only needs our agent for the passkey (see `imp`).

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
    use std::sync::Mutex;

    use windows::core::HSTRING;
    use windows::Devices::Bluetooth::BluetoothLEDevice;
    use windows::Devices::Enumeration::{
        DeviceInformationCustomPairing, DevicePairingKinds, DevicePairingRequestedEventArgs,
        DevicePairingResultStatus,
    };
    use windows::Foundation::TypedEventHandler;

    /// Bonds this run made and has not removed yet; the exit hook clears what a dropped task left.
    static BONDED: Mutex<Vec<u64>> = Mutex::new(Vec::new());

    /// Keeps the paired link open; a stale bond makes the module drop every later connection.
    pub struct Bond {
        _device: BluetoothLEDevice,
        address: u64,
    }

    impl Bond {
        pub async fn release(self) {
            let _ = tauri::async_runtime::spawn_blocking(move || {
                release_address(self.address);
                drop(self._device);
            })
            .await;
        }
    }

    fn release_address(address: u64) {
        if let Err(e) = unpair_address(address) {
            log::warn!("[ble-pair] unpair failed: {e}");
        }
        if let Ok(mut list) = BONDED.lock() {
            list.retain(|a| *a != address);
        }
    }

    /// Unpairs through a new device object: the pairing state of an older one is a stale snapshot.
    fn unpair_address(address: u64) -> windows::core::Result<()> {
        let device = BluetoothLEDevice::FromBluetoothAddressAsync(address)?.get()?;
        device
            .DeviceInformation()?
            .Pairing()?
            .UnpairAsync()?
            .get()?;
        Ok(())
    }

    /// For the exit hook: Tauri drops the running task without its cleanup.
    pub fn release_all_blocking() {
        let addresses = BONDED.lock().map(|l| l.clone()).unwrap_or_default();
        for address in addresses {
            release_address(address);
        }
    }

    fn pair_blocking(
        address: u64,
        pin: &str,
    ) -> windows::core::Result<(BluetoothLEDevice, DevicePairingResultStatus)> {
        unpair_address(address)?;
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
        let (device, status) =
            tauri::async_runtime::spawn_blocking(move || pair_blocking(address, &pin))
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| format!("pairing failed: {e}"))?;
        if status == DevicePairingResultStatus::Paired
            || status == DevicePairingResultStatus::AlreadyPaired
        {
            if let Ok(mut list) = BONDED.lock() {
                list.push(address);
            }
            Ok(Bond {
                _device: device,
                address,
            })
        } else {
            Err(format!("pairing refused (status {})", status.0))
        }
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use zbus::zvariant::ObjectPath;
    use zbus::{interface, Connection, Proxy};

    const AGENT_PATH: &str = "/org/kitegc/pairing_agent";

    /// `Device1.Pair()` cannot pair the older module: BlueZ offers Secure Connections in the pairing
    /// request and the module's BT 4.0 stack then rejects the right passkey ("confirm value failed").
    /// The kernel answers the module's own security request with a legacy pairing the module
    /// accepts, and asks bluetoothd's DEFAULT agent for the passkey. So the agent becomes the default
    /// agent while the link comes up; it answers only for that one radio and rejects everything else.
    struct Agent {
        pin: String,
        device: String,
    }

    #[derive(Debug, zbus::DBusError)]
    #[zbus(prefix = "org.bluez.Error")]
    enum AgentError {
        #[zbus(error)]
        ZBus(zbus::Error),
        Rejected(String),
    }

    impl Agent {
        fn check(&self, device: &ObjectPath<'_>) -> Result<(), AgentError> {
            if device.as_str() == self.device {
                Ok(())
            } else {
                Err(AgentError::Rejected("not the trainer radio".into()))
            }
        }
    }

    #[interface(name = "org.bluez.Agent1")]
    impl Agent {
        fn release(&self) {}
        fn request_pin_code(&self, device: ObjectPath<'_>) -> Result<String, AgentError> {
            self.check(&device)?;
            Ok(self.pin.clone())
        }
        fn request_passkey(&self, device: ObjectPath<'_>) -> Result<u32, AgentError> {
            self.check(&device)?;
            Ok(self.pin.parse().unwrap_or(0))
        }
        fn display_passkey(&self, _device: ObjectPath<'_>, _passkey: u32, _entered: u16) {}
        fn display_pin_code(&self, _device: ObjectPath<'_>, _pincode: &str) {}
        fn request_confirmation(
            &self,
            device: ObjectPath<'_>,
            _passkey: u32,
        ) -> Result<(), AgentError> {
            self.check(&device)
        }
        fn request_authorization(&self, device: ObjectPath<'_>) -> Result<(), AgentError> {
            self.check(&device)
        }
        fn authorize_service(&self, device: ObjectPath<'_>, _uuid: &str) -> Result<(), AgentError> {
            self.check(&device)
        }
        fn cancel(&self) {}
    }

    /// Holds the D-Bus connection the agent lives on; BlueZ drops the agent with it and the previous
    /// default agent (the desktop's, if it has one) takes over again.
    pub struct Bond {
        conn: Connection,
    }

    /// BlueZ has no unpair: removing the device object is the way, and it is found again by the next scan.
    async fn remove_device(conn: &Connection, device: &str) -> zbus::Result<()> {
        let adapter = device
            .rsplit_once('/')
            .map(|(a, _)| a)
            .unwrap_or("/org/bluez/hci0")
            .to_string();
        let proxy = Proxy::new(conn, "org.bluez", adapter, "org.bluez.Adapter1").await?;
        proxy
            .call_method("RemoveDevice", &(ObjectPath::try_from(device)?,))
            .await?;
        Ok(())
    }

    async fn agent_manager(conn: &Connection) -> zbus::Result<Proxy<'static>> {
        Proxy::new(conn, "org.bluez", "/org/bluez", "org.bluez.AgentManager1").await
    }

    impl Bond {
        /// The module asks for no bond, so nothing is left to remove: only the agent goes.
        pub async fn release(self) {
            let result = async {
                agent_manager(&self.conn)
                    .await?
                    .call_method("UnregisterAgent", &(ObjectPath::try_from(AGENT_PATH)?,))
                    .await
            }
            .await;
            if let Err(e) = result {
                log::warn!("[ble-pair] unregistering the pairing agent failed: {e}");
            }
        }
    }

    async fn stand_by(device: &str, pin: &str) -> zbus::Result<Bond> {
        let conn = Connection::system().await?;
        // A bond from elsewhere (the desktop's Bluetooth settings) makes the module drop the link.
        let proxy = Proxy::new(&conn, "org.bluez", device.to_string(), "org.bluez.Device1").await?;
        if super::flagged(device.trim_start_matches("/org/bluez/"))
            && proxy.get_property::<bool>("Paired").await.unwrap_or(false)
        {
            remove_device(&conn, device).await?;
            return Err(zbus::Error::Failure(
                "stale bond removed, the radio is looked for again".into(),
            ));
        }
        conn.object_server()
            .at(
                AGENT_PATH,
                Agent {
                    pin: pin.to_string(),
                    device: device.to_string(),
                },
            )
            .await?;
        let manager = agent_manager(&conn).await?;
        let path = ObjectPath::try_from(AGENT_PATH)?;
        manager
            .call_method("RegisterAgent", &(&path, "KeyboardOnly"))
            .await?;
        manager.call_method("RequestDefaultAgent", &(&path,)).await?;
        Ok(Bond { conn })
    }

    /// Puts the passkey agent in place for the next connect; the kernel pairs on the module's request.
    /// btleplug's peripheral id on Linux is the BlueZ object path without `/org/bluez/`.
    pub async fn pair(device_id: &str, pin: &str) -> Result<Bond, String> {
        let device = format!("/org/bluez/{device_id}");
        stand_by(&device, pin)
            .await
            .map_err(|e| format!("pairing agent failed: {e}"))
    }

    pub fn release_all_blocking() {}
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
mod imp {
    pub struct Bond;

    impl Bond {
        pub async fn release(self) {}
    }

    /// CoreBluetooth has no pairing call: macOS shows its own dialog when the module asks.
    pub async fn pair(_device_id: &str, _pin: &str) -> Result<Bond, String> {
        Err("this system offers no programmatic BLE pairing".into())
    }

    pub fn release_all_blocking() {}
}

/// Linux: the module hangs up within half a second when the kernel's pairing finds no agent, before
/// its GATT can tell it apart. The agent therefore stands by on every trainer connect there; a radio
/// that never asks for a passkey does not notice it.
pub const AGENT_ON_EVERY_CONNECT: bool = cfg!(target_os = "linux");

/// Linux needs the agent only while the link is being secured; Windows keeps its device object open
/// for the whole session.
pub async fn link_up(bond: &mut Option<Bond>) {
    if cfg!(target_os = "linux") {
        if let Some(b) = bond.take() {
            b.release().await;
        }
    }
}

pub use imp::{pair, release_all_blocking, Bond};
