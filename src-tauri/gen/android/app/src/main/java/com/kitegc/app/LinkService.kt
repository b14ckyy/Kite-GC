// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

package com.kitegc.app

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.util.Log
import androidx.activity.result.ActivityResultLauncher
import androidx.core.content.ContextCompat

/**
 * The Rust side's handle on [TelemetryService] (JNI from `link_presence` and the notification
 * ticker); every call hops to the main thread. The notification permission (13+) is asked once
 * per process; a refusal only hides the notification.
 */
object LinkService {
    private const val TAG = "KiteLink"
    const val EXTRA_TITLE = "title"
    const val EXTRA_TEXT = "text"

    private lateinit var activity: MainActivity
    private var permissionLauncher: ActivityResultLauncher<String>? = null
    private var permissionAsked = false

    /** Wire up from [MainActivity.onCreate]; [launcher] is the single-permission contract. */
    fun init(activity: MainActivity, launcher: ActivityResultLauncher<String>) {
        this.activity = activity
        this.permissionLauncher = launcher
    }

    /** The permission dialog's answer. Informational only. */
    fun onPermissionResult(granted: Boolean) {
        if (!granted) Log.w(TAG, "Notification permission refused — the link service runs without a visible notification")
    }

    @JvmStatic
    fun start(title: String, text: String) {
        activity.runOnUiThread {
            ensureNotificationPermission()
            ContextCompat.startForegroundService(activity, intent(title, text))
        }
    }

    /** Re-delivers the intent; [TelemetryService.onStartCommand] re-notifies. */
    @JvmStatic
    fun update(title: String, text: String) {
        activity.runOnUiThread {
            ContextCompat.startForegroundService(activity, intent(title, text))
        }
    }

    @JvmStatic
    fun stop() {
        activity.runOnUiThread {
            activity.stopService(Intent(activity, TelemetryService::class.java))
        }
    }

    private fun intent(title: String, text: String): Intent =
        Intent(activity, TelemetryService::class.java)
            .putExtra(EXTRA_TITLE, title)
            .putExtra(EXTRA_TEXT, text)

    private fun ensureNotificationPermission() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
        val granted = ContextCompat.checkSelfPermission(activity, Manifest.permission.POST_NOTIFICATIONS) ==
            PackageManager.PERMISSION_GRANTED
        if (granted || permissionAsked) return
        permissionAsked = true
        permissionLauncher?.launch(Manifest.permission.POST_NOTIFICATIONS)
    }
}
