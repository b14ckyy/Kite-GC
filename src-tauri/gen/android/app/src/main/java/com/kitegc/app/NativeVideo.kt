// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

package com.kitegc.app

import android.graphics.Color
import android.view.Surface
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.ViewGroup
import android.webkit.WebView
import android.widget.FrameLayout

/**
 * Native video layer host — the Android half of the hole-punch architecture (MOBILE_RTSP.md).
 *
 * MediaCodec (video/android_sink.rs) decodes straight onto a SurfaceView that sits BELOW the
 * transparent WebView; the DOM cuts a transparent hole over it (controllers/nativeVideo.ts —
 * the same surface router as on Windows), so OSD and controls composite above the video.
 *
 * Geometry: the router pushes TWO rects (physical px, window coords). The FULL box is where
 * the surface lives — the video is aspect-fitted and centered inside it. The VISIBLE box is
 * what scroll-container clipping left of it — a clipping FrameLayout ([clipBox]) sized to it
 * cuts the video at the container edge, exactly like scrolled DOM content, instead of
 * shrinking the picture into the remainder. When nothing is scrolled the two boxes are equal.
 *
 * `setZOrderOnTop` stays at its default (false): the surface is composited BEHIND the window
 * and the SurfaceView punches its region transparent — which is also what keeps the future
 * Activity-PiP path open (the video belongs to this activity, not to an overlay window).
 *
 * Driven only from Rust over JNI (android/native_video.rs) — R8 keep rule required (see
 * proguard-rules.pro; JNI-only callers are invisible to R8). Rust calls arrive on worker
 * threads; every view mutation hops to the UI thread here. [surface] and [generation] are
 * the two reads the decode loop does directly, off-thread — both are safe cross-thread reads.
 *
 * Codec lifecycle rule (PiP): the Rust decode loop keys its codec lifetime to [generation]
 * (bumped on surfaceCreated/surfaceDestroyed), NOT to activity pause — in PiP the activity
 * is paused yet visible and the video must keep playing.
 */
object NativeVideo {
  private lateinit var activity: MainActivity
  private var webView: WebView? = null

  /** Clipping wrapper at the VISIBLE box (clipChildren cuts the surface at its bounds). */
  private var clipBox: FrameLayout? = null

  /** The decoder's output surface, laid out at the FULL box inside [clipBox]. */
  @Volatile private var view: SurfaceView? = null

  /** Bumped on every surfaceCreated/surfaceDestroyed; read from Rust to notice a lost surface. */
  @Volatile private var surfaceGeneration = 0

  /** FULL box, VISIBLE box (window coords, physical px) and the video's pixel size —
   *  combined into the clip + aspect-fit layout whenever any of them changes. UI thread. */
  private var full = intArrayOf(0, 0, 1, 1)
  private var clip = intArrayOf(0, 0, 1, 1)
  private var videoW = 0
  private var videoH = 0

  /** Off-screen shift used instead of GONE/INVISIBLE for "hidden": both visibility states
   *  destroy a SurfaceView's surface, which would tear the decoder down on every hide. */
  private const val OFFSCREEN = -100_000f

  fun init(activity: MainActivity) {
    this.activity = activity
  }

  /** Called from MainActivity.onWebViewCreate: the WebView goes transparent while the native
   *  layer exists (its opaque background would otherwise cover the surface entirely). */
  fun attachWebView(webView: WebView) {
    this.webView = webView
  }

  /** Create (or re-layout) the layer at a window rect. The sink starts this at 1×1; the
   *  frontend surface router pushes the real rects right after. */
  @JvmStatic
  fun show(x: Int, y: Int, w: Int, h: Int) {
    activity.runOnUiThread {
      full = intArrayOf(x, y, w, h)
      clip = intArrayOf(x, y, w, h)
      var box = clipBox
      if (box == null) {
        box = FrameLayout(activity)
        box.clipChildren = true
        box.clipToPadding = true
        val v = SurfaceView(activity)
        v.holder.addCallback(object : SurfaceHolder.Callback {
          override fun surfaceCreated(holder: SurfaceHolder) {
            surfaceGeneration++
          }

          override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {}

          override fun surfaceDestroyed(holder: SurfaceHolder) {
            surfaceGeneration++
          }
        })
        box.addView(v)
        // Index 0 = behind every later child; the WebView is (or gets) appended after us
        // and therefore composites on top.
        val content = activity.findViewById<ViewGroup>(android.R.id.content)
        content.addView(box, 0)
        clipBox = box
        view = v
      }
      applyLayout()
      box.translationX = 0f
      webView?.setBackgroundColor(Color.TRANSPARENT)
    }
  }

  /** Move/resize: FULL box for the video's aspect-fit layout, VISIBLE box for the clip. */
  @JvmStatic
  fun setRect(x: Int, y: Int, w: Int, h: Int, cx: Int, cy: Int, cw: Int, ch: Int) {
    activity.runOnUiThread {
      full = intArrayOf(x, y, w, h)
      clip = intArrayOf(cx, cy, cw, ch)
      applyLayout()
    }
  }

  /** Hide by shifting off-screen — see [OFFSCREEN] for why not GONE. */
  @JvmStatic
  fun setVisible(visible: Boolean) {
    activity.runOnUiThread { clipBox?.translationX = if (visible) 0f else OFFSCREEN }
  }

  // (No orientation here: view transforms do NOT reach a SurfaceView's surface content —
  // tried and visually disproven on the M11. Mirror/rotation live in the Rust sink as
  // ANativeWindow buffer transforms instead.)

  /** The decoded video's pixel size, from the decoder's output format — drives the
   *  aspect-fit. (No holder.setFixedSize: with MediaCodec as the surface's producer the
   *  buffer geometry is the decoder's, and the compositor scales it to the view bounds.) */
  @JvmStatic
  fun setVideoSize(w: Int, h: Int) {
    activity.runOnUiThread {
      videoW = w
      videoH = h
      applyLayout()
    }
  }

  /** Remove the layer and give the WebView its opaque background back (sink stopped). */
  @JvmStatic
  fun destroy() {
    activity.runOnUiThread {
      clipBox?.let { (it.parent as? ViewGroup)?.removeView(it) }
      clipBox = null
      view = null
      videoW = 0
      videoH = 0
      // Opaque again: a transparent WebView costs composition work, and any DOM region that
      // skips painting would otherwise show the window background.
      webView?.setBackgroundColor(Color.BLACK)
    }
  }

  /** The live output surface for the decoder, or null while none exists. */
  @JvmStatic
  fun surface(): Surface? = view?.holder?.surface?.takeIf { it.isValid }

  @JvmStatic
  fun generation(): Int = surfaceGeneration

  /** Clip box at the VISIBLE rect; SurfaceView aspect-fitted into the FULL rect and placed
   *  relative to the box, so scrolled-away parts of the video hang outside the box bounds
   *  and get cut — never rescaled. Until the video size is known the full rect is used. */
  private fun applyLayout() {
    val box = clipBox ?: return
    val v = view ?: return
    box.layoutParams = FrameLayout.LayoutParams(maxOf(clip[2], 1), maxOf(clip[3], 1)).apply {
      leftMargin = clip[0]
      topMargin = clip[1]
    }
    var fx = full[0]
    var fy = full[1]
    var fw = maxOf(full[2], 1)
    var fh = maxOf(full[3], 1)
    if (videoW > 0 && videoH > 0 && full[2] > 0 && full[3] > 0) {
      val scale = minOf(full[2].toFloat() / videoW, full[3].toFloat() / videoH)
      fw = maxOf((videoW * scale).toInt(), 1)
      fh = maxOf((videoH * scale).toInt(), 1)
      fx = full[0] + (full[2] - fw) / 2
      fy = full[1] + (full[3] - fh) / 2
    }
    v.layoutParams = FrameLayout.LayoutParams(fw, fh).apply {
      leftMargin = fx - clip[0]
      topMargin = fy - clip[1]
    }
  }
}
