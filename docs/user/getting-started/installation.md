# Installation

Kite Ground Control is a small, self-contained desktop app (a few tens of MB — it uses your system's
web view rather than bundling a whole browser). Grab the build for your platform from the
[**Releases**](https://github.com/b14ckyy/Kite-GCS/releases) page and you're ready to connect.

## Downloads

| Platform | Installer | Standalone (portable-capable) |
|---|---|---|
| **Windows** | `.exe` setup (NSIS — install for just you or for all users) **or** `.msi` | a standalone `.exe` |
| **Linux** | `.deb` (Debian/Ubuntu), `.rpm` (Fedora/openSUSE), or `.AppImage` | the `.AppImage` / raw executable |

!!! note "macOS"
    Prebuilt macOS bundles aren't provided yet — macOS users can build from source (see the project's
    `BUILD` docs).

### Linux quick install

```bash
# Debian / Ubuntu
sudo dpkg -i kite-ground-control_*.deb

# Fedora / openSUSE
sudo rpm -i kite-ground-control-*.rpm

# AppImage — no install needed
chmod +x Kite*.AppImage
./Kite*.AppImage
```

## Installed vs portable mode

You can run Kite two ways:

- **Installed** (the `.exe`/`.msi`/`.deb`/`.rpm`) — integrates with your system (Start menu / app
  launcher, uninstaller) and stores its data in your user profile (see [below](#where-your-data-is-stored)).
- **Portable** — use a standalone executable (or the AppImage) and place an **empty file named
  `.portable`** next to it. Kite then keeps **everything** — the flight database, raw logs, and any
  downloaded helper tools — in a single `data/` folder **next to the executable**, and writes nothing
  to your user profile.

!!! tip "When to go portable"
    Portable mode is ideal for a USB stick or a self-contained folder you can move between PCs, or when
    you want zero footprint outside the app's own directory. To switch a portable copy back to a normal
    install, just delete the `.portable` file (your data stays in `data/`).

## Where your data is stored

In a normal install Kite follows each OS's conventions; in portable mode everything lives under
`data/` next to the executable.

| Data | Windows (installed) | Linux (installed) | Portable |
|---|---|---|---|
| Flight database (`flights.db`) | `%APPDATA%\kite-gc\` | `~/.local/share/kite-gc/` | `<app>/data/` |
| Raw logs (`.tlog`, raw-MSP) | `Documents\KiteGC\` | `~/Documents/KiteGC/` (XDG) | `<app>/data/` |
| Downloaded helper tools | `%APPDATA%\kite-gc\bin\` | `~/.local/share/kite-gc/bin/` | `<app>/data/bin/` |
| Preferences & layout (settings, widget/panel layout) | web-view storage in your user profile | web-view storage in your user profile | `<app>/data/` |
| Window size & position | `%APPDATA%\com.kitegc.app\` | `~/.config/com.kitegc.app/` | not saved in portable mode |

Your **preferences and layout** are kept in the web view's local storage — Microsoft **WebView2** on
Windows, **WebKitGTK** on Linux — **not inside the program file**. In **portable mode** Kite redirects
that storage into the `data/` folder next to the executable, so a portable copy carries its settings
with it. (One exception: on Windows, portable mode doesn't restore the **window size/position**,
because that path can't be redirected.)

!!! note "Custom locations"
    The **database folder** and the **raw-log folder** are independent and can each be pointed
    anywhere in **Settings** — handy for putting the database on a larger or faster drive. On Windows
    the Documents path follows a OneDrive relocation automatically.

## Storage requirements

The app itself is small. What grows over time is your flight data:

- **Flight database** — grows with recorded telemetry (a time-series per flight). Typical flights are
  modest; a large library built over many flights can reach tens to a few hundred MB.
- **Imported INAV blackbox logs** can optionally keep the **original log file inside the database** —
  these are the biggest single contributor. You can **delete the stored original** for a flight at any
  time (from its logbook entry) to reclaim that space while keeping the decoded data.
- **Raw logs** (`.tlog` / raw-MSP) are written separately under `Documents/KiteGC` and grow with use —
  housekeep them as you like; they're independent of the database.

Keeping it tidy:

- Deleting flights reclaims space incrementally (the database auto-shrinks over time).
- **Settings → Data → Compact Database** runs a full defragmentation for maximum reclaim.
- Move the database to another drive via **Settings** if space is tight.

## External dependencies & automatic downloads

Kite needs **nothing extra to connect and fly**. A few **optional** features rely on a small helper
program, which Kite offers to **download automatically the first time you use that feature**:

| Helper | Used for | Auto-download |
|---|---|---|
| `blackbox_decode` | Importing **INAV blackbox** logs | **Windows & Linux** |
| `ffmpeg` | **Video** (fallback decoding for some RTSP sources) | **Windows** only |
| `go2rtc` | **Video** (the RTSP → low-latency engine) | **Windows** only |

- Downloaded helpers are stored in Kite's tools folder (`…\kite-gc\bin`, or `data\bin` in portable
  mode) — they don't touch your system.
- Kite finds a helper if it's **on your `PATH`**, **next to the app**, or in that tools folder. On a
  platform without automatic download (e.g. ffmpeg/go2rtc on Linux), install the tool yourself and put
  it on your `PATH` or next to the app — Kite will pick it up.
- These downloads need **internet access**. The **map** also needs it: both **2D map tiles** and
  **3D terrain** are streamed on demand and cached after first view — there's no offline map download
  yet (it's under consideration for the future if there's enough demand). Connecting to your aircraft,
  live telemetry, and logging all work fully offline.

## First run

Launch Kite, and head to **[your first connection](first-connection.md)**. New to the interface?
The **[quick tour](quick-tour.md)** points out where everything is.
