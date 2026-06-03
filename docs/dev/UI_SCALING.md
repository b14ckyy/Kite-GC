# UI Scaling (Global Font / UI Scaler)

> Status: **In progress** (first iteration, 2026-06-03). Experimental — we have a
> clean commit snapshot, so we can iterate freely.

## Goal
The UI fonts are fine on a desktop monitor but too small on a laptop in the field
(sunlight). Add a **global UI scale** the operator can raise: **100 / 125 / 150 %**,
persisted, set from Settings → **Language** section. (We scale the whole chrome, not just
text, so ≥175 % is too much — capped at 150 %.)

The **map must stay at native resolution** (no upscaled/blurry tiles, no loss of
geographic coverage, no Leaflet/Cesium click-offset bugs). Only the *chrome* scales:
toolbar, nav-rail, side/bottom widget docks (incl. the telemetry widgets — the actual
field-readability target), nav panel, dialogs, status bar.

## Why CSS `zoom`, not a `rem` refactor
- The codebase has **258 `px` `font-size` values across 35 files, 0 `rem`, 0 `em`**.
  A `rem` refactor would scale *text only* (not padding/icons/borders) → overflow and
  clipping everywhere → large, risky rework. Rejected.
- CSS `zoom` reflows layout and scales text + spacing + icons + borders **together**,
  exactly like browser Ctrl-+. WebView2 (Windows = Chromium) supports it fully.
- **Native WebView2 zoom** (`webview.set_zoom`) would scale *everything incl. the map*
  and keep all JS pointer math internally consistent — but it cannot exclude the map.
  Rejected because the map must stay crisp.

## Chosen architecture — "hoist the map out of the zoom" (2 layers)
The map (`.map-holder`, holding Leaflet `Map` or Cesium `Map3D`) is the only element
that must NOT be zoomed. It is the same single DOM element (Leaflet/Cesium must **not**
be re-mounted), so we move it out of the zoomed subtree instead of counter-zooming it
(counter-zoom shrinks the box — dead end).

```
<div class="ui-root" style="--ui-scale: <factor>">
  <div class="layer-map">         <!-- UNZOOMED, native crisp map -->
      Map / Map3D                  <!-- single instance, no remount -->
  </div>
  <div class="ui-scale">          <!-- zoom: var(--ui-scale) -->
      dialogs (ConfirmDialog, EndFlightDialog, ContextMenu, BatchEditPopup)
      <main class="app"> … toolbar, nav-rail, panels, docks, widgets,
                            FloatingVideoWindow, map-video, status-bar … </main>
  </div>
</div>
```

- `.ui-scale { zoom: var(--ui-scale); width: calc(100vw / var(--ui-scale));
  height: calc(100vh / var(--ui-scale)); }` so post-zoom it fills exactly the viewport
  (zooming a `100vh` box directly would overflow by the scale factor — hence the
  `/scale` sizing, and `.app` switches from `height:100vh` to `height:100%`).
- `.layer-map` stays unzoomed, fills the content area. The central grid area of `.app`
  is transparent, so the map shows through from below; the chrome zones (toolbar/docks)
  paint over the map edges as before.
- The full-screen video (`.map-video`, shown only when video is primary) **stays inside
  the zoom** — scaling a full-bleed `object-fit:cover` video is visually irrelevant.

### Stacking (z-index)
- Normal: `.layer-map` **below** `.ui-scale` → map behind chrome.
- Video-primary / in-frame: `.layer-map` **above** `.ui-scale` (class flip) so the small
  map sits inside the floating window's body; the window's header/border (zoomed, in
  `.ui-scale`) still frame it because the map rect covers only the body area.
- `zoom` establishes a stacking context, so cross-boundary interleaving is not possible;
  the z-flip is how the map crosses the boundary for the in-frame case.

### The two coordinate "bridges" (unzoomed map ↔ zoomed chrome)
1. **Full-screen map offsets** must track the zoomed toolbar (53px) and status bar
   (24px): `top: calc(53px * var(--ui-scale)); bottom: calc(24px * var(--ui-scale))`.
2. **In-frame rect** (`mapFrameStyle`) is derived from the floating video window's
   position, which lives in the zoomed space. The unzoomed map must be placed at the
   window's *visual* rect → multiply `left/top/width/height` by `ui-scale` in JS.

## Test points (verify in the running app — needs visual check / sim)
1. **Floating video window** drag + resize, and the **video-primary in-frame map swap**
   — the window lives in the zoom, the map does not; the scaled in-frame rect must line
   the small map up inside the window body at every scale step.
2. **Map interaction** (Leaflet pan/click, Cesium) — must stay pixel-accurate because the
   map is unzoomed (this is the whole point; verify no offset crept in).
3. **Widget docks** — telemetry widgets scale up with the docks; check the dock
   auto-sizing math (`bottomDockH`/`sideDockW` from `clientHeight`) still fills the dock
   without clipping at 125/150/175/200 %.
4. **Dialogs / context menu / nav panel** scale and stay centred/positioned.

## Out of scope / known minor edge cases (first iteration)
- A modal opened *while video is primary* can sit behind the in-frame map (map z-flipped
  above chrome). Rare; revisit if it bites.
- Dock resize-by-drag is **not** wired today (`setBottomDockHeight`/`setSideDockWidth`
  are unused), so no pointer-math fix needed there.
- Widget drag-drop ghost uses `clientX+14`; hit-testing uses `getBoundingClientRect`
  (viewport space, consistent under zoom) so drops are correct; the ghost may sit a few
  px off at high scale. Cosmetic.

## Iteration 2 fixes (after first hands-on test)
- **Map was dead (no pan/zoom, controls + WP editor popup unresponsive).** The unzoomed
  map sits *below* `.ui-scale`. **Both** `.ui-scale` (the parent covering the viewport)
  **and** `.app` (the transparent grid container) captured events over the empty centre —
  making only `.app` click-through is not enough, the parent eats the event the moment
  `.app` passes it on. Fix: `pointer-events: none` on `.ui-scale` **and** `.app`, with
  `> *` re-capturing solid children (dialogs via `.ui-scale > *`; zones via `.app > *`),
  and docks/map-controls re-disabled so the map stays draggable under/around them.
  (The WP editor is a Leaflet popup *inside the map*, so it shares the map's fate.)
- **WP editor popup scaled to match.** It lives in the unzoomed map, so it is scaled via
  `transform: scale(var(--ui-scale))` with `transform-origin: bottom center` on the
  `.leaflet-popup-content-wrapper` — the tip stays anchored over the WP, the box grows
  upward. (`--ui-scale` inherits from `.ui-root` down into the map's popup pane.) If the
  anchor drifts at a step, a per-step translate offset can compensate (only 3 steps).
- **Side panels (Settings/Mission/…) overflowed the viewport bottom at higher scale.**
  `.nav-panel` used `calc(100vh − …)` / `calc(100vw − …)`; under the zoom, `vh`/`vw` resolve
  to the *device* viewport, not the `/scale`-sized `.ui-scale` container. Switched to
  `100%` (resolves against `.app`, which is the scaled container).
- **WP edit: selected WP flew to the map edge behind panels.** The editor is a Leaflet
  popup with default `autoPan` (fits the popup to the *container*, ignoring overlapping
  panels). Disabled `autoPan`; on fresh select we `panBy` to put the WP at ~(55 %, 60 %) of
  the container — clears the left mission panel + bottom player, leaving the upward popup
  centred. Map is unzoomed so the pixel math is scale-independent.
- **WP list squeezed to ~½ a row at 150 %.** The mission panel's fixed detail card +
  action buttons left almost no room for the scrollable list. Gave `.wp-frame` a
  `min-height` (~5 rows) and made `.mission-panel` scroll (`overflow: hidden auto`), so
  the list keeps its minimum and the rest of the panel scrolls instead.
- Scale steps capped at 150 % (was 200 %).

## Iteration 3 — scaling the map-overlay UI
Map-overlay elements live in the unzoomed map, so each is scaled with the technique that
does **not** clobber Leaflet's own positioning transform (they inherit `--ui-scale` from
`.ui-root`):
- **WP markers** — `transform: scale(var(--ui-scale))` on the marker **SVG** (the child of
  the Leaflet-positioned `.leaflet-marker-icon`, so Leaflet's translate is untouched).
  `transform-origin` follows the on-coordinate anchor via a class from `iconForWp`
  (`wp-anchor-bottom` = 50 % 100 % for teardrops, else 50 % 50 %). Hit area stays the
  original icon size (acceptable).
- **Right-click context menu** — `transform: scale()` + `transform-origin: top left` on
  `.cm-menu` (it sits *outside* the zoom so its clientX/clientY anchor stays correct; the
  viewport-clamp effect reads the scaled size via `getBoundingClientRect`).
- **Leaflet tooltips** ("hover toasts") — scaled via `font-size` + `padding` (Leaflet sets
  an inline positioning transform, so a CSS `transform` would be overridden; em/px scaling
  reflows the box).
- **WP param labels** (`wp-param-label`) — `transform: scale()` on the label box (child of
  the Leaflet-positioned wrapper), `transform-origin: top left`.
- **Native `title` panel tooltips can't be scaled** — they are rendered by WebView2/Chromium
  outside the DOM. Making them scalable needs a **custom tooltip system** (a `use:tooltip`
  action + a singleton overlay outside the zoom, `transform`-scaled like the context menu)
  replacing the ~58 `title=` sites. **Deferred** for now (decided not to build yet); this is
  the natural home for the future feature-description hint system too.
- Not yet scaled: launch/home marker.

## Implementation checklist
- [ ] `AppSettings.uiScale: number` (default `1`) + persistence (settings store).
- [ ] Settings → Language: a 100–200 % (25 % steps) `<select>`; i18n key in en + de.
- [ ] `+page.svelte`: `.ui-root` (`--ui-scale`) + `.layer-map` (map hoisted out) +
      `.ui-scale` (zoom wrapper around dialogs + `.app`); `.app` height `100vh`→`100%`.
- [ ] Scale the two bridges (map offsets `* ui-scale`; `mapFrameStyle` `* uiScale`).
- [ ] `npm run check` 0 errors + `npm run build`.
