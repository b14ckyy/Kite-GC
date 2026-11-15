package com.kitegc.app

import android.os.Build
import android.os.Bundle
import android.view.WindowManager
import android.widget.Toast
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
    NativeVideo.attachWebView(webView)
  }

  /**
   * Android reveals the system bars again after dialogs, the keyboard or another app; re-hiding on
   * focus gain keeps the app full-screen for a whole flight.
   */
  override fun onWindowFocusChanged(hasFocus: Boolean) {
    super.onWindowFocusChanged(hasFocus)
    if (hasFocus) hideSystemBars()
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
  }
}
