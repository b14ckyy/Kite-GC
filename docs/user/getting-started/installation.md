# Installation

Kite Ground Control is a small, self-contained desktop app (a few tens of MB — it uses your system's
web view rather than bundling a whole browser). Grab the build for your platform from the
[**Releases**](https://github.com/b14ckyy/Kite-GC/releases) page and you're ready to connect.

## Downloads

Every asset follows one naming scheme — `KiteGC_<OS>_<arch>_<version>_<type>` — so it's clear what you're
grabbing:

| Platform | Installer | Standalone | Portable |
|---|---|---|---|
| **Windows** (x64) | `..._installer.exe` (NSIS — install for you or all users) | — | `..._portable.zip` |
| **Linux** (x64 / arm64) | `..._installer.deb` (Debian/Ubuntu) or `..._installer.rpm` (Fedora/openSUSE) | — | `..._portable.zip` |
| **macOS** (universal) | `..._installer.dmg` | `..._standalone.zip` (the `.app`) | — |
| **Android** | `KiteGC_Android_arm64_<version>_installer.apk`, or `..._armv7_...` for a 32-bit ARM device | — | — |

- **Installer** — integrates with your system (Start menu / app launcher, uninstaller).
- **Standalone** — the macOS `.app`, zipped; runs without an install.
- **Portable** — the bare executable, zipped **with an empty `.portable` marker** already inside, so all
  data stays in a `data/` folder next to it (see [Installed vs portable mode](#installed-vs-portable-mode)).

The macOS release also carries a `..._update.tar.gz` — the app bundle packed for Kite's in-app updater
(**Update and Restart**, see [Settings → Updates](../reference/settings.md#updates)). You don't need to
download it yourself.

!!! note "macOS: signed and notarized"
    The macOS build is **universal** (Apple Silicon + Intel), **signed and notarized** since 1.0.0, so it
    opens like any other app — no Gatekeeper warning, no right-click → Open. If you are still running a
    beta or release candidate, replace it with the release build.

!!! warning "macOS 13 (Ventura) or newer"
    The macOS build targets **macOS 13**. Older systems are not supported — on macOS 11 or 12 the app
    installs but does not start.

### Android

Download the `.apk` on the device and open it; Android asks once to allow installs from that browser
or file manager. Phones get a dedicated landscape layout, tablets the desktop one. Links: **USB serial**
over an OTG cable (Android asks per device — pick Kite in the dialog when you plug the cable in),
**Bluetooth LE** (the first scan asks for the *Nearby devices* permission) and **Wi-Fi** (UDP / TCP).
Data lives in the app's private storage; **Settings → Data** exports the flight database as a file
and can move the raw logs to a shared folder. See [Connecting](../guides/connecting.md) for what happens while
Kite is in the background.

**iPhone / iPad**: no public download yet — the app builds from source (see
[Building from source](../for-developers/building.md)).

### Linux quick install

```bash
# Debian / Ubuntu
sudo dpkg -i KiteGC_Linux_*_installer.deb

# Fedora / openSUSE
sudo rpm -i KiteGC_Linux_*_installer.rpm

# Portable — no install needed
unzip KiteGC_Linux_*_portable.zip -d kite-gc
./kite-gc/kite-gc
```

There is no AppImage from Kite 2.0 on. The portable ZIP is the download that runs without an install;
it uses your system's WebKitGTK 4.1 and GStreamer, which the `.deb` and `.rpm` bring in on their own.

!!! warning "Linux: your distribution needs to be reasonably current"
    The Linux downloads are compiled on Ubuntu 24.04 and need **glibc 2.39 or newer** — Ubuntu 24.04+,
    Debian 13 (Trixie), Raspberry Pi OS Trixie, Fedora 40+ or a current rolling release. On an older
    system they refuse to start with a message like `version 'GLIBC_2.39' not found`. **Debian 12,
    Raspberry Pi OS Bookworm and Ubuntu 22.04 are below that line.** It applies to every Linux
    download, the portable ZIP included.

    **On those same older systems, video has a problem we cannot package around.** They ship
    WebKitGTK 2.50, and Kite runs inside that engine. In the 2.50 series a stopped video stream can
    leave the engine spinning — we measured a permanently busy CPU core that only quits when the app
    does. Debian 13 and Ubuntu 24.04 carry 2.52, where this does not happen. So an older distribution
    is not merely unsupported by these downloads; it is a system on which Kite's video would misbehave
    however you installed it.

    You can still **[build Kite yourself](../for-developers/building.md)** on an older distribution and
    everything else will work — but the video issue comes from your system's browser engine, so a
    self-built copy has it too. The only real fix is a newer distribution.

## Installed vs portable mode

You can run Kite two ways:

- **Installed** (the `.exe` / `.deb` / `.rpm` / `.dmg`) — integrates with your system (Start menu / app
  launcher / Applications, uninstaller) and stores its data in your user profile (see
  [below](#where-your-data-is-stored)).
- **Portable** — unzip the `..._portable.zip`: it already contains the executable **and** an empty file
  named **`.portable`**, so Kite keeps **everything** — the flight database, raw logs, and any downloaded
  helper tools — in a single `data/` folder **next to the executable**, writing nothing to your user
  profile. (You can also add a `.portable` file next to the executable yourself.)

!!! tip "When to go portable"
    Portable mode is ideal for a USB stick or a self-contained folder you can move between PCs, or when
    you want zero footprint outside the app's own directory. To switch a portable copy back to a normal
    install, just delete the `.portable` file (your data stays in `data/`). Portable mode applies to the
    **Windows** and **Linux** builds; on macOS the app runs from `/Applications` as a normal install.

## Where your data is stored

In a normal install Kite follows each OS's conventions; in portable mode everything lives under
`data/` next to the executable.

| Data | Windows (installed) | Linux (installed) | macOS (installed) | Portable |
|---|---|---|---|---|
| Flight database (`flights.db`) | `%APPDATA%\kite-gc\` | `~/.local/share/kite-gc/` | `~/Library/Application Support/kite-gc/` | `<app>/data/` |
| Raw logs (`.tlog`, raw-MSP) | `Documents\KiteGC\` | `~/Documents/KiteGC/` (XDG) | `~/Documents/KiteGC/` | `<app>/data/` |
| Downloaded helper tools | `%APPDATA%\kite-gc\bin\` | `~/.local/share/kite-gc/bin/` | `~/Library/Application Support/kite-gc/bin/` | `<app>/data/bin/` |
| [Custom maps](../guides/custom-maps.md#the-map-folder) (default location) | `%APPDATA%\kite-gc\custom_maps\` | `~/.local/share/kite-gc/custom_maps/` | `~/Library/Application Support/kite-gc/custom_maps/` | `<app>/data/custom_maps/` |
| Preferences & layout (settings, widget/panel layout) | web-view storage in your user profile | web-view storage in your user profile | web-view storage in your user profile | `<app>/data/` |
| Window size & position | `%APPDATA%\com.kitegc.app\` | `~/.config/com.kitegc.app/` | `~/Library/Application Support/com.kitegc.app/` | not saved in portable mode |

Your **preferences and layout** are kept in the web view's local storage — Microsoft **WebView2** on
Windows, **WebKitGTK** on Linux, **WKWebView** on macOS — **not inside the program file**. In **portable
mode** Kite redirects that storage into the `data/` folder next to the executable, so a portable copy
carries its settings with it. (One exception: on Windows, portable mode doesn't restore the **window
size/position**, because that path can't be redirected.)

!!! note "Custom locations"
    The **database folder** and the **raw-log folder** are independent and can each be pointed
    anywhere in **Settings** — handy for putting the database on a larger or faster drive. On Windows
    the Documents path follows a OneDrive relocation automatically.

## Memory requirements

Kite's memory use is almost entirely the map: tiles are held as textures by the web view, so it grows
with the size of the window in pixels. The 3D view and a larger screen need more, video adds very little.

| | Minimum free RAM | Recommended free RAM |
|---|---|---|
| **Android** | 1 GB | 2 GB |
| **Desktop** (Windows, Linux, macOS) | 2 GB | 4 GB |

"Free" means available to Kite while it runs, on top of the system and your other apps. The recommended
values also cover 4K and ultrawide screens with headroom.

## Storage requirements

The app itself is small. What grows over time is your flight data:

- **Flight database** — grows with recorded telemetry (a time-series per flight). Typical flights are
  modest; a large library built over many flights can reach tens to a few hundred MB.
- **Imported onboard logs** (INAV Blackbox, ArduPilot Dataflash, PX4 ULog) keep the **original log file
  inside the database** — these are the biggest single contributor. You can **delete the stored
  original** for a flight at any time (from its logbook entry) to reclaim that space while keeping the
  decoded data.
- **Raw logs** (`.tlog` / raw-MSP) are written separately under `Documents/KiteGC` and grow with use —
  housekeep them as you like; they're independent of the database.

Keeping it tidy:

- Deleting flights reclaims space incrementally (the database auto-shrinks over time).
- **Settings → Data → Compact Database** (desktop) rebuilds the whole file for maximum reclaim.
  Kite is blocked while it runs (minutes on a slow drive), so do it between flights.
- Move the database to another drive via **Settings** if space is tight.

## External dependencies & automatic downloads

Kite needs **nothing extra to connect and fly**. A few **optional** features rely on a small helper
program, which Kite offers to **download automatically the first time you use that feature**:

| Helper | Used for | How it's provided |
|---|---|---|
| `ffmpeg` | **Video** (native capture + fallback decoding for some RTSP sources on the classic path) | auto-download on **Windows & Linux**; **bundled inside the app on macOS** |
| `mediamtx` | **Video** (the classic RTSP → low-latency engine, used with the *Native RTSP client* switched off) | auto-download on **Windows, Linux & macOS** |

RTSP through Kite's own **Native RTSP client** needs neither helper — Android and iPad always use it,
the desktop by default unless it is switched off in the Video panel (see **[Video](../guides/video.md)**).

- Downloaded helpers are stored in Kite's tools folder (`…\kite-gc\bin`, or `data\bin` in portable
  mode) — they don't touch your system.
  Linux auto-downloads cover the common 64-bit CPUs (Intel/AMD `x86_64` and ARM `aarch64`).
- Kite finds a helper if it's **on your `PATH`**, **next to the app**, or in that tools folder. Where a
  helper isn't provided automatically — on an unsupported CPU like 32-bit/armv7 — install it yourself
  (e.g. `brew install ffmpeg` on macOS, or your package manager / a manual download) and put it on
  your `PATH` or next to the app; Kite will pick it up.
- These downloads need **internet access**. The **map** also needs it: both **2D map tiles** and
  **3D terrain** are streamed on demand and cached after first view — Kite has no offline map download.
  On the desktop, map tiles you already have can be shown from a folder as a
  [custom map](../guides/custom-maps.md#custom-maps-from-a-folder). Connecting to your aircraft, live telemetry, and logging all
  work fully offline.

## First run

Launch Kite, and head to **[your first connection](first-connection.md)**. New to the interface?
The **[quick tour](quick-tour.md)** points out where everything is.
