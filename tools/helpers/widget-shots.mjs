// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)
//
// Renders every telemetry widget in every size from canned data and saves the pictures for the user docs.
// The widgets are the real components, mounted one at a time by the dev-only page `/dev/widgets`
// (src/routes/dev/widgets, fixture data in src/lib/dev/widgetFixtures.ts); a headless Edge captures them
// over the Chrome DevTools protocol.
//
//   node tools/helpers/widget-shots.mjs                          # all widgets, all sizes
//   node tools/helpers/widget-shots.mjs --only ahi,liveAgl       # just these widget ids (widgetRegistry ids)
//   node tools/helpers/widget-shots.mjs --out some/dir           # write somewhere else
//   node tools/helpers/widget-shots.mjs --browser "C:\path\to\chrome.exe"   # another Chromium-family browser
//
// Output (default docs/user/assets/getting-started/widgets/): the widget's default size as `<name>.png`
// (the names the docs link), every other size as `<name>_s.png` / `_l.png` / `_w.png`; one line per file.
// Pixels: slot box + 6 px body-coloured margin, at device scale factor 2 (L = 240 px, S = 144 px, W = 480 x 240).
// `battery2` is skipped: the same component as `battery`, only titled "BAT 2".
//
// Prerequisites: Node 22+ (built-in fetch / WebSocket), `npm install` done, Microsoft Edge (found under
// Program Files on Windows) or `--browser`. Uses the dev server from src-tauri/tauri.conf.json `devUrl`:
// a running one (e.g. `npx tauri dev`) is reused, otherwise `vite dev` is started and stopped again.
// The browser runs with a throw-away profile in the system temp folder.
import { spawn } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');

// Widget id → docs file name. The default-size names are linked from docs/user/getting-started/quick-tour.md.
const FILE_NAMES = {
  ahi: 'ahi',
  speed: 'speed',
  altitude: 'altitude',
  battery: 'battery',
  gps: 'gps',
  rcLink: 'rclink',
  compass: 'compass',
  home: 'home',
  flightMode: 'flightmode',
  liveAgl: 'liveagl',
  terrainRadar: 'terrainradar',
  weather: 'weather',
  videoFeed: 'video',
};
const SKIP = new Set(['battery2']);

const args = process.argv.slice(2);
const opt = (name) => {
  const i = args.indexOf(name);
  if (i < 0) return undefined;
  const v = args[i + 1];
  if (v === undefined || v.startsWith('--')) fail(`${name} needs a value`);
  args.splice(i, 2);
  return v;
};
function fail(msg) {
  console.error(`widget-shots: ${msg}`);
  process.exit(1);
}
const only = opt('--only')?.split(',').map((s) => s.trim()).filter(Boolean);
const outDir = resolve(opt('--out') ?? join(ROOT, 'docs', 'user', 'assets', 'getting-started', 'widgets'));
const browserArg = opt('--browser');
if (args.length) fail(`unknown argument(s): ${args.join(' ')}\nusage: widget-shots.mjs [--only id,id] [--out dir] [--browser path]`);

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const tauriConf = JSON.parse(readFileSync(join(ROOT, 'src-tauri', 'tauri.conf.json'), 'utf8'));
const devUrl = new URL(tauriConf.build?.devUrl ?? fail('no build.devUrl in src-tauri/tauri.conf.json'));
const pageUrl = (query) => new URL(`/dev/widgets${query}`, devUrl).href;

async function answers(url) {
  try {
    const r = await fetch(url, { signal: AbortSignal.timeout(2000) });
    return r.ok;
  } catch {
    return false;
  }
}

let vite = null;
let browser = null;
let profileDir = null;
let ws = null;

async function startVite() {
  if (await answers(devUrl.href)) return;
  const bin = join(ROOT, 'node_modules', 'vite', 'bin', 'vite.js');
  if (!existsSync(bin)) fail('node_modules/vite missing - run npm install');
  let log = '';
  vite = spawn(process.execPath, [bin, 'dev'], { cwd: ROOT, stdio: ['ignore', 'pipe', 'pipe'] });
  vite.stdout.on('data', (d) => (log += d));
  vite.stderr.on('data', (d) => (log += d));
  const deadline = Date.now() + 90_000;
  while (Date.now() < deadline) {
    if (vite.exitCode !== null) throw new Error(`vite dev exited:\n${log}`);
    if (await answers(devUrl.href)) return;
    await sleep(300);
  }
  throw new Error(`vite dev did not answer on ${devUrl.href}:\n${log}`);
}

function findBrowser() {
  if (browserArg) {
    if (!existsSync(browserArg)) fail(`--browser: ${browserArg} not found`);
    return browserArg;
  }
  const env = process.env;
  const candidates = [
    env['ProgramFiles(x86)'] && join(env['ProgramFiles(x86)'], 'Microsoft', 'Edge', 'Application', 'msedge.exe'),
    env.ProgramFiles && join(env.ProgramFiles, 'Microsoft', 'Edge', 'Application', 'msedge.exe'),
    env.LOCALAPPDATA && join(env.LOCALAPPDATA, 'Microsoft', 'Edge', 'Application', 'msedge.exe'),
    '/usr/bin/microsoft-edge',
    '/usr/bin/chromium',
    '/usr/bin/google-chrome',
    '/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge',
  ].filter(Boolean);
  const found = candidates.find((p) => existsSync(p));
  if (!found) fail(`no browser found (looked for Microsoft Edge in:\n  ${candidates.join('\n  ')})\npass --browser <path to a Chromium-family browser>`);
  return found;
}

async function startBrowser() {
  profileDir = mkdtempSync(join(tmpdir(), 'kite-widget-shots-'));
  browser = spawn(browserPath, [
    '--headless=new',
    '--remote-debugging-port=0',
    `--user-data-dir=${profileDir}`,
    '--no-first-run',
    '--no-default-browser-check',
    '--disable-extensions',
    '--disable-sync',
    '--disable-background-networking',
    '--disable-component-update',
    '--hide-scrollbars',
    '--mute-audio',
    '--force-color-profile=srgb',
    '--lang=en-US',
    'about:blank',
  ], { stdio: 'ignore' });
  const portFile = join(profileDir, 'DevToolsActivePort');
  const deadline = Date.now() + 20_000;
  while (!existsSync(portFile) || !readFileSync(portFile, 'utf8').includes('\n')) {
    if (browser.exitCode !== null) throw new Error('the browser exited at start');
    if (Date.now() > deadline) throw new Error('the browser opened no DevTools port');
    await sleep(100);
  }
  const port = readFileSync(portFile, 'utf8').split('\n')[0].trim();
  const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
  const page = targets.find((t) => t.type === 'page');
  if (!page) throw new Error('no page target in the browser');
  ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((res, rej) => {
    ws.onopen = res;
    ws.onerror = rej;
  });
}

let nextId = 1;
const pending = new Map();
const pageErrors = [];
function onMessage(ev) {
  const m = JSON.parse(ev.data);
  if (m.id && pending.has(m.id)) {
    const { res, rej } = pending.get(m.id);
    pending.delete(m.id);
    if (m.error) rej(new Error(`${m.error.message} (${m.error.code})`));
    else res(m.result);
  } else if (m.method === 'Runtime.exceptionThrown') {
    pageErrors.push(m.params.exceptionDetails.exception?.description ?? m.params.exceptionDetails.text);
  } else if (m.method === 'Runtime.consoleAPICalled' && m.params.type === 'error') {
    pageErrors.push(m.params.args.map((a) => a.value ?? a.description ?? '').join(' '));
  }
}
const send = (method, params = {}) =>
  new Promise((res, rej) => {
    const id = nextId++;
    pending.set(id, { res, rej });
    ws.send(JSON.stringify({ id, method, params }));
  });
async function evaluate(expression) {
  const r = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
  if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description ?? r.exceptionDetails.text);
  return r.result.value;
}

/** Load `url` and wait for the page's ready mark (set after its fixture flight, fonts and animations). */
async function load(url, timeoutMs) {
  await send('Page.navigate', { url });
  const deadline = Date.now() + timeoutMs;
  const check = `location.href === ${JSON.stringify(url)} && document.documentElement.dataset.widgetReady === '1'`;
  while (Date.now() < deadline) {
    if (await evaluate(check).catch(() => false)) return;
    await sleep(100);
  }
  throw new Error(`${url} did not get ready within ${timeoutMs / 1000} s`);
}

function pngSize(buf) {
  return `${buf.readUInt32BE(16)}x${buf.readUInt32BE(20)}`;
}

async function cleanup() {
  try {
    if (ws?.readyState === WebSocket.OPEN) await Promise.race([send('Browser.close'), sleep(2000)]);
  } catch {
    /* already gone */
  }
  ws?.close();
  if (browser && browser.exitCode === null) browser.kill();
  if (vite && vite.exitCode === null) vite.kill();
  if (profileDir) {
    // The browser releases its profile a moment after it exits.
    for (let i = 0; i < 20 && existsSync(profileDir); i++) {
      try {
        rmSync(profileDir, { recursive: true, force: true });
      } catch {
        await sleep(250);
      }
    }
  }
}
process.on('SIGINT', () => void cleanup().finally(() => process.exit(130)));

const browserPath = findBrowser();

try {
  await startVite();
  await startBrowser();
  ws.addEventListener('message', onMessage);
  await send('Page.enable');
  await send('Runtime.enable');
  await send('Emulation.setDeviceMetricsOverride', { width: 1200, height: 900, deviceScaleFactor: 2, mobile: false });

  // The index lists every widget with its sizes (widgetRegistry); read it, don't load its frames.
  const indexUrl = pageUrl('');
  await send('Page.navigate', { url: indexUrl });
  let widgets = null;
  for (let i = 0; i < 600 && !widgets?.length; i++) {
    await sleep(100);
    widgets = await evaluate(
      `[...document.querySelectorAll('section[data-widget]')].map((s) => ({ id: s.dataset.widget, sizes: s.dataset.sizes.split(','), def: s.dataset.default }))`,
    ).catch(() => null);
  }
  if (!widgets?.length) throw new Error(`${indexUrl} listed no widgets`);
  if (only) {
    const unknown = only.filter((id) => !widgets.some((w) => w.id === id));
    if (unknown.length) throw new Error(`--only: unknown widget id(s) ${unknown.join(', ')}; known: ${widgets.map((w) => w.id).join(', ')}`);
  }
  const todo = widgets.filter((w) => (only ? only.includes(w.id) : !SKIP.has(w.id)));

  // First load: vite may still optimise dependencies and reload the page once.
  const first = pageUrl(`?id=${todo[0].id}&size=${todo[0].def}`);
  await load(first, 120_000);
  await sleep(1500);
  await load(first, 60_000);

  mkdirSync(outDir, { recursive: true });
  for (const w of todo) {
    const name = FILE_NAMES[w.id] ?? w.id.toLowerCase();
    if (!FILE_NAMES[w.id]) console.log(`note: ${w.id} has no docs file name in FILE_NAMES, using ${name}`);
    for (const size of w.sizes) {
      pageErrors.length = 0;
      await load(pageUrl(`?id=${encodeURIComponent(w.id)}&size=${size}`), 30_000);
      const r = await evaluate(`(() => { const b = document.querySelector('.shot').getBoundingClientRect(); return { x: b.x, y: b.y, width: b.width, height: b.height }; })()`);
      const shot = await send('Page.captureScreenshot', { format: 'png', clip: { ...r, scale: 1 }, captureBeyondViewport: false });
      const buf = Buffer.from(shot.data, 'base64');
      const file = join(outDir, size === w.def ? `${name}.png` : `${name}_${size.toLowerCase()}.png`);
      writeFileSync(file, buf);
      console.log(`${relative(ROOT, file)}  ${pngSize(buf)}`);
      for (const e of pageErrors) console.log(`  page error: ${e.split('\n')[0]}`);
    }
  }
} catch (e) {
  console.error(`widget-shots: ${e instanceof Error ? e.message : e}`);
  process.exitCode = 1;
} finally {
  await cleanup();
}
