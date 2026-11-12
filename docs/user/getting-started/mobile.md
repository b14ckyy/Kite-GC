# Kite on Android, iPhone and iPad

Kite runs on Android phones and tablets, on iPhone and on iPad. It is the same app as on the desktop,
but a phone or tablet cannot do everything a PC can — it cannot run helper programs, has no window
manager, and on iOS has no serial ports. Kite hides what a device cannot provide instead of offering
something that would not work. This page collects those differences in one place; each section links to
the full guide.

## Getting the app

| | Android | iPhone / iPad |
|---|---|---|
| **Download** | An `.apk` from the Releases page, installed by hand (sideload) | No public download yet — build it from source |
| **Details** | [Installation → Android](installation.md#android) | [Building from source](../for-developers/building.md) |

## Layout: phone or tablet

- **Phones** (Android phones and iPhone) get their own **landscape** layout: a full-screen map, the
  connection pop-out behind the 🔗 button, a widget column with two pages on the right edge, three
  widget slots along the bottom edge. See **[Quick tour → Kite on a phone](quick-tour.md#kite-on-a-phone)**.
- **Tablets** (iPad and Android tablets) use the **desktop layout** — top bar, docks, navigation rail —
  without window controls.
- On both, the frosted-glass blur of the widgets, buttons and chips over the map is always off (only
  the panels keep it), and **High-Resolution 3D** starts switched off. See
  **[Power Saving](../reference/settings.md#display)** and
  **[High-Resolution 3D](../reference/settings.md#3d-view)** in the Settings reference.

## Connecting

| | Android | iPhone / iPad |
|---|---|---|
| **Serial (USB)** | Yes, over an OTG adapter; Android asks per device whether Kite may use it | No serial transport |
| **BLE** | Yes (the first scan asks for the *Nearby devices* permission) | Yes |
| **TCP / UDP (Wi-Fi)** | Yes | Yes (the first network connection asks for *Local Network* access) |

A fresh install starts on **MAVLink** over **UDP** port `14550`, the usual Wi-Fi telemetry setup. Kite
remembers your last connection from then on, as on the desktop. Details:
**[Connecting](../guides/connecting.md)**.

**Android only:**

- **The link keeps running in the background.** While connected, a notification shows the aircraft,
  battery, flight mode and distance to home, and telemetry, recording and the track continue while
  Kite is minimised. See the Android note in **[Connecting](../guides/connecting.md)**.
- **The screen stays on** while a link is active; after you disconnect, the normal screen timeout
  applies again.

## Logbook and logs

- **Import** takes **`.kflight`**, **`.tlog`** and **`.rawmsp`** files. Onboard logs — INAV Blackbox,
  ArduPilot Dataflash, PX4 ULog — are imported on a desktop install and moved over as `.kflight`.
- The mobile database keeps **no original log files**, so **HI-RES** replay, the original-log export
  and the **Open** button (replay without importing) are desktop features.
- **Android:** the database and raw logs live in the app's private storage. **Settings → Data** can
  point them at a folder of your own; Kite copies them there, and that copy survives uninstalling the
  app.

See **[Flight logbook](../guides/logbook.md)**.

## Video

| | Android | iPad | iPhone |
|---|---|---|---|
| **Video** | Yes | Yes | **No video** — the Video panel says so |
| **RTSP** | Kite's own RTSP client: H.264 / HEVC in hardware, MJPEG directly; no helper programs | Kite's own RTSP client: H.264 / HEVC in hardware (VideoToolbox) | — |
| **Sources in the Video panel** | *Camera (device)* and *RTSP* — *Native Capture* needs ffmpeg and is desktop-only | as Android | — |

- On an **Android phone** the video sits in a **docked window** at the bottom-right of the map, with
  a camera button to park it; tablets get the floating window.
- The **detached video window** is a desktop feature.
- **Android:** with hardware decoding, the picture cannot be mirrored. While an RTSP stream runs over
  Wi-Fi, continuous GCS location updates pause.

See **[Video](../guides/video.md)**, especially its platform notes.

## Not available on mobile

- **The detached video window** and the **Native Capture** video source.
- **RC control from a gamepad or joystick** — the [RC control](../guides/rc-control.md) set-up is a
  desktop feature.
- On a **phone** also: the **≡ Raw** telemetry popup and the stick overlay beside the replay player.

## Updates and diagnostics

- **Updates:** Android tells you about a new version at start-up like the desktop, but has no in-app
  install — the notice offers the release page instead of *Update and Restart*. On iPhone and iPad the
  update check is switched off. See **[Settings → Updates](../reference/settings.md#updates)**.
- **Diagnostic log on Android:** the log folder is app-private, so **Settings → Diagnostics → Share log
  file** replaces *Open Folder* — pick the day and hand it to mail or a messenger. See
  **[Reporting a problem](../troubleshooting/reporting-issues.md)**.
