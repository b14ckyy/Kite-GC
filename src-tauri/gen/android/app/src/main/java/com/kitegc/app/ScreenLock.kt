// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

package com.kitegc.app

import android.view.WindowManager

/**
 * Keeps the display on while there is something to watch — nav-app behaviour.
 *
 * Driven from Rust (`link_presence`, which holds the rule) over JNI: on while the vehicle is armed,
 * while a link cannot tell whether it is, during a continuous raw recording, or always when the
 * user asked for it in the settings; otherwise the normal OS screen timeout applies.
 * `FLAG_KEEP_SCREEN_ON` is scoped to this window and released automatically when
 * the app leaves the foreground: no WAKE_LOCK permission, nothing kept awake in the background.
 */
object ScreenLock {
    private lateinit var activity: MainActivity

    fun init(activity: MainActivity) {
        this.activity = activity
    }

    /** Window flags are main-thread state; the Rust caller is on a worker thread. */
    @JvmStatic
    fun keepOn(on: Boolean) {
        activity.runOnUiThread {
            if (on) {
                activity.window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            } else {
                activity.window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            }
        }
    }
}
