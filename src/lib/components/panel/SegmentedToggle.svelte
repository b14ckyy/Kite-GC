<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts" module>
  import type { ButtonIcon } from './Button.svelte';
  export type SegOption = { value: string; label: string; icon?: ButtonIcon };
</script>

<script lang="ts">
  // Small multi-position slide switch placed as ONE element (e.g. Replay: Recording ↔ Blackbox
  // track). A sliding highlight marks the active segment. See docs/active/PANEL_FRAMEWORK.md.
  //
  // Segments are sized to their content (so longer labels like "MAVLink" are never truncated); the
  // highlight tracks the active segment's measured offset/width, so it slides correctly across
  // unequal segment widths.
  import { iconSvg } from './Button.svelte';

  let { options, value, onchange = undefined, size = 'md', full = false }: {
    options: SegOption[];
    value: string;
    onchange?: (value: string) => void;
    size?: 'sm' | 'md';
    /** Stretch to the parent's full width and distribute segments evenly (for fixed-width tab bars).
     *  Default = content-sized (each segment fits its label). */
    full?: boolean;
  } = $props();

  const idx = $derived(Math.max(0, options.findIndex((o) => o.value === value)));

  let segEl = $state<HTMLDivElement>();
  let btnEls = $state<HTMLButtonElement[]>([]);
  let indLeft = $state(0);
  let indWidth = $state(0);

  function measure() {
    const b = btnEls[idx];
    if (b) {
      indLeft = b.offsetLeft;
      indWidth = b.offsetWidth;
    }
  }

  // Re-measure when the active index, the options, or the rendered buttons change.
  $effect(() => {
    idx;
    options.length;
    btnEls.length;
    measure();
  });

  // Re-measure on any size change (font load, container resize, label changes).
  $effect(() => {
    if (!segEl) return;
    const ro = new ResizeObserver(() => measure());
    ro.observe(segEl);
    for (const b of btnEls) if (b) ro.observe(b);
    return () => ro.disconnect();
  });
</script>

<div class="seg seg-{size}" class:full bind:this={segEl}>
  <div class="seg-ind" style="left:{indLeft}px; width:{indWidth}px"></div>
  {#each options as o, i}
    <button bind:this={btnEls[i]} class="seg-btn" class:active={i === idx} type="button" title={o.label} onclick={() => onchange?.(o.value)}>
      {#if o.icon}<span class="seg-icon">{@html iconSvg(o.icon)}</span>{/if}
      <span class="seg-label">{o.label}</span>
    </button>
  {/each}
</div>

<style>
  .seg {
    position: relative;
    display: inline-flex;
    align-items: stretch;
    box-sizing: border-box;
    padding: 2px;
    background: #2a2a2a;
    border: 1px solid #555;
    border-radius: 5px;
  }
  .seg-md { height: 28px; font-size: 12px; }
  .seg-sm { height: 24px; font-size: 11px; }
  /* Full-width mode: fill the parent and split the width evenly across segments. Segments may shrink
     below their content here, so the label ellipsizes gracefully if a tab bar gets too narrow. */
  .seg.full { display: flex; width: 100%; }
  .seg.full .seg-btn { flex: 1; min-width: 0; }

  /* Sliding highlight: positioned + sized to the active segment (measured in JS). */
  .seg-ind {
    position: absolute;
    top: 2px;
    bottom: 2px;
    background: rgba(55, 168, 219, 0.22);
    border: 1px solid #37a8db;
    border-radius: 4px;
    transition: left 0.2s ease, width 0.2s ease;
    pointer-events: none;
  }

  .seg-btn {
    position: relative;
    z-index: 1;
    flex: 0 0 auto; /* content-sized — no truncation of longer labels */
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 5px;
    padding: 0 12px;
    background: none;
    border: none;
    color: #aaa;
    cursor: pointer;
    white-space: nowrap;
    font-family: inherit;
    transition: color 0.15s;
  }
  .seg-btn:hover:not(.active) { color: #e0e0e0; }
  .seg-btn.active { color: #37a8db; }
  /* Harmless in content mode (segment fits its label); the graceful fallback in full mode. */
  .seg-label { overflow: hidden; text-overflow: ellipsis; }
  .seg-icon { display: inline-flex; flex-shrink: 0; }
  .seg-icon :global(svg) { width: 1.2em; height: 1.2em; display: block; }
</style>
