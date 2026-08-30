package com.kitegc.app

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.hardware.usb.UsbConstants
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbDeviceConnection
import android.hardware.usb.UsbEndpoint
import android.hardware.usb.UsbInterface
import android.hardware.usb.UsbManager
import android.os.Build
import androidx.core.content.ContextCompat
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger

/**
 * USB-serial link for the Android build, driven over the Android USB Host API.
 *
 * The Rust side (`transport/serial_android.rs`) calls straight into the static methods here over JNI.
 * Everything crossing that boundary is a primitive, a String or a ByteArray on purpose: no JNI object
 * modelling, no callbacks back into Rust, and one error string to fetch when a call fails.
 *
 * Two device families are driven, chosen by descriptor:
 *   - **CDC-ACM** — every INAV / Betaflight / ArduPilot flight controller connected directly by USB,
 *     plus ESP32-S2/S3/C3 native-USB links.
 *   - **CP210x** — Silicon Labs bridges, which is what most SiK telemetry radios use.
 * FTDI and CH340 are deliberately absent rather than guessed at: FTDI prefixes every read packet with
 * two modem-status bytes that must be stripped, and CH340's baud divisors are a magic table. Both slot
 * in as another [SerialDriver] when someone can test against the hardware.
 *
 * Android grants USB access per device and per session. [open] requests it and blocks on the system
 * dialog; the `USB_DEVICE_ATTACHED` intent filter in the manifest covers the nicer path where the user
 * picks Kite when they plug the cable in, which grants permission without a prompt.
 */
object UsbSerial {
    private const val ACTION_PERMISSION = "com.kitegc.app.USB_PERMISSION"

    /** How long [open] waits for the user to answer the permission dialog before giving up. */
    private const val PERMISSION_TIMEOUT_MS = 60_000L

    /** Internal driver read codes. Anything >= 0 is a byte count. */
    private const val ERR_TIMEOUT = -1
    private const val ERR_DISCONNECTED = -2

    /** Bound on the setup control transfers; these answer immediately or not at all. */
    private const val CONTROL_TIMEOUT_MS = 2000

    private lateinit var appContext: Context
    private val handles = ConcurrentHashMap<Int, SerialDriver>()
    private val nextHandle = AtomicInteger(1)

    /**
     * The reason each handle's most recent call failed, keyed by handle.
     *
     * Per handle, not one global string: Kite can hold several links at once — a flight controller on
     * USB while a SiK radio is on a second port — and each is driven by its own Rust thread. With a
     * single slot, a write failing on one link between another link's failed call and its `lastError`
     * fetch would hand the second thread the first one's message, which is the kind of bug that only
     * shows up as an inexplicable error string in someone's log.
     *
     * [NO_HANDLE] is the slot for failures that happen before a handle exists ([open], [listDevices])
     * or when the handle itself is the thing that was invalid. Those are inherently global, but they
     * are also the calls Rust makes one at a time from the connection command.
     */
    private val lastErrors = ConcurrentHashMap<Int, String>()

    /** Key in [lastErrors] for calls that have no handle yet. Handles start at 1, so 0 is free. */
    private const val NO_HANDLE = 0

    /**
     * Must be called before Tauri starts (see `MainActivity.onCreate`) — the Rust side has no Context
     * of its own and every call below needs one.
     */
    @JvmStatic
    fun init(context: Context) {
        appContext = context.applicationContext
    }

    /**
     * The reason [handle]'s most recent call failed. Fetched by Rust only after a call reports
     * failure. Pass [NO_HANDLE] (0) for [open] / [listDevices], which have no handle of their own.
     *
     * Reading clears the slot, so a stale message can never be reported against a later, unrelated
     * failure that somehow left nothing behind.
     */
    @JvmStatic
    fun lastError(handle: Int): String = lastErrors.remove(handle) ?: ""

    /**
     * Connectable devices as a JSON array of `{path, label, type}` — the same shape the desktop
     * backend's `PortInfo` serialises to, so the port picker needs no Android-specific handling.
     *
     * `path` is the kernel device node (`/dev/bus/usb/001/002`), which is what [open] takes. It is
     * assigned at enumeration, so it changes when the cable is re-plugged — the same way a desktop
     * `/dev/ttyUSB0` can move. Devices no driver here recognises are left out rather than offered as
     * ports that cannot open.
     */
    @JvmStatic
    fun listDevices(): String {
        val out = JSONArray()
        val manager = usbManager() ?: return out.toString()
        for (device in manager.deviceList.values) {
            if (driverKindFor(device) == null) continue
            val product = device.productName
            val vendor = device.manufacturerName
            val label = when {
                product != null && vendor != null -> "${device.deviceName} — $product ($vendor)"
                product != null -> "${device.deviceName} — $product"
                vendor != null -> "${device.deviceName} — $vendor"
                else -> device.deviceName
            }
            out.put(
                JSONObject()
                    .put("path", device.deviceName)
                    .put("label", label)
                    .put("type", "USB")
            )
        }
        return out.toString()
    }

    /**
     * Open [deviceName] at [baud]. Returns a handle > 0, or -1 with the reason in [lastError].
     *
     * Blocks while the system permission dialog is up, which is why Rust must never call this from a
     * thread that has to stay responsive — it is called from the connection command, off the UI
     * thread, exactly like the desktop `SerialConnection::open`.
     */
    @JvmStatic
    fun open(deviceName: String, baud: Int): Int {
        val manager = usbManager() ?: return fail(NO_HANDLE, "USB service unavailable")
        val device = manager.deviceList[deviceName]
            ?: return fail(NO_HANDLE, "USB device $deviceName is no longer connected")

        if (!manager.hasPermission(device) && !requestPermission(manager, device)) {
            return fail(NO_HANDLE, "USB permission denied for $deviceName")
        }

        val kind = driverKindFor(device) ?: return fail(NO_HANDLE, "Unsupported USB device $deviceName")
        val connection = manager.openDevice(device)
            ?: return fail(NO_HANDLE, "Could not open $deviceName (already claimed by another app?)")

        val driver = when (kind) {
            DriverKind.CDC_ACM -> CdcAcmDriver(device, connection)
            DriverKind.CP210X -> Cp210xDriver(device, connection)
        }

        try {
            driver.open(baud)
        } catch (e: Exception) {
            connection.close()
            return fail(NO_HANDLE, "Failed to configure $deviceName: ${e.message}")
        }

        val handle = nextHandle.getAndIncrement()
        handles[handle] = driver
        return handle
    }

    /**
     * Read into [buf]. Returns the byte count, 0 on timeout (no data, link still up) or -1 with
     * [lastError] set when the device has gone away — mirroring the desktop transport, where a timeout
     * is normal and a disconnect is terminal.
     */
    @JvmStatic
    fun read(handle: Int, buf: ByteArray, timeoutMs: Int): Int {
        val driver = handles[handle] ?: return fail(NO_HANDLE, "Invalid serial handle $handle")
        return when (val n = driver.read(buf, timeoutMs)) {
            ERR_TIMEOUT -> 0
            ERR_DISCONNECTED -> fail(handle, "USB device disconnected")
            else -> n
        }
    }

    /** Write all of [data]. Returns true on success; on failure see [lastError]. */
    @JvmStatic
    fun write(handle: Int, data: ByteArray, timeoutMs: Int): Boolean {
        val driver = handles[handle]
        if (driver == null) {
            lastErrors[NO_HANDLE] = "Invalid serial handle $handle"
            return false
        }
        return try {
            driver.write(data, timeoutMs)
            true
        } catch (e: Exception) {
            lastErrors[handle] = "USB write failed: ${e.message}"
            false
        }
    }

    /**
     * Raise or drop DTR/RTS. USB-CDC devices gate their device-to-host stream on DTR
     * (`tud_cdc_n_connected()`), so a link opened without it looks dead in exactly one direction —
     * the same trap the desktop backend hit.
     */
    @JvmStatic
    fun setControlLines(handle: Int, dtr: Boolean, rts: Boolean): Boolean {
        val driver = handles[handle] ?: return false
        return try {
            driver.setControlLines(dtr, rts)
            true
        } catch (e: Exception) {
            lastErrors[handle] = "Setting DTR/RTS failed: ${e.message}"
            false
        }
    }

    /** Close and forget the handle. Safe to call on an already-closed or unknown handle. */
    @JvmStatic
    fun close(handle: Int) {
        lastErrors.remove(handle)
        handles.remove(handle)?.let {
            try {
                it.close()
            } catch (_: Exception) {
                // Closing a device that is already physically gone throws; nothing left to release.
            }
        }
    }

    // ── internals ────────────────────────────────────────────────────────────

    private fun usbManager(): UsbManager? {
        if (!::appContext.isInitialized) {
            lastErrors[NO_HANDLE] = "UsbSerial.init() was never called"
            return null
        }
        return appContext.getSystemService(Context.USB_SERVICE) as? UsbManager
    }

    private fun fail(handle: Int, message: String): Int {
        lastErrors[handle] = message
        return -1
    }

    /**
     * Ask for access to [device] and wait for the answer.
     *
     * The PendingIntent must be mutable: the system fills the granted device and result into it, and
     * an immutable one silently never reports a grant. The receiver is registered NOT_EXPORTED because
     * the broadcast comes back to this app only — required from Android 14, harmless before it.
     */
    private fun requestPermission(manager: UsbManager, device: UsbDevice): Boolean {
        val latch = CountDownLatch(1)
        var granted = false

        val receiver = object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) {
                if (intent.action != ACTION_PERMISSION) return
                granted = intent.getBooleanExtra(UsbManager.EXTRA_PERMISSION_GRANTED, false)
                latch.countDown()
            }
        }

        ContextCompat.registerReceiver(
            appContext,
            receiver,
            IntentFilter(ACTION_PERMISSION),
            ContextCompat.RECEIVER_NOT_EXPORTED
        )

        try {
            var flags = PendingIntent.FLAG_UPDATE_CURRENT
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                flags = flags or PendingIntent.FLAG_MUTABLE
            }
            val intent = PendingIntent.getBroadcast(
                appContext,
                0,
                Intent(ACTION_PERMISSION).setPackage(appContext.packageName),
                flags
            )
            manager.requestPermission(device, intent)
            latch.await(PERMISSION_TIMEOUT_MS, TimeUnit.MILLISECONDS)
        } finally {
            try {
                appContext.unregisterReceiver(receiver)
            } catch (_: IllegalArgumentException) {
                // Already gone — nothing to do.
            }
        }

        // `hasPermission` is the authority: the user may have granted it through the attach dialog
        // while we were waiting, in which case no broadcast ever arrives.
        return granted || manager.hasPermission(device)
    }

    private enum class DriverKind { CDC_ACM, CP210X }

    /** Silicon Labs. Every CP2102/CP2104/CP2105 shares this vendor id. */
    private const val VENDOR_SILABS = 0x10C4

    /**
     * Pick a driver from the descriptors, CDC first.
     *
     * The vendor check is the *fallback*, not the first test. Silicon Labs' newer parts (CP2102N in
     * its CDC configuration, and the EFM32/EFR32 boards that reuse the vendor id) enumerate as
     * standard CDC-ACM while still reporting vendor 0x10C4. Matching the vendor first would send
     * those down the CP210x path, where the vendor control transfers they do not implement fail and
     * `open` reports "control transfer 0x00 failed" for a device that plain CDC would have driven
     * without trouble. Descriptors are authoritative; the vendor id is only a hint, and it is only
     * needed for the classic CP210x parts, which declare a vendor-specific class and so match nothing
     * above.
     */
    private fun driverKindFor(device: UsbDevice): DriverKind? {
        if (device.deviceClass == UsbConstants.USB_CLASS_COMM) return DriverKind.CDC_ACM
        // Composite devices report class 0 at the device level and declare the real class per
        // interface — which is what a flight controller exposing CDC alongside MSC/DFU looks like.
        for (i in 0 until device.interfaceCount) {
            if (device.getInterface(i).interfaceClass == UsbConstants.USB_CLASS_COMM) {
                return DriverKind.CDC_ACM
            }
        }
        if (device.vendorId == VENDOR_SILABS) return DriverKind.CP210X
        return null
    }

    private interface SerialDriver {
        fun open(baud: Int)
        fun setControlLines(dtr: Boolean, rts: Boolean)
        /** Byte count, or [ERR_TIMEOUT] / [ERR_DISCONNECTED]. */
        fun read(buf: ByteArray, timeoutMs: Int): Int
        fun write(data: ByteArray, timeoutMs: Int)
        fun close()
    }

    /**
     * Shared bulk-endpoint plumbing. Both drivers move data the same way once configured — they differ
     * only in how the line is set up, which is what each subclass overrides.
     */
    private abstract class BulkDriver(
        protected val device: UsbDevice,
        protected val connection: UsbDeviceConnection
    ) : SerialDriver {
        protected val claimed = mutableListOf<UsbInterface>()
        protected var readEndpoint: UsbEndpoint? = null
        protected var writeEndpoint: UsbEndpoint? = null

        /** Claim [iface] and record it so [close] can release everything it took. */
        protected fun claim(iface: UsbInterface) {
            if (!connection.claimInterface(iface, true)) {
                throw IllegalStateException("could not claim interface ${iface.id}")
            }
            claimed.add(iface)
        }

        /** Pick the first bulk IN/OUT pair on [iface]; leaves anything already found untouched. */
        protected fun collectBulkEndpoints(iface: UsbInterface) {
            for (i in 0 until iface.endpointCount) {
                val ep = iface.getEndpoint(i)
                if (ep.type != UsbConstants.USB_ENDPOINT_XFER_BULK) continue
                if (ep.direction == UsbConstants.USB_DIR_IN && readEndpoint == null) readEndpoint = ep
                if (ep.direction == UsbConstants.USB_DIR_OUT && writeEndpoint == null) writeEndpoint = ep
            }
        }

        protected fun requireEndpoints() {
            if (readEndpoint == null || writeEndpoint == null) {
                throw IllegalStateException("device exposes no bulk IN/OUT endpoint pair")
            }
        }

        protected fun control(requestType: Int, request: Int, value: Int, index: Int, data: ByteArray?) {
            val rc = connection.controlTransfer(
                requestType, request, value, index, data, data?.size ?: 0, CONTROL_TIMEOUT_MS
            )
            if (rc < 0) {
                throw IllegalStateException("control transfer 0x%02x failed".format(request))
            }
        }

        override fun read(buf: ByteArray, timeoutMs: Int): Int {
            val ep = readEndpoint ?: return ERR_DISCONNECTED
            val n = connection.bulkTransfer(ep, buf, buf.size, timeoutMs)
            if (n >= 0) return n
            // bulkTransfer cannot tell "nothing arrived in time" from "the device vanished", so ask
            // the platform which one it is. Without this a yanked cable would look like a permanently
            // idle link and the connection would never be reported as lost.
            return if (isStillAttached()) ERR_TIMEOUT else ERR_DISCONNECTED
        }

        override fun write(data: ByteArray, timeoutMs: Int) {
            val ep = writeEndpoint ?: throw IllegalStateException("no write endpoint")
            var offset = 0
            while (offset < data.size) {
                val chunk = minOf(ep.maxPacketSize, data.size - offset)
                val slice = data.copyOfRange(offset, offset + chunk)
                val n = connection.bulkTransfer(ep, slice, chunk, timeoutMs)
                if (n < 0) throw IllegalStateException("bulk write failed at byte $offset")
                offset += n
            }
        }

        override fun close() {
            for (iface in claimed) connection.releaseInterface(iface)
            claimed.clear()
            connection.close()
        }

        private fun isStillAttached(): Boolean {
            val manager = usbManager() ?: return false
            return manager.deviceList.containsKey(device.deviceName)
        }
    }

    /**
     * USB CDC-ACM (class 0x02 control + 0x0A data), the class every mainstream flight controller
     * enumerates as.
     */
    private class CdcAcmDriver(device: UsbDevice, connection: UsbDeviceConnection) :
        BulkDriver(device, connection) {

        private var controlInterfaceId = 0

        override fun open(baud: Int) {
            var control: UsbInterface? = null
            var data: UsbInterface? = null
            for (i in 0 until device.interfaceCount) {
                val iface = device.getInterface(i)
                when (iface.interfaceClass) {
                    UsbConstants.USB_CLASS_COMM -> if (control == null) control = iface
                    UsbConstants.USB_CLASS_CDC_DATA -> if (data == null) data = iface
                }
            }
            // Some minimal CDC implementations merge both roles into a single interface; fall back to
            // whichever interface actually carries a bulk pair rather than refusing the device.
            if (data == null) {
                for (i in 0 until device.interfaceCount) {
                    val iface = device.getInterface(i)
                    var bulk = 0
                    for (e in 0 until iface.endpointCount) {
                        if (iface.getEndpoint(e).type == UsbConstants.USB_ENDPOINT_XFER_BULK) bulk++
                    }
                    if (bulk >= 2) { data = iface; break }
                }
            }
            val dataInterface = data ?: throw IllegalStateException("no CDC data interface")

            control?.let { claim(it); controlInterfaceId = it.id }
            claim(dataInterface)
            collectBulkEndpoints(dataInterface)
            requireEndpoints()

            setLineCoding(baud)
        }

        /** SET_LINE_CODING (0x20): baud LE32, stop bits, parity, data bits — fixed at 8-N-1. */
        private fun setLineCoding(baud: Int) {
            val payload = byteArrayOf(
                (baud and 0xFF).toByte(),
                ((baud shr 8) and 0xFF).toByte(),
                ((baud shr 16) and 0xFF).toByte(),
                ((baud shr 24) and 0xFF).toByte(),
                0, // 1 stop bit
                0, // no parity
                8  // 8 data bits
            )
            control(REQTYPE_HOST_TO_DEVICE_CLASS, SET_LINE_CODING, 0, controlInterfaceId, payload)
        }

        /** SET_CONTROL_LINE_STATE (0x22): bit 0 = DTR, bit 1 = RTS. */
        override fun setControlLines(dtr: Boolean, rts: Boolean) {
            val value = (if (dtr) 0x01 else 0) or (if (rts) 0x02 else 0)
            control(REQTYPE_HOST_TO_DEVICE_CLASS, SET_CONTROL_LINE_STATE, value, controlInterfaceId, null)
        }

        private companion object {
            /** Host→device | class | interface. */
            const val REQTYPE_HOST_TO_DEVICE_CLASS = 0x21
            const val SET_LINE_CODING = 0x20
            const val SET_CONTROL_LINE_STATE = 0x22
        }
    }

    /** Silicon Labs CP210x — the bridge in most SiK telemetry radios. */
    private class Cp210xDriver(device: UsbDevice, connection: UsbDeviceConnection) :
        BulkDriver(device, connection) {

        override fun open(baud: Int) {
            val iface = device.getInterface(0)
            claim(iface)
            collectBulkEndpoints(iface)
            requireEndpoints()

            control(REQTYPE_HOST_TO_DEVICE_VENDOR, IFC_ENABLE, UART_ENABLE, 0, null)
            // SET_BAUDRATE takes the rate itself as a 4-byte LE payload (the older SET_BAUDDIV
            // divisor request is not used on any part still in the field).
            control(
                REQTYPE_HOST_TO_DEVICE_VENDOR, SET_BAUDRATE, 0, 0,
                byteArrayOf(
                    (baud and 0xFF).toByte(),
                    ((baud shr 8) and 0xFF).toByte(),
                    ((baud shr 16) and 0xFF).toByte(),
                    ((baud shr 24) and 0xFF).toByte()
                )
            )
            // SET_LINE_CTL packs data bits in the high byte, parity in bits 4-7, stop bits in 0-3:
            // 0x0800 = 8 data bits, no parity, 1 stop bit.
            control(REQTYPE_HOST_TO_DEVICE_VENDOR, SET_LINE_CTL, LINE_CTL_8N1, 0, null)
        }

        /** SET_MHS: low byte sets DTR/RTS, high byte is the write mask for those same bits. */
        override fun setControlLines(dtr: Boolean, rts: Boolean) {
            val state = (if (dtr) 0x01 else 0) or (if (rts) 0x02 else 0)
            control(REQTYPE_HOST_TO_DEVICE_VENDOR, SET_MHS, 0x0300 or state, 0, null)
        }

        private companion object {
            /** Host→device | vendor | device. */
            const val REQTYPE_HOST_TO_DEVICE_VENDOR = 0x41
            const val IFC_ENABLE = 0x00
            const val SET_LINE_CTL = 0x03
            const val SET_MHS = 0x07
            const val SET_BAUDRATE = 0x1E
            const val UART_ENABLE = 0x0001
            const val LINE_CTL_8N1 = 0x0800
        }
    }
}
