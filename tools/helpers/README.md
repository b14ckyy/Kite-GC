# Helpers

Small scripts for the development loop itself: translating, iterating on the phone UI, and the widget
pictures of the user docs.

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

## `i18n-check.mjs` — locale files against `en.json`

Compares every locale file with `en.json`. `de.json` has to match it exactly (same keys, same
`{placeholders}`); the community locales may lack keys, which then fall back to English, but must not
carry keys `en.json` does not have, and a translated string must have the same placeholders. An empty
string counts as missing. Exit code 1 on an error.

```sh
npm run check:i18n                                  # or: node tools/helpers/i18n-check.mjs
node tools/helpers/i18n-check.mjs --all             # also list the keys a community locale lacks
node tools/helpers/i18n-check.mjs --strict de,fr    # locales that must be complete (replaces the default, de)
```

It does not find hard-coded text in components; that stays a review point.

## `phone-emu.ps1` — Android emulator / device loop (Windows)

Boots an Android emulator (or uses the USB-attached phone) and runs the app through
`tauri android dev`: Rust is built once, the UI comes from the Vite dev server with hot reload, so a
saved `.svelte` file shows up on the device about a second later — no APK, no `adb install`.

```powershell
.\tools\helpers\phone-emu.ps1                # list the AVDs
.\tools\helpers\phone-emu.ps1 Kite_21x9      # boot that AVD + tauri android dev on it
.\tools\helpers\phone-emu.ps1 -Device        # skip the emulator: dev on the USB device
.\tools\helpers\phone-emu.ps1 -Device -Beside   # as "Kite Dev", beside a copy installed from Google Play
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

## `widget-shots.mjs` — widget pictures for the user docs

Renders every telemetry widget in every size from the same canned in-flight state and saves the
pictures to `docs/user/assets/getting-started/widgets/`. The widgets are the real components, mounted
one at a time by the dev-only page `/dev/widgets` (fixture data and the mocked backend answers in
`src/lib/dev/widgetFixtures.ts`); a headless Microsoft Edge captures each slot over the Chrome DevTools
protocol. No npm dependency beyond the project's own.

```sh
node tools/helpers/widget-shots.mjs                        # all widgets, all sizes
node tools/helpers/widget-shots.mjs --only ahi,liveAgl     # these widget ids only (widgetRegistry ids)
node tools/helpers/widget-shots.mjs --out some/dir         # another output folder
node tools/helpers/widget-shots.mjs --browser <path>       # another Chromium-family browser
```

- The default size is written as `<name>.png` (the names the docs link), the other sizes as
  `<name>_s.png` / `_l.png` / `_w.png`; slot box plus a 6 px body-coloured margin, at device scale
  factor 2. `battery2` is skipped (the `battery` component with the title "BAT 2").
- Uses the dev server of `src-tauri/tauri.conf.json` `devUrl`: a running one is reused, otherwise the
  script starts `vite dev` and stops it again. The browser gets a throw-away profile in the temp folder.
- Repeated runs produce byte-identical files on the same machine. Another font set or browser version
  changes the pixels.
- Open `http://localhost:1420/dev/widgets` in a dev session to see all widgets in all sizes on one page;
  `?id=<widget>&size=S|L|W` shows one. The page does not exist in a production build.
