// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

package com.kitegc.app

/**
 * The Settings switch for the mini view: leaving Kite enters picture-in-picture while it is on.
 * Driven from Rust over JNI (android/pip.rs) — R8 keep rule required (proguard-rules.pro).
 */
object MiniView {
    private lateinit var activity: MainActivity

    @Volatile
    var enabled = false
        private set

    fun init(activity: MainActivity) {
        this.activity = activity
    }

    /** PiP params are activity state on the main thread; the Rust caller is on a worker thread. */
    @JvmStatic
    fun setEnabled(on: Boolean) {
        enabled = on
        activity.runOnUiThread { activity.applyPipParams() }
    }
}
