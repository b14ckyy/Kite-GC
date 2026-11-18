// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

package com.kitegc.app

import android.os.Build
import android.util.Log
import android.webkit.WebView
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/** Which known ground-station hardware Kite runs on; every device-specific adaptation keys off
 *  [profile]. The AX12 firmware carries only MediaTek's reference-product name (tb8788p1_64_bsp), no
 *  RadioMaster identifier. Rust calls the @JvmStatic members over JNI only — keep rule in
 *  proguard-rules.pro. */
object DeviceProfile {
  private const val TAG = "KiteDevice"

  const val AX12 = "ax12"

  /** MediaTek reference-product name the AX12 firmware reports in `Build.DEVICE` / `PRODUCT` /
   *  `MODEL` and in the build fingerprint (`alps/full_tb8788p1_64_bsp/tb8788p1_64_bsp:9/…`). */
  private const val AX12_BSP = "tb8788p1_64_bsp"

  /** The AX12's internal UART: UART1 of the MT8788, wired to the ELRS module. UART0 (`ttyS0`) is
   *  the radio firmware's own bus to the stick/switch MCU and must never be touched; `ttyS2` is
   *  root-only. */
  private const val AX12_INTERNAL_UART = "/dev/ttyS1"

  private val name: String by lazy { detect() }

  private fun detect(): String {
    val fields = listOf(Build.DEVICE, Build.PRODUCT, Build.MODEL, Build.BOARD, Build.FINGERPRINT)
    val ax12 = fields.any { it != null && it.contains(AX12_BSP, ignoreCase = true) }
    val profile = if (ax12) AX12 else ""
    Log.i(TAG, "profile=\"$profile\" ${buildSummary()}")
    return profile
  }

  /** The profile name — "" when nothing matched. JNI entry point for Rust. */
  @JvmStatic
  fun profile(): String = name

  /** The build fields the decision is made from, for the log — so a report from an unknown device
   *  (or a false match) carries what is needed to extend or fix [detect]. */
  @JvmStatic
  fun buildSummary(): String =
    "manufacturer=${Build.MANUFACTURER} brand=${Build.BRAND} model=${Build.MODEL} " +
      "device=${Build.DEVICE} product=${Build.PRODUCT} board=${Build.BOARD} " +
      "hardware=${Build.HARDWARE} sdk=${Build.VERSION.SDK_INT} display=${Build.DISPLAY} " +
      "fingerprint=${Build.FINGERPRINT}"

  /** Built-in serial ports as JSON {path, label, type} (the UsbSerial.listDevices shape). A node
   *  this app cannot read and write is left out: listing a port the open would fail on is worse. */
  @JvmStatic
  fun internalSerialPorts(): String {
    val out = JSONArray()
    if (name == AX12) {
      val node = File(AX12_INTERNAL_UART)
      when {
        !node.exists() -> Log.w(TAG, "$AX12_INTERNAL_UART does not exist — internal UART not offered")
        !node.canRead() || !node.canWrite() ->
          Log.w(TAG, "$AX12_INTERNAL_UART is not readable+writable by this app — internal UART not offered")
        else -> out.put(
          JSONObject()
            .put("path", AX12_INTERNAL_UART)
            .put("label", AX12_INTERNAL_UART)
            .put("type", "internal-uart")
        )
      }
    }
    return out.toString()
  }

  /** Must run before the first page load (platform.ts reads the token at module load); no-op on an
   *  unknown device, so stock user-agents stay untouched. */
  fun tagUserAgent(webView: WebView) {
    if (name.isEmpty()) return
    val settings = webView.settings
    settings.userAgentString = "${settings.userAgentString} KiteDevice/$name"
    Log.i(TAG, "user-agent tagged: KiteDevice/$name")
  }
}
