# Building from source

How to set up a development environment and build Kite Ground Control yourself. The project uses
**[just](https://github.com/casey/just)** as its task runner for a consistent interface across Windows
and Linux.

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

### Android
Android support (Tauri Mobile) was experimented with but is **on hold** — it needs a separate UI and
build pipeline. Don't run `tauri android init`; it isn't supported right now.

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
just clean           # clean build artifacts
```

The classic commands still work too (`npm install`, `npm run tauri dev`, `npm run tauri build`).

### Build outputs

Every build (`just build` / `build-windows` / `build-linux`) gathers its final artifacts into a
**`release/`** folder at the repo root, so you don't have to dig through `target/release/bundle/`:

- **Windows:** the standalone `kite-gc.exe`, the NSIS `*-setup.exe` installer and the `*.msi`.
- **Linux:** the standalone `kite-gc` binary plus the `*.deb`, `*.AppImage` and `*.rpm` packages.

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

- **`blackbox_decode` is an external runtime dependency** (not bundled). Blackbox *import* shells out to
  INAV's `blackbox_decode`; the app looks for it next to the executable, then on `PATH`. Without it, only
  Blackbox import is affected — live recording and replay still work.
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
