// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)
//
// Talk to the running Kite Android debug build's WebView over the Chrome DevTools protocol
// (debug builds enable WebView debugging). Two jobs the phone-UI loop needs:
//
//   node tools/phone-devtools.mjs shot out.png            # screenshot of the PAGE pixels
//   node tools/phone-devtools.mjs eval "innerWidth + 'x' + innerHeight"   # run JS, print result
//
// Why not `adb screencap`: Kite's activity window is transparent (native-video hole punch) and the
// EMULATOR's screencap returns a black frame for it, while the page itself renders fine.
// Page.captureScreenshot reads the WebView's own pixels, independent of the compositor.
// Caveat: the DevTools capture does NOT include WebGL content — in 3D map mode the map area comes
// out white. On a REAL device `adb exec-out screencap -p` works and shows everything; use that there.
//
// Options: --serial <adb serial> (default: the first emulator, else the first device),
//          --port <local port> (default 9333). Needs adb on PATH or ANDROID_SDK_ROOT/LOCALAPPDATA.
import { execFileSync } from 'node:child_process';
import { writeFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';

const args = process.argv.slice(2);
const opt = (name, def) => {
  const i = args.indexOf(name);
  if (i < 0) return def;
  const v = args[i + 1];
  args.splice(i, 2);
  return v;
};
const port = opt('--port', '9333');
let serial = opt('--serial', '');
const [cmd, arg] = args;

const sdk = process.env.ANDROID_SDK_ROOT || process.env.ANDROID_HOME || join(process.env.LOCALAPPDATA ?? '', 'Android', 'Sdk');
const adbPath = existsSync(join(sdk, 'platform-tools', 'adb.exe')) ? join(sdk, 'platform-tools', 'adb.exe') : 'adb';
const adb = (...a) => execFileSync(adbPath, serial ? ['-s', serial, ...a] : a, { encoding: 'utf8' }).trim();

if (!serial) {
  const rows = adb('devices').split('\n').slice(1).map((l) => l.trim().split(/\s+/)).filter((r) => r[1] === 'device');
  serial = (rows.find((r) => r[0].startsWith('emulator-')) ?? rows[0])?.[0] ?? '';
  if (!serial) { console.error('no adb device'); process.exit(1); }
}
const pid = adb('shell', 'pidof', 'com.kitegc.app');
if (!pid) { console.error(`Kite is not running on ${serial}`); process.exit(1); }
adb('forward', `tcp:${port}`, `localabstract:webview_devtools_remote_${pid}`);

const pages = await (await fetch(`http://localhost:${port}/json`)).json();
const page = pages.find((p) => p.type === 'page');
if (!page) { console.error('no page target', pages); process.exit(1); }
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });
const send = (method, params) =>
  new Promise((res) => {
    const id = Math.floor(Math.random() * 1e9);
    const onMsg = (ev) => {
      const m = JSON.parse(ev.data);
      if (m.id === id) { ws.removeEventListener('message', onMsg); res(m); }
    };
    ws.addEventListener('message', onMsg);
    ws.send(JSON.stringify({ id, method, params }));
  });

if (cmd === 'shot') {
  const out = arg || `phone-shot-${new Date().toISOString().replace(/[:.]/g, '-')}.png`;
  const r = await send('Page.captureScreenshot', { format: 'png' });
  if (!r.result?.data) { console.error('capture failed', JSON.stringify(r)); process.exit(1); }
  const buf = Buffer.from(r.result.data, 'base64');
  writeFileSync(out, buf);
  console.log(`${serial} → ${out} (${buf.length} bytes)`);
} else if (cmd === 'eval') {
  const r = await send('Runtime.evaluate', { expression: arg ?? 'location.href', returnByValue: true, awaitPromise: true });
  console.log(JSON.stringify(r.result?.result?.value ?? r, null, 1));
} else {
  console.error('usage: phone-devtools.mjs shot [out.png] | eval "<js>"   [--serial S] [--port P]');
  process.exit(1);
}
ws.close();
