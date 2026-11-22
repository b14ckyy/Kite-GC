package com.kitegc.app

import android.app.PictureInPictureParams
import android.content.pm.PackageManager
import android.content.res.Configuration
import android.graphics.Point
import android.os.Build
import android.os.Bundle
import android.util.Rational
import android.view.WindowManager
import android.webkit.WebView
import android.widget.Toast
import androidx.annotation.RequiresApi
import androidx.activity.OnBackPressedCallback
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.graphics.Insets
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat

class MainActivity : TauriActivity() {
  /** Timestamp of the last back press, for the confirm-to-exit guard below. 0 = none pending. */
  private var lastBackPress = 0L

  private var webView: WebView? = null

  /** System folder picker for the custom database / raw-log locations. A field initializer on
   *  purpose: activity-result contracts must be registered before the activity reaches STARTED. */
  private val folderPicker =
    registerForActivityResult(ActivityResultContracts.OpenDocumentTree()) { uri ->
      StorageAccess.onFolderPicked(uri)
    }

  /** Runtime permissions for BLE (scan + connect on Android 12+, location before that). Same
   *  field-initializer rule as the folder picker. */
  private val blePermissions =
    registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { result ->
      BleSerial.onPermissionResult(result)
    }

  /** Notification permission (Android 13+) for the link's foreground service. Same rule. */
  private val notificationPermission =
    registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
      LinkService.onPermissionResult(granted)
    }

  override fun onCreate(savedInstanceState: Bundle?) {
    // Before super.onCreate, which is what starts Tauri and therefore the Rust side: the USB-serial
    // bridge has no Context of its own and the first port enumeration can arrive as soon as the
    // frontend is up.
    UsbSerial.init(this)
    StorageAccess.init(this, folderPicker)
    BleSerial.init(this, blePermissions)
    ScreenLock.init(this)
    LinkService.init(this, notificationPermission)
    NativeVideo.init(this)
    NetInfo.init(this)
    InstallSource.init(this)
    MiniView.init(this)

    enableEdgeToEdge()

    // Extend into the display cutout: in landscape the default mode letterboxes the window away from
    // the notch (a black bar on punch-hole phones); app.html's safe-area rules keep the UI off the hole.
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
      window.attributes = window.attributes.apply {
        layoutInDisplayCutoutMode =
          WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES
      }
    }

    super.onCreate(savedInstanceState)

    hideSystemBars()

    // Android 9 keeps reporting the hidden bars as insets, which the WebView turns into
    // env(safe-area-inset-*) and the phone chrome pads itself off the screen. Strip the system-bar
    // insets; the display cutout stays (that is what the safe-area rules are for).
    ViewCompat.setOnApplyWindowInsetsListener(window.decorView) { v, insets ->
      val stripped = WindowInsetsCompat.Builder(insets)
        .setInsets(WindowInsetsCompat.Type.systemBars(), Insets.NONE)
        .setInsetsIgnoringVisibility(WindowInsetsCompat.Type.systemBars(), Insets.NONE)
        .build()
      ViewCompat.onApplyWindowInsets(v, stripped)
    }

    // The keep-screen-on rule lives in link_presence.rs (driven through ScreenLock).

    // Require a deliberate double-press to leave: an accidental edge swipe while panning the map would
    // otherwise tear down the telemetry link without confirmation.
    onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
      override fun handleOnBackPressed() {
        val now = System.currentTimeMillis()
        if (now - lastBackPress < BACK_TO_EXIT_WINDOW_MS) {
          finish()
        } else {
          lastBackPress = now
          Toast.makeText(this@MainActivity, R.string.back_to_exit, Toast.LENGTH_SHORT).show()
        }
      }
    })
  }

  /** Hand the WebView to the native-video host: it must go transparent while the hole-punch
   *  video layer below it is visible (see NativeVideo). */
  override fun onWebViewCreate(webView: android.webkit.WebView) {
    this.webView = webView
    NativeVideo.attachWebView(webView)
    // Before the first navigation (wry calls this hook before it loads the URL): the frontend reads
    // the device profile from the user-agent at module load, so the token must already be there.
    DeviceProfile.tagUserAgent(webView)
    // A WebView scales text by the system font size, but not the boxes around it: labels overflow
    // their buttons. The app has its own UI scale.
    webView.settings.textZoom = 100
  }

  /**
   * Android reveals the system bars again after dialogs, the keyboard or another app; re-hiding on
   * focus gain keeps the app full-screen for a whole flight.
   */
  override fun onWindowFocusChanged(hasFocus: Boolean) {
    super.onWindowFocusChanged(hasFocus)
    if (hasFocus) hideSystemBars()
  }

  /** Mini view on: leaving Kite enters PiP. API 31+ enters by itself (auto-enter), also on the
   *  gestures that skip [onUserLeaveHint]; below that only the hint does it. */
  internal fun applyPipParams() {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O || !hasPip()) return
    setPictureInPictureParams(pipParams())
  }

  override fun onUserLeaveHint() {
    super.onUserLeaveHint()
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O || Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) return
    if (!MiniView.enabled || !hasPip() || isInPictureInPictureMode) return
    try {
      enterPictureInPictureMode(pipParams())
    } catch (_: IllegalStateException) {
      // The device refused PiP (e.g. disabled for the app in system settings).
    }
  }

  /** The page swaps its whole UI for the mini view while in PiP (MiniViewPip.svelte). */
  override fun onPictureInPictureModeChanged(isInPictureInPictureMode: Boolean, newConfig: Configuration) {
    super.onPictureInPictureModeChanged(isInPictureInPictureMode, newConfig)
    webView?.evaluateJavascript(
      "window.dispatchEvent(new CustomEvent('kite-pip',{detail:$isInPictureInPictureMode}))",
      null,
    )
  }

  /** The params are set ahead of time for auto-enter, so a rotation must refresh the aspect. */
  override fun onConfigurationChanged(newConfig: Configuration) {
    super.onConfigurationChanged(newConfig)
    if (MiniView.enabled && !isInPictureInPictureMode) applyPipParams()
  }

  private fun hasPip(): Boolean =
    packageManager.hasSystemFeature(PackageManager.FEATURE_PICTURE_IN_PICTURE)

  /** The display's shape in its current orientation, inside Android's PiP range (1:2.39 .. 2.39:1,
   *  checked as a float — hence the margin). */
  private fun pipAspect(): Rational {
    val w: Int
    val h: Int
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
      val b = windowManager.maximumWindowMetrics.bounds
      w = b.width()
      h = b.height()
    } else {
      val p = Point()
      @Suppress("DEPRECATION")
      windowManager.defaultDisplay.getRealSize(p)
      w = p.x
      h = p.y
    }
    if (w <= 0 || h <= 0) return Rational(16, 9)
    val r = w.toFloat() / h
    return when {
      r > MAX_PIP_ASPECT -> Rational(238, 100)
      r < 1f / MAX_PIP_ASPECT -> Rational(100, 238)
      else -> Rational(w, h)
    }
  }

  @RequiresApi(Build.VERSION_CODES.O)
  private fun pipParams(): PictureInPictureParams {
    val b = PictureInPictureParams.Builder().setAspectRatio(pipAspect())
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
      b.setAutoEnterEnabled(MiniView.enabled)
      // No live video to stretch: a plain cross-fade instead of a seamless resize.
      b.setSeamlessResizeEnabled(false)
    }
    return b.build()
  }

  /**
   * Full immersive: a ground station is a full-screen landscape instrument. The bars come back on an
   * edge swipe and hide again; the back gesture still goes through the press-twice-to-exit guard.
   */
  private fun hideSystemBars() {
    WindowCompat.getInsetsController(window, window.decorView).apply {
      hide(WindowInsetsCompat.Type.systemBars())
      systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
    }
  }

  private companion object {
    /** How long the first back press stays "armed". Long enough to read the toast, short enough that
     *  a press minutes later is not treated as a confirmation. */
    const val BACK_TO_EXIT_WINDOW_MS = 2000L
    const val MAX_PIP_ASPECT = 2.38f
  }
}
