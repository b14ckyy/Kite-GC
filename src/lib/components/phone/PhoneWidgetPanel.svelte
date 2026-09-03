<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- PhoneWidgetPanel — the full-height smoked-glass widget column on the phone's right edge
     (Dev-Docs active/PHONE_UI.md D2/D6–D9). One slot = usable height / PHONE_GRID_ROWS; the packer
     (helpers/phoneGridPacker.ts) turns the persisted order into page/row/col placements, and this
     component draws them: widgets edge to edge on the glass (their own card glass is switched off
     here), two pages under a vertical snap scroll, page dots at the bottom. The panel is 1 or 2
     slots wide as the packer decides and reports its width (`widthPx`) so the page grid reserves
     exactly that column. Phase 3 adds the edit mode (long-press, drag, resize). -->
<script lang="ts">
  import { t } from 'svelte-i18n';
  import { PHONE_GRID_ROWS, PHONE_GRID_PAGES } from '$lib/config/phoneGrid';
  import { packPhone, type PhoneWidgetsConfig } from '$lib/controllers/phoneWidgetController';
  import WidgetRenderer from '$lib/components/WidgetRenderer.svelte';
  import type { TelemetryData } from '$lib/stores/telemetry';
  import type { InterfaceSettings } from '$lib/stores/settings';

  let {
    config,
    telem,
    interfaceSettings,
    editing = false,
    widthPx = $bindable(0),
  }: {
    config: PhoneWidgetsConfig;
    telem: TelemetryData;
    interfaceSettings: InterfaceSettings;
    editing?: boolean;
    /** OUT: the panel's rendered width in css px. */
    widthPx?: number;
  } = $props();

  const PAD = 4;
  let heightPx = $state(0);
  const packed = $derived(packPhone(config));
  const cols = $derived(packed.cols);
  // Usable height per page = the panel minus its padding; one slot = that / rows.
  // Bounded by the VIEWPORT height, never by the measured box alone: the box must not be able to
  // feed back into the slot (a grid row that grows with the pages did exactly that once).
  const viewportH = $derived(typeof window === 'undefined' ? 0 : window.innerHeight);
  const pageH = $derived(Math.max(0, Math.min(heightPx, viewportH || heightPx) - 2 * PAD));
  const slot = $derived(Math.max(40, Math.floor(pageH / PHONE_GRID_ROWS)));
  $effect(() => {
    const w = slot * cols + 2 * PAD;
    if (w !== widthPx) widthPx = w;
  });

  const pages = $derived(Array.from({ length: PHONE_GRID_PAGES }, (_, p) => p));
  const onPage = (p: number) => packed.placements.filter((x) => x.page === p);

  // Current page for the dots: derived from the scroll position (snap → whole pages).
  let scroller = $state<HTMLDivElement>();
  let page = $state(0);
  function onScroll() {
    if (!scroller || pageH <= 0) return;
    page = Math.round(scroller.scrollTop / pageH);
  }
</script>

<div class="pwp" bind:clientHeight={heightPx} style="--slot:{slot}px; --pad:{PAD}px; --cols:{cols}; --page-h:{pageH}px">
  <div class="pages" bind:this={scroller} onscroll={onScroll}>
    {#each pages as p (p)}
      <div class="page">
        {#each onPage(p) as pl (pl.id)}
          <div
            class="cell"
            class:editing
            style="left:{pl.col * slot}px; top:{pl.row * slot}px; width:{pl.w * slot}px; height:{pl.h * slot}px;"
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
          </div>
        {/each}
      </div>
    {/each}
  </div>
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
    background: rgba(30, 30, 30, 0.75);
    backdrop-filter: blur(10px);
    -webkit-backdrop-filter: blur(10px);
    border-left: 1px solid rgba(255, 255, 255, 0.08);
    pointer-events: auto;
    overflow: hidden;
  }

  /* Vertical page scroller with snap — one page = the usable height. */
  .pages {
    height: 100%;
    overflow-y: auto;
    overflow-x: hidden;
    scroll-snap-type: y mandatory;
    scrollbar-width: none;
    touch-action: pan-y;
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
