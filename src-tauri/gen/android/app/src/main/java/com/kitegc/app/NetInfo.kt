// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

package com.kitegc.app

import android.net.ConnectivityManager
import android.net.NetworkCapabilities

/**
 * Active-network transport lookup, called from Rust over JNI (android/net.rs) — R8 keep
 * rule required (JNI-only callers are invisible to R8).
 *
 * One question, one answer: is the device's active network on Wi-Fi? Used to decide
 * whether continuous GCS location updates must pause while an RTSP stream runs — fused
 * location triggers a Wi-Fi scan every ~10 s, each scan takes the radio off-channel, and
 * the resulting RTP loss bursts corrupt the video until the next keyframe (measured on the
 * Teclast M11). Streams over cellular or Ethernet don't care about Wi-Fi scans.
 */
object NetInfo {
  private lateinit var activity: MainActivity

  fun init(activity: MainActivity) {
    this.activity = activity
  }

  /**
   * True when the active network runs over Wi-Fi. A VPN's capabilities carry the
   * underlying transport where the system knows it, so VPN-over-LTE reports cellular; a
   * VPN whose underlay is unknown — and any lookup failure — reports true, because the
   * caller pauses location updates during video and pausing needlessly is the safer error.
   */
  @JvmStatic
  fun activeNetIsWifi(): Boolean {
    val cm = activity.getSystemService(ConnectivityManager::class.java) ?: return true
    val caps = cm.getNetworkCapabilities(cm.activeNetwork) ?: return true
    if (caps.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR)) return false
    if (caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)) return false
    return true
  }
}
