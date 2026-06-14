// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

/**
 * Shared mission WP-editor popup framework (INAV style), used by both InavMissionLayer and
 * ArduMissionLayer. Holds the popup *scaffolding* — lifecycle, HTML primitives, event wiring — while
 * each layer provides its own domain content (INAV WpAction rows / ArduPilot catalog rows).
 *
 * The lifecycle (`renderEditorPopup`) carries a **content-signature redraw guard**: it only rewrites
 * the popup DOM when the rendered HTML actually changes, so unrelated map redraws (telemetry / home
 * ticks) don't tear down and re-create an open `<select>` mid-interaction.
 *
 * Styling lives in the global `.wp-editor-popup` / `.wpe-*` classes (defined once in the mission
 * layers' `:global` styles).
 */

import L from 'leaflet';

// ── Lifecycle ────────────────────────────────────────────────────────────────

export interface PopupState {
  popup: L.Popup | undefined;
  anchorKey: number;   // identifies the current target (e.g. selected waypoint index)
  html: string;        // last-written HTML — the content signature for the redraw guard
}

export function newPopupState(): PopupState {
  return { popup: undefined, anchorKey: -1, html: '' };
}

export interface RenderPopupOpts {
  popupOptions?: L.PopupOptions;
  /** Called once when the popup is freshly created (e.g. to pan it into view). */
  onCreate?: (popup: L.Popup, latLng: L.LatLng) => void;
}

const DEFAULT_POPUP_OPTIONS: L.PopupOptions = {
  closeButton: false, autoClose: false, closeOnClick: false,
  className: 'wp-editor-popup-container', offset: L.point(0, -30), maxWidth: 240, minWidth: 190,
};

/**
 * Create or update the editor popup with the redraw guard. `attach` wires the event handlers and is
 * (re)run only when the DOM is actually (re)written.
 */
export function renderEditorPopup(
  map: L.Map,
  state: PopupState,
  anchorKey: number,
  latLng: L.LatLng,
  html: string,
  attach: (popup: L.Popup) => void,
  opts: RenderPopupOpts = {},
): void {
  const keep = !!state.popup && state.anchorKey === anchorKey;
  if (keep && state.popup) {
    state.popup.setLatLng(latLng); // cheap reposition — no DOM teardown
    if (html !== state.html) {
      const contentEl = state.popup.getElement()?.querySelector('.leaflet-popup-content');
      if (contentEl) { contentEl.innerHTML = html; state.html = html; queueAttach(state, attach); }
    }
  } else {
    if (state.popup) map.removeLayer(state.popup);
    state.popup = L.popup({ ...DEFAULT_POPUP_OPTIONS, ...opts.popupOptions })
      .setLatLng(latLng).setContent(html).addTo(map);
    state.html = html;
    opts.onCreate?.(state.popup, latLng);
    queueAttach(state, attach);
  }
  state.anchorKey = anchorKey;
}

function queueAttach(state: PopupState, attach: (popup: L.Popup) => void): void {
  setTimeout(() => { if (state.popup) attach(state.popup); }, 50);
}

export function closeEditorPopup(map: L.Map, state: PopupState): void {
  if (state.popup) map.removeLayer(state.popup);
  state.popup = undefined; state.anchorKey = -1; state.html = '';
}

// ── HTML primitives ──────────────────────────────────────────────────────────

/** Number input with −/+ steppers. `dataAttrs` is the full attribute string the caller binds to. */
export function numInputHtml(dataAttrs: string, value: number, opts: { step?: number; min?: number; max?: number } = {}): string {
  const step = opts.step ?? 1;
  const minAttr = opts.min !== undefined ? `min="${opts.min}"` : '';
  const maxAttr = opts.max !== undefined ? `max="${opts.max}"` : '';
  return `<div class="wpe-num-ctrl"><button class="wpe-num-btn" data-numdir="-1">−</button>`
    + `<input type="number" ${dataAttrs} value="${value}" step="${step}" ${minAttr} ${maxAttr}/>`
    + `<button class="wpe-num-btn" data-numdir="1">+</button></div>`;
}

/** Enum dropdown. `dataAttrs` is the full attribute string the caller binds to. */
export function enumSelectHtml(dataAttrs: string, options: [number, string][], current: number): string {
  const opts = options.map(([v, l]) => `<option value="${v}"${v === current ? ' selected' : ''}>${l}</option>`).join('');
  return `<select ${dataAttrs} class="wpe-row-select">${opts}</select>`;
}

/** A label + control row; optional unit suffix and an (i) tooltip carrying the param description. */
export function paramRow(label: string, controlHtml: string, opts: { unit?: string; tooltip?: string } = {}): string {
  const info = opts.tooltip ? ` <span class="wpe-info" title="${escapeAttr(opts.tooltip)}">ⓘ</span>` : '';
  const unit = opts.unit ? `<span class="wpe-unit">${opts.unit}</span>` : '';
  return `<div class="wpe-row"><label>${label}${info}</label>${controlHtml}${unit}</div>`;
}

/** Friendly name + an optional canonical (MAV_CMD) secondary label, rendered smaller/grey. */
export function canonicalLabel(friendly: string, canonical?: string): string {
  return canonical ? `${friendly} <span class="wpe-canonical">${canonical}</span>` : friendly;
}

export function actionsHtml(opts: {
  disableUp?: boolean; disableDown?: boolean; upTitle: string; downTitle: string; removeTitle: string;
}): string {
  return `<div class="wpe-actions">`
    + `<button data-action="moveUp" ${opts.disableUp ? 'disabled' : ''} title="${opts.upTitle}">▲</button>`
    + `<button data-action="moveDown" ${opts.disableDown ? 'disabled' : ''} title="${opts.downTitle}">▼</button>`
    + `<button data-action="remove" class="wpe-remove" title="${opts.removeTitle}">✕</button>`
    + `</div>`;
}

/** A modifier sub-section (INAV-style): a header (with a remove button) + a body of param rows. */
export function modifierSection(headerInner: string, bodyHtml: string, removeAttr: string, removeTitle: string): string {
  return `<div class="wpe-mod-section"><div class="wpe-mod-header">${headerInner}`
    + `<button ${removeAttr} class="wpe-mod-remove" title="${removeTitle}">✕</button></div>${bodyHtml}</div>`;
}

/** The "+ Add modifier…" dropdown wrapper. `innerOptionsHtml` is the full <option>/<optgroup> markup. */
export function addModifierSelect(innerOptionsHtml: string): string {
  return `<div class="wpe-add-mod"><select data-field="addMod">${innerOptionsHtml}</select></div>`;
}

// ── Event primitives ─────────────────────────────────────────────────────────

/** Wire the −/+ steppers in a popup: adjust the sibling <input> by its step, clamped to min/max. */
export function attachNumBtnEvents(el: HTMLElement): void {
  el.querySelectorAll('.wpe-num-btn').forEach((btn) => {
    btn.addEventListener('click', (e) => {
      e.preventDefault(); e.stopPropagation();
      const parent = (btn as HTMLElement).closest('.wpe-num-ctrl');
      const input = parent?.querySelector('input') as HTMLInputElement | null;
      if (!input) return;
      const dir = Number((btn as HTMLElement).dataset.numdir);
      const step = Number(input.step) || 1;
      const min = input.min !== '' ? Number(input.min) : -Infinity;
      const max = input.max !== '' ? Number(input.max) : Infinity;
      let val = Number(input.value) + dir * step;
      val = Math.max(min, Math.min(max, val));
      input.value = String(Math.round(val * 1e6) / 1e6);
      input.dispatchEvent(new Event('change', { bubbles: true }));
    });
  });
}

/** Stop popup form controls from bubbling clicks/drags to the map. */
export function disablePopupPropagation(el: HTMLElement): void {
  el.querySelectorAll('input, select, button').forEach((node) => {
    L.DomEvent.disableClickPropagation(node as HTMLElement);
  });
}

function escapeAttr(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}
