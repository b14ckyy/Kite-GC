<!-- WidgetPanel — a panel strip (horizontal or vertical) that holds widgets with drag-and-drop reordering -->
<script lang="ts">
  import type { TelemetryData } from "$lib/stores/telemetry";
  import { WIDGET_MAP, LARGE_BASE_VMIN, SMALL_BASE_VMIN, MIN_SCALE, type WidgetClass } from "$lib/config/widgetRegistry";
  import AHI from "./widgets/AHI.svelte";
  import SpeedWidget from "./widgets/SpeedWidget.svelte";
  import AltWidget from "./widgets/AltWidget.svelte";
  import BatteryWidget from "./widgets/BatteryWidget.svelte";
  import GpsWidget from "./widgets/GpsWidget.svelte";
  import CompassWidget from "./widgets/CompassWidget.svelte";
  import HomeWidget from "./widgets/HomeWidget.svelte";
  import RawTelemetryWidget from "./widgets/RawTelemetryWidget.svelte";

  let {
    widgetIds = [],
    orientation = 'horizontal',
    availableVmin = 80,
    telem,
    editing = false,
    onreorder,
    onreceive,
    panelId,
  }: {
    widgetIds: string[];
    orientation: 'horizontal' | 'vertical';
    availableVmin: number;
    telem: TelemetryData;
    editing: boolean;
    onreorder: (panelId: string, widgetIds: string[]) => void;
    onreceive: (targetPanel: string, widgetId: string, index: number) => void;
    panelId: string;
  } = $props();

  // Compute widget sizes based on available space
  function computeSizes(): { id: string; size: number; wclass: WidgetClass }[] {
    if (widgetIds.length === 0) return [];

    // Calculate total "units" needed (large=1, small=0.6)
    let totalUnits = 0;
    const items = widgetIds.map(id => {
      const def = WIDGET_MAP.get(id);
      const wclass: WidgetClass = def?.widgetClass ?? 'small';
      const units = wclass === 'large' ? 1 : 0.6;
      totalUnits += units;
      return { id, wclass, units };
    });

    // Gap between widgets in vmin
    const gapVmin = 0.5;
    const totalGaps = (widgetIds.length - 1) * gapVmin;
    const usableVmin = availableVmin - totalGaps;

    // Base: large = LARGE_BASE_VMIN, small = SMALL_BASE_VMIN
    // Check if they fit at base size
    const baseTotal = totalUnits * LARGE_BASE_VMIN;
    const scale = baseTotal <= usableVmin ? 1 : usableVmin / baseTotal;

    return items.map(item => ({
      id: item.id,
      wclass: item.wclass,
      size: (item.wclass === 'large' ? LARGE_BASE_VMIN : SMALL_BASE_VMIN) * scale,
    }));
  }

  let sizes = $derived(computeSizes());

  // Check if panel can accept one more widget
  function canAcceptMore(): boolean {
    // Simulate adding a small widget
    const simIds = [...widgetIds, '_test'];
    let totalUnits = 0;
    for (const id of simIds) {
      const def = WIDGET_MAP.get(id);
      const wclass = def?.widgetClass ?? 'small';
      totalUnits += wclass === 'large' ? 1 : 0.6;
    }
    const totalGaps = (simIds.length - 1) * 0.5;
    const usableVmin = availableVmin - totalGaps;
    const baseTotal = totalUnits * LARGE_BASE_VMIN;
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
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = 'move';
      e.dataTransfer.setData('application/widget-drag', JSON.stringify({
        panelId,
        widgetId: widgetIds[idx],
        sourceIdx: idx,
      }));
      // Needed for Firefox
      e.dataTransfer.setData('text/plain', widgetIds[idx]);
    }
  }

  function handleDragOver(e: DragEvent, idx: number) {
    if (!editing) return;
    e.preventDefault();
    e.stopPropagation();
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
    dragIdx = -1;
    insertIdx = -1;
    externalDragOver = false;
  }

  // Panel-level handlers — fires on empty space + catches slot drops (bubbled)
  function handlePanelDragOver(e: DragEvent) {
    if (!editing) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    externalDragOver = true;
  }

  function handlePanelDragLeave(e: DragEvent) {
    const related = e.relatedTarget as HTMLElement | null;
    const current = e.currentTarget as HTMLElement;
    if (related && current.contains(related)) return;
    externalDragOver = false;
    insertIdx = -1;
  }

  function handlePanelDrop(e: DragEvent) {
    if (!editing) return;
    e.preventDefault();
    const dropAt = insertIdx >= 0 ? insertIdx : widgetIds.length;
    dragIdx = -1;
    insertIdx = -1;
    externalDragOver = false;

    const raw = e.dataTransfer?.getData('application/widget-drag');
    if (!raw) return;
    try {
      const data = JSON.parse(raw);
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
    } catch { /* ignore */ }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="widget-panel {orientation}"
  class:editing
  class:empty={widgetIds.length === 0}
  class:drag-hover={externalDragOver}
  ondragover={handlePanelDragOver}
  ondragleave={handlePanelDragLeave}
  ondrop={handlePanelDrop}
>
  {#each sizes as item, idx (item.id)}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="widget-slot"
      class:dragging={dragIdx === idx}
      class:editing
      class:insert-before={showInsert && insertIdx === idx}
      class:insert-after={showInsert && insertIdx === widgetIds.length && idx === widgetIds.length - 1}
      draggable={editing ? 'true' : 'false'}
      ondragstart={(e) => handleDragStart(e, idx)}
      ondragover={(e) => handleDragOver(e, idx)}
      ondragleave={(e) => handleDragLeave(e)}
      ondragend={handleDragEnd}
    >
      {#if editing}
        <div class="drag-handle">⠿</div>
        <div class="drag-overlay"></div>
      {/if}
      <div class="widget-content">
        {#if item.id === 'ahi'}
          <AHI {telem} size={item.size} />
        {:else if item.id === 'speed'}
          <SpeedWidget {telem} size={item.size} />
        {:else if item.id === 'altitude'}
          <AltWidget {telem} size={item.size} />
        {:else if item.id === 'battery'}
          <BatteryWidget {telem} size={item.size} />
        {:else if item.id === 'gps'}
          <GpsWidget {telem} size={item.size} />
        {:else if item.id === 'compass'}
          <CompassWidget {telem} size={item.size} />
        {:else if item.id === 'home'}
          <HomeWidget {telem} size={item.size} />
        {:else if item.id === 'rawTelemetry'}
          <RawTelemetryWidget {telem} size={item.size} />
        {/if}
      </div>
    </div>
  {/each}
</div>

<style>
  .widget-panel {
    display: flex;
    gap: 0.5vmin;
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
    min-width: 4vmin;
    min-height: 4vmin;
    padding: 0.5vmin;
    background: rgba(0, 0, 0, 0.25);
  }

  .widget-panel.editing.empty {
    outline-style: dotted;
    outline-color: rgba(55, 168, 219, 0.5);
  }

  .widget-panel.horizontal.editing.empty {
    min-width: 13.5vmin;
    min-height: 13.5vmin;
  }

  .widget-panel.vertical.editing.empty {
    min-width: 13.5vmin;
    min-height: 13.5vmin;
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

  /* Transparent overlay captures pointer events in edit mode so
     drag events reach the parent .widget-slot handlers. */
  .drag-overlay {
    position: absolute;
    inset: 0;
    z-index: 5;
    cursor: grab;
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
    left: -0.4vmin;
    top: 0; bottom: 0;
    width: 3px;
  }
  .widget-panel.horizontal .widget-slot.insert-after::after {
    right: -0.4vmin;
    top: 0; bottom: 0;
    width: 3px;
  }

  /* Vertical: horizontal insertion lines */
  .widget-panel.vertical .widget-slot.insert-before::before {
    top: -0.4vmin;
    left: 0; right: 0;
    height: 3px;
  }
  .widget-panel.vertical .widget-slot.insert-after::after {
    bottom: -0.4vmin;
    left: 0; right: 0;
    height: 3px;
  }

  .widget-content {
    /* pass-through, no layout interference */
  }

  .drag-handle {
    position: absolute;
    top: -0.3vmin;
    left: 50%;
    transform: translateX(-50%);
    font-size: 1.8vmin;
    color: rgba(55, 168, 219, 0.7);
    z-index: 10;
    pointer-events: none;
    line-height: 1;
  }

  .widget-panel.vertical .drag-handle {
    top: 50%;
    left: -1.5vmin;
    transform: translateY(-50%) rotate(90deg);
  }
</style>
