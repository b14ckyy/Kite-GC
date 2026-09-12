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
//      window > widget tile); the others show a placeholder,
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
// hole (the unzoomed map layer). The video widget's card and the floating window's frame are
// deliberately NOT clip targets: per-frame clip churn on their backdrop-filtered glass
// flickered during window resizes — the armed surface paints its bezel with ring-only
// properties instead (see VideoWidget.svelte / FloatingVideoWindow.svelte).
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

export type NativeSurfaceId = 'main' | 'floating' | 'widget';

/** Highest first: the fullscreen map-swap view beats the floating window beats the widget tile. */
const PRIORITY: NativeSurfaceId[] = ['main', 'floating', 'widget'];

/** How many surfaces the sink serves at once (VIDEO_MULTISINK_WINDOW.md D1): the widget tile plus
 *  one large surface. The backend caps this too — this is the polite half of the contract. */
const MAX_SURFACES = 2;

/** The surfaces the native video is presented in right now — up to two at once since the sink
 *  became a hub (VIDEO_MULTISINK_WINDOW.md §4.3). A surface renders its transparent hole only while
 *  it is IN this set, a placeholder otherwise, and it only joins once the backend has confirmed the
 *  list that contains it (see `ackedKeys`) — so a hole never opens over a layer that is not there. */
export const activeNativeSurfaces = writable<Set<NativeSurfaceId>>(new Set());

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
export const nativeHoleDebug = writable<NativeHoleDebug[]>([]);

// One SET of elements per id, not one element: the phone grid re-creates the video widget when a
// drag carries it across a page edge, and a transient instance (the drag preview) can mount and
// unmount while the surviving one is alive. A single slot was overwritten by the transient and then
// deleted with it — the live tile ended up unregistered ("video runs elsewhere" until a swap).
const regs = new Map<NativeSurfaceId, Set<HTMLElement>>();

let running = false;
let raf = 0;
let lastRectKey = '';
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
  ackedKeys = new Set();
  pending = null;
  raf = requestAnimationFrame(tick);
}

/** Stop and undo every DOM alteration (clip paths, ground div). Idempotent. */
export function stopNativeSurfaceRouter(): void {
  if (!running) return;
  running = false;
  cancelAnimationFrame(raf);
  clearClips();
  removeGround();
  activeNativeSurfaces.set(new Set());
  ackedKeys = new Set();
}

/** Highest-priority candidate that actually has an on-screen box — a mounted but hidden
 *  surface (collapsed dock, display:none ancestor) must not win and blank the video. */
interface LiveSurface {
  id: NativeSurfaceId;
  el: HTMLElement;
  /** The surface's own box. */
  rect: DOMRect;
  /** What is left of it after every clipping ancestor — the hole, and the sink's clip box. */
  vis: DOMRect;
}

/** Every registered surface that has an on-screen box, highest priority first and capped at what
 *  the sink serves. One element per id: a mounted but hidden instance (collapsed dock, display:none
 *  ancestor, the phone grid's transient drag copy) must not take the slot from the live one. */
function visibleSurfaces(): LiveSurface[] {
  const out: LiveSurface[] = [];
  for (const id of PRIORITY) {
    const els = regs.get(id);
    if (!els) continue;
    for (const el of els) {
      if (!el.isConnected) continue;
      const rect = el.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) continue;
      const vis = visibleRect(el, rect, id);
      if (!vis) continue;
      out.push({ id, el, rect, vis });
      break;
    }
    if (out.length >= MAX_SURFACES) break;
  }
  return out;
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
  // The viewport itself: a surface sliding off the screen (the floating window parking, its
  // transform carries it past the left edge) keeps its full box for the layout and loses the
  // off-screen part of the clip — the sinks never see a negative or oversized visible box.
  if (x1 < 0) { x1 = 0; byL = 'viewport'; }
  if (y1 < 0) { y1 = 0; byT = 'viewport'; }
  if (x2 > window.innerWidth) { x2 = window.innerWidth; byR = 'viewport'; }
  if (y2 > window.innerHeight) { y2 = window.innerHeight; byB = 'viewport'; }
  if (x2 <= x1 || y2 <= y1) return null;
  if (Math.abs(x1 - rect.left) < SNAP) { x1 = rect.left; byL = ''; }
  if (Math.abs(y1 - rect.top) < SNAP) { y1 = rect.top; byT = ''; }
  if (Math.abs(x2 - rect.right) < SNAP) { x2 = rect.right; byR = ''; }
  if (Math.abs(y2 - rect.bottom) < SNAP) { y2 = rect.bottom; byB = ''; }
  if (import.meta.env.DEV) lastClipBy = [byL, byT, byR, byB].join('|');
  return new DOMRect(x1, y1, x2 - x1, y2 - y1);
}

/** Dev diagnostics: which ancestor produced each clipped edge in the last visibleRect. */
let lastClipBy = '';

// The surface list is pushed over IPC, which is a QUEUE: a drag produces a new geometry every
// animation frame, and anything that cannot keep up turns that stream into a growing backlog — the
// layer then trails the DOM by the whole queue instead of by a frame. So only ever ONE call is in
// flight; while it is, the newest list is remembered and sent when it settles. Older lists are
// simply dropped: nobody wants a position the window has already left.
type SurfacePayload = {
  id: NativeSurfaceId;
  x: number; y: number; w: number; h: number;
  cx: number; cy: number; cw: number; ch: number;
};

let inFlight = false;
let pending: { payload: SurfacePayload[]; keys: Set<NativeSurfaceId> } | null = null;
/** The surface set the BACKEND has confirmed. A surface cuts its hole only once it is in here —
 *  its output exists by then, so the transparency never arrives before the picture. */
let ackedKeys = new Set<NativeSurfaceId>();

function sendSurfaces(payload: SurfacePayload[], keys: Set<NativeSurfaceId>): void {
  if (inFlight) {
    pending = { payload, keys };
    return;
  }
  inFlight = true;
  void invoke('video_rtsp_native_sink_surfaces', { surfaces: payload })
    .then(() => {
      ackedKeys = keys;
    })
    .catch(() => {})
    .finally(() => {
      inFlight = false;
      const next = pending;
      pending = null;
      if (next) sendSurfaces(next.payload, next.keys);
    });
}

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

/** One hole to cut: whose surface it is, where it is (viewport px, device-pixel-snapped) and how
 *  its corners round. */
interface Hole {
  id: NativeSurfaceId;
  rect: DOMRect;
  radii: HoleRadii;
}

function setsEqual(a: Set<NativeSurfaceId>, b: Set<NativeSurfaceId>): boolean {
  return a.size === b.size && [...a].every((v) => b.has(v));
}

function tick(): void {
  if (!running) return;
  const live = visibleSurfaces();
  const dpr = window.devicePixelRatio || 1;
  const px = (v: number) => Math.round(v * dpr);

  // Two rects go to the sink per surface: its FULL box for the video layout (aspect fit), and the
  // VISIBLE part as a clip — a scroll-clipped surface then shows a video cut at the container edge
  // (like scrolled DOM content), not one shrunk into the remainder. A `data-nv-cover` surface (the
  // video widget tile) wants crop-to-fill instead of a letterbox: its full box is the aspect box
  // that COVERS the tile, centred on it, and the tile stays the visible part.
  const payload: SurfacePayload[] = live.map((s) => {
    const full = coverBox(s.el, s.rect);
    return {
      id: s.id,
      x: px(full.x),
      y: px(full.y),
      w: px(full.width),
      h: px(full.height),
      cx: px(s.vis.x),
      cy: px(s.vis.y),
      cw: px(s.vis.width),
      ch: px(s.vis.height),
    };
  });
  const key = payload
    .map((p) => `${p.id}:${p.x},${p.y},${p.w},${p.h},${p.cx},${p.cy},${p.cw},${p.ch}`)
    .join('|');
  if (key !== lastRectKey) {
    lastRectKey = key;
    sendSurfaces(payload, new Set(payload.map((p) => p.id)));
  }

  // Only surfaces the backend has acknowledged may go transparent (see `ackedKeys`).
  const active = new Set(live.filter((s) => ackedKeys.has(s.id)).map((s) => s.id));
  if (!setsEqual(get(activeNativeSurfaces), active)) activeNativeSurfaces.set(active);

  const holes: Hole[] = [];
  const dbg: NativeHoleDebug[] = [];
  for (let i = 0; i < live.length; i++) {
    const s = live[i];
    if (!active.has(s.id)) continue;
    const p = payload[i];
    // Clip with the rect the NATIVE layer actually got (device-pixel-snapped): a hole a fraction
    // wider than the native layer exposes a hairline of whatever is behind it.
    const snapped = new DOMRect(p.cx / dpr, p.cy / dpr, p.cw / dpr, p.ch / dpr);
    // The surface's corner rounding, in viewport px (the hole div declares it in CSS; the chrome
    // layer may be scaled by --ui-scale). The hole is cut with these corners rounded, so the layers
    // behind keep painting the corner caps over the native layer's square corners — the frame looks
    // exactly like the DOM-rendered video did. A corner produced by scroll-CLIPPING is not a real
    // corner: the video slides under the container edge there, so that edge stays square.
    const surfScale = s.el.offsetWidth ? s.rect.width / s.el.offsetWidth : 1;
    const radius = (parseFloat(getComputedStyle(s.el).borderTopLeftRadius) || 0) * surfScale;
    const cutTop = s.vis.top > s.rect.top + 0.5;
    const cutLeft = s.vis.left > s.rect.left + 0.5;
    const cutRight = s.vis.right < s.rect.right - 0.5;
    const cutBottom = s.vis.bottom < s.rect.bottom - 0.5;
    holes.push({
      id: s.id,
      rect: snapped,
      radii: {
        tl: cutTop || cutLeft ? 0 : radius,
        tr: cutTop || cutRight ? 0 : radius,
        bl: cutBottom || cutLeft ? 0 : radius,
        br: cutBottom || cutRight ? 0 : radius,
      },
    });
    if (import.meta.env.DEV) {
      const f = (v: number) => (Math.round(v * 100) / 100).toString();
      dbg.push({
        id: s.id,
        radius: Math.round(radius * 100) / 100,
        cut: [cutTop && 'T', cutLeft && 'L', cutRight && 'R', cutBottom && 'B'].filter(Boolean).join('') || '—',
        clip: `${f(s.vis.left - s.rect.left)}/${f(s.vis.top - s.rect.top)}/${f(s.rect.right - s.vis.right)}/${f(s.rect.bottom - s.vis.bottom)}`,
        by: lastClipBy,
      });
    }
  }
  // Topmost surface first (DOM stacking is the reverse of the priority order): the one that owns
  // the shared pixels keeps its hole whole, the ones below it are trimmed around it.
  applyClips(disjoint([...holes].reverse()));
  if (import.meta.env.DEV) {
    const prev = get(nativeHoleDebug);
    const same =
      prev.length === dbg.length &&
      prev.every((h, i) => {
        const n = dbg[i];
        return h.id === n.id && h.radius === n.radius && h.cut === n.cut && h.clip === n.clip && h.by === n.by;
      });
    if (!same) nativeHoleDebug.set(dbg);
  }
  raf = requestAnimationFrame(tick);
}


/** How far the clip's outer ring reaches beyond the element's own box (layout px) — enough for the
 *  bezel shadows the video surfaces paint outside theirs. */
const OUTER_MARGIN_PX = 24;

/** Two holes that OVERLAP would cancel each other out: the outer ring winds one way, each hole the
 *  other, so a point inside both counts +1 −1 −1 and the nonzero fill rule paints it again — the map
 *  reappeared exactly where a floating window covered the widget tile (Marc, 2026-09-08). Nothing
 *  about the fill rule fixes that (even-odd has the same parity problem), so the holes are made
 *  disjoint first: every hole is cut down to the parts no earlier hole already covers. The union is
 *  unchanged, which is all the sink cares about — the topmost native layer owns the shared pixels.
 *
 *  Order matters beyond that: the caller passes the TOPMOST surface first, so the hole that survives
 *  the overlap intact is the one whose layer is actually on top. `holesFor` then has a whole hole to
 *  cut the surfaces below out of — trimming it here would leave exactly the overlap uncut, which is
 *  where their bezels were showing through. */
function disjoint(holes: Hole[]): Hole[] {
  if (holes.length < 2) return holes;
  const out: Hole[] = [];
  for (const h of holes) {
    let pieces = [h];
    for (const done of out) pieces = pieces.flatMap((p) => subtractHole(p, done.rect));
    out.push(...pieces);
  }
  return out;
}

/** The parts of `a` outside `b` — up to four bands, each keeping only the corner radii it still owns
 *  (a cut edge is square, the surviving outer corners stay rounded). */
function subtractHole(a: Hole, b: DOMRect): Hole[] {
  const r = a.rect;
  const ix1 = Math.max(r.left, b.left);
  const iy1 = Math.max(r.top, b.top);
  const ix2 = Math.min(r.right, b.right);
  const iy2 = Math.min(r.bottom, b.bottom);
  if (ix2 <= ix1 || iy2 <= iy1) return [a]; // no overlap
  const out: Hole[] = [];
  const band = (x1: number, y1: number, x2: number, y2: number) => {
    if (x2 - x1 < 0.5 || y2 - y1 < 0.5) return; // sub-pixel slivers cut nothing
    out.push({ id: a.id, rect: new DOMRect(x1, y1, x2 - x1, y2 - y1), radii: keptCorners(a, x1, y1, x2, y2) });
  };
  const midTop = Math.max(r.top, iy1);
  const midBottom = Math.min(r.bottom, iy2);
  band(r.left, r.top, r.right, iy1); // above
  band(r.left, iy2, r.right, r.bottom); // below
  band(r.left, midTop, ix1, midBottom); // left of the overlap
  band(ix2, midTop, r.right, midBottom); // right of it
  return out;
}

/** Which of `a`'s rounded corners the piece (x1,y1)-(x2,y2) still has; the rest go square. */
function keptCorners(a: Hole, x1: number, y1: number, x2: number, y2: number): HoleRadii {
  const EPS = 0.5;
  const r = a.rect;
  const l = Math.abs(x1 - r.left) < EPS;
  const t = Math.abs(y1 - r.top) < EPS;
  const ri = Math.abs(x2 - r.right) < EPS;
  const b = Math.abs(y2 - r.bottom) < EPS;
  return {
    tl: l && t ? a.radii.tl : 0,
    tr: ri && t ? a.radii.tr : 0,
    bl: l && b ? a.radii.bl : 0,
    br: ri && b ? a.radii.br : 0,
  };
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
function holePath(el: HTMLElement, holes: Hole[]): string | null {
  const b = el.getBoundingClientRect();
  if (b.width <= 0 || b.height <= 0) return null;
  const w = el.offsetWidth || b.width;
  const h = el.offsetHeight || b.height;
  const sx = b.width / w;
  const sy = b.height / h;
  const f = (v: number) => v.toFixed(2);
  // Outer ring clockwise, every hole counter-clockwise inside it — the nonzero fill rule then
  // leaves each of them unpainted, so two surfaces cut two holes out of the same layer. The ring
  // reaches PAST the element's box: a clip path cuts an element's box-shadow as well, and the
  // surfaces paint their opaque bezel as one (that is what the frames are made of).
  const m = OUTER_MARGIN_PX;
  let d = `M${f(-m)} ${f(-m)}H${f(w + m)}V${f(h + m)}H${f(-m)}Z`;
  let cut = false;
  for (const { rect: hole, radii } of holes) {
    const ring = holeRing(hole, radii, b, sx, sy);
    if (!ring) continue;
    d += ring;
    cut = true;
  }
  return cut ? `path('${d}')` : null;
}

/** One counter-clockwise ring for `hole`, in `el`'s LOCAL layout px, or null when it misses.
 *  Clamped to the element's box PLUS the outer margin, never to the box itself: an element's bezel
 *  is a box-shadow painted just outside its box, and a hole that stopped at the box left exactly
 *  that ring standing — the few pixels that kept showing through the surface above (verified in a
 *  headless render, 2026-09-08). */
function holeRing(hole: DOMRect, radii: HoleRadii, b: DOMRect, sx: number, sy: number): string | null {
  const mx = OUTER_MARGIN_PX * sx;
  const my = OUTER_MARGIN_PX * sy;
  const x1 = Math.max(hole.left, b.left - mx);
  const y1 = Math.max(hole.top, b.top - my);
  const x2 = Math.min(hole.right, b.right + mx);
  const y2 = Math.min(hole.bottom, b.bottom + my);
  if (x2 <= x1 || y2 <= y1) return null;
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
    ` M${f(hx1 + r.tlx)} ${f(hy1)}` +
    ` ${arc(r.tlx, r.tly)} ${f(hx1)} ${f(hy1 + r.tly)}` +
    ` V${f(hy2 - r.bly)}` +
    ` ${arc(r.blx, r.bly)} ${f(hx1 + r.blx)} ${f(hy2)}` +
    ` H${f(hx2 - r.brx)}` +
    ` ${arc(r.brx, r.bry)} ${f(hx2)} ${f(hy2 - r.bry)}` +
    ` V${f(hy1 + r.try_)}` +
    ` ${arc(r.trx, r.try_)} ${f(hx2 - r.trx)} ${f(hy1)}` +
    `Z`
  );
}

/** Which holes cut into `el`: all of them for a plain layer (the map, the page ground), and for an
 *  element that BELONGS to a surface (`data-nv-clip="floating"`) only the holes of surfaces painted
 *  ABOVE it. That is the video frames' case: the floating window's bezel used to shine through the
 *  widget tile's hole, because the tile is transparent there and the bezel sits behind it. DOM
 *  stacking is the reverse of the surface priority (the widget dock paints over the floating window,
 *  which paints over the fullscreen swap), so "above me" is "later in PRIORITY". A surface must
 *  never be cut by its OWN hole — its overlays and its bezel live exactly there. */
function holesFor(own: string | undefined, holes: Hole[]): Hole[] {
  const rank = own ? PRIORITY.indexOf(own as NativeSurfaceId) : -1;
  if (rank < 0) return holes;
  return holes.filter((h) => PRIORITY.indexOf(h.id) > rank);
}

/** A clip that cuts nothing — deliberately NOT "no clip". On Chromium a `path()` clip is a mask on
 *  the layer's render surface, and adding or removing that mask rebuilds the compositor layers
 *  underneath: every map tile rasterises again, one or two frames of dark map while they trickle
 *  back (the Sony, 2026-09-12: docking or parking the video window blanked the tiles both ways, a
 *  visible flash each time). A mask that merely CHANGES costs nothing visible, so while the router
 *  runs every target keeps one — between holes this ring with a one-pixel hole outside the
 *  element's box. A plain rectangle is not enough: Chromium turns that back into a rect clip, no
 *  mask, and the rebuild happens on the way back to a real hole (measured). */
function idlePath(el: HTMLElement): string {
  const m = OUTER_MARGIN_PX;
  const w = el.offsetWidth;
  const h = el.offsetHeight;
  return `path('M${-m} ${-m}H${w + m}V${h + m}H${-m}Z M${1 - m} ${1 - m}h1v1h-1Z')`;
}

function applyClips(holes: Hole[]): void {
  const targets = new Set<HTMLElement>();
  for (const el of document.querySelectorAll<HTMLElement>('[data-nv-clip]')) targets.add(el);
  targets.add(ensureGround());
  for (const el of targets) {
    const mine = holesFor(el.dataset.nvClip, holes);
    const path = (mine.length > 0 ? holePath(el, mine) : null) ?? idlePath(el);
    if (clipped.get(el) !== path) {
      el.style.clipPath = path;
      clipped.set(el, path);
    }
  }
  // Layers that left the target set (unmounted branch, panel closed) keep no stale clip.
  for (const el of [...clipped.keys()]) {
    if (!targets.has(el)) {
      el.style.clipPath = '';
      clipped.delete(el);
    }
  }
  applyInsets(holes);
}

function clearClips(): void {
  for (const el of clipped.keys()) el.style.clipPath = '';
  clipped.clear();
  for (const el of insetClipped.keys()) el.style.clipPath = '';
  insetClipped.clear();
}

/** Elements carrying `data-nv-inset` → the applied `inset()` clip (written on change only). */
const insetClipped = new Map<HTMLElement, string>();

/** `data-nv-inset` elements are cut with a RECTANGULAR clip instead of a mask: the band of the
 *  element under a hole — right, left, bottom or top, the smallest one that covers the whole overlap
 *  — is removed with `inset()`. A plain clip needs no render surface, so the backdrop-filter glass
 *  of the widgets INSIDE keeps blurring the map behind them; under a mask (`data-nv-clip`) that
 *  glass would only see the masked subtree, i.e. nothing. The price: a hole that ends inside the
 *  element cuts the whole band, not an L — the phone's bottom widget tiles, the one user, are never
 *  taller than the docked video window they meet, so the band is exact there. The value means what
 *  it means on `data-nv-clip` (only the holes of surfaces painted above the element). */
function applyInsets(holes: Hole[]): void {
  for (const el of document.querySelectorAll<HTMLElement>('[data-nv-inset]')) {
    const clip = insetPath(el, holesFor(el.dataset.nvInset, holes));
    if (clip) {
      if (insetClipped.get(el) !== clip) {
        el.style.clipPath = clip;
        insetClipped.set(el, clip);
      }
    } else if (insetClipped.has(el)) {
      el.style.clipPath = '';
      insetClipped.delete(el);
    }
  }
  for (const el of [...insetClipped.keys()]) {
    if (!el.isConnected) insetClipped.delete(el);
  }
}

function insetPath(el: HTMLElement, holes: Hole[]): string | null {
  if (holes.length === 0) return null;
  const b = el.getBoundingClientRect();
  if (b.width <= 0 || b.height <= 0) return null;
  // The bounding box of every overlap (viewport px) — one band must cover them all.
  let x1 = Infinity;
  let y1 = Infinity;
  let x2 = -Infinity;
  let y2 = -Infinity;
  for (const { rect: h } of holes) {
    const ix1 = Math.max(b.left, h.left);
    const iy1 = Math.max(b.top, h.top);
    const ix2 = Math.min(b.right, h.right);
    const iy2 = Math.min(b.bottom, h.bottom);
    if (ix2 - ix1 < 0.5 || iy2 - iy1 < 0.5) continue;
    x1 = Math.min(x1, ix1);
    y1 = Math.min(y1, iy1);
    x2 = Math.max(x2, ix2);
    y2 = Math.max(y2, iy2);
  }
  if (x1 === Infinity) return null;
  // inset(top right bottom left) in the element's own layout px (it may sit in the scaled chrome).
  const sx = b.width / (el.offsetWidth || b.width);
  const sy = b.height / (el.offsetHeight || b.height);
  const f = (v: number) => v.toFixed(2);
  const bands = [
    { area: (b.right - x1) * b.height, css: `0 ${f((b.right - x1) / sx)}px 0 0` },
    { area: (x2 - b.left) * b.height, css: `0 0 0 ${f((x2 - b.left) / sx)}px` },
    { area: (y2 - b.top) * b.width, css: `${f((y2 - b.top) / sy)}px 0 0 0` },
    { area: (b.bottom - y1) * b.width, css: `0 0 ${f((b.bottom - y1) / sy)}px 0` },
  ];
  bands.sort((p, q) => p.area - q.area);
  return `inset(${bands[0].css})`;
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
