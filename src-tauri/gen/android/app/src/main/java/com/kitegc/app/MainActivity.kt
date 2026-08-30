package com.kitegc.app

import android.os.Build
import android.os.Bundle
import android.view.WindowManager
import android.widget.Toast
import androidx.activity.OnBackPressedCallback
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
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

  override fun onCreate(savedInstanceState: Bundle?) {
    // Before super.onCreate, which is what starts Tauri and therefore the Rust side: the USB-serial
    // bridge has no Context of its own and the first port enumeration can arrive as soon as the
    // frontend is up.
    UsbSerial.init(this)
    StorageAccess.init(this, folderPicker)
    BleSerial.init(this, blePermissions)
    ScreenLock.init(this)

    enableEdgeToEdge()

    // Let the window extend into the display cutout region. `enableEdgeToEdge()` alone does not do
    // this: in landscape the default cutout mode makes the system letterbox the window away from the
    // notch entirely, which on a centre-punch-hole phone costs a black bar down the whole leading
    // edge. SHORT_EDGES lets us have those pixels; the `env(safe-area-inset-*)` rules in app.html
    // then keep the actual UI clear of the hole, so the map gains the area and no control hides
    // behind the camera.
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
      window.attributes = window.attributes.apply {
        layoutInDisplayCutoutMode =
          WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES
      }
    }

    super.onCreate(savedInstanceState)

    hideSystemBars()

    // The display stays on only while a telemetry link is active — nav-app behaviour, driven from
    // Rust through ScreenLock on connect / disconnect / lost link. Unconnected, the normal screen
    // timeout applies.

    // Require a deliberate double-press to leave. The default back behaviour finishes the activity
    // immediately, which on a gesture-navigation device means an edge swipe — easy to trigger by
    // accident while panning the map — tears down the telemetry link with no confirmation.
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

  /**
   * Android reveals the system bars again after various events — a permission dialog, the keyboard,
   * returning from another app — and leaves them up. Re-hiding on focus gain is what keeps the app
   * full-screen for a whole flight rather than only until the first interruption.
   */
  override fun onWindowFocusChanged(hasFocus: Boolean) {
    super.onWindowFocusChanged(hasFocus)
    if (hasFocus) hideSystemBars()
  }

  /**
   * Full immersive: no status bar, no navigation bar.
   *
   * A ground station is a full-screen instrument held in landscape — the clock and the nav buttons
   * are pure loss, and on a 3-button device the navigation bar eats a strip down the entire trailing
   * edge. Bars come back on a swipe from the edge (BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE) and hide
   * themselves again, so nothing is unreachable; the back gesture still works and still goes through
   * the press-twice-to-exit guard above.
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
