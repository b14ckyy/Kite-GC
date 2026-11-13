# Helpers

Small scripts for the development loop itself: translating, and iterating on the phone UI.

---

## `i18n-key-paths.py` — see every i18n key on screen

Writes a pseudo-locale `src/lib/i18n/locales/xx.json` from `en.json` in which every string is replaced
by its own key path (`sensors.gyro`, `rcLink.noLink`, …). Switch the running app to it and the
interface shows, at every position, the key you need to edit instead of the text. By Teodor Yantcheff.

```sh
python tools/helpers/i18n-key-paths.py
```

Then register the locale **locally** in `src/lib/i18n/index.ts` — `register('xx', () =>
import('./locales/xx.json'))` and `{ code: 'xx', label: 'i18n-paths' }` in `SUPPORTED_LOCALES` — start
the dev app and pick "i18n-paths" as the language. `xx.json` is git-ignored; take the `index.ts`
registration out again before committing.

## `phone-emu.ps1` — Android emulator / device loop (Windows)

Boots an Android emulator (or uses the USB-attached phone) and runs the app through
`tauri android dev`: Rust is built once, the UI comes from the Vite dev server with hot reload, so a
saved `.svelte` file shows up on the device about a second later — no APK, no `adb install`.

```powershell
.\tools\helpers\phone-emu.ps1                # list the AVDs
.\tools\helpers\phone-emu.ps1 Kite_21x9      # boot that AVD + tauri android dev on it
.\tools\helpers\phone-emu.ps1 -Device        # skip the emulator: dev on the USB device
.\tools\helpers\phone-emu.ps1 -Shot          # screenshot of the running app (via phone-devtools.mjs)
```

- Needs the Android SDK under `%LOCALAPPDATA%\Android\Sdk` (emulator + platform-tools) and Node.
- The dev server is reached over `adb reverse tcp:1420` (the emulator's NAT path stalled), every adb
  call is pinned to one serial, so a phone on the cable next to the emulator does not confuse it.
- The AVD set (5" 16:9, 6.4" 20:9, 6" 21:9, and two RadioMaster AX12 stand-ins at 1280×720) is listed in
  the script header, with the one rule that bites: `Kite_AX12_A9` must boot with `-writable-system`
  or Android drops its sideloaded WebView (the script adds the flag).
- Inside the emulator the host is `10.0.2.2` (SITL over TCP, MAVLink UDP), or import a `.kflight`.
  Ctrl+C ends tauri dev + vite; close the emulator window separately. "Port 1420 is already in use"
  means a previous vite is still alive.

## `phone-devtools.mjs` — screenshots and JS on the phone build

Talks to the running Kite Android **debug** build's WebView over the Chrome DevTools protocol (debug
builds enable WebView debugging; the script sets up `adb forward` to the WebView's devtools socket,
local port 9333 by default).

```sh
node tools/helpers/phone-devtools.mjs shot out.png                            # screenshot of the PAGE pixels
node tools/helpers/phone-devtools.mjs eval "innerWidth + 'x' + innerHeight"   # run JS in the page, print the result
```

Why not `adb screencap`: Kite's activity window is transparent (native-video hole punch) and the
**emulator's** screencap returns a black frame for it, while the page itself renders fine —
`Page.captureScreenshot` reads the WebView's own pixels. Caveat: that capture has no WebGL content, so
in 3D map mode the map area comes out white. On a **real device** `adb exec-out screencap -p` works and
shows everything — use that there. Options: `--serial <adb serial>` (default:
the first emulator, else the first device), `--port <local port>`.
