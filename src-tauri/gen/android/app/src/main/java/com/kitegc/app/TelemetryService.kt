// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

package com.kitegc.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.util.Log
import androidx.core.app.NotificationCompat

/**
 * Foreground service for the life of a telemetry link (Dev-Docs active/BACKGROUND_TELEMETRY.md).
 * It does no work itself — being a foreground service is what keeps Android from freezing or
 * reaping the process while the activity is not in front. Type `connectedDevice` (no 6-hour cap,
 * unlike `dataSync`; the manifest's CHANGE_NETWORK_STATE is its Android 14 prerequisite).
 * `START_NOT_STICKY`: a restart after a kill would have no link to report.
 */
class TelemetryService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        createChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val title = intent?.getStringExtra(LinkService.EXTRA_TITLE) ?: getString(R.string.app_name)
        val text = intent?.getStringExtra(LinkService.EXTRA_TEXT) ?: getString(R.string.link_notification_default)
        val notification = build(title, text)
        try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE)
            } else {
                startForeground(NOTIFICATION_ID, notification)
            }
        } catch (e: Exception) {
            // Refused: the link still runs, only without the foreground guarantee.
            Log.w(TAG, "startForeground refused: $e")
        }
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        stopForeground(STOP_FOREGROUND_REMOVE)
        super.onDestroy()
    }

    private fun build(title: String, text: String): Notification {
        val launch = packageManager.getLaunchIntentForPackage(packageName)
        val tap = launch?.let {
            PendingIntent.getActivity(this, 0, it, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        }
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_launcher_foreground)
            .setContentTitle(title)
            .setContentText(text)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setSilent(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
            .apply { if (tap != null) setContentIntent(tap) }
            .build()
    }

    private fun createChannel() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val channel = NotificationChannel(
            CHANNEL_ID,
            getString(R.string.link_channel_name),
            NotificationManager.IMPORTANCE_LOW, // no sound, no heads-up: a status line, not an alert
        ).apply { description = getString(R.string.link_channel_desc) }
        (getSystemService(NOTIFICATION_SERVICE) as NotificationManager).createNotificationChannel(channel)
    }

    private companion object {
        const val TAG = "KiteLink"
        const val CHANNEL_ID = "telemetry-link"
        const val NOTIFICATION_ID = 1
    }
}
