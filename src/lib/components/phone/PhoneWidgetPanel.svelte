<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- PhoneWidgetPanel — the full-height smoked-glass widget column on the phone's right edge
     (Dev-Docs active/PHONE_UI.md D2/D6/D7). Grid of 4 rows × `cols` columns; one slot = the panel's
     height / 4, so the widgets scale with the screen. PHASE 1 (skeleton): the glass surface and the
     slot raster only, so the layout can be judged on the emulators — the packer, the pages and the
     widgets themselves land in phase 2. The panel reports its width (`widthPx`) so the page grid
     reserves exactly that column. -->
<script lang="ts">
  import { PHONE_GRID_ROWS, PHONE_GRID_MAX_COLS } from '$lib/config/phoneGrid';

  let {
    cols = PHONE_GRID_MAX_COLS,
    widthPx = $bindable(0),
  }: {
    /** Slot columns, 1 … PHONE_GRID_MAX_COLS (auto-narrowed by the packer in phase 2). */
    cols?: number;
    /** OUT: the panel's rendered width in css px. */
    widthPx?: number;
  } = $props();

  const PAD = 4;
  let heightPx = $state(0);
  // One slot = the usable height split into PHONE_GRID_ROWS (config/phoneGrid.ts — the raster may
  // change to 5 rows; nothing here assumes 4).
  const slot = $derived(Math.max(40, Math.floor((heightPx - 2 * PAD) / PHONE_GRID_ROWS)));
  $effect(() => {
    const w = slot * cols + 2 * PAD;
    if (w !== widthPx) widthPx = w;
  });
  const slots = $derived(Array.from({ length: PHONE_GRID_ROWS * cols }, (_, i) => i));
</script>

<div class="pwp" bind:clientHeight={heightPx} style="--slot:{slot}px; --pad:{PAD}px; --cols:{cols}">
  <div class="grid">
    {#each slots as i (i)}
      <div class="slot"></div>
    {/each}
  </div>
</div>

<style>
  .pwp {
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

  .grid {
    display: grid;
    grid-template-columns: repeat(var(--cols), var(--slot));
    grid-auto-rows: var(--slot);
  }

  /* Skeleton raster — replaced by the packed widgets in phase 2. */
  .slot {
    box-sizing: border-box;
    border: 1px dashed rgba(55, 168, 219, 0.25);
  }
</style>
