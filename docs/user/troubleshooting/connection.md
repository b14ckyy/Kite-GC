# Connection problems

Can't get connected? Work through the checks below — they're ordered from most to least common. For
how the connection controls work in the first place, see the **[Connecting guide](../guides/connecting.md)**.

## No COM port / serial device in the list

- **Plug the FC in directly** with a known-good **data** USB cable (some cables are charge-only) and
  wait a few seconds for the OS to enumerate it.
- **Install the USB-serial driver.** Boards using a CP210x or CH340 USB-to-serial chip need a one-time
  driver from the chip vendor (Silicon Labs / WCH). After installing it, unplug and replug the board.
- **Confirm it's the FC.** Open the port dropdown, unplug the board, reopen the list — the entry that
  disappears is your flight controller. Plug it back in and select that one.
- On **Linux**, your user must be allowed to use serial devices — add yourself to the `dialout` group
  (`sudo usermod -aG dialout $USER`) and log out/in once.

## It connects to the wrong thing / nothing happens

- **Match the protocol to the firmware:** **MSP** for INAV, **MAVLink** for ArduPilot/PX4. Connecting
  with the wrong protocol won't handshake.
- **Check the baud rate.** Kite presets **115200** for MSP and **57600** for MAVLink. If you changed
  the FC's port speed, set the matching rate here. A wrong baud is the most common "connects but no
  data" cause on serial links.

## "Port already in use" / access denied

A serial port can be open in **only one application at a time**. If **INAV Configurator**, **Mission
Planner**, **QGroundControl**, or another GCS still holds the port, disconnect it there first. A stale
connection from a crashed app sometimes needs a replug (or, rarely, a reboot) to release the port.

## Bluetooth (SPP) won't connect

Classic-Bluetooth serial (SPP) links have a couple of quirks Kite already smooths over, plus one you
may still hit:

- **Pair the device in the OS first.** It then appears as a normal COM port (Windows) or `rfcomm`/`tty`
  device (Linux).
- **Use the right port.** A paired SPP device creates *two* COM ports on Windows — outgoing and
  incoming. Kite **hides the incoming (server) one** automatically, so just pick the port it shows.
- **First open fails with a "semaphore timeout" (Windows error 121).** This is a known Bluetooth
  behaviour: the very first open can time out while the radio brings the RFCOMM channel up. Kite
  **retries the open automatically** (three attempts), so it usually succeeds on the second try. If it
  still fails:
    - Make sure the device is **paired and in range**, and not connected in another app.
    - Try connecting **once more** — a cold radio sometimes needs a moment.
    - If another GCS (e.g. Mission Planner) *can* open the same port but Kite can't, grab the log (see
      below) and share it — it records the exact OS error and how each Bluetooth port was classified.

!!! note "BLE is different"
    The above is for **classic Bluetooth SPP** (a COM port). **Bluetooth Low Energy** adapters use the
    **BLE** transport instead — see the [Connecting guide](../guides/connecting.md#ble-bluetooth-low-energy).

## Network (TCP/UDP) links

- **Host and port** must match your endpoint. Kite defaults to **TCP 5761** and **UDP 14550**; a
  simulator or router may use something else (e.g. **SITL on 5762**).
- Make sure the bridge/router/simulator is actually **running and reachable** (firewall, same network),
  and that the **protocol** matches what it speaks.

## The link drops shortly after connecting

- **Power.** A board running only off USB may brown out when GPS/peripherals draw current — power the
  craft from its normal supply (**props off** on the bench).
- **Cable / radio quality.** A marginal USB cable or a weak Bluetooth/telemetry link causes dropouts;
  try a different cable or move closer.

## Getting a diagnostic log

Kite writes a plain-text log you can hand back when something won't connect.

1. Open **Settings → Diagnostics**.
2. Set **Log Level** to **Debug (verbose)** to capture the full connection sequence (the Bluetooth
   diagnostics are recorded even at the default **Warnings** level).
3. Reproduce the failed connection.
4. Click **Log File → Open Folder** and grab the log.

The log lives in your app-data folder and the previous run is always kept alongside it:

| | Location |
|---|---|
| **Windows** | `%APPDATA%\kite-gc\kite-gc.log` |
| **Linux** | `~/.local/share/kite-gc/kite-gc.log` |
| **Portable** | `data\kite-gc.log` (next to the executable) |

The prior session is rotated to `kite-gc.log.prev`, so a failure you only notice after restarting Kite
isn't lost.

!!! tip "Sharing a log"
    The log contains port names and firmware/version strings but no personal data. Attach it to a
    GitHub issue or the relevant chat when reporting a connection problem — it usually pinpoints the
    cause immediately.
