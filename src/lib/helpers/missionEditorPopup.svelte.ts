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
 * Numeric fields are the app's `NumberStepper` component, MOUNTED into the popup (Svelte `mount()`)
 * rather than imitated in HTML — one stepper for the whole UI: same look, same ± / wheel behaviour,
 * and a design or behaviour change lands here without a second implementation. The HTML carries only
 * a placeholder for each (no value), so a value change never alters the content signature: the DOM
 * stays, the mounted stepper keeps focus, and the wheel can step on. Its value is pushed in place on
 * every render instead (a change from elsewhere — batch edit, terrain correction — still shows).
 * Until now each step rewrote the popup and the focused field died with it: one notch, then nothing
 * (Marc, 2026-09-14, Linux).
 *
 * Styling lives in the global `.wp-editor-popup` / `.wpe-*` classes (defined once in the mission
 * layers' `:global` styles).
 */

import L from 'leaflet';
import { mount, unmount } from 'svelte';
import NumberStepper from '$lib/components/NumberStepper.svelte';
import { iconSvg } from '$lib/components/panel/Button.svelte';

/** The trash can of the panel buttons, for the popup's remove buttons — an X reads as "close". */
export const REMOVE_ICON_HTML = iconSvg('delete');

// ── Lifecycle ────────────────────────────────────────────────────────────────

export interface PopupState {
  popup: L.Popup | undefined;
  anchorKey: number;   // identifies the current target (e.g. selected waypoint index)
  html: string;        // last-written HTML — the content signature for the redraw guard
  /** Mounted steppers of the current DOM, by their placeholder key (the data-attribute string). */
  steppers: Map<string, MountedStepper>;
  /** A layer render held back while a stepper is being worked (see `deferWhileStepping`). */
  renderTimer: ReturnType<typeof setTimeout> | undefined;
  /** For re-placing the popup when the window or the map resizes. */
  latLng: L.LatLng | undefined;
  onResize: (() => void) | undefined;
  /** The pending handler wiring of the last DOM write (`queueAttach`). */
  attachTimer: ReturnType<typeof setTimeout> | undefined;
}

export function newPopupState(): PopupState {
  return { popup: undefined, anchorKey: -1, html: '', steppers: new Map(), renderTimer: undefined, latLng: undefined, onResize: undefined, attachTimer: undefined };
}

/** How long the layer waits after the last stepper change before it redraws its markers. */
const STEPPING_RENDER_DELAY_MS = 500;

/**
 * Hold a layer's full re-render while one of this popup's steppers has focus: every ± / wheel step
 * updates the mission store, and the layers redraw ALL markers on each store change — a fast wheel
 * made them flicker (Marc, 2026-09-14). Returns true when `render` was scheduled (trailing, restarted
 * on every call) instead of run; false means "not stepping — render now". A render from any other
 * cause while nothing is focused cancels a pending one and runs immediately, so drag, add and
 * selection stay instant.
 */
export function deferWhileStepping(state: PopupState, render: () => void): boolean {
  const active = document.activeElement;
  const stepping = !!active && [...state.steppers.values()].some((m) => m.input === active);
  if (state.renderTimer !== undefined) { clearTimeout(state.renderTimer); state.renderTimer = undefined; }
  if (!stepping) return false;
  state.renderTimer = setTimeout(() => { state.renderTimer = undefined; render(); }, STEPPING_RENDER_DELAY_MS);
  return true;
}

// ── Mounted NumberSteppers ───────────────────────────────────────────────────

interface StepperOpts { step?: number; min?: number; max?: number; unit?: string; decimals?: number }
interface StepperSpec extends StepperOpts { key: string; value: number }
/** Reactive props of a mounted stepper — writing them updates the component in place, and the
 *  component writes its own value back (a `$state` props object under `mount()` is two-way). */
interface StepperProps { value: number; min: number; max: number; step: number; unit: string; decimals: number | undefined; onchange: (e: Event) => void }
interface MountedStepper { props: StepperProps; instance: Record<string, unknown>; input: HTMLInputElement | null }

/** Steppers the HTML build in progress asked for — consumed by the next `renderEditorPopup`. The
 *  layers build their HTML and render it in one synchronous go, so this never interleaves. */
let pendingSteppers: StepperSpec[] = [];

function mountSteppers(state: PopupState, contentEl: Element, specs: StepperSpec[]): void {
  unmountSteppers(state);
  contentEl.querySelectorAll<HTMLElement>('.wpe-ns').forEach((slot) => {
    const key = slot.dataset.ns ?? '';
    const spec = specs.find((sp) => sp.key === key);
    if (!spec) return;
    const props: StepperProps = $state({
      value: spec.value,
      min: spec.min ?? -Infinity,
      max: spec.max ?? Infinity,
      step: spec.step ?? 1,
      unit: spec.unit ?? '',
      decimals: spec.decimals,
      onchange: (e: Event) => {
        const m = state.steppers.get(key);
        if (!m?.input) return;
        // A typed value arrives as the input's own change event, already on its way through the
        // layer's listener; a ± / wheel step arrives as the component's synthetic event — make it a
        // real one on the input, which is where the layers listen (`input[data-field=…]`).
        if (e.target === m.input) return;
        m.input.value = String(m.props.value);
        m.input.dispatchEvent(new CustomEvent('change', { bubbles: true }));
      },
    });
    const instance = mount(NumberStepper, { target: slot, props });
    const input = slot.querySelector('input');
    // The layers address the field by the data attributes they gave `numInputHtml` — hand them to
    // the component's input, so their `querySelector` + change wiring is unchanged.
    if (input) for (const [, name, val] of key.matchAll(/([\w-]+)="([^"]*)"/g)) input.setAttribute(name, val);
    state.steppers.set(key, { props, instance, input });
  });
}

function syncSteppers(state: PopupState, specs: StepperSpec[]): void {
  for (const spec of specs) {
    const m = state.steppers.get(spec.key);
    if (!m) continue;
    // Not while the pilot is in the field: unrelated renders (telemetry / home ticks) must not
    // overwrite a half-typed value — the field already holds the newest value it produced itself.
    const editing = !!m.input && document.activeElement === m.input;
    if (!editing && m.props.value !== spec.value) m.props.value = spec.value;
    const unit = spec.unit ?? '';
    if (m.props.unit !== unit) m.props.unit = unit;
    if (m.props.min !== (spec.min ?? -Infinity)) m.props.min = spec.min ?? -Infinity;
    if (m.props.max !== (spec.max ?? Infinity)) m.props.max = spec.max ?? Infinity;
  }
}

function unmountSteppers(state: PopupState): void {
  for (const m of state.steppers.values()) unmount(m.instance);
  state.steppers.clear();
}

export interface RenderPopupOpts {
  popupOptions?: L.PopupOptions;
  /** Called once when the popup is freshly created (e.g. to pan it into view). */
  onCreate?: (popup: L.Popup, latLng: L.LatLng) => void;
}

const DEFAULT_POPUP_OPTIONS: L.PopupOptions = {
  closeButton: false, autoClose: false, closeOnClick: false,
  className: 'wp-editor-popup-container', offset: L.point(0, -30), maxWidth: 290, minWidth: 220,
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
  const specs = pendingSteppers;
  pendingSteppers = [];
  state.latLng = latLng;
  if (!state.onResize) {
    // A resized window can change the answer between marker and dock.
    state.onResize = () => { if (state.popup) queuePlacement(map, state); };
    map.on('resize', state.onResize);
    window.addEventListener('resize', state.onResize);
  }
  const keep = !!state.popup && state.anchorKey === anchorKey;
  if (keep && state.popup) {
    state.popup.setLatLng(latLng); // cheap reposition — no DOM teardown
    if (html !== state.html) {
      const contentEl = state.popup.getElement()?.querySelector('.leaflet-popup-content');
      if (contentEl) {
        unmountSteppers(state);
        contentEl.innerHTML = html;
        state.html = html;
        mountSteppers(state, contentEl, specs);
        queueAttach(state, attach);
        queuePlacement(map, state);
      }
    } else {
      syncSteppers(state, specs);
    }
  } else {
    if (state.popup) { unmountSteppers(state); map.removeLayer(state.popup); }
    state.popup = L.popup({ ...DEFAULT_POPUP_OPTIONS, ...opts.popupOptions })
      .setLatLng(latLng).setContent(html).addTo(map);
    state.html = html;
    const contentEl = state.popup.getElement()?.querySelector('.leaflet-popup-content');
    if (contentEl) mountSteppers(state, contentEl, specs);
    opts.onCreate?.(state.popup, latLng);
    queueAttach(state, attach);
    queuePlacement(map, state);
  }
  state.anchorKey = anchorKey;
}

// ── Placement: at the marker, or docked ──────────────────────────────────────

/** Share of the free map height above which the editor leaves its marker and docks at the right; it
 *  returns below the lower share, so an editor near the limit does not switch on every change. */
const DOCK_HEIGHT_FRACTION = 0.75;
const UNDOCK_HEIGHT_FRACTION = 0.65;
const DOCK_MARGIN_PX = 8;
/** Where the waypoint sits once the editor is docked: this far left of the editor's edge. */
const DOCK_MARKER_GAP_PX = 70;
const DOCK_MOVE_MS = 220;

/** `translate`, not `transform`: Leaflet owns `transform`, and the docked class pins it with `!important`. */
function slideFrom(el: HTMLElement, before: DOMRect): void {
  const after = el.getBoundingClientRect();
  const dx = before.left - after.left;
  const dy = before.top - after.top;
  if (Math.abs(dx) < 1 && Math.abs(dy) < 1) return;
  el.animate([{ translate: `${dx}px ${dy}px` }, { translate: '0 0' }], { duration: DOCK_MOVE_MS, easing: 'ease-out' });
}

/** What the UI's bars and widget docks cover of the map, in px from the map container's edges. */
function coveredEdges(mapRect: DOMRect): { top: number; right: number; bottom: number } {
  const edge = (selector: string) => {
    const el = document.querySelector<HTMLElement>(selector);
    const r = el?.getBoundingClientRect();
    return r && r.width > 0 && r.height > 0 ? r : null;
  };
  const toolbar = edge('.zone-toolbar');
  const side = edge('.zone-side-dock:not(.zone-hidden)') ?? edge('.zone-phone-widgets .pwp');
  const bottom = edge('.zone-bottom-dock:not(.zone-hidden)') ?? edge('.pbb');
  // A phone's widget panel sits below the map in portrait: only a column at the side narrows the map.
  const sideIsColumn = side != null && side.height > side.width;
  return {
    top: toolbar ? Math.max(0, toolbar.bottom - mapRect.top) : 0,
    right: sideIsColumn ? Math.max(0, mapRect.right - side.left) : 0,
    bottom: bottom ? Math.max(0, mapRect.bottom - bottom.top) : 0,
  };
}

function queuePlacement(map: L.Map, state: PopupState): void {
  requestAnimationFrame(() => placeEditor(map, state));
}

/** An editor too tall for the free map height is docked at the right edge of that area, where it
 *  scrolls. While docked the element lives in the map container; the popup still owns content,
 *  steppers and removal, and the docked class overrides the position Leaflet keeps writing. */
function placeEditor(map: L.Map, state: PopupState): void {
  const el = state.popup?.getElement();
  const inner = el?.querySelector<HTMLElement>('.wp-editor-popup');
  const latLng = state.latLng;
  if (!el || !inner || !latLng) return;
  // Moving the element takes the focus from a field the pilot is typing in.
  if (el.contains(document.activeElement)) return;
  const container = map.getContainer();
  const mapRect = container.getBoundingClientRect();
  const covered = coveredEdges(mapRect);
  const free = mapRect.height - covered.top - covered.bottom;
  const docked = el.classList.contains('wp-editor-docked');
  // Screen px, like `free`: the editor is drawn at the UI scale. Heading-up rotates the map container
  // and a docked child with it, so the editor stays at its marker then.
  const height = inner.getBoundingClientRect().height;
  const rotated = container.style.transform.includes('rotate');
  const dock = !rotated && height > free * (docked ? UNDOCK_HEIGHT_FRACTION : DOCK_HEIGHT_FRACTION);
  if (dock) {
    el.style.setProperty('--wpe-dock-top', `${covered.top + DOCK_MARGIN_PX}px`);
    el.style.setProperty('--wpe-dock-right', `${covered.right + DOCK_MARGIN_PX}px`);
    el.style.setProperty('--wpe-dock-max-height', `${Math.max(120, free - 2 * DOCK_MARGIN_PX)}px`);
    if (docked) return;
    const before = el.getBoundingClientRect();
    el.classList.add('wp-editor-docked');
    container.appendChild(el);
    // Only the popup's content node stops the wheel: over the docked scrollbar it would zoom the map.
    L.DomEvent.disableScrollPropagation(el);
    // Measured before the slide starts: a running animation is part of the element's rect.
    const sheetLeft = el.getBoundingClientRect().left - mapRect.left;
    slideFrom(el, before);
    // The waypoint ends up just left of the editor, at its own height where that is free: the map then
    // moves the way the editor went, and the marker stays clear of a panel on the left.
    map.stop();
    const marker = map.latLngToContainerPoint(latLng);
    const target = L.point(
      Math.max(DOCK_MARKER_GAP_PX, sheetLeft - DOCK_MARKER_GAP_PX),
      Math.min(Math.max(marker.y, covered.top + DOCK_MARKER_GAP_PX), mapRect.height - covered.bottom - DOCK_MARKER_GAP_PX / 2),
    );
    map.panBy(marker.subtract(target), { animate: true });
  } else if (docked) {
    const before = el.getBoundingClientRect();
    el.classList.remove('wp-editor-docked');
    map.getPane('popupPane')?.appendChild(el);
    // Not `update()`: that writes the popup's original content back and drops the mounted steppers.
    state.popup?.setLatLng(latLng);
    // The editor opens upward from its marker: bring it down where it would start under the toolbar.
    const overTop = mapRect.top + covered.top + DOCK_MARGIN_PX - el.getBoundingClientRect().top;
    slideFrom(el, before);
    if (overTop > 0) { map.stop(); map.panBy([0, -overTop], { animate: true }); }
  }
}

/** One pending wiring at a time: a second DOM write within the delay must not leave the first write's
 *  handlers — with its waypoint — to land on the newer content as well. */
function queueAttach(state: PopupState, attach: (popup: L.Popup) => void): void {
  if (state.attachTimer !== undefined) clearTimeout(state.attachTimer);
  state.attachTimer = setTimeout(() => {
    state.attachTimer = undefined;
    if (state.popup) attach(state.popup);
  }, 50);
}

export function closeEditorPopup(map: L.Map, state: PopupState): void {
  if (state.renderTimer !== undefined) { clearTimeout(state.renderTimer); state.renderTimer = undefined; }
  if (state.attachTimer !== undefined) { clearTimeout(state.attachTimer); state.attachTimer = undefined; }
  unmountSteppers(state);
  pendingSteppers = [];
  if (state.popup) map.removeLayer(state.popup);
  if (state.onResize) {
    map.off('resize', state.onResize);
    window.removeEventListener('resize', state.onResize);
    state.onResize = undefined;
  }
  state.popup = undefined; state.anchorKey = -1; state.html = ''; state.latLng = undefined;
}

// ── HTML primitives ──────────────────────────────────────────────────────────

/** A `NumberStepper` field. `dataAttrs` is the full attribute string the caller binds to (it ends up
 *  on the stepper's input); the value is NOT part of the HTML — see the module docs. */
export function numInputHtml(dataAttrs: string, value: number, opts: StepperOpts = {}): string {
  pendingSteppers.push({ key: dataAttrs, value, ...opts });
  return `<span class="wpe-ns" data-ns="${escapeAttr(dataAttrs)}"></span>`;
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
    + `<button data-action="remove" class="wpe-remove" title="${opts.removeTitle}">${REMOVE_ICON_HTML}</button>`
    + `</div>`;
}

/** A modifier sub-section (INAV-style): a header (with a remove button) + a body of param rows. */
export function modifierSection(headerInner: string, bodyHtml: string, removeAttr: string, removeTitle: string): string {
  return `<div class="wpe-mod-section"><div class="wpe-mod-header">${headerInner}`
    + `<button ${removeAttr} class="wpe-mod-remove" title="${removeTitle}">${REMOVE_ICON_HTML}</button></div>${bodyHtml}</div>`;
}

/** The "+ Add modifier…" dropdown wrapper. `innerOptionsHtml` is the full <option>/<optgroup> markup. */
export function addModifierSelect(innerOptionsHtml: string): string {
  return `<div class="wpe-add-mod"><select data-field="addMod">${innerOptionsHtml}</select></div>`;
}

// ── Event primitives ─────────────────────────────────────────────────────────

/** Stop popup form controls from bubbling clicks/drags to the map. */
export function disablePopupPropagation(el: HTMLElement): void {
  el.querySelectorAll('input, select, button').forEach((node) => {
    L.DomEvent.disableClickPropagation(node as HTMLElement);
  });
}

function escapeAttr(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}
