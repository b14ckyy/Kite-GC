# Building from source

How to set up a development environment and build Kite Ground Control yourself. The project uses
**[just](https://github.com/casey/just)** as its task runner for a consistent interface across Windows,
Linux and macOS.

!!! note "Heavy system dependencies are manual"
    The toolchains and system libraries below must be installed **manually** — they need administrative
    rights and significant system changes, so there are no automated scripts for them.

## Prerequisites

### All platforms
- **[Node.js](https://nodejs.org/)** LTS (v20 or v24)
- **npm 11+** — keep it in sync across your dev machines. Older npm (10.x) rewrites `package-lock.json`
  on every install by stripping the `libc` fields of the optional native deps, so mixing versions makes
  the lockfile flip-flop. Upgrade with `npm install -g npm@latest` (Node 22.9+ / 24 already bundle 11).
- **[Rust](https://rustup.rs/)** (via rustup)
- **[just](https://github.com/casey/just)** — strongly recommended

### Windows (primary platform)
1. **Visual Studio Build Tools 2022** with the **"Desktop development with C++"** workload (the MSVC
   compiler/linker that Rust needs).
2. **WebView2 Runtime** — usually already present on Windows 10/11; otherwise from Microsoft.

```powershell
winget install OpenJS.NodeJS.LTS
winget install Casey.Just.Just
# Rust: install from https://rustup.rs/
```

!!! warning "Restart your terminal"
    After installing any of these, **fully restart your terminal and editor** so the new PATH entries
    are picked up. A blank terminal that "can't find just/cargo/node" is almost always this.

### Linux (Debian / Ubuntu based)

```bash
sudo apt update
sudo apt install -y \
    build-essential pkg-config curl wget file \
    libssl-dev libgtk-3-dev libwebkit2gtk-4.1-dev \
    libayatana-appindicator3-dev librsvg2-dev libxdo-dev \
    libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev
```

!!! note
    `libwebkit2gtk-4.1-dev` is the Tauri 2 / WebKitGTK 4.1 package — you need a distro new enough to
    ship 4.1 (Ubuntu 22.04+ / Debian 12+). The two GStreamer dev packages are required by the native
    RTSP video sink (the app links the system GStreamer at build time). Install Node.js, Rust and just
    via their official methods.

!!! warning "Build on the oldest system you want to support"
    Two things follow from the build host, and both reach the user.

    **glibc is never bundled** — not even in the AppImage — so the highest `GLIBC_*` symbol in the
    binary becomes a hard minimum for everyone running it. Building on Debian 13 produces something
    that will not start on Ubuntu 22.04.

    **The AppImage bundles the build host's WebKitGTK** (linuxdeploy takes what is installed), so the
    build machine decides the browser engine every AppImage user gets. Avoid the **2.50 series**:
    stopping a video stream can leave it spinning on a full CPU core until the app exits. 2.52 is
    clean. That is why the release workflow builds on Ubuntu 24.04 rather than an older base — the
    `.deb` and `.rpm` use the *system* engine and are unaffected either way, but they share the job.

### macOS

1. **Xcode Command Line Tools** — `xcode-select --install` (compiler / linker + SDK).
2. Node.js, Rust and just via their official installers (e.g. `brew install node just`, Rust from rustup).

The macOS build is **universal** (arm64 + x86_64). `just build-macos` adds both Rust targets and fetches
the bundled ffmpeg for you, then builds the `.app` + `.dmg`. The result is **unsigned**; `just
notarize-macos` signs + notarizes it for distribution without a Gatekeeper prompt (needs an Apple
Developer account, credentials read from your environment / keychain — never committed).

### Android

Mobile is **not part of the 1.0 line** — nothing mobile builds from `master`, and both mobile ports
target a release after 1.0. The iOS / iPadOS port lives on `development` (see the iOS notes above);
Android is described here.

Android is **experimental**. The app builds and installs, and connects over **USB serial** (OTG),
**Bluetooth LE** or **UDP / TCP**. The interface is still the desktop one: it fits a tablet in
landscape, and is cramped on a phone.

USB serial goes through the Android USB Host API, so the driver lives in Kotlin
(`gen/android/app/src/main/java/com/kitegc/app/UsbSerial.kt`) with a JNI shim in front of it
(`src-tauri/src/transport/serial_android.rs`). Two device families are driven:

| Family | Covers |
| --- | --- |
| **CDC-ACM** | INAV / Betaflight / ArduPilot flight controllers over USB, ESP32-S2/S3/C3 native USB |
| **CP210x** | Silicon Labs bridges — most SiK telemetry radios |

FTDI (RFD900) and CH340 are **not** driven yet; both slot in as another `SerialDriver` in that Kotlin
file. Android grants USB access per device and per session: plugging the cable in and picking Kite
from the system dialog grants it up front, otherwise the first connect raises the permission prompt.

Bluetooth LE follows the same shape on the platform's own GATT stack (`BleSerial.kt` with
`src-tauri/src/transport/ble_android.rs` in front of it) — the desktop's `btleplug` has no usable
Android backend without a companion Java library. The known BLE-serial profiles (CC2541 / HM-10,
Nordic UART, SpeedyBee) live in `src-tauri/src/transport/ble_profiles.rs`, shared with the desktop
backend, so a new adapter family is a one-line Rust change. The first scan raises the system's
"nearby devices" permission prompt (Android 12+; location before that).

#### Getting an APK without building one

The **Android** workflow in GitHub Actions builds the APK and uploads it as a run artifact — Actions →
Android → *Run workflow*. That is the recommended route: the runner already has the SDK, the JDK and
`sdkmanager`, so nothing has to be installed locally.

#### Signing

Release builds are signed with the **debug** keystore unless you supply a real one, because an APK
with no signature at all cannot be installed — Android rejects it as an invalid package. That is
enough to sideload, and not enough to publish.

⚠️ **The debug keystore is generated per machine, and a CI runner is a fresh machine every run.** So
consecutive CI builds have *different* signatures, and Android refuses to install one over the other
("package conflicts with an existing package"). Updating then means uninstalling — which erases the
flight database. Supply a real key and the problem disappears for good.

**For CI**, add four repository secrets and the workflow signs every build with the same key:

```bash
keytool -genkey -v -keystore kite.jks -alias kite -keyalg RSA -keysize 2048 -validity 10000
base64 -w0 kite.jks     # → KEYSTORE_B64
```

| Secret | Value |
| --- | --- |
| `KEYSTORE_B64` | base64 of the `.jks` |
| `KEYSTORE_PASSWORD` | store password |
| `KEY_ALIAS` | key alias (`kite` above) |
| `KEY_PASSWORD` | key password |

With none set the build still succeeds — it just warns and uses the throwaway key. Keep the `.jks`
and its passwords somewhere safe: lose them and you can never update an installed app again, only
replace it.

To sign with a real key, drop a `key.properties` next to `gen/android/app/build.gradle.kts`:

```properties
storeFile=/absolute/path/to/keystore.jks
storePassword=…
keyAlias=…
keyPassword=…
```

It is gitignored, so the key never lands in the repository.

#### Building locally

1. **JDK 17** — `sudo apt install openjdk-17-jdk` (or Temurin on Windows/macOS).
2. **Android SDK** — Android Studio, or just the command-line tools. Set `ANDROID_HOME`.
3. **NDK r27** — `sdkmanager --install "ndk;27.2.12479018"`, then point `NDK_HOME` at it.
4. **Rust targets** — `rustup target add aarch64-linux-android` (add `armv7-linux-androideabi`,
   `i686-linux-android`, `x86_64-linux-android` only if you need those ABIs; arm64 covers every
   Android phone and tablet made in the last decade).

```bash
npm ci
npm run tauri android build -- --apk --target aarch64   # APK in src-tauri/gen/android/app/build/outputs/apk
npm run tauri android dev                               # on-device dev build with hot reload
```

`src-tauri/gen/android` is **committed** — it is the Android app project (manifest, Gradle build,
icons, the Rust Gradle plugin), edited by hand. Do **not** run `tauri android init`: it would
regenerate the project and overwrite the manifest's permissions and the landscape orientation.

To type-check the Rust side for Android without a full Gradle build, use `just check-android`, or by
hand:

```bash
BIN="$NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin"
export CC_aarch64_linux_android="$BIN/aarch64-linux-android24-clang"
export AR_aarch64_linux_android="$BIN/llvm-ar"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$CC_aarch64_linux_android"
cargo check --manifest-path src-tauri/Cargo.toml --target aarch64-linux-android --lib
```

Those variables are not optional: `cc-rs` looks for an `aarch64-linux-android-clang` that the NDK does
not ship — its compilers are API-versioned (`…android24-clang`, matching `minSdk`) — so without them
the C dependencies (`ring`, bundled SQLite, `zstd`) fail to configure. `tauri android build` sets this
up itself, so only the bare `cargo check` needs it. (On Windows the prebuilt directory is
`windows-x86_64` and the compilers are `.cmd` wrappers.)

#### What does not work yet on Android

| Feature | State | Why |
| --- | --- | --- |
| UDP / TCP links | ✅ works | Plain sockets. |
| USB serial (CDC-ACM, CP210x) | ✅ works | Android USB Host API, driver in `UsbSerial.kt`. Needs an OTG cable. |
| USB serial (FTDI, CH340) | ❌ not implemented | Those two chips need their own driver; the devices are not listed as ports. |
| Flight log, missions, fleet & battery manager | ✅ works | SQLite in app-private storage (`android::app_data_dir`). |
| Maps, terrain, weather | ✅ works | Network + the same tile cache as desktop. |
| Widget layout editing by touch | ✅ works | Drag widgets between the docks with a finger in edit mode, same as with a mouse on desktop. |
| Screen stays on while connected | ✅ works | Like a navigation app: the display does not time out while a telemetry link is active; disconnected, the normal OS timeout applies. |
| Bluetooth LE | ✅ works | Native GATT via `BleSerial.kt`; the same serial profiles as desktop. Listen-only mode (the GATT dump) is not implemented yet. |
| Joystick / HID RC control | ❌ not implemented | The backend is per-OS (WGI / evdev / IOKit); Android has none, so no device is ever listed. |
| Log import (.kflight / .rawmsp / .tlog) | ✅ works | Parsed in-process. A `.kflight` brings the flight, its track and its records; the archived original log files inside it (INAV blackbox, ArduPilot dataflash) are left behind — the mobile database never stores originals. ArduPilot `.bin` import is desktop-only. |
| Blackbox import (.txt / .bbl / .bfl) | ❌ impossible | Needs the external `blackbox_decode` tool, and Android forbids executing a downloaded binary. Import on a desktop and bring flights over as `.kflight`. |
| Video: camera / OTG capture device | ✅ works (tablets) | The WebView's own capture path (`getUserMedia`), no helper involved. Hidden on phones until the phone UI decides. |
| Video: RTSP (MJPEG / H.264 / HEVC) | ✅ works | Runs entirely on Kite's built-in RTSP client — no engine helpers (an Android app cannot spawn them, so the desktop engines are not offered). H.264/HEVC decode in device hardware onto a native surface below the WebView; MJPEG rides the multipart path. Native capture (ffmpeg) is not offered — the OS's own capture devices arrive through the camera kind. |
| Custom database / raw-log folder | ✅ mirror | Browse grants ONE folder via the system picker (scoped storage — no permission). The app keeps working app-private (SQLite and the raw writers need real paths, which a SAF grant cannot provide) and **mirrors into the folder at session end**: raw logs as files, the database as an atomic snapshot. That copy survives an uninstall. |
| Exports to `Documents/` | ⚠️ different | Scoped storage makes Documents a MediaStore collection, not a path — exports go through the system share/save dialog instead. |

Where a feature is missing the backend returns a clear message rather than failing to build, so the
rest of the app is unaffected.

## Workflow

```bash
just install      # install frontend dependencies (npm install)
just dev          # start development mode with hot reload
```

Other useful commands:

```bash
just --list          # list all commands
just check           # svelte-check + cargo check
just build           # production build for the current platform
just build-windows   # explicit Windows release build
just build-linux     # explicit Linux release build
just build-macos     # explicit macOS universal build (unsigned; adds mac targets + ffmpeg)
just clean           # clean build artifacts
```

The classic commands still work too (`npm install`, `npm run tauri dev`, `npm run tauri build`).

### Build outputs

Every build (`just build` / `build-windows` / `build-linux` / `build-macos`) gathers its final artifacts
into a **`release/`** folder at the repo root, renamed to a unified scheme so local builds match the CI
release builds:

```
KiteGC_<OS>_<arch>_<version>_<type>.<ext>
```

- **Type** = `installer` (`.exe` / `.deb` / `.rpm` / `.dmg`), `standalone` (`.AppImage`, or the macOS
  `.app` zipped), or `portable` (the bare executable zipped **with an empty `.portable` marker**).
- The naming logic lives in `scripts/collect-release.*` and is shared by local builds and the GitHub
  release workflow (`.github/workflows/release.yml`), so filenames match everywhere.

The folder is refreshed on each build and is git-ignored (local to your machine). The raw outputs also
remain in `src-tauri/target/release/` (and its `bundle/` subfolders) as usual.

## Quality checks

```bash
just check
```

runs `npm run check` (svelte-check + TypeScript) and `cargo check`. `just check` / `just clean` have
platform-specific variants (via just's `[windows]` / `[unix]` attributes) and pick the right one
automatically. CI runs the same checks (plus clippy) on every push and pull request; full release builds
are not run in CI.

## Troubleshooting

??? question "`just` is not recognized"
    Restart your terminal/editor completely; verify with `where.exe just` (Windows). winget installs it
    for the current user.

??? question "Editor terminal can't find just / cargo / node"
    Very common on Windows — fully close and reopen the editor (or reload the window).

??? question "Linker errors during cargo check / build (Windows)"
    You're missing the C++ build tools — install **"Desktop development with C++"** via the Visual Studio
    Installer.

## Linux runtime notes

These are things to know when **running** the packaged Linux app (not build errors):

- **Three helpers are fetched at runtime, not bundled**: `blackbox_decode` (Blackbox import shells out
  to INAV's decoder), `ffmpeg` (the classic video image path) and `mediamtx` (the classic RTSP → WebRTC
  engine) — the native RTSP client needs neither. Kite
  looks for each next to the executable, then in its own app-data `bin/` directory, then on `PATH`, and
  offers to download it on first use. Each failure is local to its feature — without `blackbox_decode`
  only Blackbox import is affected, live recording and replay still work. On macOS `ffmpeg` is bundled
  as a universal sidecar instead; the other two are fetched the same way as everywhere else.
- **Serial permissions:** add yourself to the `dialout` group (`sudo usermod -aG dialout "$USER"`, then
  log out/in). Some distros use `uucp`.
- **Blank 3D globe / WebView (WebKitGTK):** the 3D map runs in WebKitGTK, which is more fragile than
  Windows' WebView2 (notably on some Nvidia setups). Try launching with the DMA-BUF renderer disabled:
  ```bash
  WEBKIT_DISABLE_DMABUF_RENDERER=1 ./kite-gc      # most common fix
  WEBKIT_DISABLE_COMPOSITING_MODE=1 ./kite-gc     # if compositing misbehaves
  ```
- **Which package to test:** `tauri build` produces `.deb`, `.AppImage` and `.rpm` in
  `src-tauri/target/release/bundle/`. The `.deb` (or the raw binary in `…/release/`) is usually the most
  reliable first smoke test; AppImage adds its own FUSE/sandbox layer — and **video does not work
  inside the AppImage at all** (its launcher environment hides the system GStreamer plugins; see the
  user guide's platform notes).
- **Data locations:** installed mode stores the flight DB + terrain cache under `~/.local/share/kite-gc/`.
  A **portable** build (a `.portable` marker file next to the binary) keeps everything in a `data/` folder
  beside the binary.
