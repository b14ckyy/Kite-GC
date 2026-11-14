// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

package com.kitegc.app

import android.os.Build

/**
 * Where this installation came from, called from Rust over JNI (android/install_source.rs) — R8
 * keep rule required (JNI-only callers are invisible to R8).
 *
 * One question, one answer: was the app installed by the Google Play Store? The update notice
 * depends on it: a Play installation is updated by Play and must not be pointed at an APK outside
 * of it, so its notice opens the store listing; an APK installed by hand gets the GitHub release
 * page.
 */
object InstallSource {
  private const val PLAY_STORE = "com.android.vending"

  private lateinit var activity: MainActivity

  fun init(activity: MainActivity) {
    this.activity = activity
  }

  /**
   * True when the package installer on record is the Play Store. The record follows the LAST
   * install: an APK installed by hand over a Play installation reports false from then on. No
   * record (adb, some file managers) and any lookup failure report false — the APK behaviour.
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
