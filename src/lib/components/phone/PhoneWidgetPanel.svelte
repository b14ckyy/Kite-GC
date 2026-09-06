<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- PhoneWidgetPanel — the full-height smoked-glass widget column on the phone's right edge
     (Dev-Docs active/PHONE_UI.md D2/D6–D10). One slot = usable height / PHONE_GRID_ROWS; the packer
     (helpers/phoneGridPacker.ts) turns the persisted positions into page/row/col placements, and
     this component draws them: widgets edge to edge on the glass (their own card glass is switched
     off here), two pages under a vertical snap scroll, page dots at the bottom. The panel is 1 or 2
     slots wide as the packer decides and reports its width (`widthPx`).

     Edit mode (D10): a long-press (500 ms, on Android's own haptic) on a widget arms it — every
     widget shows a dashed frame and a corner resize button (one tap = next size state). In edit
     mode a touch that HOLDS 250 ms picks the widget up (the slot under the finger stays under the
     finger — consistent for 2×2 wherever it is grabbed), and while it is dragged the OTHER widgets
     already show where the packer would settle them for that drop (live preview = the same packer
     the commit uses); holding near the panel's top/bottom edge flips the page. A quicker flick
     flips the page instead of grabbing. A tap anywhere outside the panel ends edit mode. -->
<script lang="ts">
  import { t } from 'svelte-i18n';
  import { PHONE_GRID_ROWS, PHONE_GRID_PAGES, PHONE_GRID_PAD } from '$lib/config/phoneGrid';
  import { movePhoneWidget, packPhone, type PhoneWidgetsConfig } from '$lib/controllers/phoneWidgetController';
  import WidgetRenderer from '$lib/components/WidgetRenderer.svelte';
  import type { TelemetryData } from '$lib/stores/telemetry';
  import type { InterfaceSettings } from '$lib/stores/settings';

  let {
    config,
    telem,
    interfaceSettings,
    onresize,
    onmove,
    widthPx = $bindable(0),
  }: {
    config: PhoneWidgetsConfig;
    telem: TelemetryData;
    interfaceSettings: InterfaceSettings;
    /** Edit mode: step the widget to its next size state. */
    onresize?: (id: string) => void;
    /** Edit mode: the user dropped the widget at (page, row, col). */
    onmove?: (id: string, page: number, row: number, col: number) => void;
    /** OUT: the panel's rendered width in css px. */
    widthPx?: number;
  } = $props();

  const PAD = PHONE_GRID_PAD;
  // Android's own long-press haptic fires at ~500 ms; arming later than that reads as "press
  // longer than the buzz" (Marc) — so arm exactly there.
  const LONG_PRESS_MS = 500;
  const DRAG_SLOP_PX = 8;
  /** In edit mode a touch must HOLD this long before it picks the widget up; a quicker flick
   *  scrolls the pages instead (otherwise every touch grabs a widget and the pages are stuck). */
  const DRAG_HOLD_MS = 250;
  /** A flick must travel this far (css px) to flip the page. */
  const FLICK_PX = 40;
  const EDGE_FLIP_PX = 24;
  const EDGE_FLIP_MS = 500;

  let heightPx = $state(0);
  // Bounded by the VIEWPORT height, never by the measured box alone: the box must not be able to
  // feed back into the slot (a grid row that grows with the pages did exactly that once).
  const viewportH = $derived(typeof window === 'undefined' ? 0 : window.innerHeight);
  const pageH = $derived(Math.max(0, Math.min(heightPx, viewportH || heightPx) - 2 * PAD));
  const slot = $derived(Math.max(40, Math.floor(pageH / PHONE_GRID_ROWS)));

  // ── Edit mode + drag state ──
  let editing = $state(false);
  let dragId = $state<string | null>(null);
  /** Where the dragged widget currently hovers (the drop the preview is computed for). */
  let dragTarget = $state<{ page: number; row: number; col: number } | null>(null);
  /** Pointer position (panel-relative css px) for the ghost. */
  let dragX = $state(0);
  let dragY = $state(0);
  /** Offset from the widget's top-left to the finger, so the ghost doesn't jump on pickup. */
  let grabDx = 0;
  let grabDy = 0;
  /** Which slot of the widget the finger holds (row/col within the span): the drop target keeps
   *  THAT slot under the finger, so a 2×2 grabbed at its bottom-right lands where the finger is
   *  minus one row and one column — consistent wherever you grab it. */
  let grabSlotRow = 0;
  let grabSlotCol = 0;

  // The layout on screen: the committed config, or — mid-drag — the preview of the hovered drop.
  const shownConfig = $derived(
    dragId && dragTarget ? movePhoneWidget(config, dragId, dragTarget.page, dragTarget.row, dragTarget.col) : config,
  );
  const packed = $derived(packPhone(shownConfig));
  const cols = $derived(packed.cols);
  $effect(() => {
    const w = slot * cols + 2 * PAD;
    if (w !== widthPx) widthPx = w;
  });

  const pages = $derived(Array.from({ length: PHONE_GRID_PAGES }, (_, p) => p));
  const onPage = (p: number) => packed.placements.filter((x) => x.page === p);
  const dragPlacement = $derived(dragId ? packed.placements.find((p) => p.id === dragId) ?? null : null);

  // Current page for the dots: derived from the scroll position (snap → whole pages).
  let rootEl = $state<HTMLDivElement>();
  let scroller = $state<HTMLDivElement>();
  let page = $state(0);
  function onScroll() {
    if (!scroller || pageH <= 0) return;
    page = Math.round(scroller.scrollTop / pageH);
  }
  function flipTo(p: number) {
    if (!scroller) return;
    const target = Math.max(0, Math.min(PHONE_GRID_PAGES - 1, p));
    scroller.scrollTo({ top: target * pageH, behavior: 'smooth' });
  }

  // ── Long-press to arm, drag to move ──
  let pressTimer: ReturnType<typeof setTimeout> | null = null;
  let pressId: string | null = null;
  let pressX = 0;
  let pressY = 0;
  /** Edit mode: a touch that moved before the hold elapsed is a page flick, tracked here. */
  let flickY: number | null = null;
  /** The current press was relayed from the swapped-in mini map (a layer over a tile). */
  let relayedPress = false;
  let edgeTimer: ReturnType<typeof setTimeout> | null = null;
  let edgeDir = 0;

  function clearPress() {
    if (pressTimer) clearTimeout(pressTimer);
    pressTimer = null;
    pressId = null;
  }
  // Published on the root for layers outside the panel: the swapped-in mini map goes touch-free
  // while a widget is being rearranged (+page CSS `html.phone-editing`).
  $effect(() => {
    document.documentElement.classList.toggle('phone-editing', editing);
    return () => document.documentElement.classList.remove('phone-editing');
  });
  function clearEdge() {
    if (edgeTimer) clearTimeout(edgeTimer);
    edgeTimer = null;
    edgeDir = 0;
  }

  /** Panel-relative pointer position → grid cell (page from the scroll offset). */
  function cellAt(clientX: number, clientY: number): { page: number; row: number; col: number } | null {
    if (!scroller || slot <= 0) return null;
    const r = scroller.getBoundingClientRect();
    const x = clientX - r.left;
    const y = clientY - r.top + scroller.scrollTop;
    const p = Math.max(0, Math.min(PHONE_GRID_PAGES - 1, Math.floor(y / pageH)));
    const row = Math.max(0, Math.min(PHONE_GRID_ROWS - 1, Math.floor((y - p * pageH) / slot)));
    const col = Math.max(0, Math.min(cols - 1, Math.floor(x / slot)));
    return { page: p, row, col };
  }

  /** The grid cell under a viewport point, looking THROUGH layers above the panel (the swapped-in
   *  mini map sits over its tile) — the tile's own pointerdown never fires then. */
  function cellElAt(x: number, y: number): HTMLElement | null {
    if (!rootEl) return null;
    return (
      (document.elementsFromPoint(x, y).find((el) => el.classList.contains('cell') && rootEl!.contains(el)) as
        | HTMLElement
        | undefined) ?? null
    );
  }

  function onCellPointerDown(e: PointerEvent, id: string) {
    if (e.button !== 0) return;
    // A second finger (pinch on the mini map) is never a press.
    if (!e.isPrimary) {
      clearPress();
      return;
    }
    pressX = e.clientX;
    pressY = e.clientY;
    if (editing) {
      // Already editing: hold DRAG_HOLD_MS to pick the widget up; moving earlier is a page flick
      // (the resize button stops propagation before we get here).
      pressId = id;
      pressTimer = setTimeout(() => {
        pressTimer = null;
        pressId = null;
        startDrag(e, id);
      }, DRAG_HOLD_MS);
      return;
    }
    // Arm the long-press; movement or release before it fires cancels it (scrolling the pages).
    pressId = id;
    pressTimer = setTimeout(() => {
      pressTimer = null;
      editing = true;
      try {
        navigator.vibrate?.(30);
      } catch {
        /* no haptics */
      }
      startDrag(e, id);
    }, LONG_PRESS_MS);
  }

  function startDrag(e: PointerEvent, id: string) {
    const cell = (e.target as HTMLElement).closest<HTMLElement>('.cell') ?? cellElAt(e.clientX, e.clientY);
    const r = cell?.getBoundingClientRect();
    grabDx = r ? e.clientX - r.left : 0;
    grabDy = r ? e.clientY - r.top : 0;
    grabSlotCol = slot > 0 ? Math.max(0, Math.floor(grabDx / slot)) : 0;
    grabSlotRow = slot > 0 ? Math.max(0, Math.floor(grabDy / slot)) : 0;
    dragId = id;
    dragTarget = null;
    console.log('[phoneGrid] drag start', id, Math.round(e.clientX), Math.round(e.clientY));
    updateDrag(e.clientX, e.clientY);
    // No pointer capture on purpose: the widget under the finger re-renders while the preview
    // re-packs, and a captured target that leaves the DOM takes the pointer stream with it. The
    // window listeners see every move/up anyway.
  }

  function updateDrag(clientX: number, clientY: number) {
    if (!rootEl || !scroller) return;
    const rr = rootEl.getBoundingClientRect();
    dragX = clientX - rr.left - grabDx;
    dragY = clientY - rr.top - grabDy;
    // The slot under the FINGER minus the slot the finger grabbed within the widget.
    const under = cellAt(clientX, clientY);
    const c = under
      ? { page: under.page, row: Math.max(0, under.row - grabSlotRow), col: Math.max(0, under.col - grabSlotCol) }
      : null;
    if (c && (!dragTarget || c.page !== dragTarget.page || c.row !== dragTarget.row || c.col !== dragTarget.col)) {
      dragTarget = c;
    }
    // Edge hover flips the page.
    const sr = scroller.getBoundingClientRect();
    const dir = clientY < sr.top + EDGE_FLIP_PX ? -1 : clientY > sr.bottom - EDGE_FLIP_PX ? 1 : 0;
    if (dir !== edgeDir) {
      clearEdge();
      edgeDir = dir;
      if (dir !== 0) {
        edgeTimer = setTimeout(() => {
          flipTo(page + dir);
          edgeTimer = null;
          edgeDir = 0;
        }, EDGE_FLIP_MS);
      }
    }
  }

  function endDrag(commit: boolean) {
    clearEdge();
    console.log('[phoneGrid] drop', dragId, dragTarget, commit ? 'commit' : 'cancel');
    if (dragId && dragTarget && commit) onmove?.(dragId, dragTarget.page, dragTarget.row, dragTarget.col);
    dragId = null;
    dragTarget = null;
  }

  $effect(() => {
    const onMove = (e: PointerEvent) => {
      if (dragId) {
        updateDrag(e.clientX, e.clientY);
        return;
      }
      if (flickY != null) return; // page flick in progress — decided on release
      if (pressId && Math.hypot(e.clientX - pressX, e.clientY - pressY) > DRAG_SLOP_PX) {
        // Moved before the hold elapsed: not a pickup. In edit mode the cells block native
        // scrolling (touch-action none), so the flick is ours to turn into a page change — and so
        // is a swipe that started on the swapped-in mini map (a layer outside the scroller, which
        // never sees it; the video tile scrolls natively, the map tile must match).
        const relayed = relayedPress;
        clearPress();
        if (editing || relayed) flickY = pressY;
      }
    };
    const onUp = (e: PointerEvent) => {
      clearPress();
      relayedPress = false;
      if (dragId) endDrag(true);
      if (flickY != null) {
        const dy = e.clientY - flickY;
        flickY = null;
        if (dy < -FLICK_PX) flipTo(page + 1);
        else if (dy > FLICK_PX) flipTo(page - 1);
      }
    };
    const onCancel = () => {
      clearPress();
      relayedPress = false;
      flickY = null;
      if (dragId) endDrag(false);
    };
    // Tap outside the panel → leave edit mode (capture, so a surface that stops propagation counts).
    // A touch on the swapped-in mini map (a layer OVER one of our tiles, outside the panel's DOM)
    // is relayed to that tile as a press, so a long-press there still enters edit mode; in edit
    // mode the map layer is touch-free (html.phone-editing) and the tile gets the events itself.
    const onDown = (e: PointerEvent) => {
      const target = e.target as HTMLElement | null;
      if (!editing && e.pointerType !== 'mouse' && target?.closest('.layer-map.in-frame')) {
        const cell = cellElAt(e.clientX, e.clientY);
        if (cell?.dataset.id) {
          relayedPress = true;
          onCellPointerDown(e, cell.dataset.id);
        }
        return;
      }
      if (editing && rootEl && !rootEl.contains(target as Node)) editing = false;
    };
    // All in the CAPTURE phase: the mini map swallows pointer events on its container (zoom-only
    // map), which would otherwise hide a release from us — the relayed press then ran into its
    // long-press timer and a short tap on a waypoint popup opened edit mode.
    window.addEventListener('pointermove', onMove, true);
    window.addEventListener('pointerup', onUp, true);
    window.addEventListener('pointercancel', onCancel, true);
    window.addEventListener('pointerdown', onDown, true);
    return () => {
      window.removeEventListener('pointermove', onMove, true);
      window.removeEventListener('pointerup', onUp, true);
      window.removeEventListener('pointercancel', onCancel, true);
      window.removeEventListener('pointerdown', onDown, true);
      clearPress();
      clearEdge();
    };
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="pwp"
  class:editing
  bind:this={rootEl}
  bind:clientHeight={heightPx}
  style="--slot:{slot}px; --pad:{PAD}px; --cols:{cols}; --page-h:{pageH}px"
  oncontextmenu={(e) => e.preventDefault()}
>
  <!-- The glass is its OWN layer under the tiles, and it is the one that opts into the native
       sink's hole (data-nv-clip). A clip-path also removes an element from hit-testing inside the
       cut, so clipping the panel root would let touches on the video tile fall through to the map
       below (edit mode died, double-tap never arrived); the tiles above stay unclipped. -->
  <div class="glass" data-nv-clip></div>
  <div class="pages" class:dragging={!!dragId} bind:this={scroller} onscroll={onScroll}>
    {#each pages as p (p)}
      <div class="page">
        {#each onPage(p) as pl (pl.id)}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div
            class="cell"
            data-id={pl.id}
            class:lifted={pl.id === dragId}
            style="left:{pl.col * slot}px; top:{pl.row * slot}px; width:{pl.w * slot}px; height:{pl.h * slot}px;"
            onpointerdown={(e) => onCellPointerDown(e, pl.id)}
          >
            <WidgetRenderer
              id={pl.id}
              {telem}
              {interfaceSettings}
              sizePx={pl.h * slot}
              wPx={pl.w * slot}
              hPx={pl.h * slot}
              {editing}
            />
            {#if editing}
              <div class="edit-frame"></div>
              <button
                class="resize-btn"
                type="button"
                title={$t('widgets.resize')}
                aria-label={$t('widgets.resize')}
                onpointerdown={(e) => e.stopPropagation()}
                onclick={() => onresize?.(pl.id)}
              >
                <svg viewBox="0 0 24 24" aria-hidden="true">
                  <rect x="3" y="3" width="18" height="18" rx="1.5" />
                  <rect x="6" y="11" width="9" height="7" rx="1" />
                </svg>
              </button>
            {/if}
          </div>
        {/each}
      </div>
    {/each}
  </div>

  <!-- Ghost of the dragged widget under the finger (the real cell shows its preview slot). -->
  {#if dragId && dragPlacement}
    <div
      class="ghost"
      style="left:{dragX}px; top:{dragY}px; width:{dragPlacement.w * slot}px; height:{dragPlacement.h * slot}px;"
    >
      <WidgetRenderer
        id={dragId}
        {telem}
        {interfaceSettings}
        sizePx={dragPlacement.h * slot}
        wPx={dragPlacement.w * slot}
        hPx={dragPlacement.h * slot}
        ghost
      />
    </div>
  {/if}

  {#if PHONE_GRID_PAGES > 1}
    <div class="dots" aria-hidden="true">
      {#each pages as p (p)}
        <span class="dot" class:on={p === page} title={$t('widgets.phonePage', { values: { n: p + 1 } })}></span>
      {/each}
    </div>
  {/if}
</div>

<style>
  .pwp {
    position: relative;
    height: 100%;
    box-sizing: border-box;
    padding: var(--pad) calc(var(--pad) + var(--safe-right, 0px)) var(--pad) var(--pad);
    pointer-events: auto;
    overflow: hidden;
    user-select: none;
    -webkit-user-select: none;
    -webkit-touch-callout: none;
  }
  .glass {
    position: absolute;
    inset: 0;
    z-index: 0;
    background: rgba(30, 30, 30, 0.75);
    backdrop-filter: blur(10px);
    -webkit-backdrop-filter: blur(10px);
    border-left: 1px solid rgba(255, 255, 255, 0.08);
    pointer-events: none;
  }
  .pwp.editing .glass {
    border-left-color: rgba(55, 168, 219, 0.6);
  }

  /* Vertical page scroller with snap — one page = the usable height. While a widget is dragged
     the finger must not scroll the pages (the edge-hover flips them instead). */
  .pages {
    position: relative;
    z-index: 1;
    height: 100%;
    overflow-y: auto;
    overflow-x: hidden;
    scroll-snap-type: y mandatory;
    scrollbar-width: none;
    touch-action: pan-y;
  }
  .pages.dragging {
    overflow-y: hidden;
    touch-action: none;
  }
  .pages::-webkit-scrollbar {
    display: none;
  }
  .page {
    position: relative;
    height: var(--page-h);
    width: calc(var(--slot) * var(--cols));
    scroll-snap-align: start;
  }

  .cell {
    position: absolute;
    box-sizing: border-box;
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
    transition: left 0.15s ease, top 0.15s ease;
  }
  .pwp.editing .cell {
    touch-action: none; /* the finger drags the widget, not the page */
  }
  .cell.lifted {
    opacity: 0.25; /* the ghost shows it; the faded cell marks the preview slot */
  }

  /* The panel IS the glass (D6): strip the widgets' own card surface so they sit edge to edge
     without seams or rounded corners. The round instruments keep their instrument disc. */
  .cell :global(.widget-card) {
    background: transparent;
    backdrop-filter: none;
    -webkit-backdrop-filter: none;
    border-color: transparent;
    border-radius: 0;
    box-shadow: none;
  }

  .edit-frame {
    position: absolute;
    inset: 2px;
    border: 1px dashed rgba(55, 168, 219, 0.7);
    pointer-events: none;
  }
  .resize-btn {
    position: absolute;
    right: 4px;
    bottom: 4px;
    width: 26px;
    height: 26px;
    padding: 3px;
    z-index: 30;
    display: flex;
    align-items: center;
    justify-content: center;
    border: 1px solid rgba(55, 168, 219, 0.6);
    border-radius: 6px;
    background: rgba(30, 30, 30, 0.85);
    color: #37a8db;
    cursor: pointer;
    touch-action: manipulation;
  }
  .resize-btn svg {
    width: 100%;
    height: 100%;
    fill: none;
    stroke: currentColor;
    stroke-width: 2;
    stroke-linejoin: round;
  }

  .ghost {
    position: absolute;
    z-index: 40;
    pointer-events: none;
    opacity: 0.9;
    border: 1px solid rgba(55, 168, 219, 0.8);
    background: rgba(30, 30, 30, 0.6);
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.45);
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
  }
  .ghost :global(.widget-card) {
    background: transparent;
    backdrop-filter: none;
    -webkit-backdrop-filter: none;
    border-color: transparent;
    border-radius: 0;
    box-shadow: none;
  }

  .dots {
    position: absolute;
    left: 0;
    right: var(--safe-right, 0px);
    bottom: 2px;
    display: flex;
    justify-content: center;
    gap: 6px;
    pointer-events: none;
  }
  .dot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.25);
  }
  .dot.on {
    background: #37a8db;
  }
</style>
