// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)
/**
 * Upright overlays on a rotated map (heading-up mode).
 *
 * The 2D map rotates as a whole — the `.map` container carries `transform: rotate(var(--map-rotation))`
 * — so everything Leaflet draws into it turns with the tiles: marker icons, popups, tooltips. Vector
 * geometry (tracks, zones) should, but a waypoint teardrop, a callsign label or the "Fly here"
 * popup must stay readable: bound to its geographic anchor, drawn upright.
 *
 * Leaflet positions those elements with an inline `transform: translate3d(...)` that it rewrites on
 * every move (`DomUtil.setPosition`), so a CSS rule cannot add the counter-rotation. This module
 * patches the few prototype methods that write that transform and appends
 * `rotate(var(--kite-upright, 0deg))`, pivoting on the element's anchor point:
 *   - `L.Marker._setPos`        — the icon's anchor sits at (-marginLeft, -marginTop) of its box;
 *   - `L.Draggable._updatePosition` — a dragged icon is moved by the Draggable, not by `_setPos`;
 *   - `L.Popup._updatePosition` / `_animateZoom` — the popup box hangs above the anchor by
 *     `margin-bottom` plus the popup offset (see `_updatePosition` in Leaflet's DivOverlay);
 *   - `L.Tooltip._setPosition` — the box is placed by `direction` (top/bottom/left/right/center)
 *     minus offset and anchor.
 * The angle itself is a CSS custom property set by Map.svelte on the `.map` element
 * (`--kite-upright: calc(-1 * var(--map-rotation))` while heading-up, unset = 0deg otherwise), so a
 * heading change is one variable write, not a walk over every marker. Elements whose orientation IS
 * geographic (the UAV model, radar silhouettes) opt out by redefining the variable to 0deg.
 */
import L from 'leaflet';

const ROT = ' rotate(var(--kite-upright, 0deg))';

let installed = false;

/** Append the counter-rotation to the transform Leaflet just wrote, pivoting on `origin`. */
function upright(el: HTMLElement, origin?: string): void {
  if (!el.style.transform.includes(ROT)) el.style.transform += ROT;
  if (origin !== undefined) el.style.transformOrigin = origin;
}

type MarkerInternals = { _icon?: HTMLElement; _setPos(pos: L.Point): void };
type DraggableInternals = { _element: HTMLElement; _updatePosition(): void };
type PopupInternals = {
  _container: HTMLElement;
  _containerWidth: number;
  _kiteMarginBottom?: number;
  options: L.PopupOptions;
  _getAnchor(): L.Point;
  _updatePosition(): void;
  _animateZoom(e: unknown): void;
};
type TooltipInternals = {
  _container: HTMLElement;
  options: L.TooltipOptions;
  _getAnchor(): L.Point;
  _setPosition(pos: L.Point): void;
};

/** Install the prototype patches once per document (both the main map and the mini map share them). */
export function installUprightOverlays(): void {
  if (installed) return;
  installed = true;

  // ── Markers ─────────────────────────────────────────────────────────────────────────────────────
  const marker = L.Marker.prototype as unknown as MarkerInternals;
  const origSetPos = marker._setPos;
  marker._setPos = function (this: MarkerInternals, pos: L.Point) {
    origSetPos.call(this, pos);
    const icon = this._icon;
    if (!icon) return;
    // Leaflet shifts the icon by negative margins so the anchor lands on the coordinate.
    const ax = -(parseFloat(icon.style.marginLeft) || 0);
    const ay = -(parseFloat(icon.style.marginTop) || 0);
    upright(icon, `${ax}px ${ay}px`);
  };

  const draggable = L.Draggable.prototype as unknown as DraggableInternals;
  const origDragUpdate = draggable._updatePosition;
  draggable._updatePosition = function (this: DraggableInternals) {
    origDragUpdate.call(this);
    // Only marker icons — the map pane is dragged through the same class. The origin was set by _setPos.
    if (this._element.classList.contains('leaflet-marker-icon')) upright(this._element);
  };

  // ── Popups ──────────────────────────────────────────────────────────────────────────────────────
  const popup = L.Popup.prototype as unknown as PopupInternals;
  const origPopupUpdate = popup._updatePosition;
  popup._updatePosition = function (this: PopupInternals) {
    origPopupUpdate.call(this);
    // `.leaflet-popup { margin-bottom }` (20px in leaflet.css) keeps the box above the anchor. Read
    // once the container is in the DOM (a detached element has no computed style).
    if (this._kiteMarginBottom === undefined) {
      this._kiteMarginBottom = parseFloat(getComputedStyle(this._container).marginBottom) || 0;
    }
    // Leaflet places the box with its horizontal centre `offset.x + anchor.x` right of the coordinate
    // and its bottom margin edge `offset.y + anchor.y` below it (as `left`/`bottom` on top of the
    // translated point, or in `left`/`bottom` alone when zoom animation is off — same geometry).
    // The coordinate relative to the box is therefore
    // (round(width/2) - offset.x - anchor.x, height + marginBottom - offset.y - anchor.y).
    const offset = L.point(this.options.offset ?? [0, 0]);
    const anchor = this._getAnchor();
    const x = Math.round(this._containerWidth / 2) - offset.x - anchor.x;
    const y = this._kiteMarginBottom - offset.y - anchor.y;
    upright(this._container, `${x}px calc(100% + ${y}px)`);
  };
  const origPopupZoom = popup._animateZoom;
  popup._animateZoom = function (this: PopupInternals, e: unknown) {
    origPopupZoom.call(this, e);
    upright(this._container);
  };

  // ── Tooltips ────────────────────────────────────────────────────────────────────────────────────
  const tooltip = L.Tooltip.prototype as unknown as TooltipInternals;
  const origTipSetPos = tooltip._setPosition;
  tooltip._setPosition = function (this: TooltipInternals, pos: L.Point) {
    origTipSetPos.call(this, pos);
    // The box is placed at coordinate - sub + offset + anchor, `sub` depending on the resolved
    // direction (Leaflet leaves it as a class); `auto` resolving to "left" subtracts the offset and
    // anchor twice. leaflet.css then nudges the box away from the point with a 6px margin per
    // direction, which shifts the border box the origin is measured against. The coordinate relative
    // to the box is therefore sub - offset - anchor - margin.
    const c = this._container;
    const offset = L.point(this.options.offset ?? [0, 0]);
    const anchor = this._getAnchor();
    const cs = getComputedStyle(c);
    const dx = offset.x + anchor.x + (parseFloat(cs.marginLeft) || 0);
    const dy = offset.y + anchor.y + (parseFloat(cs.marginTop) || 0);
    const cls = c.classList;
    let subX = '50%';
    let subY = '50%';
    if (cls.contains('leaflet-tooltip-top')) subY = '100%';
    else if (cls.contains('leaflet-tooltip-bottom')) subY = '0%';
    else if (cls.contains('leaflet-tooltip-right')) subX = '0%';
    else if (cls.contains('leaflet-tooltip-left')) {
      subX = this.options.direction === 'auto' ? `calc(100% + ${2 * (offset.x + anchor.x)}px)` : '100%';
    }
    upright(c, `calc(${subX} - ${dx}px) calc(${subY} - ${dy}px)`);
  };
}
