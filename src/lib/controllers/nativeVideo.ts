// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Native video sink surface router (MOBILE_RTSP.md P2.1, Windows-first).
//
// When the in-process RTSP client selects an H264 track, the picture is NOT rendered by the
// DOM: the backend decodes it in hardware and presents it on a D3D child window BELOW the
// WebView. The window is only visible where the DOM's pixels are transparent (the hole-punch
// architecture the spike proved), so this router does three jobs, every animation frame while
// the sink is live:
//
//   1. pick the ONE surface that shows the video — exactly one hardware layer exists, so the
//      registered candidate with the highest priority wins (fullscreen map-swap > floating
//      window > widget tile > panel preview); the others show a placeholder,
//   2. push that surface's rect (physical px) to the backend so the native layer tracks it,
//   3. cut the hole: a clip-path with a reversed inner ring on every DOM layer that paints
//      UNDER the surface (the map, the surface's own glass container, the page ground) — and
//      ONLY there. DOM above the hole still renders, which is the point: OSD, overlays and
//      corner controls composite over the native video.
//
// The hole and the native rect must match exactly: a transparent DOM pixel with no native
// layer behind it shows the DESKTOP through the transparent app window (spike finding).
//
// Layers opt in: elements carrying `data-nv-clip` are clipped wherever they intersect the
// hole (the unzoomed map layer). The active surface's PanelShell ancestor (`.ps`, glass +
// backdrop-filter) is picked up automatically for the panel preview. The video widget's
// card is deliberately NOT a clip target: per-frame clip churn on its backdrop-filtered
// glass flickered during window resizes — the armed tile paints its bezel with ring-only
// properties instead (see VideoWidget.svelte).
// The page ground (`body`'s background) cannot be clipped directly — clip-path on `body`
// would clip the whole app — so while the router runs, the body background moves onto an
// injected fixed div behind everything, and THAT gets the hole.
//
// The hole is cut with the surface's corner radius (read from the hole div's own CSS), so
// the layers behind keep painting the corner caps over the native layer's square corners.
//
// Known stage-1 limits (P4 polish): layers not opted in (arbitrary widgets under a freely
// dragged floating window, glass panels opened OVER a video surface) are not clipped — a
// semi-transparent layer over the hole lets the video bleed through faintly.

import { writable, get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';

export type NativeSurfaceId = 'main' | 'floating' | 'widget' | 'preview';

/** Highest first: the fullscreen map-swap view beats the floating window beats the widget
 *  tile beats the panel preview (the preview must never steal the picture from a flight
 *  surface just because the panel was opened). */
const PRIORITY: NativeSurfaceId[] = ['main', 'floating', 'widget', 'preview'];

/** The surface currently showing the native video (null = none → sink hidden). Surfaces
 *  render their transparent hole only while THEY are active, a placeholder otherwise. */
export const activeNativeSurface = writable<NativeSurfaceId | null>(null);

const regs = new Map<NativeSurfaceId, HTMLElement>();

let running = false;
let raf = 0;
let lastRectKey = '';
let lastVisible: boolean | null = null;
let groundEl: HTMLDivElement | null = null;
/** Elements currently carrying a hole clip → the exact clip string applied. Writing style
 *  only on change matters: a same-value write still invalidates style every frame, and on
 *  backdrop-filtered layers that churn is visible. */
const clipped = new Map<HTMLElement, string>();

/** Svelte action: register `el` as a native-video surface candidate while it is mounted.
 *  Mount it only in the branch that would show the video (mirrors the MJPEG conditions). */
export function nativeSurface(el: HTMLElement, id: NativeSurfaceId): { destroy(): void } {
  regs.set(id, el);
  return {
    destroy() {
      if (regs.get(id) === el) regs.delete(id);
    },
  };
}

/** Start following surfaces (call when the sink route reports live). Idempotent. */
export function startNativeSurfaceRouter(): void {
  if (running || typeof window === 'undefined') return;
  running = true;
  lastRectKey = '';
  lastVisible = null;
  raf = requestAnimationFrame(tick);
}

/** Stop and undo every DOM alteration (clip paths, ground div). Idempotent. */
export function stopNativeSurfaceRouter(): void {
  if (!running) return;
  running = false;
  cancelAnimationFrame(raf);
  clearClips();
  removeGround();
  activeNativeSurface.set(null);
}

/** Highest-priority candidate that actually has an on-screen box — a mounted but hidden
 *  surface (collapsed dock, display:none ancestor) must not win and blank the video. */
function chosen(): { id: NativeSurfaceId; el: HTMLElement; rect: DOMRect } | null {
  for (const id of PRIORITY) {
    const el = regs.get(id);
    if (!el || !el.isConnected) continue;
    const rect = el.getBoundingClientRect();
    if (rect.width > 0 && rect.height > 0) return { id, el, rect };
  }
  return null;
}

function tick(): void {
  if (!running) return;
  const c = chosen();
  if (get(activeNativeSurface) !== (c?.id ?? null)) activeNativeSurface.set(c?.id ?? null);
  if (!c) {
    if (lastVisible !== false) {
      lastVisible = false;
      void invoke('video_rtsp_native_sink_visible', { visible: false }).catch(() => {});
    }
    clearClips();
    lastRectKey = '';
  } else {
    const hole = c.rect;
    const dpr = window.devicePixelRatio || 1;
    const phys = {
      x: Math.round(hole.x * dpr),
      y: Math.round(hole.y * dpr),
      w: Math.round(hole.width * dpr),
      h: Math.round(hole.height * dpr),
    };
    if (lastVisible !== true) {
      lastVisible = true;
      void invoke('video_rtsp_native_sink_visible', { visible: true }).catch(() => {});
    }
    const key = `${phys.x},${phys.y},${phys.w},${phys.h}`;
    if (key !== lastRectKey) {
      lastRectKey = key;
      void invoke('video_rtsp_native_sink_rect', phys).catch(() => {});
    }
    // Clip with the rect the NATIVE window actually got (device-pixel-snapped): a hole a
    // fraction wider than the native layer exposes a hairline of whatever is behind it.
    const snapped = new DOMRect(phys.x / dpr, phys.y / dpr, phys.w / dpr, phys.h / dpr);
    // The surface's corner rounding, in viewport px (the hole div declares it in CSS; the
    // chrome layer may be scaled by --ui-scale). The hole is cut with these corners
    // rounded, so the layers behind keep painting the corner caps over the native layer's
    // square corners — the frame looks exactly like the DOM-rendered video did.
    const surfScale = c.el.offsetWidth ? c.rect.width / c.el.offsetWidth : 1;
    const radius = (parseFloat(getComputedStyle(c.el).borderTopLeftRadius) || 0) * surfScale;
    applyClips(c.el, snapped, radius);
  }
  raf = requestAnimationFrame(tick);
}

/** Build the hole clip for `el` as an SVG `path()`: outer ring clockwise, inner (the hole,
 *  with rounded corners) counter-clockwise — the default nonzero fill rule then leaves the
 *  intersection with `hole` unpainted. Coordinates are the element's LOCAL layout px:
 *  clip-path applies before CSS transforms, and the chrome layer is scaled by --ui-scale,
 *  so viewport px must be mapped back through the element's own layout-vs-visual ratio.
 *  `radius` is the hole's corner rounding in viewport px (0 = square; an SVG arc with zero
 *  radii degrades to a line per spec, so no special case is needed). Null when the element
 *  doesn't intersect the hole. */
function holePath(el: HTMLElement, hole: DOMRect, radius: number): string | null {
  const b = el.getBoundingClientRect();
  if (b.width <= 0 || b.height <= 0) return null;
  const x1 = Math.max(hole.left, b.left);
  const y1 = Math.max(hole.top, b.top);
  const x2 = Math.min(hole.right, b.right);
  const y2 = Math.min(hole.bottom, b.bottom);
  if (x2 <= x1 || y2 <= y1) return null;
  const w = el.offsetWidth || b.width;
  const h = el.offsetHeight || b.height;
  const sx = b.width / w;
  const sy = b.height / h;
  const hx1 = (x1 - b.left) / sx;
  const hy1 = (y1 - b.top) / sy;
  const hx2 = (x2 - b.left) / sx;
  const hy2 = (y2 - b.top) / sy;
  const rx = Math.min(radius / sx, (hx2 - hx1) / 2);
  const ry = Math.min(radius / sy, (hy2 - hy1) / 2);
  const f = (v: number) => v.toFixed(2);
  const arc = `A ${f(rx)} ${f(ry)} 0 0 0`;
  return (
    `path('M0 0H${f(w)}V${f(h)}H0Z` +
    ` M${f(hx1 + rx)} ${f(hy1)}` +
    ` ${arc} ${f(hx1)} ${f(hy1 + ry)}` +
    ` V${f(hy2 - ry)}` +
    ` ${arc} ${f(hx1 + rx)} ${f(hy2)}` +
    ` H${f(hx2 - rx)}` +
    ` ${arc} ${f(hx2)} ${f(hy2 - ry)}` +
    ` V${f(hy1 + ry)}` +
    ` ${arc} ${f(hx2 - rx)} ${f(hy1)}` +
    `Z')`
  );
}

function applyClips(surfaceEl: HTMLElement, hole: DOMRect, radius: number): void {
  const targets = new Set<HTMLElement>();
  for (const el of document.querySelectorAll<HTMLElement>('[data-nv-clip]')) targets.add(el);
  // The panel preview sits on a glass PanelShell — clip that instance too.
  const shell = surfaceEl.closest<HTMLElement>('.ps');
  if (shell) targets.add(shell);
  targets.add(ensureGround());
  for (const el of targets) {
    const path = holePath(el, hole, radius);
    if (path) {
      if (clipped.get(el) !== path) {
        el.style.clipPath = path;
        clipped.set(el, path);
      }
    } else if (clipped.has(el)) {
      el.style.clipPath = '';
      clipped.delete(el);
    }
  }
  // Layers that left the target set (unmounted branch, panel closed) keep no stale clip.
  for (const el of [...clipped.keys()]) {
    if (!targets.has(el)) {
      el.style.clipPath = '';
      clipped.delete(el);
    }
  }
}

function clearClips(): void {
  for (const el of clipped.keys()) el.style.clipPath = '';
  clipped.clear();
}

/** Move the page ground off `body` onto a clip-able fixed div (see module docs). */
function ensureGround(): HTMLDivElement {
  if (!groundEl) {
    groundEl = document.createElement('div');
    groundEl.id = 'nv-ground';
    const s = groundEl.style;
    s.position = 'fixed';
    s.inset = '0';
    s.zIndex = '-1';
    s.pointerEvents = 'none';
    s.background = getComputedStyle(document.body).backgroundColor || '#3d3f3e';
    document.body.appendChild(groundEl);
    document.body.style.backgroundColor = 'transparent';
  }
  return groundEl;
}

function removeGround(): void {
  if (!groundEl) return;
  groundEl.remove();
  groundEl = null;
  document.body.style.backgroundColor = '';
}
