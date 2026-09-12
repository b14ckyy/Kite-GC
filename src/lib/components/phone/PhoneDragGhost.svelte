<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- PhoneDragGhost — the copy of the widget under the finger while it is dragged between the
     phone's widget column and the bottom slots (stores/phoneEdit.svelte.ts). Drawn at the root in
     viewport px: the column clips its own overflow, so a ghost inside it could never leave it. The
     real tile shows the preview slot; this one is pointer-transparent so the drop target under
     the finger stays hit-testable. -->
<script lang="ts">
  import WidgetRenderer from '$lib/components/WidgetRenderer.svelte';
  import { phoneEdit } from '$lib/stores/phoneEdit.svelte';
  import type { TelemetryData } from '$lib/stores/telemetry';
  import type { InterfaceSettings } from '$lib/stores/settings';

  let { telem, interfaceSettings }: { telem: TelemetryData; interfaceSettings: InterfaceSettings } = $props();
  const d = $derived(phoneEdit.drag);
</script>

{#if d}
  <div class="ghost" style="left:{d.x}px; top:{d.y}px; width:{d.w}px; height:{d.h}px;">
    <WidgetRenderer id={d.id} {telem} {interfaceSettings} sizePx={d.h} wPx={d.w} hPx={d.h} ghost />
  </div>
{/if}

<style>
  /* `:global(.app) >`: see PhoneBottomBar — +page re-enables pointer events on .app's children. */
  :global(.app) > .ghost {
    position: fixed;
    z-index: 180; /* over the panels (150 / 160) and the Debug button (170), under the dialogs */
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
</style>
