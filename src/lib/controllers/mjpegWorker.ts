// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

/// <reference lib="webworker" />

// MJPEG multipart reader — runs OFF the main thread.
//
// Why this exists: an <img> fed from a multipart stream needs a main-thread rendering-lifecycle pass
// per frame (resource callback → paint invalidation → commit), and the decoded bitmaps live in the
// engine's image cache where we cannot influence their lifetime. A 720p feed pushes ~3.7 MB of RGBA
// per frame through that cache; at 50 fps the housekeeping alone produced multi-hundred-millisecond
// freezes. Measured against ffplay on the same stream (27.07.2026): 32.6 vs 49.9 fps displayed, 40.4 %
// of the time a still picture, worst freeze 667 ms — on Windows, so this is structural and not a
// WebKitGTK quirk. The H.264/WebRTC path was level with ffplay in the same series, because a <video>
// frame reaches the compositor without touching the main thread at all.
//
// So we do what a real player does: read the stream ourselves, decode off-thread, draw into
// OffscreenCanvases the surfaces hand over, and — crucially — DROP a frame that is already superseded
// instead of queueing it. Frame lifetime is explicit (`ImageBitmap.close()`), so no garbage collector
// is involved in the hot path.
//
// One reader serves every visible surface: previously each <img> opened its own HTTP stream and
// decoded independently.

/** A surface that has handed its canvas over (`transferControlToOffscreen`). */
interface Sink {
  canvas: OffscreenCanvas;
  ctx: OffscreenCanvasRenderingContext2D;
}

type InMessage =
  | { type: 'start'; url: string }
  | { type: 'stop' }
  // No `canvas` = the main thread owns that surface and draws the frames we send it. WebKit needs
  // that: it crashes its own web process when a worker draws into more than one transferred canvas
  // (measured — see `OFFSCREEN_DRAW` in mjpegSink.ts).
  | { type: 'attach'; id: number; canvas?: OffscreenCanvas }
  | { type: 'detach'; id: number }
  | { type: 'buffer'; frames: number }
  | { type: 'drawn' };

const CRLFCRLF = new Uint8Array([13, 10, 13, 10]);
const STATS_INTERVAL_MS = 1000;
const HEADER_DECODER = new TextDecoder();

/** Rates a source plausibly sends. The measured arrival interval is snapped to the nearest of these
 *  before it becomes the release cadence — a raw median wobbles, and a cadence that wobbles is the
 *  very thing this buffer exists to remove. Same list and same reasoning as the WebRTC jitter target
 *  in `stores/video.ts`. */
const STANDARD_FPS = [15, 24, 25, 30, 48, 50, 60, 90, 120];
/** Arrival intervals kept for that median (one second at 60 fps). */
const INTERVAL_SAMPLES = 60;
/** Upper bound for the knob, independent of what the panel currently offers — a value the UI cannot
 *  produce must still not be able to park the picture minutes behind the drone. */
const MAX_BUFFER_FRAMES = 30;

/** Grace period before the last detached surface stops the stream. A map↔video swap destroys one
 *  canvas and creates another, which would otherwise tear the connection down and back up. */
const IDLE_STOP_MS = 1500;

const sinks = new Map<number, Sink>();
/** Surfaces the main thread draws for us — ids only; the frames go over `postMessage`. */
const remoteSinks = new Set<number>();
/** A frame is with the main thread and not yet drawn back. While it is, arrivals are dropped
 *  undecoded — never decoded first and binned afterwards, which would be the most expensive way
 *  imaginable to throw a picture away. */
let inFlight = false;

let abort: AbortController | null = null;
let statsTimer: ReturnType<typeof setInterval> | undefined;
let idleTimer: ReturnType<typeof setTimeout> | undefined;
/** The feed we are meant to be showing, independent of whether we are currently connected: with no
 *  visible surface we disconnect entirely, exactly as the `<img>` sinks did — the server then stops
 *  the transcode, which is the expensive half on a small board. */
let wantUrl: string | null = null;
let everDrew = false;

/** The newest frame that has not been decoded yet. A second arrival while a decode is in flight
 *  replaces it — that is the drop-on-late behaviour, and the reason a busy machine falls behind in
 *  frame rate instead of in latency. */
let pending: Blob | null = null;
let decoding = false;

// ── Receive-side smoothing (the panel's "Smoothing buffer", in FRAME TIMES) ──────────────────────
// The WebRTC path hands this knob to the engine's own jitter buffer; the image path has no engine, so
// it is built here. A link that delivers the right number of frames but in bursts — the arrival trace
// of a stuttering feed showed exactly 60.0 fps with 120–130 ms holes every ~12 s — looks broken
// without one and fine with one, because a cushion of already-arrived frames covers the hole.
//
// At 0 (the default) NOTHING below runs: `emit` takes the old path, newest frame wins, nothing is
// held. That is deliberate — the low-latency behaviour must stay byte-for-byte what it was.
/** Cushion depth in frame times. */
let bufferFrames = 0;
/** Frames waiting for their release slot, oldest first. Empty while `bufferFrames` is 0. */
const queue: Blob[] = [];
let releaseTimer: ReturnType<typeof setTimeout> | undefined;
/** When the next frame is due. Advanced by whole frame times so releases keep a steady cadence even
 *  when arrivals do not — otherwise the burst after a hole would simply be replayed as a burst. */
let nextReleaseAt = 0;
/** False while the cushion is (re)filling: after a hole drains the queue there is nothing to smooth
 *  with, so it builds up again before playback resumes. */
let releasing = false;
let intervalMs = 1000 / 60;
const intervals: number[] = [];

let frameW = 0;
let frameH = 0;
let lastFrameAt = 0;
let lastDrawAt = 0;
// Per-interval counters (reset on each stats post) and cumulative ones (kept for the whole run).
let arrived = 0;
let drawn = 0;
let bytes = 0;
let dropped = 0;
let corrupt = 0;
let statsAt = 0;
// The figures that separate the failure modes. A visible freeze is by definition a long gap between
// two *drawn* frames; whether the matching gap on the *arrival* side is just as long says whether we
// are looking at the source/network or at this machine's decoding.
let droppedNow = 0;
let gapIn = 0;
let gapDraw = 0;
let decodeSum = 0;
let decodeMax = 0;
let decodeCount = 0;

function post(message: unknown, transfer?: Transferable[]): void {
  const scope = self as unknown as DedicatedWorkerGlobalScope;
  if (transfer) scope.postMessage(message, transfer);
  else scope.postMessage(message);
}

/** First index of `needle` in `hay` within `[from, end)`, or -1. */
function indexOfSeq(hay: Uint8Array, needle: Uint8Array, from: number, end: number): number {
  const last = end - needle.length;
  outer: for (let i = Math.max(0, from); i <= last; i++) {
    for (let j = 0; j < needle.length; j++) {
      if (hay[i + j] !== needle[j]) continue outer;
    }
    return i;
  }
  return -1;
}

/** `\r\n--<boundary>` from the response Content-Type, used only when a part omits Content-Length. */
function separatorOf(contentType: string | null): Uint8Array | null {
  const m = /boundary=(?:"([^"]+)"|([^;\s]+))/i.exec(contentType ?? '');
  const b = m?.[1] ?? m?.[2];
  return b ? new TextEncoder().encode(`\r\n--${b}`) : null;
}

/** Body length from a part's header block: the byte count, or -1 when the part carries no
 *  Content-Length (then the body runs to the next boundary). ffmpeg's mpjpeg muxer spells the header
 *  `Content-length`, others `Content-Length` — hence the case-insensitive match. */
function bodyLength(head: Uint8Array): number {
  const m = /content-length:\s*(\d+)/i.exec(HEADER_DECODER.decode(head));
  return m?.[1] ? Number(m[1]) : -1;
}

function emit(view: Uint8Array): void {
  arrived++;
  const now = performance.now();
  if (lastFrameAt) {
    gapIn = Math.max(gapIn, now - lastFrameAt);
    noteInterval(now - lastFrameAt);
  }
  lastFrameAt = now;
  const frame = new Blob([view], { type: 'image/jpeg' });
  if (bufferFrames > 0) {
    enqueue(frame, now);
    return;
  }
  if (pending) {
    // Superseded before it could be decoded — exactly what we want under load.
    dropped++;
    droppedNow++;
  }
  pending = frame;
  if (!decoding) void drain();
}

/** Learn the source's frame interval from arrivals. A gap long enough to be a stall carries no rate
 *  information and is ignored, so a hole cannot slow the cadence that is meant to paper over it. */
function noteInterval(gap: number): void {
  if (gap <= 0 || gap > 500) return;
  intervals.push(gap);
  if (intervals.length > INTERVAL_SAMPLES) intervals.shift();
  if (intervals.length < 10 || intervals.length % 15 !== 0) return;
  const median = [...intervals].sort((a, b) => a - b)[intervals.length >> 1];
  let best = STANDARD_FPS[0];
  for (const f of STANDARD_FPS) {
    if (Math.abs(1000 / f - median) < Math.abs(1000 / best - median)) best = f;
  }
  intervalMs = 1000 / best;
}

function enqueue(frame: Blob, now: number): void {
  queue.push(frame);
  // Latency stays bounded by the cushion the user asked for, plus one frame of slack for a burst.
  // Dropping the OLDEST here is what keeps the picture live: those frames are already late.
  while (queue.length > bufferFrames + 1) {
    queue.shift();
    dropped++;
    droppedNow++;
  }
  if (!releasing && queue.length > bufferFrames) {
    releasing = true;
    nextReleaseAt = now;
    scheduleRelease(now);
  }
}

function scheduleRelease(now: number): void {
  if (releaseTimer !== undefined) return;
  releaseTimer = setTimeout(release, Math.max(0, nextReleaseAt - now));
}

/** Hand the oldest queued frame to the decoder and book the next slot one frame time later. */
function release(): void {
  releaseTimer = undefined;
  const frame = queue.shift();
  if (!frame) {
    releasing = false; // starved — refill the cushion before playing again
    return;
  }
  if (pending) {
    // The decoder is still busy with the previous slot (a long decode, or the main thread holding
    // the last bitmap). Its frame is now the older one, so it is the one that goes.
    dropped++;
    droppedNow++;
  }
  pending = frame;
  if (!decoding) void drain();
  const now = performance.now();
  if (nextReleaseAt < now) nextReleaseAt = now; // fell behind → resume from here, don't burst
  nextReleaseAt += intervalMs;
  scheduleRelease(now);
}

/** Apply the panel's setting. 0 restores the unbuffered path immediately, including for frames
 *  already queued — the newest of them is worth keeping, the rest are by definition late. */
function setBuffer(frames: number): void {
  const next = Math.max(0, Math.min(MAX_BUFFER_FRAMES, Math.round(frames)));
  if (next === bufferFrames) return;
  bufferFrames = next;
  if (bufferFrames > 0) return;
  const newest = queue.pop();
  dropped += queue.length;
  droppedNow += queue.length;
  queue.length = 0;
  clearTimeout(releaseTimer);
  releaseTimer = undefined;
  releasing = false;
  if (newest) {
    pending = newest;
    if (!decoding) void drain();
  }
}

/** Hand a decoded frame to the main thread, which draws it into every surface and closes it. */
function deliver(bmp: ImageBitmap): void {
  inFlight = true;
  post({ type: 'frame', bitmap: bmp }, [bmp]); // transferred — the main thread owns it now
  countDrawn();
}

function countDrawn(): void {
  drawn++;
  everDrew = true;
  const now = performance.now();
  if (lastDrawAt) gapDraw = Math.max(gapDraw, now - lastDrawAt);
  lastDrawAt = now;
}

/** Decode and draw whatever is pending, newest first, until the queue of one runs dry. */
async function drain(): Promise<void> {
  decoding = true;
  try {
    while (pending) {
      // Nothing to decode for while the main thread still holds the last frame — leave it pending
      // (a newer arrival will replace it) and pick up again on the acknowledgement.
      if (remoteSinks.size && inFlight) break;
      const blob = pending;
      pending = null;
      let bmp: ImageBitmap;
      const t0 = performance.now();
      try {
        bmp = await createImageBitmap(blob);
      } catch {
        corrupt++; // a truncated frame must not take the stream down
        continue;
      }
      const decodeMs = performance.now() - t0;
      decodeSum += decodeMs;
      decodeCount++;
      if (decodeMs > decodeMax) decodeMax = decodeMs;
      if (bmp.width !== frameW || bmp.height !== frameH) {
        frameW = bmp.width;
        frameH = bmp.height;
        post({ type: 'size', width: frameW, height: frameH });
      }
      if (remoteSinks.size) {
        deliver(bmp);
      } else {
        for (const s of sinks.values()) {
          // The backing store carries the source resolution and the element is scaled by CSS
          // (`object-fit`), exactly as the <img> was — no fit maths, no resize plumbing here.
          if (s.canvas.width !== frameW || s.canvas.height !== frameH) {
            s.canvas.width = frameW;
            s.canvas.height = frameH;
          }
          s.ctx.drawImage(bmp, 0, 0);
        }
        bmp.close(); // deterministic free — the whole point of doing this ourselves
        countDrawn();
      }
    }
  } finally {
    decoding = false;
  }
}

function postStats(): void {
  const now = performance.now();
  const dt = Math.max(1, now - statsAt) / 1000;
  statsAt = now;
  post({
    type: 'stats',
    fpsIn: arrived / dt,
    fpsOut: drawn / dt,
    kbps: (bytes * 8) / 1000 / dt,
    dropped,
    droppedNow,
    corrupt,
    gapIn,
    gapDraw,
    decodeAvgMs: decodeCount ? decodeSum / decodeCount : 0,
    decodeMaxMs: decodeMax,
    width: frameW,
    height: frameH,
    sinceFrameMs: lastFrameAt ? now - lastFrameAt : -1,
    // What the smoothing buffer is actually holding right now — the number that answers "is the
    // cushion deep enough for this link", which no other figure here can.
    bufferedMs: queue.length * intervalMs,
    bufferFrames,
  });
  arrived = 0;
  drawn = 0;
  bytes = 0;
  droppedNow = 0;
  gapIn = 0;
  gapDraw = 0;
  decodeSum = 0;
  decodeMax = 0;
  decodeCount = 0;
}

async function run(url: string): Promise<void> {
  const ctl = new AbortController();
  abort = ctl;
  // ONE buffer for the whole run, holding `[read, write)`. The previous version built a fresh array
  // per chunk (`buf = append(buf, value)`), which at 60 fps is several megabytes of garbage a second
  // — enough that the collector stopped this worker for up to a third of a second every few seconds.
  // Measured live through WebKitGTK, 55 s per variant: the copying parser collapsed in 14 of those
  // seconds (worst arrival gap 336 ms), the reusing one in a single second (111 ms), with the main
  // thread untouched at one frame throughout. That periodic stall was the stutter.
  let store = new Uint8Array(1 << 20);
  let read = 0;
  let write = 0;
  let headScan = 0; // resume point for the header search, relative to `read` — keeps re-scans short
  let bodyScan = 0;
  let need = 0; // 0 = expecting a header block · >0 = fixed-length body · -1 = body runs to the boundary

  try {
    const res = await fetch(url, { signal: ctl.signal, cache: 'no-store' });
    if (!res.ok || !res.body) throw new Error(`HTTP ${res.status}`);
    const sep = separatorOf(res.headers.get('content-type'));
    const reader = res.body.getReader();

    for (;;) {
      const { done, value } = await reader.read();
      if (ctl.signal.aborted) return;
      if (done) throw new Error('stream ended');
      bytes += value.byteLength;

      // Make room: slide the unread tail to the front, and only grow when even that isn't enough
      // (a frame larger than the buffer). Both are rare — the tail is normally a partial frame.
      if (write + value.byteLength > store.length) {
        const live = write - read;
        if (live + value.byteLength > store.length) {
          const bigger = new Uint8Array(Math.max(store.length * 2, live + value.byteLength));
          bigger.set(store.subarray(read, write));
          store = bigger;
        } else {
          store.copyWithin(0, read, write);
        }
        write = live;
        read = 0;
      }
      store.set(value, write);
      write += value.byteLength;

      for (;;) {
        if (need === 0) {
          const h = indexOfSeq(store, CRLFCRLF, read + headScan, write);
          if (h < 0) {
            headScan = Math.max(0, write - read - 3);
            break;
          }
          need = bodyLength(store.subarray(read, h));
          read = h + 4;
          headScan = 0;
          bodyScan = 0;
        }
        if (need > 0) {
          if (write - read < need) break;
          emit(store.subarray(read, read + need));
          read += need;
          need = 0;
        } else {
          // No Content-Length: the body ends at the next boundary, which we leave in the buffer so
          // the header search picks it up again.
          if (!sep) throw new Error('multipart stream without Content-Length or boundary');
          const e = indexOfSeq(store, sep, read + bodyScan, write);
          if (e < 0) {
            bodyScan = Math.max(0, write - read - sep.length + 1);
            break;
          }
          emit(store.subarray(read, e));
          read = e;
          need = 0;
          bodyScan = 0;
        }
      }
    }
  } catch (e) {
    if (ctl.signal.aborted) return; // our own stop
    // `everDrew` decides how the main thread reads this: a failure before the first frame means this
    // path never worked here (a blocked cross-origin fetch looks exactly like this) and it falls back
    // to the <img> sink; a failure afterwards is a stream that died and drives the normal reconnect.
    post({ type: 'error', message: e instanceof Error ? e.message : String(e), everDrew });
  } finally {
    // Leave the worker connectable again — but only if nothing newer has taken over meanwhile.
    if (abort === ctl) {
      abort = null;
      if (statsTimer) {
        clearInterval(statsTimer);
        statsTimer = undefined;
      }
    }
  }
}

/** Disconnect but keep `wantUrl` — a surface coming back restarts the stream. */
function disconnect(): void {
  abort?.abort();
  abort = null;
  if (statsTimer) {
    clearInterval(statsTimer);
    statsTimer = undefined;
  }
  pending = null;
  inFlight = false;
  queue.length = 0;
  clearTimeout(releaseTimer);
  releaseTimer = undefined;
  releasing = false;
}

function connect(): void {
  if (!wantUrl || abort || sinks.size + remoteSinks.size === 0) return;
  inFlight = false;
  intervals.length = 0;
  frameW = 0;
  frameH = 0;
  lastFrameAt = 0;
  lastDrawAt = 0;
  arrived = 0;
  drawn = 0;
  bytes = 0;
  droppedNow = 0;
  gapIn = 0;
  gapDraw = 0;
  decodeSum = 0;
  decodeMax = 0;
  decodeCount = 0;
  statsAt = performance.now();
  statsTimer = setInterval(postStats, STATS_INTERVAL_MS);
  void run(wantUrl);
}

self.onmessage = (e: MessageEvent<InMessage>) => {
  const msg = e.data;
  switch (msg.type) {
    case 'attach': {
      if (msg.canvas) {
        const ctx = msg.canvas.getContext('2d');
        if (!ctx) break;
        sinks.set(msg.id, { canvas: msg.canvas, ctx });
      } else {
        remoteSinks.add(msg.id);
      }
      clearTimeout(idleTimer);
      idleTimer = undefined;
      connect();
      break;
    }
    case 'detach':
      sinks.delete(msg.id);
      remoteSinks.delete(msg.id);
      if (sinks.size + remoteSinks.size === 0 && !idleTimer) {
        idleTimer = setTimeout(disconnect, IDLE_STOP_MS);
      }
      break;
    case 'drawn':
      inFlight = false;
      if (!decoding) void drain();
      break;
    case 'buffer':
      setBuffer(msg.frames);
      break;
    case 'start':
      disconnect();
      wantUrl = msg.url;
      dropped = 0;
      corrupt = 0;
      connect();
      break;
    case 'stop':
      wantUrl = null;
      disconnect();
      break;
  }
};
