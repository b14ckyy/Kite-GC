// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

package com.kitegc.app

import android.os.Build
import android.util.Log
import android.webkit.WebView
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/**
 * Which known ground-station hardware Kite is running on — one decision, three consumers.
 *
 * Kite runs on ordinary phones and tablets, and on Android-based RC transmitters that wire a
 * flight-controller link into the device itself. Those need adaptations a stock device must never
 * see: an internal UART that no USB-host enumeration can find, and a form factor the WebView
 * misreports. Everything device-specific is keyed off the profile name this object computes, so
 * the identification rule lives in exactly one place and a wrong match is corrected here alone.
 *
 * Consumers:
 *   - [tagUserAgent]: the frontend learns the profile from a `KiteDevice/<name>` token appended to
 *     the WebView user-agent (`platform.ts`). A token, not a Tauri command, because the form-factor
 *     decision is made synchronously at module load, before any command could answer.
 *   - Rust over JNI (`android/device.rs`): [profile] for the log, [internalSerialPorts] for the
 *     serial port picker. JNI-only callers are invisible to R8 — keep rule in proguard-rules.pro.
 *
 * Known profiles:
 *   - [AX12] — RadioMaster AX12 (MediaTek MT8788, Android 9). Its ELRS module is wired to the SoC's
 *     UART1, which the firmware exposes as a world-accessible `/dev/ttyS1` (0666, SELinux
 *     permissive) at the module's 460800 baud; RadioMaster's own QGroundControl build reaches it the
 *     same way (Qt's stock serial port on a raw tty node). The firmware is built from MediaTek's
 *     `tb8788p1_64_bsp` reference product and carries that name in the build fields — there is no
 *     RadioMaster-specific identifier anywhere in the build properties. Blind implementation from
 *     the ax12-research audit (2026-04) and the Radiomaster-RC/qgroundcontrol fork; not verified on
 *     hardware yet.
 */
object DeviceProfile {
  private const val TAG = "KiteDevice"

  /** Profile name of the RadioMaster AX12. */
  const val AX12 = "ax12"

  /** MediaTek reference-product name the AX12 firmware reports in `Build.DEVICE` / `PRODUCT` /
   *  `MODEL` and in the build fingerprint (`alps/full_tb8788p1_64_bsp/tb8788p1_64_bsp:9/…`). */
  private const val AX12_BSP = "tb8788p1_64_bsp"

  /** The AX12's internal UART: UART1 of the MT8788, wired to the ELRS module. UART0 (`ttyS0`) is
   *  the radio firmware's own bus to the stick/switch MCU and must never be touched; `ttyS2` is
   *  root-only. */
  private const val AX12_INTERNAL_UART = "/dev/ttyS1"

  /** The matched profile name, or "" on a device Kite does not recognise. Computed once. */
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
      "hardware=${Build.HARDWARE} sdk=${Build.VERSION.SDK_INT} fingerprint=${Build.FINGERPRINT}"

  /**
   * Serial ports built into the device, as a JSON array of `{path, label, type}` — the same shape
   * `UsbSerial.listDevices` answers with, so Rust appends them to one list. Empty on an unknown
   * device, and on a known one whose node is not accessible after all: a port the open would fail
   * on is worse than no port, and the log says which check failed.
   */
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
            .put("label", "$AX12_INTERNAL_UART — internal UART (ELRS module)")
            .put("type", "internal-uart")
        )
      }
    }
    return out.toString()
  }

  /**
   * Append the `KiteDevice/<name>` token to the WebView's user-agent so the frontend can key its
   * form-factor decision off the profile. Must run before the first page load — it does, from
   * `MainActivity.onWebViewCreate`, which wry calls before it navigates. No-op on an unknown device,
   * so the user-agent of every stock phone and tablet stays exactly as it was.
   */
  fun tagUserAgent(webView: WebView) {
    if (name.isEmpty()) return
    val settings = webView.settings
    settings.userAgentString = "${settings.userAgentString} KiteDevice/$name"
    Log.i(TAG, "user-agent tagged: KiteDevice/$name")
  }
}
