// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

/** The floating video window's gestures — MOVE (the bottom-left corner handle, the only way to
 *  move it) and RESIZE (the bezel's top-right corner, aspect-locked, bottom edge anchored). Shared
 *  by FloatingVideoWindow and the corners +page draws on the mini-map frame, which sit ABOVE the
 *  window when the map is swapped in (the map layer covers the chrome). A body drag and a right
 *  mouse / two-finger grab on the map used to move it too; both went (Marc, 2026-09-10) so the 3D
 *  map keeps its tilt button and a touchscreen keeps pinch and tilt.
 *
 *  Geometry in the chrome layer's logical px (`vw`/`vh` = viewport / uiScale); pointer positions
 *  arrive in viewport px and are scaled here. Move: the first real movement un-snaps, a drop near
 *  the bottom-left corner re-snaps. */
import { get } from 'svelte/store';
import {
  videoState,
  setFloatPos,
  setFloatSnapped,
  setFloatHeightFrac,
  FLOAT_MARGIN_PX,
  FLOAT_SNAP_BOTTOM_PX,
  FLOAT_FRAC_MIN,
  FLOAT_FRAC_MAX,
  FLOAT_MIN_H_PX,
} from '$lib/stores/video';

export interface FloatFrame {
  left: number;
  top: number;
  width: number;
  height: number;
  vw: number;
  vh: number;
}

const SNAP_THRESHOLD = 56;

/** Viewport px → logical px of the zoomed chrome layer. */
function scaleOf(frame: FloatFrame): number {
  return frame.vw > 0 ? window.innerWidth / frame.vw : 1;
}

/** A move session started at viewport point (cx, cy): feed it the pointer, end it on release. */
function beginFloatMove(cx: number, cy: number, frame: FloatFrame): { moveTo(cx: number, cy: number): void; end(): void } {
  const { left, top, width, height, vw, vh } = frame;
  const scale = scaleOf(frame);
  let moved = false;
  let curX = left;
  let curY = top;
  return {
    moveTo(x, y) {
      const dx = (x - cx) / scale;
      const dy = (y - cy) / scale;
      if (!moved && Math.hypot(dx, dy) < 4) return;
      if (!moved) {
        moved = true;
        setFloatSnapped(false); // first real movement detaches from the corner
      }
      curX = Math.max(0, Math.min(left + dx, vw - width));
      curY = Math.max(0, Math.min(top + dy, vh - height));
      setFloatPos(curX, curY);
    },
    end() {
      if (!moved) return;
      const nearLeft = curX <= FLOAT_MARGIN_PX + SNAP_THRESHOLD;
      const nearBottom = curY + height >= vh - FLOAT_SNAP_BOTTOM_PX - SNAP_THRESHOLD;
      if (nearLeft && nearBottom) setFloatSnapped(true);
    },
  };
}

/** Left-button / single-touch drag of the move handle: a move session on window listeners. */
export function startFloatMove(e: PointerEvent, frame: FloatFrame): void {
  if (e.button !== 0) return;
  e.preventDefault();
  e.stopPropagation();
  const session = beginFloatMove(e.clientX, e.clientY, frame);
  const onMove = (ev: PointerEvent) => session.moveTo(ev.clientX, ev.clientY);
  const onUp = () => {
    window.removeEventListener('pointermove', onMove);
    window.removeEventListener('pointerup', onUp);
    window.removeEventListener('pointercancel', onUp);
    session.end();
  };
  window.addEventListener('pointermove', onMove);
  window.addEventListener('pointerup', onUp);
  window.addEventListener('pointercancel', onUp);
}

/** Resize from the top-right corner: drag up = bigger (the height fraction of the viewport, aspect
 *  follows). A free (un-snapped) window keeps its bottom edge where it is; a snapped one is anchored
 *  by the corner anyway. */
export function startFloatResize(e: PointerEvent, frame: FloatFrame): void {
  if (e.button !== 0) return;
  e.preventDefault();
  e.stopPropagation();
  const { top, height, vh } = frame;
  const scale = scaleOf(frame);
  const s = get(videoState);
  const startY = e.clientY;
  const startFrac = s.floatHeightFrac;
  const startBottom = top + height;
  const snapped = s.floatSnapped;
  const fracMin = Math.max(FLOAT_FRAC_MIN, FLOAT_MIN_H_PX / vh); // honour the px floor
  const onMove = (ev: PointerEvent) => {
    const delta = (startY - ev.clientY) / scale / vh; // drag up → larger
    const frac = Math.min(FLOAT_FRAC_MAX, Math.max(fracMin, startFrac + delta));
    setFloatHeightFrac(frac);
    if (!snapped) setFloatPos(get(videoState).floatX, Math.max(0, startBottom - frac * vh));
  };
  const onUp = () => {
    window.removeEventListener('pointermove', onMove);
    window.removeEventListener('pointerup', onUp);
    window.removeEventListener('pointercancel', onUp);
  };
  window.addEventListener('pointermove', onMove);
  window.addEventListener('pointerup', onUp);
  window.addEventListener('pointercancel', onUp);
}
