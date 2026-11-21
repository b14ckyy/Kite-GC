<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- Per-widget settings for the Battery widget, opened by tapping the widget: where the shown
     voltage comes from — the FC's own battery monitor (default), or one ESC's telemetry (a boat with
     two ESCs and no battery sensor reads its pack voltage there). -->
<script lang="ts">
  import { t } from 'svelte-i18n';
  import { settings } from '$lib/stores/settings';

  let {
    widgetId,
    rect,
    onclose,
  }: {
    /** 'battery' or 'battery2' — the setting is stored per widget. */
    widgetId: string;
    /** The widget's viewport box (the popover sits above it, or below when there is no room). */
    rect: { left: number; right: number; top: number; bottom: number; width: number; height: number };
    onclose: () => void;
  } = $props();

  const ESCS = [1, 2, 3, 4, 5, 6, 7, 8];
  const source = $derived($settings.batteryVoltageSource?.[widgetId] ?? 'fc');

  function setSource(value: string): void {
    settings.update((s) => ({
      ...s,
      batteryVoltageSource: { ...s.batteryVoltageSource, [widgetId]: value },
    }));
  }

  const POP_W = 250;
  const POP_H = 150;
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

<div class="wsp-pop" style={style} role="dialog" aria-label={$t('widgets.batterySettings')}>
  <div class="wsp-head">
    <span>{$t('widgets.batterySettings')}</span>
    <button class="wsp-close" type="button" onclick={onclose} aria-label={$t('window.close')}>✕</button>
  </div>

  <div class="wsp-row">
    <label class="wsp-label" for="wsp-bat-source">{$t('widgets.batterySource')}</label>
    <select id="wsp-bat-source" class="wsp-select" value={source}
      onchange={(e) => setSource((e.target as HTMLSelectElement).value)}>
      <option value="fc">{$t('widgets.batterySourceFc')}</option>
      {#each ESCS as n}<option value="esc:{n}">{$t('widgets.batterySourceEsc', { values: { n } })}</option>{/each}
    </select>
  </div>

  <p class="wsp-hint">{$t('widgets.batterySourceHint')}</p>
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
    max-width: 130px;
  }
  .wsp-hint {
    margin: 0;
    font-size: 11px;
    line-height: 1.35;
    color: #949494;
  }
</style>
