<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- WidgetPanel — a panel strip (horizontal or vertical) that holds widgets with drag-and-drop reordering -->
<script lang="ts">
  import { t } from 'svelte-i18n';
  import type { TelemetryData } from "$lib/stores/telemetry";
  import { WIDGET_MAP, LARGE_BASE_VMIN, SMALL_BASE_VMIN, MIN_SCALE, type WidgetClass } from "$lib/config/widgetRegistry";
  import AHI from "./widgets/AHI.svelte";
  import SpeedWidget from "./widgets/SpeedWidget.svelte";
  import AltWidget from "./widgets/AltWidget.svelte";
  import BatteryWidget from "./widgets/BatteryWidget.svelte";
  import GpsWidget from "./widgets/GpsWidget.svelte";
  import RcLinkWidget from "./widgets/RcLinkWidget.svelte";
  import CompassWidget from "./widgets/CompassWidget.svelte";
  import HomeWidget from "./widgets/HomeWidget.svelte";
  import FlightModeWidget from "./widgets/FlightModeWidget.svelte";
  import RawTelemetryWidget from "./widgets/RawTelemetryWidget.svelte";
  import LiveAglWidget from "./widgets/LiveAglWidget.svelte";
  import TerrainRadarWidget from "./widgets/TerrainRadarWidget.svelte";
  import VideoWidget from "./widgets/VideoWidget.svelte";
  import type { InterfaceSettings } from "$lib/stores/settings";

  let {
    widgetIds = [],
    orientation = 'horizontal',
    availableVmin = 80,
    maxWidgetVmin = Infinity,
    pxPerVmin = 1,
    telem,
    interfaceSettings = { speedUnit: 'kmh', altitudeUnit: 'm', distanceUnit: 'metric', verticalSpeedUnit: 'ms', temperatureUnit: 'c' },
    editing = false,
    onreorder,
    onreceive,
    panelId,
  }: {
    widgetIds: string[];
    orientation: 'horizontal' | 'vertical';
    availableVmin: number;
    maxWidgetVmin?: number;
    pxPerVmin?: number;
    telem: TelemetryData;
    interfaceSettings?: InterfaceSettings;
    editing: boolean;
    onreorder: (panelId: string, widgetIds: string[]) => void;
    onreceive: (targetPanel: string, widgetId: string, index: number) => void;
    panelId: string;
  } = $props();

  type DragPayload = { panelId: string; widgetId: string; sourceIdx: number };

  let panelEl: HTMLDivElement | null = null;
  let activeDragPayload: DragPayload | null = null;
  const DND_DEBUG = import.meta.env.DEV;
  let ghostX = $state(0);
  let ghostY = $state(0);
  let ghostW = $state(140);
  let ghostH = $state(140);
  let ghostWidgetId = $state<string | null>(null);
  let ghostWidgetLabel = $derived.by(() => {
    if (!ghostWidgetId) return '';
    const def = WIDGET_MAP.get(ghostWidgetId);
    return def ? $t(def.labelKey) : ghostWidgetId;
  });

  function setGlobalDragPayload(payload: DragPayload | null) {
    (window as any).__KITE_WIDGET_DND_PAYLOAD = payload;
  }

  function getGlobalDragPayload(): DragPayload | null {
    return ((window as any).__KITE_WIDGET_DND_PAYLOAD ?? null) as DragPayload | null;
  }

  function clearGhost() {
    ghostWidgetId = null;
  }

  // WebView-safe DnD: capture dragover/drop on panel so drop targets stay valid (no 🚫 cursor).
  $effect(() => {
    if (!panelEl) return;
    const onEnter = (e: DragEvent) => handlePanelDragOver(e);
    const onOver = (e: DragEvent) => handlePanelDragOver(e);
    const onDropCapture = (e: DragEvent) => handlePanelDrop(e);
    panelEl.addEventListener('dragenter', onEnter, true);
    panelEl.addEventListener('dragover', onOver, true);
    panelEl.addEventListener('drop', onDropCapture, true);
    return () => {
      panelEl?.removeEventListener('dragenter', onEnter, true);
      panelEl?.removeEventListener('dragover', onOver, true);
      panelEl?.removeEventListener('drop', onDropCapture, true);
    };
  });

  // Mouse-based fallback DnD (independent from HTML5 drag events).
  $effect(() => {
    const onWindowMouseMove = (e: MouseEvent) => {
      if (!editing) return;
      const payload = getGlobalDragPayload();
      if (!payload) {
        // Another panel may have consumed the drop; clear stale local drag visuals.
        if (dragIdx !== -1 || insertIdx !== -1 || externalDragOver) {
          dragIdx = -1;
          insertIdx = -1;
          externalDragOver = false;
          activeDragPayload = null;
          clearGhost();
        }
        return;
      }

      // Only source panel renders the visual drag ghost.
      if (payload.panelId === panelId) {
        ghostX = e.clientX;
        ghostY = e.clientY;
      }

      const inside = isPointInsidePanel(e.clientX, e.clientY);
      externalDragOver = inside;
      insertIdx = inside ? computeInsertIdxFromPoint(e.clientX, e.clientY) : -1;
      if (DND_DEBUG) console.log('[WIDGET-DND] mousemove', { panelId, inside, insertIdx, payload });
    };

    const onWindowMouseUp = (e: MouseEvent) => {
      if (!editing) return;
      const payload = getGlobalDragPayload();
      if (!payload) return;
      const inside = isPointInsidePanel(e.clientX, e.clientY);
      if (inside) {
        const dropAt = computeInsertIdxFromPoint(e.clientX, e.clientY);
        if (DND_DEBUG) console.log('[WIDGET-DND] mouseup drop', { panelId, dropAt, payload });
        executeDrop(payload, dropAt);
        setGlobalDragPayload(null);
        dragIdx = -1;
        insertIdx = -1;
        externalDragOver = false;
        activeDragPayload = null;
        clearGhost();
        return;
      }

      // Important for cross-panel moves: if source panel sees mouseup first while cursor is
      // over another widget panel, do NOT clear global payload yet.
      if (payload.panelId === panelId) {
        const el = document.elementFromPoint(e.clientX, e.clientY) as HTMLElement | null;
        const overWidgetPanel = !!el?.closest('.widget-panel');
        if (!overWidgetPanel) {
          if (DND_DEBUG) console.log('[WIDGET-DND] mouseup cancel outside panels', { panelId, payload });
          setGlobalDragPayload(null);
          dragIdx = -1;
          insertIdx = -1;
          externalDragOver = false;
          activeDragPayload = null;
          clearGhost();
        }
      }
    };

    window.addEventListener('mousemove', onWindowMouseMove, true);
    window.addEventListener('mouseup', onWindowMouseUp, true);
    return () => {
      window.removeEventListener('mousemove', onWindowMouseMove, true);
      window.removeEventListener('mouseup', onWindowMouseUp, true);
    };
  });

  function isPointInsidePanel(x: number, y: number): boolean {
    if (!panelEl) return false;
    const r = panelEl.getBoundingClientRect();
    return x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;
  }

  function computeInsertIdxFromPoint(x: number, y: number): number {
    if (!panelEl) return widgetIds.length;
    const el = document.elementFromPoint(x, y) as HTMLElement | null;
    const slot = el?.closest('.widget-slot') as HTMLElement | null;
    if (!slot || !panelEl.contains(slot)) {
      return widgetIds.length;
    }
    const idxAttr = slot.getAttribute('data-slot-idx');
    const idx = idxAttr != null ? Number(idxAttr) : widgetIds.length;
    const rect = slot.getBoundingClientRect();
    const pastMid = orientation === 'horizontal'
      ? x > rect.left + rect.width / 2
      : y > rect.top + rect.height / 2;
    return pastMid ? idx + 1 : idx;
  }

  // Fallback path for WebView quirks where panel dragover/drop handlers are not dispatched.
  $effect(() => {
    const onWindowDragOver = (e: DragEvent) => {
      if (!editing) return;
      const payload = getGlobalDragPayload();
      if (!payload) return;
      e.preventDefault();
      if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';

      const inside = isPointInsidePanel(e.clientX, e.clientY);
      externalDragOver = inside;
      insertIdx = inside ? computeInsertIdxFromPoint(e.clientX, e.clientY) : -1;
      if (DND_DEBUG) console.log('[WIDGET-DND] window dragover', { panelId, inside, insertIdx, payload });
    };

    const onWindowDrop = (e: DragEvent) => {
      if (!editing) return;
      const payload = getGlobalDragPayload();
      if (!payload) return;
      if (!isPointInsidePanel(e.clientX, e.clientY)) return;
      e.preventDefault();
      const dropAt = computeInsertIdxFromPoint(e.clientX, e.clientY);
      if (DND_DEBUG) console.log('[WIDGET-DND] window drop', { panelId, dropAt, payload });
      executeDrop(payload, dropAt);
      setGlobalDragPayload(null);
      dragIdx = -1;
      insertIdx = -1;
      externalDragOver = false;
      activeDragPayload = null;
    };

    window.addEventListener('dragover', onWindowDragOver, true);
    window.addEventListener('drop', onWindowDrop, true);
    return () => {
      window.removeEventListener('dragover', onWindowDragOver, true);
      window.removeEventListener('drop', onWindowDrop, true);
    };
  });

  // Compute widget sizes based on available space
  function computeSizes(): { id: string; size: number; wclass: WidgetClass }[] {
    if (widgetIds.length === 0) return [];

    // Cross-axis determines the target large widget size (fill dock height/width).
    // Cap at LARGE_BASE_VMIN so widgets don't grow beyond their designed max.
    const effectiveLargeBase = Math.min(maxWidgetVmin, LARGE_BASE_VMIN);
    const smallRatio = SMALL_BASE_VMIN / LARGE_BASE_VMIN; // 0.6

    // Calculate total "units" of main-axis space (large=1, small=smallRatio,
    // wide=2:1 → 2 units in the horizontal dock, 0.5 units in the vertical dock).
    const wideUnits = orientation === 'horizontal' ? 2 : 0.5;
    let totalUnits = 0;
    const items = widgetIds.map(id => {
      const def = WIDGET_MAP.get(id);
      const wclass: WidgetClass = def?.widgetClass ?? 'small';
      const units = wclass === 'large' ? 1 : wclass === 'wide' ? wideUnits : smallRatio;
      totalUnits += units;
      return { id, wclass, units };
    });

    // Gap between widgets in vmin
    const gapVmin = 0.5;
    const totalGaps = (widgetIds.length - 1) * gapVmin;
    const usableVmin = availableVmin - totalGaps;

    // Check if widgets fit at their effective base size, scale down if not
    const baseTotal = totalUnits * effectiveLargeBase;
    const scale = baseTotal <= usableVmin ? 1 : Math.max(MIN_SCALE, usableVmin / baseTotal);

    // `size` = the cross-axis fill (height in horizontal dock, width in vertical).
    // large + wide fill the cross axis; small is 0.6×.
    return items.map(item => ({
      id: item.id,
      wclass: item.wclass,
      size: (item.wclass === 'small' ? effectiveLargeBase * smallRatio : effectiveLargeBase) * scale,
    }));
  }

  // Convert vmin sizes → px for container-relative rendering
  let sizes = $derived(computeSizes().map(s => ({ ...s, sizePx: s.size * pxPerVmin })));
  const gapPx = $derived(0.5 * pxPerVmin);

  // Check if panel can accept one more widget
  function canAcceptMore(): boolean {
    // Simulate adding a small widget
    const simIds = [...widgetIds, '_test'];
    const effBase = Math.min(maxWidgetVmin, LARGE_BASE_VMIN);
    const smallRatio = SMALL_BASE_VMIN / LARGE_BASE_VMIN;
    const wideUnits = orientation === 'horizontal' ? 2 : 0.5;
    let totalUnits = 0;
    for (const id of simIds) {
      const def = WIDGET_MAP.get(id);
      const wclass = def?.widgetClass ?? 'small';
      totalUnits += wclass === 'large' ? 1 : wclass === 'wide' ? wideUnits : smallRatio;
    }
    const totalGaps = (simIds.length - 1) * 0.5;
    const usableVmin = availableVmin - totalGaps;
    const baseTotal = totalUnits * effBase;
    const scale = baseTotal <= usableVmin ? 1 : usableVmin / baseTotal;
    return scale >= MIN_SCALE;
  }

  // Drag state
  let dragIdx = $state(-1);
  let insertIdx = $state(-1); // insertion point (between slots)
  let externalDragOver = $state(false);

  // Show insertion indicator only when drop would change order
  let showInsert = $derived(
    insertIdx >= 0 &&
    !(dragIdx >= 0 && (insertIdx === dragIdx || insertIdx === dragIdx + 1))
  );

  function handleDragStart(e: DragEvent, idx: number) {
    if (!editing) { e.preventDefault(); return; }
    dragIdx = idx;
    activeDragPayload = { panelId, widgetId: widgetIds[idx], sourceIdx: idx };
    setGlobalDragPayload(activeDragPayload);
    if (DND_DEBUG) console.log('[WIDGET-DND] dragstart', activeDragPayload);
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = 'move';
      const payload = JSON.stringify(activeDragPayload);
      // Some WebView/engine combos drop custom MIME types on drop; keep a plain-text JSON fallback.
      e.dataTransfer.setData('application/widget-drag', payload);
      e.dataTransfer.setData('text/plain', `widget-drag:${payload}`);
    }
  }

  function handlePointerDown(e: MouseEvent, idx: number) {
    if (!editing || e.button !== 0) return;
    e.preventDefault();
    dragIdx = idx;
    activeDragPayload = { panelId, widgetId: widgetIds[idx], sourceIdx: idx };
    setGlobalDragPayload(activeDragPayload);
    ghostWidgetId = activeDragPayload.widgetId;
    ghostX = e.clientX + 14;
    ghostY = e.clientY + 14;
    const el = e.currentTarget as HTMLElement;
    const rect = el.getBoundingClientRect();
    ghostW = Math.max(100, Math.round(rect.width));
    ghostH = Math.max(100, Math.round(rect.height));
    if (DND_DEBUG) console.log('[WIDGET-DND] mousedown start', activeDragPayload);
  }

  function parseDropPayload(e: DragEvent): DragPayload | null {
    const custom = e.dataTransfer?.getData('application/widget-drag');
    if (custom) {
      try {
        return JSON.parse(custom);
      } catch {
        // fall through to plain-text fallback
      }
    }

    const plain = e.dataTransfer?.getData('text/plain') ?? '';
    if (!plain.startsWith('widget-drag:')) return null;
    try {
      return JSON.parse(plain.slice('widget-drag:'.length));
    } catch {
      return activeDragPayload;
    }
  }

  function handleDragOver(e: DragEvent, idx: number) {
    if (!editing) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';

    // Determine insertion side based on cursor position vs slot midpoint
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const pastMid = orientation === 'horizontal'
      ? e.clientX > rect.left + rect.width / 2
      : e.clientY > rect.top + rect.height / 2;
    insertIdx = pastMid ? idx + 1 : idx;
  }

  function handleDragLeave(e: DragEvent) {
    const related = e.relatedTarget as HTMLElement | null;
    const current = e.currentTarget as HTMLElement;
    if (related && current.contains(related)) return;
    insertIdx = -1;
  }

  function handleDragEnd() {
    if (DND_DEBUG) console.log('[WIDGET-DND] dragend', { panelId, dragIdx, insertIdx, activeDragPayload });
    dragIdx = -1;
    insertIdx = -1;
    externalDragOver = false;
    setGlobalDragPayload(null);
    activeDragPayload = null;
    clearGhost();
  }

  // Panel-level handlers — fires on empty space + catches slot drops (bubbled)
  function handlePanelDragOver(e: DragEvent) {
    if (!editing) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    externalDragOver = true;
    if (DND_DEBUG) console.log('[WIDGET-DND] dragover panel', { panelId, insertIdx, activeDragPayload });
  }

  function handlePanelDragLeave(e: DragEvent) {
    const related = e.relatedTarget as HTMLElement | null;
    const current = e.currentTarget as HTMLElement;
    if (related && current.contains(related)) return;
    externalDragOver = false;
    insertIdx = -1;
  }

  function executeDrop(data: DragPayload, dropAt: number) {
    try {
      if (data.panelId === panelId) {
        // Same-panel reorder
        const srcIdx: number = data.sourceIdx;
        const newIds = [...widgetIds];
        const [moved] = newIds.splice(srcIdx, 1);
        const adjIdx = srcIdx < dropAt ? dropAt - 1 : dropAt;
        newIds.splice(adjIdx, 0, moved);
        onreorder(panelId, newIds);
      } else {
        // Cross-panel move
        if (canAcceptMore()) {
          onreceive(panelId, data.widgetId, dropAt);
        }
      }
    } catch {
      // ignore malformed drop attempts
    }
  }

  function handlePanelDrop(e: DragEvent) {
    if (!editing) return;
    e.preventDefault();
    const dropAt = insertIdx >= 0 ? insertIdx : widgetIds.length;
    const data = parseDropPayload(e) ?? activeDragPayload ?? getGlobalDragPayload();
    if (DND_DEBUG) console.log('[WIDGET-DND] drop', { panelId, dropAt, data, activeDragPayload });
    if (data) executeDrop(data, dropAt);
    dragIdx = -1;
    insertIdx = -1;
    externalDragOver = false;
    setGlobalDragPayload(null);
    activeDragPayload = null;
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  bind:this={panelEl}
  class="widget-panel {orientation}"
  class:editing
  class:empty={widgetIds.length === 0}
  class:drag-hover={externalDragOver}
  style="gap: {gapPx}px;"
>
  {#each sizes as item, idx (item.id)}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="widget-slot"
      data-slot-idx={idx}
      class:dragging={dragIdx === idx}
      class:editing
      class:insert-before={showInsert && insertIdx === idx}
      class:insert-after={showInsert && insertIdx === widgetIds.length && idx === widgetIds.length - 1}
      draggable={false}
      onmousedown={(e) => handlePointerDown(e, idx)}
    >
      {#if editing}
        <div class="drag-handle">⠿</div>
        <div class="drag-overlay"></div>
      {/if}
      <div class="widget-content">
        {#if item.id === 'ahi'}
          <AHI {telem} size={item.sizePx} />
        {:else if item.id === 'speed'}
          <SpeedWidget {telem} size={item.sizePx} {interfaceSettings} />
        {:else if item.id === 'altitude'}
          <AltWidget {telem} size={item.sizePx} {interfaceSettings} />
        {:else if item.id === 'battery'}
          <BatteryWidget {telem} size={item.sizePx} widgetId="battery" />
        {:else if item.id === 'battery2'}
          <BatteryWidget {telem} size={item.sizePx} widgetId="battery2" />
        {:else if item.id === 'gps'}
          <GpsWidget {telem} size={item.sizePx} />
        {:else if item.id === 'rcLink'}
          <RcLinkWidget {telem} size={item.sizePx} />
        {:else if item.id === 'compass'}
          <CompassWidget {telem} size={item.sizePx} {interfaceSettings} />
        {:else if item.id === 'home'}
          <HomeWidget {telem} size={item.sizePx} {interfaceSettings} />
        {:else if item.id === 'flightMode'}
          <FlightModeWidget {telem} size={item.sizePx} />
        {:else if item.id === 'rawTelemetry'}
          <RawTelemetryWidget {telem} size={item.sizePx} {interfaceSettings} />
        {:else if item.id === 'liveAgl'}
          <LiveAglWidget
            {telem}
            {interfaceSettings}
            width={orientation === 'horizontal' ? item.sizePx * 2 : item.sizePx}
            height={orientation === 'horizontal' ? item.sizePx : item.sizePx / 2}
          />
        {:else if item.id === 'terrainRadar'}
          <TerrainRadarWidget {telem} {interfaceSettings} size={item.sizePx} />
        {:else if item.id === 'videoFeed'}
          <VideoWidget
            width={orientation === 'horizontal' ? item.sizePx * 2 : item.sizePx}
            height={orientation === 'horizontal' ? item.sizePx : item.sizePx / 2}
          />
        {/if}
      </div>
    </div>
  {/each}

  {#if editing && ghostWidgetId && getGlobalDragPayload()?.panelId === panelId}
    <div
      class="drag-ghost"
      style="left:{ghostX}px; top:{ghostY}px; width:{ghostW}px; height:{ghostH}px;"
    >
      <span>{ghostWidgetLabel}</span>
    </div>
  {/if}
</div>

<style>
  .widget-panel {
    display: flex;
    /* gap is set inline via style="gap: {gapPx}px" */
    transition: outline 0.2s;
  }

  .widget-panel.horizontal {
    flex-direction: row;
    align-items: flex-end;
    justify-content: center;
  }

  .widget-panel.vertical {
    flex-direction: column;
    align-items: flex-end;
    justify-content: center;
  }

  /* Edit mode: show panel outline, accept drops */
  .widget-panel.editing {
    outline: 2px dashed rgba(55, 168, 219, 0.7);
    outline-offset: 4px;
    border-radius: 8px;
    min-width: 40px;
    min-height: 40px;
    padding: 4px;
    background: rgba(0, 0, 0, 0.25);
  }

  .widget-panel.editing.empty {
    outline-style: dotted;
    outline-color: rgba(55, 168, 219, 0.5);
  }

  .widget-panel.horizontal.editing.empty {
    min-width: 100px;
    min-height: 100px;
  }

  .widget-panel.vertical.editing.empty {
    min-width: 100px;
    min-height: 100px;
  }

  .widget-panel.drag-hover {
    outline-color: #37a8db;
    outline-style: solid;
    background: rgba(55, 168, 219, 0.05);
  }

  .widget-slot {
    position: relative;
    transition: opacity 0.15s, transform 0.15s;
    cursor: default;
  }

  /* Overlay is visual-only; events must hit the slot directly for reliable DnD in WebView. */
  .drag-overlay {
    position: absolute;
    inset: 0;
    z-index: 5;
    cursor: grab;
    pointer-events: none;
  }

  .drag-overlay:active {
    cursor: grabbing;
  }

  .widget-panel.editing .widget-slot {
    cursor: grab;
  }

  .widget-panel.editing .widget-slot:active {
    cursor: grabbing;
  }

  .widget-slot.dragging {
    opacity: 0.3;
    transform: scale(0.95);
  }

  /* Insertion indicator — line showing where dragged widget will land */
  .widget-slot.insert-before::before,
  .widget-slot.insert-after::after {
    content: '';
    position: absolute;
    background: #37a8db;
    border-radius: 2px;
    z-index: 20;
  }

  /* Horizontal: vertical insertion lines */
  .widget-panel.horizontal .widget-slot.insert-before::before {
    left: -3px;
    top: 0; bottom: 0;
    width: 3px;
  }
  .widget-panel.horizontal .widget-slot.insert-after::after {
    right: -3px;
    top: 0; bottom: 0;
    width: 3px;
  }

  /* Vertical: horizontal insertion lines */
  .widget-panel.vertical .widget-slot.insert-before::before {
    top: -3px;
    left: 0; right: 0;
    height: 3px;
  }
  .widget-panel.vertical .widget-slot.insert-after::after {
    bottom: -3px;
    left: 0; right: 0;
    height: 3px;
  }

  .drag-handle {
    position: absolute;
    top: -3px;
    left: 50%;
    transform: translateX(-50%);
    font-size: 14px;
    color: rgba(55, 168, 219, 0.7);
    z-index: 10;
    pointer-events: none;
    line-height: 1;
  }

  .widget-panel.vertical .drag-handle {
    top: 50%;
    left: -14px;
    transform: translateY(-50%) rotate(90deg);
  }

  .drag-ghost {
    position: fixed;
    /* left/top track the cursor; centre the ghost on it so the frame sits under the pointer (intuitive
       placement) instead of hanging to the bottom-right. */
    transform: translate(-50%, -50%);
    pointer-events: none;
    z-index: 1000;
    border: 1px solid rgba(55, 168, 219, 0.75);
    border-radius: 8px;
    background: rgba(30, 30, 30, 0.48);
    backdrop-filter: blur(4px);
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.35);
    display: flex;
    align-items: center;
    justify-content: center;
    color: rgba(255, 255, 255, 0.85);
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
</style>
