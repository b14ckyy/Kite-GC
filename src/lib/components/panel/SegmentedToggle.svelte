<script lang="ts" module>
  import type { ButtonIcon } from './Button.svelte';
  export type SegOption = { value: string; label: string; icon?: ButtonIcon };
</script>

<script lang="ts">
  // Small multi-position slide switch placed as ONE element (e.g. Replay: Recording ↔ Blackbox
  // track). A sliding highlight marks the active segment. See docs/dev/PANEL_FRAMEWORK.md.
  import { iconSvg } from './Button.svelte';

  let { options, value, onchange = undefined, size = 'md' }: {
    options: SegOption[];
    value: string;
    onchange?: (value: string) => void;
    size?: 'sm' | 'md';
  } = $props();

  const idx = $derived(Math.max(0, options.findIndex((o) => o.value === value)));
</script>

<div class="seg seg-{size}" style="--n:{options.length}; --i:{idx}">
  <div class="seg-ind"></div>
  {#each options as o, i}
    <button class="seg-btn" class:active={i === idx} type="button" title={o.label} onclick={() => onchange?.(o.value)}>
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

  /* Sliding highlight: one segment wide, translated to the active index. */
  .seg-ind {
    position: absolute;
    top: 2px;
    bottom: 2px;
    left: 2px;
    width: calc((100% - 4px) / var(--n));
    transform: translateX(calc(var(--i) * 100%));
    background: rgba(55, 168, 219, 0.22);
    border: 1px solid #37a8db;
    border-radius: 4px;
    transition: transform 0.2s ease;
    pointer-events: none;
  }

  .seg-btn {
    position: relative;
    z-index: 1;
    flex: 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 5px;
    min-width: 0;
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
  .seg-label { overflow: hidden; text-overflow: ellipsis; }
  .seg-icon { display: inline-flex; flex-shrink: 0; }
  .seg-icon :global(svg) { width: 1.2em; height: 1.2em; display: block; }
</style>
