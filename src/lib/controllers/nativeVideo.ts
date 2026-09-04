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

/** Live geometry of the active hole for the Debug Monitor: how far the visible rect was
 *  clipped on each side (viewport px), the radius read from the surface's CSS, and which
 *  corner-cut flags fired — the numbers behind a square-corner complaint. */
export interface NativeHoleDebug {
  id: NativeSurfaceId;
  radius: number;
  cut: string;
  clip: string;
  /** Which ancestor set each clipped edge (L|T|R|B) — empty side = unclipped. */
  by: string;
}
export const nativeHoleDebug = writable<NativeHoleDebug | null>(null);

// One SET of elements per id, not one element: the phone grid re-creates the video widget when a
// drag carries it across a page edge, and a transient instance (the drag preview) can mount and
// unmount while the surviving one is alive. A single slot was overwritten by the transient and then
// deleted with it — the live tile ended up unregistered ("video runs elsewhere" until a swap).
const regs = new Map<NativeSurfaceId, Set<HTMLElement>>();

let running = false;
let raf = 0;
let lastRectKey = '';
let lastVisible: boolean | null = null;
let groundEl: HTMLDivElement | null = null;
/** Elements currently carrying a hole clip → the exact clip string applied. Writing style
 *  only on change matters: a same-value write still invalidates style every frame, and on
 *  backdrop-filtered layers that churn is visible. */
const clipped = new Map<HTMLElement, string>();

// Dev: the registry as seen from the DevTools console (which surfaces exist, connected, sized).
if (import.meta.env.DEV && typeof window !== 'undefined') {
  (window as unknown as { __kiteNvRegs?: () => string }).__kiteNvRegs = () =>
    [...regs.entries()]
      .flatMap(([id, els]) =>
        [...els].map((el) => {
          const r = el.getBoundingClientRect();
          return `${id}:${el.isConnected ? 'on' : 'OFF'}:${Math.round(r.width)}x${Math.round(r.height)}@${Math.round(r.left)},${Math.round(r.top)}`;
        }),
      )
      .join(' ');
}

/** Svelte action: register `el` as a native-video surface candidate while it is mounted.
 *  Mount it only in the branch that would show the video (mirrors the MJPEG conditions). */
export function nativeSurface(el: HTMLElement, id: NativeSurfaceId): { destroy(): void } {
  let set = regs.get(id);
  if (!set) {
    set = new Set();
    regs.set(id, set);
  }
  if (import.meta.env.DEV) console.debug(`[nv] register ${id} (others=${set.size}, connected=${el.isConnected})`);
  set.add(el);
  return {
    destroy() {
      if (import.meta.env.DEV) console.debug(`[nv] destroy ${id} (left=${(regs.get(id)?.size ?? 1) - 1})`);
      regs.get(id)?.delete(el);
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
    const els = regs.get(id);
    if (!els) continue;
    for (const el of els) {
      if (!el.isConnected) continue;
      const rect = el.getBoundingClientRect();
      if (rect.width > 0 && rect.height > 0) return { id, el, rect };
    }
  }
  return null;
}

/** The part of `rect` that is actually visible: intersected with every overflow-clipping
 *  ancestor of `el`. A scrolling container (the video panel's body on a small screen)
 *  clips the hole div in the DOM — but its bounding rect still reaches outside, and using
 *  it raw cuts the hole into the panel's HEADER and paints the native layer over it (found
 *  on the Android tablet; the same bug was latent on Windows, where the panel never
 *  scrolls). Null when the box is clipped away entirely. */
function visibleRect(el: HTMLElement, rect: DOMRect, id: NativeSurfaceId): DOMRect | null {
  let x1 = rect.left;
  let y1 = rect.top;
  let x2 = rect.right;
  let y2 = rect.bottom;
  let byL = '';
  let byT = '';
  let byR = '';
  let byB = '';
  const tag = (n: Element) => n.tagName.toLowerCase() + (n.classList[0] ? `.${n.classList[0]}` : '');
  const clips = (v: string) => v === 'auto' || v === 'scroll' || v === 'hidden' || v === 'clip';
  for (let node = el.parentElement; node; node = node.parentElement) {
    const cs = getComputedStyle(node);
    if (clips(cs.overflowX) || clips(cs.overflowY)) {
      // Overflow clips at the CLIENT box (inside the border, minus scrollbars) — the
      // border box would let the video slide over the container's frame line and the
      // scrollbar gutter. client* are layout px; map to visual px through the element's
      // own layout-vs-visual ratio (--ui-scale).
      const b = node.getBoundingClientRect();
      const sx = node.offsetWidth ? b.width / node.offsetWidth : 1;
      const sy = node.offsetHeight ? b.height / node.offsetHeight : 1;
      const left = b.left + node.clientLeft * sx;
      const top = b.top + node.clientTop * sy;
      if (left > x1) { x1 = left; byL = tag(node); }
      if (top > y1) { y1 = top; byT = tag(node); }
      const right = left + node.clientWidth * sx;
      const bottom = top + node.clientHeight * sy;
      if (right < x2) { x2 = right; byR = tag(node); }
      if (bottom < y2) { y2 = bottom; byB = tag(node); }
      if (x2 <= x1 || y2 <= y1) return null;
    }
  }
  // client* are integers: a fractional layout size (any --ui-scale ≠ 1 produces them) rounds
  // the computed client edge by up to half a layout px — enough to read as a scroll-clip in
  // `tick` (which then squares the corner caps on that side) although nothing is clipped.
  // Anything within a px of the surface's own edge is that rounding, not a cut.
  // (Measured on Linux at uiScale 1.25: symmetric ~1.7 px "clips" top+bottom from integer
  // client sizes — a real scroll-clip moves in whole wheel steps, far past 3 px.)
  const SNAP = 3;
  // Phone: the widget column is an OVERLAY, not an ancestor — a surface sliding under it (the
  // docked window parking, PHONE_VIDEO.md D3) must lose its picture at the column's edge, not
  // shine through the glass. The bound clips like a scroll container: full box stays, the
  // visible part shrinks, the sink cuts the picture at the edge without rescaling it. Surfaces
  // that live INSIDE the column (the widget tile) are exempt — the column's own overflow clips
  // them, and the bound would blank them entirely.
  if (rightBound != null && id !== 'widget' && x2 > rightBound) { x2 = rightBound; byR = 'bound'; }
  if (x2 <= x1) return null;
  if (Math.abs(x1 - rect.left) < SNAP) { x1 = rect.left; byL = ''; }
  if (Math.abs(y1 - rect.top) < SNAP) { y1 = rect.top; byT = ''; }
  if (Math.abs(x2 - rect.right) < SNAP) { x2 = rect.right; byR = ''; }
  if (Math.abs(y2 - rect.bottom) < SNAP) { y2 = rect.bottom; byB = ''; }
  if (import.meta.env.DEV) lastClipBy = [byL, byT, byR, byB].join('|');
  return new DOMRect(x1, y1, x2 - x1, y2 - y1);
}

/** Dev diagnostics: which ancestor produced each clipped edge in the last visibleRect. */
let lastClipBy = '';

/** Viewport-x (css px) beyond which no surface is visible — the phone's widget column edge; null
 *  = no bound. Set by +page (it owns the column width and its replay-player slide). */
let rightBound: number | null = null;
export function setNativeRightBound(px: number | null): void {
  rightBound = px;
}

/** The sink's FULL (layout) box for a surface: the surface rect itself, or — for a
 *  `data-nv-cover` surface — the smallest box of the stream's aspect ratio
 *  (`data-nv-aspect`, w/h) that covers the rect, centred on it. */
function coverBox(el: HTMLElement, rect: DOMRect): DOMRect {
  if (el.dataset.nvCover === undefined) return rect;
  const aspect = parseFloat(el.dataset.nvAspect ?? '') || 16 / 9;
  if (rect.width <= 0 || rect.height <= 0) return rect;
  const scale = Math.max(rect.width / aspect, rect.height); // box height that covers both axes
  const h = scale;
  const w = scale * aspect;
  return new DOMRect(rect.x + (rect.width - w) / 2, rect.y + (rect.height - h) / 2, w, h);
}

function tick(): void {
  if (!running) return;
  const c = chosen();
  if (get(activeNativeSurface) !== (c?.id ?? null)) activeNativeSurface.set(c?.id ?? null);
  const vis = c ? visibleRect(c.el, c.rect, c.id) : null;
  if (!c || !vis) {
    // No surface — or the active one is scrolled entirely out of its container.
    if (lastVisible !== false) {
      lastVisible = false;
      void invoke('video_rtsp_native_sink_visible', { visible: false }).catch(() => {});
    }
    clearClips();
    lastRectKey = '';
    if (import.meta.env.DEV) nativeHoleDebug.set(null); // no hole → no stale readout
  } else {
    // Two rects go to the sink: the surface's FULL box for video layout (aspect fit), and
    // the VISIBLE part as a clip — a scroll-clipped surface then shows a video cut at the
    // container edge (like scrolled DOM content), not one shrunk into the remainder.
    // A `data-nv-cover` surface (the video widget tile) wants crop-to-fill instead of a
    // letterbox: its full box is the aspect box that COVERS the tile, centred on it, and the
    // tile stays the visible part — the sink lays the picture out in the full box and cuts it
    // at the tile edge, the very path a scrolled panel uses. No sink-side change.
    const hole = vis;
    const full = coverBox(c.el, c.rect);
    const dpr = window.devicePixelRatio || 1;
    const phys = {
      x: Math.round(full.x * dpr),
      y: Math.round(full.y * dpr),
      w: Math.round(full.width * dpr),
      h: Math.round(full.height * dpr),
      cx: Math.round(hole.x * dpr),
      cy: Math.round(hole.y * dpr),
      cw: Math.round(hole.width * dpr),
      ch: Math.round(hole.height * dpr),
    };
    if (lastVisible !== true) {
      lastVisible = true;
      void invoke('video_rtsp_native_sink_visible', { visible: true }).catch(() => {});
    }
    const key = `${phys.x},${phys.y},${phys.w},${phys.h},${phys.cx},${phys.cy},${phys.cw},${phys.ch}`;
    if (key !== lastRectKey) {
      lastRectKey = key;
      void invoke('video_rtsp_native_sink_rect', phys).catch(() => {});
    }
    // Clip with the rect the NATIVE layer actually got (device-pixel-snapped): a hole a
    // fraction wider than the native layer exposes a hairline of whatever is behind it.
    const snapped = new DOMRect(phys.cx / dpr, phys.cy / dpr, phys.cw / dpr, phys.ch / dpr);
    // The surface's corner rounding, in viewport px (the hole div declares it in CSS; the
    // chrome layer may be scaled by --ui-scale). The hole is cut with these corners
    // rounded, so the layers behind keep painting the corner caps over the native layer's
    // square corners — the frame looks exactly like the DOM-rendered video did. A corner
    // produced by scroll-CLIPPING is not a real corner: the video slides under the
    // container edge there, so that edge stays square.
    const surfScale = c.el.offsetWidth ? c.rect.width / c.el.offsetWidth : 1;
    const radius = (parseFloat(getComputedStyle(c.el).borderTopLeftRadius) || 0) * surfScale;
    const cutTop = hole.top > c.rect.top + 0.5;
    const cutLeft = hole.left > c.rect.left + 0.5;
    const cutRight = hole.right < c.rect.right - 0.5;
    const cutBottom = hole.bottom < c.rect.bottom - 0.5;
    if (import.meta.env.DEV) {
      const f = (v: number) => (Math.round(v * 100) / 100).toString();
      const dbg: NativeHoleDebug = {
        id: c.id,
        radius: Math.round(radius * 100) / 100,
        cut: [cutTop && 'T', cutLeft && 'L', cutRight && 'R', cutBottom && 'B'].filter(Boolean).join('') || '—',
        clip: `${f(hole.left - c.rect.left)}/${f(hole.top - c.rect.top)}/${f(c.rect.right - hole.right)}/${f(c.rect.bottom - hole.bottom)}`,
        by: lastClipBy,
      };
      const prev = get(nativeHoleDebug);
      if (!prev || prev.id !== dbg.id || prev.radius !== dbg.radius || prev.cut !== dbg.cut || prev.clip !== dbg.clip || prev.by !== dbg.by) {
        nativeHoleDebug.set(dbg);
      }
    }
    applyClips(c.el, snapped, {
      tl: cutTop || cutLeft ? 0 : radius,
      tr: cutTop || cutRight ? 0 : radius,
      bl: cutBottom || cutLeft ? 0 : radius,
      br: cutBottom || cutRight ? 0 : radius,
    });
  }
  raf = requestAnimationFrame(tick);
}

/** Per-corner rounding of the hole, viewport px (0 = square corner). */
export interface HoleRadii {
  tl: number;
  tr: number;
  bl: number;
  br: number;
}

/** Build the hole clip for `el` as an SVG `path()`: outer ring clockwise, inner (the hole,
 *  with rounded corners) counter-clockwise — the default nonzero fill rule then leaves the
 *  intersection with `hole` unpainted. Coordinates are the element's LOCAL layout px:
 *  clip-path applies before CSS transforms, and the chrome layer is scaled by --ui-scale,
 *  so viewport px must be mapped back through the element's own layout-vs-visual ratio.
 *  `radii` is the hole's per-corner rounding in viewport px (0 = square; an SVG arc with
 *  zero radii degrades to a line per spec, so no special case is needed). Null when the
 *  element doesn't intersect the hole. */
function holePath(el: HTMLElement, hole: DOMRect, radii: HoleRadii): string | null {
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
  const cap = (r: number, axisScale: number, span: number) =>
    Math.min(r / axisScale, span / 2);
  const r = {
    tlx: cap(radii.tl, sx, hx2 - hx1),
    tly: cap(radii.tl, sy, hy2 - hy1),
    trx: cap(radii.tr, sx, hx2 - hx1),
    try_: cap(radii.tr, sy, hy2 - hy1),
    blx: cap(radii.bl, sx, hx2 - hx1),
    bly: cap(radii.bl, sy, hy2 - hy1),
    brx: cap(radii.br, sx, hx2 - hx1),
    bry: cap(radii.br, sy, hy2 - hy1),
  };
  const f = (v: number) => v.toFixed(2);
  const arc = (rx: number, ry: number) => `A ${f(rx)} ${f(ry)} 0 0 0`;
  return (
    `path('M0 0H${f(w)}V${f(h)}H0Z` +
    ` M${f(hx1 + r.tlx)} ${f(hy1)}` +
    ` ${arc(r.tlx, r.tly)} ${f(hx1)} ${f(hy1 + r.tly)}` +
    ` V${f(hy2 - r.bly)}` +
    ` ${arc(r.blx, r.bly)} ${f(hx1 + r.blx)} ${f(hy2)}` +
    ` H${f(hx2 - r.brx)}` +
    ` ${arc(r.brx, r.bry)} ${f(hx2)} ${f(hy2 - r.bry)}` +
    ` V${f(hy1 + r.try_)}` +
    ` ${arc(r.trx, r.try_)} ${f(hx2 - r.trx)} ${f(hy1)}` +
    `Z')`
  );
}

function applyClips(surfaceEl: HTMLElement, hole: DOMRect, radius: HoleRadii): void {
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
