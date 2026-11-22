// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

package com.kitegc.app

import android.Manifest
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattDescriptor
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothProfile
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import androidx.activity.result.ActivityResultLauncher
import androidx.core.content.ContextCompat
import org.json.JSONArray
import org.json.JSONObject
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CountDownLatch
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger

/** BLE-serial link on the platform GATT stack, driven over JNI by transport/ble_android.rs ([UsbSerial] contract).
 *  Rust picks the profile's UUIDs; only [listen] chooses characteristics here. GATT callbacks arrive on
 *  binder threads; every wait is a latch with a timeout. */
object BleSerial {
    private const val TAG = "BleSerial"

    /** Sentinel handle for errors that belong to no link (scan, permissions). */
    private const val NO_HANDLE = -1

    private const val PERMISSION_TIMEOUT_MS = 60_000L
    private const val CONNECT_TIMEOUT_MS = 15_000L
    private const val DISCOVERY_TIMEOUT_MS = 10_000L
    private const val DESCRIPTOR_TIMEOUT_MS = 5_000L
    private const val MTU_TIMEOUT_MS = 2_000L

    /** Largest MTU worth asking for (BLE 4.2+ caps at 247 → 244 payload bytes per write). */
    private const val REQUESTED_MTU = 247

    /** Client Characteristic Configuration descriptor — the "turn notifications on" switch. */
    private val CCCD: UUID = UUID.fromString("00002902-0000-1000-8000-00805f9b34fb")

    /** Generic Access, Generic Attribute, Device Information: never telemetry, and subscribing to
     *  Service Changed (0x2A05) makes some stacks ask for pairing. */
    private val STANDARD_SERVICES: Set<UUID> = setOf("1800", "1801", "180a")
        .map { UUID.fromString("0000$it-0000-1000-8000-00805f9b34fb") }.toSet()

    private lateinit var activity: MainActivity
    private var permissionLauncher: ActivityResultLauncher<Array<String>>? = null
    private var permissionLatch: CountDownLatch? = null
    @Volatile private var permissionGranted = false

    private val handles = ConcurrentHashMap<Int, Link>()
    private val nextHandle = AtomicInteger(1)
    private val lastErrors = ConcurrentHashMap<Int, String>()

    // ── Scan state ───────────────────────────────────────────────────────────────────────────────

    private class Found(val name: String, @Volatile var rssi: Int, val services: MutableSet<String>)

    private val found = ConcurrentHashMap<String, Found>()
    private var scanCallback: ScanCallback? = null

    /** Wire up from [MainActivity.onCreate]; [launcher] is the runtime-permission contract, which
     *  must be registered as a field initializer (before the activity reaches STARTED). */
    fun init(activity: MainActivity, launcher: ActivityResultLauncher<Array<String>>) {
        this.activity = activity
        this.permissionLauncher = launcher
    }

    /** The permission dialog's answer (main thread). */
    fun onPermissionResult(result: Map<String, Boolean>) {
        permissionGranted = result.values.all { it }
        permissionLatch?.countDown()
    }

    /** Fetch-and-clear the error behind the last failed call for [handle] ([NO_HANDLE] for scan /
     *  permission failures). Empty when there is none. */
    @JvmStatic
    fun lastError(handle: Int): String = lastErrors.remove(handle) ?: ""

    /** Records [reason] without losing a drop the GATT callback recorded first: that one says why. */
    private fun fail(handle: Int, reason: String) {
        lastErrors.merge(handle, reason) { earlier, now -> "$now; $earlier" }
    }

    // ── Permissions ──────────────────────────────────────────────────────────────────────────────

    private fun requiredPermissions(): Array<String> =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            arrayOf(Manifest.permission.BLUETOOTH_SCAN, Manifest.permission.BLUETOOTH_CONNECT)
        } else {
            // Before Android 12 the Bluetooth permissions are install-time; scanning needs location.
            arrayOf(Manifest.permission.ACCESS_FINE_LOCATION)
        }

    private fun hasPermissions(): Boolean = requiredPermissions().all {
        ContextCompat.checkSelfPermission(activity, it) == PackageManager.PERMISSION_GRANTED
    }

    /**
     * Make sure the runtime permissions are held, raising the system dialog and waiting for the
     * answer if not. Blocks the calling (Rust worker) thread for up to a minute — the same shape as
     * the USB permission wait.
     */
    @JvmStatic
    fun ensurePermissions(): Boolean {
        if (hasPermissions()) return true
        val launcher = permissionLauncher ?: run {
            lastErrors[NO_HANDLE] = "permission launcher not initialised"
            return false
        }
        val latch = CountDownLatch(1)
        permissionLatch = latch
        permissionGranted = false
        activity.runOnUiThread { launcher.launch(requiredPermissions()) }
        try {
            if (!latch.await(PERMISSION_TIMEOUT_MS, TimeUnit.MILLISECONDS)) {
                lastErrors[NO_HANDLE] = "Bluetooth permission dialog timed out"
                return false
            }
        } catch (e: InterruptedException) {
            Thread.currentThread().interrupt()
            return false
        } finally {
            permissionLatch = null
        }
        if (!permissionGranted) lastErrors[NO_HANDLE] = "Bluetooth permission denied"
        return permissionGranted
    }

    private fun adapter(): BluetoothAdapter? {
        val mgr = activity.getSystemService(Context.BLUETOOTH_SERVICE) as? BluetoothManager
        return mgr?.adapter
    }

    private fun readyAdapter(handle: Int): BluetoothAdapter? {
        val a = adapter()
        if (a == null) {
            lastErrors[handle] = "this device has no Bluetooth adapter"
            return null
        }
        if (!a.isEnabled) {
            lastErrors[handle] = "Bluetooth is turned off"
            return null
        }
        return a
    }

    // ── Scanning ─────────────────────────────────────────────────────────────────────────────────

    /** Start a low-latency scan with no filter — most BLE-serial adapters advertise no service UUID at
     *  all, so a filtered scan would hide exactly the devices we are looking for. */
    @JvmStatic
    fun scanStart(): Boolean {
        val adapter = readyAdapter(NO_HANDLE) ?: return false
        val scanner = adapter.bluetoothLeScanner ?: run {
            lastErrors[NO_HANDLE] = "BLE scanner unavailable (Bluetooth off?)"
            return false
        }
        scanStop()
        found.clear()
        val cb = object : ScanCallback() {
            override fun onScanResult(callbackType: Int, result: ScanResult) = record(result)
            override fun onBatchScanResults(results: MutableList<ScanResult>) = results.forEach { record(it) }
            override fun onScanFailed(errorCode: Int) {
                lastErrors[NO_HANDLE] = "BLE scan failed (code $errorCode)"
            }
        }
        val settings = ScanSettings.Builder()
            .setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY)
            .build()
        return try {
            scanner.startScan(null, settings, cb)
            scanCallback = cb
            true
        } catch (e: SecurityException) {
            lastErrors[NO_HANDLE] = "BLE scan refused: ${e.message}"
            false
        }
    }

    private fun record(result: ScanResult) {
        val device = result.device ?: return
        val address = device.address ?: return
        val name = try {
            result.scanRecord?.deviceName ?: device.name ?: ""
        } catch (_: SecurityException) {
            result.scanRecord?.deviceName ?: ""
        }
        val services = result.scanRecord?.serviceUuids?.map { it.uuid.toString().lowercase() } ?: emptyList()
        val entry = found[address]
        if (entry == null) {
            found[address] = Found(name, result.rssi, services.toMutableSet())
        } else {
            entry.rssi = result.rssi
            entry.services.addAll(services)
        }
    }

    /** Everything seen since [scanStart], as JSON: `[{id, name, rssi, services:[…]}]`. Unnamed
     *  devices are left out — they cannot be told apart in a picker. */
    @JvmStatic
    fun scanPoll(): String {
        val arr = JSONArray()
        for ((address, f) in found) {
            if (f.name.isEmpty()) continue
            val o = JSONObject()
            o.put("id", address)
            o.put("name", f.name)
            o.put("rssi", f.rssi)
            o.put("services", JSONArray(f.services.toList()))
            arr.put(o)
        }
        return arr.toString()
    }

    @JvmStatic
    fun scanStop() {
        val cb = scanCallback ?: return
        scanCallback = null
        try {
            adapter()?.bluetoothLeScanner?.stopScan(cb)
        } catch (_: Exception) {
            // Adapter turned off mid-scan; nothing left to stop.
        }
    }

    // ── Connections ──────────────────────────────────────────────────────────────────────────────

    /** Blocking (≤ ~25 s): [newLink] + [openLink]. The serial characteristics are chosen afterwards by
     *  the Rust side via [services] + [subscribe]. */
    @JvmStatic
    fun connect(address: String): Int {
        val handle = newLink(address)
        if (handle < 0) return -1
        return if (openLink(handle)) handle else -1
    }

    /** Bluetooth on and, from Android 12, the connect permission held; no dialog. False with the reason
     *  in [lastError] ([NO_HANDLE]). */
    @JvmStatic
    fun canConnect(): Boolean {
        readyAdapter(NO_HANDLE) ?: return false
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S &&
            ContextCompat.checkSelfPermission(activity, Manifest.permission.BLUETOOTH_CONNECT) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            lastErrors[NO_HANDLE] = "Bluetooth permission revoked"
            return false
        }
        return true
    }

    /** A handle for [address] before any connect, so a [close] can cancel the [openLink] in progress.
     *  -1 with the reason in [lastError] ([NO_HANDLE]). */
    @JvmStatic
    fun newLink(address: String): Int {
        val adapter = readyAdapter(NO_HANDLE) ?: return -1
        val device: BluetoothDevice = try {
            adapter.getRemoteDevice(address)
        } catch (e: IllegalArgumentException) {
            lastErrors[NO_HANDLE] = "not a Bluetooth address: $address"
            return -1
        }
        val handle = nextHandle.getAndIncrement()
        handles[handle] = Link(handle, device)
        return handle
    }

    /** Connect [handle]'s link and discover its services; blocks up to ~25 s, or until [close]. False with
     *  the reason in [lastError] ([handle]), the link closed. */
    @JvmStatic
    fun openLink(handle: Int): Boolean {
        val link = handles[handle] ?: run { lastErrors[handle] = "link is closed"; return false }

        // connectGatt on the main thread: on a number of vendor stacks a background-thread call gets
        // a callback storm or none at all; the main thread is the one path every device supports.
        val started = CountDownLatch(1)
        activity.runOnUiThread {
            try {
                link.attach { link.device.connectGatt(activity, false, link, BluetoothDevice.TRANSPORT_LE) }
            } catch (e: SecurityException) {
                lastErrors[handle] = "connect refused: ${e.message}"
            }
            started.countDown()
        }
        started.await(5_000, TimeUnit.MILLISECONDS)
        if (link.gatt == null) {
            if (!lastErrors.containsKey(handle)) lastErrors[handle] = "connectGatt returned nothing"
            close(handle) // a connectGatt still queued on the main thread must not open a link
            return false
        }
        if (!link.connectLatch.await(CONNECT_TIMEOUT_MS, TimeUnit.MILLISECONDS) || !link.connected) {
            lastErrors[handle] = link.failure ?: "connect timed out"
            close(handle)
            return false
        }
        val gatt = link.gatt ?: run { lastErrors[handle] = "link is closed"; return false }
        if (!gatt.discoverServices()) {
            fail(handle, "service discovery could not be started")
            close(handle)
            return false
        }
        if (!link.servicesLatch.await(DISCOVERY_TIMEOUT_MS, TimeUnit.MILLISECONDS)) {
            fail(handle, "service discovery timed out")
            close(handle)
            return false
        }
        if (link.closed) {
            lastErrors[handle] = "link is closed"
            return false
        }
        return true
    }

    /** The connected device's service UUIDs as a JSON array of strings (lowercase, 128-bit form). */
    @JvmStatic
    fun services(handle: Int): String {
        val link = handles[handle] ?: return "[]"
        val arr = JSONArray()
        link.gatt?.services?.forEach { arr.put(it.uuid.toString().lowercase()) }
        return arr.toString()
    }

    /**
     * Turn the link into a serial port: subscribe to [readUuid] (notify or indicate, whichever the
     * characteristic offers) and remember [writeUuid] for [write]. Also negotiates a larger MTU —
     * best effort; a stack that refuses just keeps 20-byte chunks.
     */
    @JvmStatic
    fun subscribe(handle: Int, serviceUuid: String, readUuid: String, writeUuid: String): Boolean {
        val link = handles[handle] ?: run { lastErrors[handle] = "no such link"; return false }
        val gatt = link.gatt ?: run { lastErrors[handle] = "link is closed"; return false }
        val service = gatt.getService(UUID.fromString(serviceUuid)) ?: run {
            lastErrors[handle] = "service $serviceUuid not on device" + (link.failure?.let { " ($it)" } ?: "")
            return false
        }
        val readChar = service.getCharacteristic(UUID.fromString(readUuid)) ?: run {
            lastErrors[handle] = "read characteristic $readUuid not found"; return false
        }
        val writeChar = service.getCharacteristic(UUID.fromString(writeUuid)) ?: run {
            lastErrors[handle] = "write characteristic $writeUuid not found"; return false
        }

        // MTU first — before notifications, so the peripheral's first burst already uses it.
        link.mtuLatch = CountDownLatch(1)
        if (!link.closed && gatt.requestMtu(REQUESTED_MTU)) {
            link.mtuLatch?.await(MTU_TIMEOUT_MS, TimeUnit.MILLISECONDS)
        }

        enableNotifications(link, gatt, readChar)?.let { fail(handle, it); return false }

        link.writeChar = writeChar
        // Write-without-response where the adapter offers it: no round trip per chunk, which is
        // what a telemetry stream wants. Otherwise the acknowledged write, paced by its callback.
        link.writeType = if (writeChar.properties and BluetoothGattCharacteristic.PROPERTY_WRITE_NO_RESPONSE != 0) {
            BluetoothGattCharacteristic.WRITE_TYPE_NO_RESPONSE
        } else {
            BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT
        }
        // The two numbers that make one device's BLE link behave differently from another's — worth
        // a logcat line whenever a link comes up, so a field comparison needs no extra build.
        android.util.Log.i(
            TAG,
            "link $handle up: mtu=${link.mtu} writeType=" +
                (if (link.writeType == BluetoothGattCharacteristic.WRITE_TYPE_NO_RESPONSE) "no-response" else "acknowledged")
        )
        return true
    }

    /** Null on success, else the reason for [lastErrors]. */
    private fun enableNotifications(link: Link, gatt: BluetoothGatt, ch: BluetoothGattCharacteristic): String? {
        if (!gatt.setCharacteristicNotification(ch, true)) return "could not enable notifications locally"
        val cccd = ch.getDescriptor(CCCD) ?: return "characteristic has no CCCD descriptor"
        val props = ch.properties
        val value = if (props and BluetoothGattCharacteristic.PROPERTY_NOTIFY != 0) {
            BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE
        } else if (props and BluetoothGattCharacteristic.PROPERTY_INDICATE != 0) {
            BluetoothGattDescriptor.ENABLE_INDICATION_VALUE
        } else {
            return "characteristic neither notifies nor indicates"
        }
        // One descriptor write at a time: the stack answers a second one with "busy".
        val latch = CountDownLatch(1)
        link.pendingCccd = cccd
        link.descriptorStatus = BluetoothGatt.GATT_SUCCESS
        link.descriptorLatch = latch
        // Checked after publishing the latch: a close() that missed it has set the flag by now.
        if (link.closed) return "link is closed"
        val ok = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            gatt.writeDescriptor(cccd, value) == BluetoothGatt.GATT_SUCCESS
        } else {
            @Suppress("DEPRECATION")
            run { cccd.value = value; gatt.writeDescriptor(cccd) }
        }
        if (!ok) return "CCCD write could not be started"
        if (!latch.await(DESCRIPTOR_TIMEOUT_MS, TimeUnit.MILLISECONDS)) return "enabling notifications timed out"
        return if (link.closed) "link is closed" else null
    }

    /** The device's answer to the last CCCD write; a refusal (e.g. pairing required) leaves the link silent. */
    private fun cccdRefusal(link: Link): String? =
        if (link.descriptorStatus == BluetoothGatt.GATT_SUCCESS) null
        else "device refused the notification switch (GATT status ${link.descriptorStatus})"

    /** Subscribes to every notifying characteristic outside [STANDARD_SERVICES], all feeding [read]; no write
     *  characteristic. Returns the GATT table `[{uuid, characteristics: [{uuid, properties, subscribed}]}]`, or ""
     *  with the reason in [lastError] when nothing could be subscribed. */
    @JvmStatic
    fun listen(handle: Int): String {
        val link = handles[handle] ?: run { lastErrors[handle] = "no such link"; return "" }
        val gatt = link.gatt ?: run { lastErrors[handle] = "link is closed"; return "" }

        link.mtuLatch = CountDownLatch(1)
        if (!link.closed && gatt.requestMtu(REQUESTED_MTU)) {
            link.mtuLatch?.await(MTU_TIMEOUT_MS, TimeUnit.MILLISECONDS)
        }

        val table = JSONArray()
        var subscribed = 0
        var candidates = 0
        var lastErr: String? = null
        for (service in gatt.services) {
            val standard = service.uuid in STANDARD_SERVICES
            val chars = JSONArray()
            for (ch in service.characteristics) {
                val p = ch.properties
                val names = JSONArray()
                if (p and BluetoothGattCharacteristic.PROPERTY_READ != 0) names.put("Read")
                if (p and BluetoothGattCharacteristic.PROPERTY_WRITE != 0) names.put("Write")
                if (p and BluetoothGattCharacteristic.PROPERTY_WRITE_NO_RESPONSE != 0) names.put("WriteNR")
                if (p and BluetoothGattCharacteristic.PROPERTY_NOTIFY != 0) names.put("Notify")
                if (p and BluetoothGattCharacteristic.PROPERTY_INDICATE != 0) names.put("Indicate")
                val notifies = p and
                    (BluetoothGattCharacteristic.PROPERTY_NOTIFY or BluetoothGattCharacteristic.PROPERTY_INDICATE) != 0
                var on = false
                if (notifies && !standard) {
                    candidates++
                    val err = enableNotifications(link, gatt, ch) ?: cccdRefusal(link)
                    on = err == null
                    if (!on) {
                        lastErr = "${ch.uuid}: $err"
                        android.util.Log.w(TAG, "listen $handle: ${ch.uuid} not subscribed: $err")
                    }
                }
                if (on) subscribed++
                chars.put(
                    JSONObject().put("uuid", ch.uuid.toString().lowercase())
                        .put("properties", names).put("subscribed", on)
                )
            }
            table.put(JSONObject().put("uuid", service.uuid.toString().lowercase()).put("characteristics", chars))
        }
        if (subscribed == 0) {
            lastErrors[handle] = link.failure
                ?: if (candidates == 0) "nothing on this device notifies"
                else "no notifying characteristic could be subscribed ($lastErr)"
            return ""
        }
        android.util.Log.i(TAG, "link $handle listening: $subscribed characteristic(s), mtu=${link.mtu}")
        return table.toString()
    }

    /**
     * Read into [buf], waiting up to [timeoutMs] for notified bytes. Returns the byte count, 0 on an
     * idle timeout, -1 once the link is gone and drained — the same contract as [UsbSerial.read].
     */
    @JvmStatic
    fun read(handle: Int, buf: ByteArray, timeoutMs: Int): Int {
        val link = handles[handle] ?: return -1
        return link.read(buf, timeoutMs)
    }

    /**
     * Write [data], chunked to the negotiated MTU, each chunk waited for on its callback (up to
     * [timeoutMs]) and followed by [delayMs] where the profile asks for pacing. Returns false with
     * the reason in [lastError].
     */
    @JvmStatic
    fun write(handle: Int, data: ByteArray, timeoutMs: Int, delayMs: Int): Boolean {
        val link = handles[handle] ?: run { lastErrors[handle] = "no such link"; return false }
        val err = link.write(data, timeoutMs, delayMs)
        if (err != null) fail(handle, err)
        return err == null
    }

    @JvmStatic
    fun close(handle: Int) {
        val link = handles.remove(handle) ?: return
        link.close()
    }

    // ── One GATT link ────────────────────────────────────────────────────────────────────────────

    private class Link(val handle: Int, val device: BluetoothDevice) : BluetoothGattCallback() {
        @Volatile var gatt: BluetoothGatt? = null
            private set
        private val lock = Any()
        /** Set by [close]; a link is never reopened. */
        @Volatile var closed = false
            private set
        @Volatile var connected = false
        @Volatile var failure: String? = null

        val connectLatch = CountDownLatch(1)
        val servicesLatch = CountDownLatch(1)
        @Volatile var descriptorLatch: CountDownLatch? = null
        @Volatile var pendingCccd: BluetoothGattDescriptor? = null
        @Volatile var descriptorStatus = BluetoothGatt.GATT_SUCCESS
        @Volatile var mtuLatch: CountDownLatch? = null
        @Volatile var writeLatch: CountDownLatch? = null
        @Volatile var writeStatus = BluetoothGatt.GATT_SUCCESS

        @Volatile var mtu = 23
        var writeChar: BluetoothGattCharacteristic? = null
        var writeType = BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT

        /** Notified payloads, in arrival order. */
        private val rx = LinkedBlockingQueue<ByteArray>()
        /** The unread tail of the last dequeued payload, when the caller's buffer was smaller. */
        private var carry: ByteArray = ByteArray(0)
        private var carryOff = 0

        fun read(buf: ByteArray, timeoutMs: Int): Int {
            if (carryOff < carry.size) return drainCarry(buf)
            val next = if (connected) {
                rx.poll(timeoutMs.toLong().coerceAtLeast(1), TimeUnit.MILLISECONDS)
            } else {
                rx.poll()
            }
            if (next == null) return if (connected) 0 else -1
            carry = next
            carryOff = 0
            return drainCarry(buf)
        }

        private fun drainCarry(buf: ByteArray): Int {
            val n = minOf(buf.size, carry.size - carryOff)
            System.arraycopy(carry, carryOff, buf, 0, n)
            carryOff += n
            return n
        }

        fun write(data: ByteArray, timeoutMs: Int, delayMs: Int): String? {
            val g = gatt ?: return "link is closed"
            val ch = writeChar ?: return "not subscribed to a serial profile"
            if (!connected) return "link is disconnected"
            val chunk = (mtu - 3).coerceAtLeast(20)
            var off = 0
            while (off < data.size) {
                val n = minOf(chunk, data.size - off)
                val part = data.copyOfRange(off, off + n)
                val latch = CountDownLatch(1)
                writeLatch = latch
                writeStatus = BluetoothGatt.GATT_SUCCESS
                // After publishing the latch: a drop that released the previous one is seen here.
                if (!connected) return "link is disconnected"
                val started = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    g.writeCharacteristic(ch, part, writeType) == BluetoothGatt.GATT_SUCCESS
                } else {
                    @Suppress("DEPRECATION")
                    run {
                        ch.writeType = writeType
                        ch.value = part
                        g.writeCharacteristic(ch)
                    }
                }
                if (!started) return "characteristic write could not be started (stack busy?)"
                // Both write types report back through onCharacteristicWrite; issuing the next one
                // before that arrives is what earns GATT_BUSY on most stacks.
                if (!latch.await(timeoutMs.toLong().coerceAtLeast(1), TimeUnit.MILLISECONDS)) {
                    return "write acknowledgement timed out"
                }
                if (!connected) return "link is disconnected"
                if (writeStatus != BluetoothGatt.GATT_SUCCESS) return "write failed (GATT status $writeStatus)"
                off += n
                if (delayMs > 0 && off < data.size) Thread.sleep(delayMs.toLong())
            }
            return null
        }

        /** Main thread: keep the [connectGatt] result, unless [close] came first. */
        fun attach(connectGatt: () -> BluetoothGatt?) {
            synchronized(lock) {
                if (!closed) gatt = connectGatt()
            }
        }

        fun close() {
            val g = synchronized(lock) {
                closed = true
                connected = false
                gatt.also { gatt = null }
            }
            try {
                g?.disconnect()
                g?.close()
            } catch (_: Exception) {
                // Already gone; nothing to release.
            }
            // Release every wait, so a connect or setup in progress on another thread returns now. A reader
            // blocked in poll() wakes; the next poll() is non-blocking and returns -1 once drained.
            connectLatch.countDown()
            servicesLatch.countDown()
            mtuLatch?.countDown()
            descriptorLatch?.countDown()
            writeLatch?.countDown()
            rx.offer(ByteArray(0))
        }

        // ── GATT callbacks (binder threads) ──────────────────────────────────────────────────

        override fun onConnectionStateChange(g: BluetoothGatt, status: Int, newState: Int) {
            if (newState == BluetoothProfile.STATE_CONNECTED && status == BluetoothGatt.GATT_SUCCESS) {
                connected = true
            } else {
                if (!connected) failure = "connect failed (GATT status $status)"
                else lastErrors[handle] = "link dropped (GATT status $status)"
                connected = false
                writeLatch?.countDown() // a write in flight fails now, not at its timeout
                rx.offer(ByteArray(0)) // release a blocked reader
            }
            connectLatch.countDown()
        }

        override fun onServicesDiscovered(g: BluetoothGatt, status: Int) {
            if (status != BluetoothGatt.GATT_SUCCESS) failure = "service discovery failed (GATT status $status)"
            servicesLatch.countDown()
        }

        override fun onMtuChanged(g: BluetoothGatt, newMtu: Int, status: Int) {
            if (status == BluetoothGatt.GATT_SUCCESS) mtu = newMtu
            mtuLatch?.countDown()
        }

        override fun onDescriptorWrite(g: BluetoothGatt, d: BluetoothGattDescriptor, status: Int) {
            // A late answer to an earlier, timed-out write must not release the next one.
            if (d.characteristic.uuid != pendingCccd?.characteristic?.uuid) return
            descriptorStatus = status
            descriptorLatch?.countDown()
        }

        override fun onCharacteristicWrite(g: BluetoothGatt, c: BluetoothGattCharacteristic, status: Int) {
            writeStatus = status
            writeLatch?.countDown()
        }

        // API 33+ delivers the value alongside; older stacks put it on the characteristic.
        override fun onCharacteristicChanged(g: BluetoothGatt, c: BluetoothGattCharacteristic, value: ByteArray) {
            if (value.isNotEmpty()) rx.offer(value.copyOf())
        }

        @Deprecated("pre-API-33 delivery path")
        override fun onCharacteristicChanged(g: BluetoothGatt, c: BluetoothGattCharacteristic) {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) return // the new overload ran
            @Suppress("DEPRECATION")
            val v = c.value ?: return
            if (v.isNotEmpty()) rx.offer(v.copyOf())
        }
    }
}
