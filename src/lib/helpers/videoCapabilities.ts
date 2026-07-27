// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Native-capture capability logic (pure). The backend `video_probe_device` returns the device's real
// modes; we surface ALL of them as a dependent cascade — Format (codec) → Resolution → Framerate —
// instead of a curated catalog. The `native` source streams via ffmpeg (device-verified control), so
// the codec IS a user control here (MJPEG is the efficient default: the camera encodes JPEG in
// hardware, ffmpeg stream-copies it). See docs (VIDEO_NATIVE_CAPTURE.md).

/** One capture mode reported by the backend probe. Resolutions are a range (min..max); V4L2 devices
 *  report discrete modes (min === max), DirectShow reports ranges. `fpsList` carries the exact discrete
 *  framerates (V4L2); when empty (DirectShow / unknown) the UI derives them from `fpsMin..fpsMax`. */
export interface CaptureMode {
  codec: string;
  minWidth: number;
  minHeight: number;
  maxWidth: number;
  maxHeight: number;
  fpsMin: number;
  fpsMax: number;
  fpsList: number[];
}

/** A device discovered by the backend enumeration. */
export interface NativeDevice {
  id: string;
  name: string;
}

/** The user's chosen native-capture configuration. */
export interface NativeSelection {
  codec: string;
  width: number;
  height: number;
  fps: number;
}

// Codec display order (efficient/preferred first); unknown codecs are appended alphabetically.
const CODEC_ORDER = ['mjpeg', 'h264', 'hevc', 'yuyv', 'nv12', 'grey'];
// Common resolutions used to enumerate a DirectShow range mode into discrete picks (V4L2 modes are
// already discrete and bypass this).
const COMMON_RES: ReadonlyArray<readonly [number, number]> = [
  [640, 480], [720, 480], [720, 576], [800, 600], [1024, 768],
  [1280, 720], [1600, 900], [1920, 1080], [2560, 1440], [3840, 2160],
];
// Common framerates for a DirectShow range (V4L2 uses the exact `fpsList` instead).
const COMMON_FPS = [24, 25, 30, 50, 60, 90, 120];

/** Does a mode's resolution range cover `w×h`? */
function covers(m: CaptureMode, w: number, h: number): boolean {
  return w >= m.minWidth && w <= m.maxWidth && h >= m.minHeight && h <= m.maxHeight;
}

/** Distinct codecs the device offers, in preference order (MJPEG first). */
export function codecsFor(modes: CaptureMode[]): string[] {
  const set = new Set(modes.map((m) => m.codec));
  const known = CODEC_ORDER.filter((c) => set.has(c));
  const rest = [...set].filter((c) => !CODEC_ORDER.includes(c)).sort();
  return [...known, ...rest];
}

/** All resolutions the device offers in the given codec — discrete V4L2 modes verbatim, DirectShow
 *  ranges expanded against the common set. Deduped, largest first. */
export function resolutionsFor(modes: CaptureMode[], codec: string): Array<{ width: number; height: number }> {
  const seen = new Set<string>();
  const out: Array<{ width: number; height: number }> = [];
  const add = (w: number, h: number) => {
    const k = `${w}x${h}`;
    if (!seen.has(k)) {
      seen.add(k);
      out.push({ width: w, height: h });
    }
  };
  for (const m of modes.filter((m) => m.codec === codec)) {
    if (m.minWidth === m.maxWidth && m.minHeight === m.maxHeight) {
      add(m.maxWidth, m.maxHeight);
    } else {
      for (const [w, h] of COMMON_RES) if (covers(m, w, h)) add(w, h);
    }
  }
  return out.sort((a, b) => b.width * b.height - a.width * a.height);
}

/** Framerates the device offers for `codec` at `w×h`. Uses the exact discrete `fpsList` (V4L2) so no
 *  unsupported value is shown; falls back to common rates inside `fpsMin..fpsMax` for range sources. */
export function frameratesFor(modes: CaptureMode[], codec: string, w: number, h: number): number[] {
  const cov = modes.filter((m) => m.codec === codec && covers(m, w, h));
  if (cov.length === 0) return [30];

  const discrete = new Set<number>();
  for (const m of cov) for (const f of m.fpsList) discrete.add(Math.round(f));
  if (discrete.size > 0) return [...discrete].sort((a, b) => a - b);

  const lo = Math.min(...cov.map((m) => m.fpsMin).filter((f) => f > 0), Infinity);
  const hi = Math.max(...cov.map((m) => m.fpsMax), 0);
  if (hi <= 0) return [30, 60];
  const inRange = COMMON_FPS.filter((f) => (lo === Infinity || f >= lo) && f <= hi + 0.5);
  const maxR = Math.round(hi);
  if (!inRange.includes(maxR)) inRange.push(maxR);
  return inRange.length ? inRange.sort((a, b) => a - b) : [maxR];
}

/** Smart default: MJPEG (or the first codec) at the highest resolution that still does ≥30 fps. */
export function defaultSelection(modes: CaptureMode[]): NativeSelection {
  const codec = codecsFor(modes)[0] ?? 'mjpeg';
  const res = resolutionsFor(modes, codec);
  const pick = res.find((r) => frameratesFor(modes, codec, r.width, r.height).some((f) => f >= 30)) ?? res[0];
  if (!pick) return { codec, width: 1280, height: 720, fps: 30 };
  const fps = frameratesFor(modes, codec, pick.width, pick.height);
  return { codec, width: pick.width, height: pick.height, fps: fps.includes(30) ? 30 : (fps[fps.length - 1] ?? 30) };
}

/** Re-validate a selection against the available modes, repairing each field down the cascade
 *  (codec → resolution → framerate) when it no longer fits. Empty probe → keep the selection. */
export function validateSelection(modes: CaptureMode[], sel: NativeSelection): NativeSelection {
  if (modes.length === 0) return sel;
  const codecs = codecsFor(modes);
  const codec = codecs.includes(sel.codec) ? sel.codec : (codecs[0] ?? sel.codec);
  const res = resolutionsFor(modes, codec);
  let { width, height } = sel;
  if (!res.some((r) => r.width === width && r.height === height)) {
    const top = res[0];
    if (!top) return defaultSelection(modes);
    width = top.width;
    height = top.height;
  }
  const fps = frameratesFor(modes, codec, width, height);
  const chosenFps = fps.includes(sel.fps) ? sel.fps : fps.includes(30) ? 30 : (fps[fps.length - 1] ?? 30);
  return { codec, width, height, fps: chosenFps };
}

/** Friendly resolution label (falls back to raw `W×H`). */
export function resolutionLabel(width: number, height: number): string {
  const names: Record<string, string> = {
    '640x480': 'VGA',
    '720x480': 'NTSC',
    '720x576': 'PAL',
    '1280x720': '720p',
    '1920x1080': '1080p',
    '2560x1440': '1440p',
    '3840x2160': '4K',
  };
  const n = names[`${width}x${height}`];
  return n ? `${width}×${height} (${n})` : `${width}×${height}`;
}

/** Friendly codec/format label for the picker (codec tokens are proper nouns → not translated). */
export function codecLabel(codec: string): string {
  const m: Record<string, string> = {
    mjpeg: 'MJPEG',
    h264: 'H.264',
    hevc: 'H.265',
    yuyv: 'YUYV (raw)',
    nv12: 'NV12 (raw)',
    grey: 'Grey (IR)',
  };
  return m[codec] ?? codec.toUpperCase();
}
