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
    libayatana-appindicator3-dev librsvg2-dev libxdo-dev
```

!!! note
    `libwebkit2gtk-4.1-dev` is the Tauri 2 / WebKitGTK 4.1 package — you need a distro new enough to
    ship 4.1 (Ubuntu 22.04+ / Debian 12+). Install Node.js, Rust and just via their official methods.

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

### Android / iOS
Mobile (Tauri Mobile) is **not part of the 1.0 line**. The Android and iOS / iPadOS ports are both
merged on `development` (each with its own UI tier and build pipeline) and target a release after
1.0. Nothing mobile builds from `master` — don't run `tauri android init` or `tauri ios init`
against a 1.0 checkout.

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
  to INAV's decoder), `ffmpeg` (the video image path) and `mediamtx` (the RTSP → WebRTC engine). Kite
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
  reliable first smoke test; AppImage adds its own FUSE/sandbox layer.
- **Data locations:** installed mode stores the flight DB + terrain cache under `~/.local/share/kite-gc/`.
  A **portable** build (a `.portable` marker file next to the binary) keeps everything in a `data/` folder
  beside the binary.
