<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- PhoneBottomBar — three fixed widget slots on the bottom edge of the phone's map area
     (Dev-Docs active/PHONE_BOTTOM_WIDGETS.md). Left / right: square, 20 % of the map viewport
     height; centre: 30 %, square or 2:1 wide (the wide widget family only). No surface of its own —
     the widgets show their natural card glass, an empty slot takes no space (B2) — and the triplet
     is centred on the CENTRE slot, whose width collapses to zero while it is empty outside edit
     mode, so two side widgets close up around the middle line (B3).

     The tiles must fit between the arming pill — or the open nav rail, whichever reaches further
     right — plus clearance, and the map corner controls on either side of the middle line; when they
     don't, all three scale down by the missing factor (B4). Of the chip row only the arming pill
     counts (its widest state seen, so arming never resizes the tiles); the sensor
     chip stacks above the tiles while a slot is filled (`html.phone-bar`, B5), as do the other
     bottom-band elements (`--phone-bar-lift`).

     Edit mode + drag are shared with the column (stores/phoneEdit.svelte.ts): a long-press on a
     tile arms edit mode and picks the widget up; in edit mode every slot shows at its real size
     with a dashed frame, a hold picks a widget up, the drop lands on a slot (a filled one swaps) or
     in a column cell. The docked video window is not considered — it lies above the tiles (B8). -->
<script lang="ts">
  import { t } from 'svelte-i18n';
  import { WIDGET_MAP } from '$lib/config/widgetRegistry';
  import {
    PHONE_BOTTOM_CENTRE_FRAC,
    PHONE_BOTTOM_CLEARANCE,
    PHONE_BOTTOM_GAP,
    PHONE_BOTTOM_SIDE_FRAC,
  } from '$lib/config/phoneGrid';
  import {
    EMPTY_BOTTOM,
    PHONE_BOTTOM_SLOTS,
    movePhoneWidget,
    movePhoneWidgetToBottom,
    type PhoneBottomSlot,
    type PhoneWidgetsConfig,
  } from '$lib/controllers/phoneWidgetController';
  import { phoneEdit, phoneHitTests, bottomSlotAt, sameTarget, type PhoneDragTarget } from '$lib/stores/phoneEdit.svelte';
  import WidgetRenderer from '$lib/components/WidgetRenderer.svelte';
  import type { TelemetryData } from '$lib/stores/telemetry';
  import type { InterfaceSettings } from '$lib/stores/settings';

  let {
    config,
    telem,
    interfaceSettings,
    frameShift = 0,
    leftReserve = 0,
    onmovetobottom,
    onmovetocolumn,
    ontogglewide,
    barH = $bindable(0),
  }: {
    config: PhoneWidgetsConfig;
    telem: TelemetryData;
    interfaceSettings: InterfaceSettings;
    /** `--phone-shift`: how far the widget column has slid out (the map frame grows by it). */
    frameShift?: number;
    /** Right edge (css px) of a left-edge overlay the tiles must clear besides the arming pill — the
     *  open nav rail (Marc, 2026-09-12: its lower buttons reach into the tiles' band). 0 = none. */
    leftReserve?: number;
    /** Edit mode: the user dropped a widget on a slot. */
    onmovetobottom?: (id: string, slot: PhoneBottomSlot) => void;
    /** Edit mode: the user dropped a slotted widget in a column cell. */
    onmovetocolumn?: (id: string, page: number, row: number, col: number) => void;
    /** Edit mode: the centre tile's wide ↔ square button. */
    ontogglewide?: () => void;
    /** OUT: the tallest FILLED tile in css px (0 when every slot is empty) — the map's bottom inset. */
    barH?: number;
  } = $props();

  const LONG_PRESS_MS = 500;
  const DRAG_HOLD_MS = 250;
  const DRAG_SLOP_PX = 8;
  /** The map corner controls: 8 px + 38 px column + 8 px from the frame's right edge. */
  const CORNER_CONTROLS_PX = 54;

  // ── Frame geometry ──

  let innerW = $state(0);
  let innerH = $state(0);
  const frameW = $derived(innerW + frameShift);
  const cx = $derived(frameW / 2);

  // The arming pill's right edge (viewport px, published by PhoneBottomChips as --phone-arming-w).
  // The widest value seen in this session is what counts: DISARMED is wider than ARMED, and the
  // tiles must not resize at arming.
  let armingRight = $state(0);
  $effect(() => {
    const root = document.documentElement;
    const read = () => {
      const v = parseFloat(root.style.getPropertyValue('--phone-arming-w')) || 0;
      if (v > armingRight) armingRight = v;
    };
    read();
    const obs = new MutationObserver(read);
    obs.observe(root, { attributes: true, attributeFilter: ['style'] });
    return () => obs.disconnect();
  });

  // ── Edit mode + drag (shared) ──
  const editing = $derived(phoneEdit.editing);
  const drag = $derived(phoneEdit.drag);
  const ownDrag = $derived(phoneEdit.drag?.owner === 'bottom');

  // The slots on screen: the committed config, or — mid-drag — the preview of the hovered drop
  // (the same controller calls the commit makes).
  const shownBottom = $derived.by(() => {
    if (!drag?.target) return config.bottom ?? EMPTY_BOTTOM;
    const next =
      drag.target.kind === 'bottom'
        ? movePhoneWidgetToBottom(config, drag.id, drag.target.slot)
        : movePhoneWidget(config, drag.id, drag.target.page, drag.target.row, drag.target.col);
    return (next ?? config).bottom ?? EMPTY_BOTTOM;
  });

  // ── Sizes (B1, B4) ──
  const sideNat = $derived(innerH * PHONE_BOTTOM_SIDE_FRAC);
  const centreNat = $derived(innerH * PHONE_BOTTOM_CENTRE_FRAC);
  const centreWide = $derived(!!shownBottom.centreWide && isWideFamily(shownBottom.centre));
  /** A slot takes room when filled, or always in edit mode (B2/B3). */
  const present = $derived({
    left: editing || !!shownBottom.left,
    centre: editing || !!shownBottom.centre,
    right: editing || !!shownBottom.right,
  });
  /** Natural extent from the middle line to the outer edge of the tiles on one side. */
  function extent(side: boolean): number {
    const half = present.centre ? (centreWide ? centreNat : centreNat / 2) : 0;
    const gap = present.centre ? PHONE_BOTTOM_GAP : PHONE_BOTTOM_GAP / 2;
    return half + (side ? gap + sideNat : 0);
  }
  const scale = $derived.by(() => {
    const extL = extent(present.left);
    const extR = extent(present.right);
    const freeL = cx - (Math.max(armingRight, leftReserve) + PHONE_BOTTOM_CLEARANCE);
    const freeR = frameW - CORNER_CONTROLS_PX - cx;
    let k = 1;
    if (extL > 0) k = Math.min(k, freeL / extL);
    if (extR > 0) k = Math.min(k, freeR / extR);
    return Math.max(0, k);
  });
  const sideH = $derived(Math.round(sideNat * scale));
  const centreH = $derived(Math.round(centreNat * scale));
  const centreW = $derived(centreWide ? 2 * centreH : centreH);

  /** Tile boxes in the inner box's coordinates (x from the left, bottom-aligned). */
  const tiles = $derived.by(() => {
    const gap = present.centre ? PHONE_BOTTOM_GAP : PHONE_BOTTOM_GAP / 2;
    const halfC = present.centre ? centreW / 2 : 0;
    return {
      left: { x: cx - halfC - gap - sideH, w: sideH, h: sideH },
      centre: { x: cx - halfC, w: present.centre ? centreW : 0, h: centreH },
      right: { x: cx + halfC + gap, w: sideH, h: sideH },
    };
  });

  const filledH = $derived(
    Math.max(shownBottom.left ? sideH : 0, shownBottom.right ? sideH : 0, shownBottom.centre ? centreH : 0),
  );
  const anyFilled = $derived(filledH > 0);
  $effect(() => {
    if (barH !== filledH) barH = filledH;
  });
  // Published on the root: the sensor chip, the error bar and the resume banner move up by the
  // lift while a tile is on the bottom edge (or the slots show in edit mode). The Debug button and
  // the Leaflet credit stay on the bottom edge — a tile may cover the credit (Marc: no priority).
  $effect(() => {
    const lift = anyFilled || editing ? Math.max(filledH, editing ? Math.max(sideH, centreH) : 0) + PHONE_BOTTOM_GAP : 0;
    const root = document.documentElement;
    root.classList.toggle('phone-bar', lift > 0);
    root.style.setProperty('--phone-bar-lift', `${lift}px`);
    return () => {
      root.classList.remove('phone-bar');
      root.style.removeProperty('--phone-bar-lift');
    };
  });

  function isWideFamily(id: string | null): boolean {
    return !!id && WIDGET_MAP.get(id)?.shape === 'wide';
  }

  // ── Long-press to arm, hold to drag ──
  let pressTimer: ReturnType<typeof setTimeout> | null = null;
  let pressX = 0;
  let pressY = 0;
  let grabDx = 0;
  let grabDy = 0;

  function clearPress() {
    if (pressTimer) clearTimeout(pressTimer);
    pressTimer = null;
  }

  function onTilePointerDown(e: PointerEvent, id: string) {
    if (e.button !== 0 || !e.isPrimary) return;
    pressX = e.clientX;
    pressY = e.clientY;
    clearPress();
    pressTimer = setTimeout(() => {
      pressTimer = null;
      if (!phoneEdit.editing) {
        phoneEdit.editing = true;
        try {
          navigator.vibrate?.(30);
        } catch {
          /* no haptics */
        }
      }
      startDrag(e, id);
    }, phoneEdit.editing ? DRAG_HOLD_MS : LONG_PRESS_MS);
  }

  function startDrag(e: PointerEvent, id: string) {
    const tile = (e.target as HTMLElement).closest<HTMLElement>('.tile');
    const r = tile?.getBoundingClientRect();
    grabDx = r ? e.clientX - r.left : 0;
    grabDy = r ? e.clientY - r.top : 0;
    phoneEdit.drag = {
      id,
      owner: 'bottom',
      target: null,
      x: e.clientX - grabDx,
      y: e.clientY - grabDy,
      w: r?.width ?? sideH,
      h: r?.height ?? sideH,
    };
    console.log('[phoneBar] drag start', id, Math.round(e.clientX), Math.round(e.clientY));
    updateDrag(e.clientX, e.clientY);
  }

  function updateDrag(clientX: number, clientY: number) {
    const d = phoneEdit.drag;
    if (!d) return;
    d.x = clientX - grabDx;
    d.y = clientY - grabDy;
    const slot = bottomSlotAt(clientX, clientY);
    let target: PhoneDragTarget | null = slot ? { kind: 'bottom', slot } : null;
    if (!target) {
      const cell = phoneHitTests.column?.(clientX, clientY);
      if (cell) target = { kind: 'column', ...cell };
    }
    // Over nothing: keep the last target (a finger just past a slot's edge still means that slot).
    if (target && !sameTarget(target, d.target)) d.target = target;
  }

  function endDrag(commit: boolean) {
    const d = phoneEdit.drag;
    console.log('[phoneBar] drop', d?.id, $state.snapshot(d?.target), commit ? 'commit' : 'cancel');
    if (d?.target && commit) {
      if (d.target.kind === 'bottom') onmovetobottom?.(d.id, d.target.slot);
      else onmovetocolumn?.(d.id, d.target.page, d.target.row, d.target.col);
    }
    phoneEdit.drag = null;
  }

  $effect(() => {
    const onMove = (e: PointerEvent) => {
      if (phoneEdit.drag) {
        if (ownDrag) updateDrag(e.clientX, e.clientY);
        return;
      }
      if (pressTimer && Math.hypot(e.clientX - pressX, e.clientY - pressY) > DRAG_SLOP_PX) clearPress();
    };
    const onUp = () => {
      clearPress();
      if (phoneEdit.drag && ownDrag) endDrag(true);
    };
    const onCancel = () => {
      clearPress();
      if (phoneEdit.drag && ownDrag) endDrag(false);
    };
    window.addEventListener('pointermove', onMove, true);
    window.addEventListener('pointerup', onUp, true);
    window.addEventListener('pointercancel', onCancel, true);
    return () => {
      window.removeEventListener('pointermove', onMove, true);
      window.removeEventListener('pointerup', onUp, true);
      window.removeEventListener('pointercancel', onCancel, true);
      clearPress();
    };
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="pbb" class:editing data-phone-edit-zone oncontextmenu={(e) => e.preventDefault()}>
  <!-- The inner box is the map viewport minus the safe insets (B1's reference height). -->
  <div class="inner" bind:clientWidth={innerW} bind:clientHeight={innerH}>
    {#each PHONE_BOTTOM_SLOTS as slot (slot)}
      {@const id = shownBottom[slot]}
      {@const box = tiles[slot]}
      {#if id || editing}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="tile"
          class:empty={!id}
          class:lifted={id != null && id === drag?.id}
          class:hover={drag?.target?.kind === 'bottom' && drag.target.slot === slot}
          data-bottom-slot={slot}
          style="left:{box.x}px; width:{box.w}px; height:{box.h}px;"
          onpointerdown={(e) => id && onTilePointerDown(e, id)}
        >
          {#if id}
            <WidgetRenderer {id} {telem} {interfaceSettings} sizePx={box.h} wPx={box.w} hPx={box.h} {editing} />
            {#if editing && slot === 'centre' && isWideFamily(id)}
              <button
                class="wide-btn"
                type="button"
                title={$t('widgets.resize')}
                aria-label={$t('widgets.resize')}
                onpointerdown={(e) => e.stopPropagation()}
                onclick={() => ontogglewide?.()}
              >
                <svg viewBox="0 0 24 24" aria-hidden="true">
                  <rect x="3" y="3" width="18" height="18" rx="1.5" />
                  <rect x="6" y="11" width="9" height="7" rx="1" />
                </svg>
              </button>
            {/if}
          {/if}
          {#if editing}
            <div class="edit-frame"></div>
          {/if}
        </div>
      {/if}
    {/each}
  </div>
</div>

<style>
  /* The bar spans the map area (left of the widget column); nothing but the tiles takes touches.
     `:global(.app) >` outranks +page's `.app > * { pointer-events: auto }` (same specificity as a
     plain scoped .pbb, and the page's rule came later — the bar swallowed every map touch). */
  :global(.app) > .pbb {
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    right: var(--phone-panel-w, 0px);
    z-index: 50; /* over the map, under the docked video (60+), the chips (110) and the panels */
    pointer-events: none;
    box-sizing: border-box;
    padding: var(--safe-top, 0px) 0 var(--safe-bottom, 0px) var(--safe-left, 0px);
    user-select: none;
    -webkit-user-select: none;
    -webkit-touch-callout: none;
  }
  .inner {
    position: relative;
    width: 100%;
    height: 100%;
  }
  .tile {
    position: absolute;
    bottom: 8px; /* the chip row's baseline */
    box-sizing: border-box;
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
    pointer-events: auto;
    transition: left 0.15s ease, width 0.15s ease;
  }
  .pbb.editing .tile {
    touch-action: none; /* the finger drags the widget, not the map */
  }
  .tile.empty {
    pointer-events: auto; /* a drop target */
  }
  .tile.lifted {
    opacity: 0.25; /* the ghost shows it; the faded tile marks the preview slot */
  }
  .edit-frame {
    position: absolute;
    inset: 2px;
    border: 1px dashed rgba(55, 168, 219, 0.7);
    pointer-events: none;
  }
  .tile.hover .edit-frame {
    border-style: solid;
    border-color: rgba(55, 168, 219, 0.95);
    background: rgba(55, 168, 219, 0.12);
  }
  .wide-btn {
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
  .wide-btn svg {
    width: 100%;
    height: 100%;
    fill: none;
    stroke: currentColor;
    stroke-width: 2;
    stroke-linejoin: round;
  }
</style>
