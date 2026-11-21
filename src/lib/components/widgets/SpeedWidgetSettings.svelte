<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- Per-widget settings for the Speed widget, opened by tapping the widget. Only the differential
     thrust (dual-motor boat) option for now: left/right thrust bars from two FC servo outputs,
     replacing the throttle + acceleration pair. -->
<script lang="ts">
  import { t } from 'svelte-i18n';
  import { settings, DEFAULT_SPEED_WIDGET, type SpeedWidgetSettings } from '$lib/stores/settings';
  import Toggle from '$lib/components/panel/Toggle.svelte';

  let {
    rect,
    onclose,
  }: {
    /** The widget's viewport box (the popover sits above it, or below when there is no room). */
    rect: { left: number; right: number; top: number; bottom: number; width: number; height: number };
    onclose: () => void;
  } = $props();

  const cfg = $derived($settings.speedWidget ?? DEFAULT_SPEED_WIDGET);
  const OUTPUTS = [1, 2, 3, 4, 5, 6, 7, 8];

  function patch(next: Partial<SpeedWidgetSettings>): void {
    settings.patch({ speedWidget: { ...cfg, ...next } });
  }

  // Popover size estimate — only used to keep it on screen; the real box measures itself.
  const POP_W = 250;
  const POP_H = 190;
  const style = $derived.by(() => {
    const vw = typeof window !== 'undefined' ? window.innerWidth : 1024;
    const vh = typeof window !== 'undefined' ? window.innerHeight : 768;
    const left = Math.min(Math.max(8, rect.left + rect.width / 2 - POP_W / 2), Math.max(8, vw - POP_W - 8));
    const above = rect.top - POP_H - 10;
    const top = above >= 8 ? above : Math.min(rect.bottom + 10, Math.max(8, vh - POP_H - 8));
    return `left:${Math.round(left)}px; top:${Math.round(top)}px; width:${POP_W}px;`;
  });

  function onWindowPointerDown(e: PointerEvent): void {
    if ((e.target as HTMLElement | null)?.closest('.wsp-pop')) return;
    onclose();
  }
  function onWindowKeyDown(e: KeyboardEvent): void {
    if (e.key === 'Escape') onclose();
  }
</script>

<svelte:window onpointerdown={onWindowPointerDown} onkeydown={onWindowKeyDown} />

<div class="wsp-pop" style={style} role="dialog" aria-label={$t('widgets.speedSettings')}>
  <div class="wsp-head">
    <span>{$t('widgets.speedSettings')}</span>
    <button class="wsp-close" type="button" onclick={onclose} aria-label={$t('window.close')}>✕</button>
  </div>

  <label class="wsp-row" for="wsp-diff">
    <span class="wsp-label">{$t('widgets.diffThrust')}</span>
    <Toggle checked={cfg.diffThrust} id="wsp-diff" onchange={(c) => patch({ diffThrust: c })} />
  </label>

  {#if cfg.diffThrust}
    <div class="wsp-row">
      <label class="wsp-label" for="wsp-left">{$t('widgets.leftOutput')}</label>
      <select id="wsp-left" class="wsp-select" value={String(cfg.left)}
        onchange={(e) => patch({ left: Number((e.target as HTMLSelectElement).value) })}>
        {#each OUTPUTS as n}<option value={String(n)}>S{n}</option>{/each}
      </select>
    </div>
    <div class="wsp-row">
      <label class="wsp-label" for="wsp-right">{$t('widgets.rightOutput')}</label>
      <select id="wsp-right" class="wsp-select" value={String(cfg.right)}
        onchange={(e) => patch({ right: Number((e.target as HTMLSelectElement).value) })}>
        {#each OUTPUTS as n}<option value={String(n)}>S{n}</option>{/each}
      </select>
    </div>
  {/if}

  <p class="wsp-hint">{$t('widgets.diffThrustHint')}</p>
</div>

<style>
  .wsp-pop {
    position: fixed;
    z-index: 4000;
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: calc(100vw - 16px);
    padding: 10px 12px 8px;
    background: rgba(46, 46, 46, 0.97);
    border: 1px solid #272727;
    border-radius: 8px;
    box-shadow: 0 8px 28px rgba(0, 0, 0, 0.5);
    backdrop-filter: var(--glass-blur, blur(6px));
    -webkit-backdrop-filter: var(--glass-blur, blur(6px));
    font-family: 'Segoe UI', Tahoma, sans-serif;
    color: #e0e0e0;
  }
  .wsp-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: 13px;
    font-weight: 600;
    color: #37a8db;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .wsp-close {
    background: none;
    border: none;
    color: #949494;
    font-size: 14px;
    line-height: 1;
    cursor: pointer;
    padding: 2px 4px;
  }
  .wsp-close:hover { color: #d40000; }
  .wsp-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    font-size: 12.5px;
  }
  .wsp-label { color: #cfcfcf; }
  .wsp-select {
    background: #3d3f3e;
    color: #e0e0e0;
    border: 1px solid #555;
    border-radius: 4px;
    padding: 3px 6px;
    font-size: 12.5px;
    min-width: 64px;
  }
  .wsp-hint {
    margin: 0;
    font-size: 11px;
    line-height: 1.35;
    color: #949494;
  }
</style>
