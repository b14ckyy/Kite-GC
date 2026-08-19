// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Main-thread side of the off-thread MJPEG reader (see `mjpegWorker.ts` for the why and the
// measurements). One worker reads the multipart stream and draws into every attached canvas; this
// module owns the worker, hands surfaces in via a Svelte action, and republishes its stats.
//
// Everything degrades to the old <img> sink when a WebView lacks a piece — `canvasSinkAvailable` is
// the single switch the surfaces branch on.

import { writable, get } from 'svelte/store';
import { isWebKit } from '$lib/platform';

/** Live figures from the reader, refreshed once a second while a feed runs. `fpsIn` is what arrives
 *  from the server, `fpsOut` what actually reaches a canvas — the gap is the machine falling behind. */
export interface MjpegStats {
  fpsIn: number;
  fpsOut: number;
  kbps: number;
  /** Frames skipped in total and in the last second. */
  dropped: number;
  droppedNow: number;
  corrupt: number;
  /** Longest interval in the last second between two arrivals / two drawn frames. A visible freeze
   *  IS a long `gapDraw`; a matching `gapIn` puts the cause upstream of this machine. */
  gapIn: number;
  gapDraw: number;
  decodeAvgMs: number;
  decodeMaxMs: number;
  width: number;
  height: number;
  /** Milliseconds since the last frame arrived, or -1 before the first one. */
  sinceFrameMs: number;
  /** What the smoothing buffer holds right now, and the depth it was asked for. Both 0 with the
   *  buffer off — which is what says whether a chosen cushion is deep enough for this link. */
  bufferedMs: number;
  bufferFrames: number;
}

export const mjpegStats = writable<MjpegStats | null>(null);

/** Longest gap between two main-thread animation frames in the last second. The worker draws
 *  independently of this, so a smooth `gapDraw` next to a large `uiJankMs` means the picture is being
 *  produced fine and something else in the app is stalling — a different problem with a different fix. */
export const uiJankMs = writable<number | null>(null);

let jankRaf = 0;

/** Only runs while someone is looking (the Debug Monitor's video tab) — a permanent rAF loop would
 *  keep the page ticking for nothing on a machine that has little to spare. */
export function startJankProbe(): void {
  if (jankRaf || typeof requestAnimationFrame !== 'function') return;
  let prev = performance.now();
  let worst = 0;
  let windowStart = prev;
  const tick = (now: number) => {
    worst = Math.max(worst, now - prev);
    prev = now;
    if (now - windowStart >= 1000) {
      uiJankMs.set(worst);
      worst = 0;
      windowStart = now;
    }
    jankRaf = requestAnimationFrame(tick);
  };
  jankRaf = requestAnimationFrame(tick);
}

export function stopJankProbe(): void {
  if (jankRaf) cancelAnimationFrame(jankRaf);
  jankRaf = 0;
  uiJankMs.set(null);
}

type OutMessage =
  | ({ type: 'stats' } & MjpegStats)
  | { type: 'size'; width: number; height: number }
  | { type: 'frame'; bitmap: ImageBitmap }
  | { type: 'error'; message: string; everDrew: boolean };

/** Whether the worker may own the surfaces' canvases (`transferControlToOffscreen`) and draw into
 *  them itself, or whether it hands each decoded frame to the main thread to draw.
 *
 *  **WebKit gets the second one because the first crashes it.** Measured on WebKitGTK 2.52.5, a
 *  worker drawing a 720p bitmap at 60 fps: one transferred canvas runs clean (59.4 fps on the main
 *  thread), **two kill the web process** — a hard renderer crash, not a memory limit, which is what
 *  froze the window a few frames after the stream came up. Handing the `ImageBitmap` over and drawing
 *  on the main thread carries all four surfaces at 59 fps with the same worst-case gap.
 *
 *  Kite shows up to four surfaces at once (panel, widget, floating window, map swap), so one canvas
 *  is not a usable ceiling. What matters is kept either way: the JPEG decode stays off the main
 *  thread, frame lifetime stays explicit, and one reader still feeds every surface. Only the final
 *  `drawImage` moves back. WebView2 keeps the transferred path it already ships. */
const OFFSCREEN_DRAW = !isWebKit;

/** Can this WebView run the off-thread path at all? Checked once: a worker, off-thread JPEG decoding
 *  and a readable response body — plus, only where the worker draws for itself, a canvas it can take
 *  over and a 2D context on it (WebKit shipped OffscreenCanvas for WebGL first). */
function detect(): boolean {
  if (typeof window === 'undefined' || typeof Worker === 'undefined') return false;
  if (typeof createImageBitmap !== 'function') return false;
  if (typeof Response === 'undefined' || !('body' in Response.prototype)) return false;
  if (!OFFSCREEN_DRAW) return true;
  if (typeof OffscreenCanvas === 'undefined') return false;
  if (!('transferControlToOffscreen' in HTMLCanvasElement.prototype)) return false;
  try {
    if (!new OffscreenCanvas(1, 1).getContext('2d')) return false;
  } catch {
    return false;
  }
  return true;
}

/** Whether the surfaces render a worker-drawn `<canvas>` (true) or the plain `<img>` (false). A store
 *  rather than a constant because it can still turn itself off at runtime: if the reader never gets a
 *  single frame — a cross-origin fetch the WebView refuses looks precisely like that — falling back
 *  silently is far better than leaving the user with an endlessly reconnecting stream. */
export const canvasSink = writable(detect());

let worker: Worker | null = null;
let nextId = 1;
/** Surfaces the main thread draws into — populated only where `OFFSCREEN_DRAW` is false. */
const targets = new Map<number, { canvas: HTMLCanvasElement; ctx: CanvasRenderingContext2D }>();
let handlers: {
  onSize?: (w: number, h: number) => void;
  onError?: () => void;
  onLog?: (level: 'warn' | 'info', message: string) => void;
} = {};

function disableCanvasSink(reason: string): void {
  // At `warn`, and through the log file rather than only the console: which sink is live decides how
  // the picture is produced, and a tester cannot open a console. Answering "why is Linux back on the
  // <img> path" took a measurement session that one log line would have ended.
  handlers.onLog?.('warn', `off-thread MJPEG reader unavailable (${reason}) — using the <img> sink`);
  canvasSink.set(false);
  mjpegStats.set(null);
  worker?.postMessage({ type: 'stop' });
}

/** Called once from the video store so the reader can report a picture size (the surfaces size
 *  themselves from it), a dead stream (which drives the existing reconnect), and its own diagnostics
 *  (the store owns the log route; importing it here would be a cycle). */
export function setMjpegSinkHandlers(h: typeof handlers): void {
  handlers = h;
}

/** The URL the reader fetches. On WebKit that is the `?raw=1` variant of the same stream — see
 *  `isWebKit` for what WebKit does to a `multipart/x-mixed-replace` response and why the `<img>`
 *  fallback still gets the multipart one. */
function readerUrl(url: string): string {
  if (!isWebKit) return url;
  return `${url}${url.includes('?') ? '&' : '?'}raw=1`;
}

function ensureWorker(): Worker | null {
  if (worker) return worker;
  let w: Worker;
  try {
    w = new Worker(new URL('./mjpegWorker.ts', import.meta.url), { type: 'module' });
  } catch (e) {
    disableCanvasSink(e instanceof Error ? e.message : String(e));
    return null;
  }
  w.onmessage = (e: MessageEvent<OutMessage>) => {
    const msg = e.data;
    if (msg.type === 'frame') {
      // Main-thread draw path (see `OFFSCREEN_DRAW`). The bitmap was transferred, so it is ours to
      // close, and the acknowledgement is what the reader overlaps its next decode with. It must be
      // sent whatever happens here — a single throwing draw would otherwise leave the reader waiting
      // for it forever, i.e. a permanently frozen picture.
      try {
        for (const target of targets.values()) {
          if (target.canvas.width !== msg.bitmap.width || target.canvas.height !== msg.bitmap.height) {
            target.canvas.width = msg.bitmap.width;
            target.canvas.height = msg.bitmap.height;
          }
          target.ctx.drawImage(msg.bitmap, 0, 0);
        }
      } finally {
        msg.bitmap.close();
        w.postMessage({ type: 'drawn' });
      }
    } else if (msg.type === 'stats') {
      const { type: _t, ...stats } = msg;
      mjpegStats.set(stats);
    } else if (msg.type === 'size') {
      handlers.onSize?.(msg.width, msg.height);
    } else if (msg.everDrew) {
      handlers.onLog?.('warn', `MJPEG reader: ${msg.message}`);
      handlers.onError?.();
    } else {
      // Never delivered a frame → this is the path failing, not the feed. Hand back to the <img>
      // sink, which then reports a genuine problem through its own error event.
      disableCanvasSink(msg.message);
    }
  };
  worker = w;
  return w;
}

/** Cushion depth in frame times, mirrored here because the worker outlives no setting: a reader that
 *  starts later, or restarts after a reconnect, has to be told again. */
let bufferFrames = 0;

/** The panel's smoothing buffer, applied to the image path (the WebRTC path hands the same number to
 *  the engine's own jitter buffer instead — see `applyJitterTarget` in the video store). */
export function setMjpegBuffer(frames: number): void {
  bufferFrames = frames;
  worker?.postMessage({ type: 'buffer', frames });
}

export function startMjpegSink(url: string): void {
  if (!get(canvasSink)) {
    handlers.onLog?.('warn', 'MJPEG sink: <img> (this WebView cannot run the off-thread reader)');
    return;
  }
  const target = readerUrl(url);
  handlers.onLog?.('info', `MJPEG sink: off-thread reader (${target})`);
  const w = ensureWorker();
  w?.postMessage({ type: 'buffer', frames: bufferFrames });
  w?.postMessage({ type: 'start', url: target });
}

export function stopMjpegSink(): void {
  worker?.postMessage({ type: 'stop' });
  mjpegStats.set(null);
}

/** Svelte action for a `<canvas>` video surface: registers it with the reader for its lifetime.
 *  The element keeps whatever CSS its surface gives it — the backing store carries the source
 *  resolution and `object-fit` scales it, exactly as with the <img> it replaces.
 *
 *  Which half of this runs is `OFFSCREEN_DRAW`: either the canvas is handed to the worker outright,
 *  or it stays here and the worker sends frames to be drawn into it. The surfaces don't know the
 *  difference — same markup, same action. */
export function mjpegSink(node: HTMLCanvasElement) {
  const id = nextId++;
  try {
    if (OFFSCREEN_DRAW) {
      const off = node.transferControlToOffscreen();
      ensureWorker()?.postMessage({ type: 'attach', id, canvas: off }, [off]);
    } else {
      const ctx = node.getContext('2d');
      if (!ctx) throw new Error('no 2D context on the video canvas');
      targets.set(id, { canvas: node, ctx });
      // No canvas in the message: that is what tells the reader to send frames here instead.
      ensureWorker()?.postMessage({ type: 'attach', id });
    }
  } catch (e) {
    disableCanvasSink(e instanceof Error ? e.message : String(e));
  }
  return {
    destroy() {
      targets.delete(id);
      worker?.postMessage({ type: 'detach', id });
    },
  };
}
