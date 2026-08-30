# Add project specific ProGuard rules here.
# You can control the set of applied configuration files using the
# proguardFiles setting in build.gradle.
#
# For more details, see
#   http://developer.android.com/guide/developing/tools/proguard.html

# If your project uses WebView with JS, uncomment the following
# and specify the fully qualified class name to the JavaScript interface
# class:
#-keepclassmembers class fqcn.of.javascript.interface.for.webview {
#   public *;
#}

# Uncomment this to preserve the line number information for
# debugging stack traces.
#-keepattributes SourceFile,LineNumberTable

# If you keep the line number information, uncomment this to
# hide the original source file name.
#-renamesourcefileattribute SourceFile

# ── Kite ─────────────────────────────────────────────────────────────────────
# The USB-serial bridge is called only from Rust over JNI (see
# src-tauri/src/transport/serial_android.rs), which R8 cannot see: no Java or Kotlin
# code calls listDevices/open/read/write/setControlLines/close, so in a minified
# release build they are dead code and get removed or renamed. The app then installs
# and starts normally and only fails when you plug a cable in, with a
# NoSuchMethodError from a class that is plainly right there in the source.
#
# Keep the class and every member. `init` is reached from MainActivity and would
# survive on its own; nothing else would.
-keep class com.kitegc.app.UsbSerial { *; }

# Same JNI-only story for the storage bridge (android/storage.rs → StorageAccess):
# pickFolder/getLastError/syncDirToTree have no Kotlin caller, so R8 strips them and
# the folder picker dies with a NoSuchMethodError only in RELEASE builds — the debug
# build does not minify, which is exactly how this trap stayed invisible in testing.
-keep class com.kitegc.app.StorageAccess { *; }

# And the BLE bridge (transport/ble_android.rs → BleSerial) — JNI-only, same rule.
-keep class com.kitegc.app.BleSerial { *; }