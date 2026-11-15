// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

package com.kitegc.app

import android.os.Build

/**
 * Was the app installed by the Google Play Store? Decides whether the update notice opens the store
 * listing or the GitHub release page. Called from Rust over JNI (android/install_source.rs) — R8
 * keep rule required (JNI-only callers are invisible to R8).
 */
object InstallSource {
  private const val PLAY_STORE = "com.android.vending"

  private lateinit var activity: MainActivity

  fun init(activity: MainActivity) {
    this.activity = activity
  }

  /**
   * True when the installer on record (the LAST install) is the Play Store. No record (adb, some
   * file managers) and any lookup failure report false — the APK behaviour.
   */
  @JvmStatic
  fun isPlayStore(): Boolean {
    val pm = activity.packageManager
    val installer = try {
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
        pm.getInstallSourceInfo(activity.packageName).installingPackageName
      } else {
        @Suppress("DEPRECATION")
        pm.getInstallerPackageName(activity.packageName)
      }
    } catch (e: Exception) {
      null
    }
    return installer == PLAY_STORE
  }
}
