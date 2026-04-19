<!-- Battery widget — voltage bar, voltage, current, mAh -->
<script lang="ts">
  import type { TelemetryData } from "$lib/stores/telemetry";
  import { t } from 'svelte-i18n';

  let { telem, size = 9 }: { telem: TelemetryData; size?: number } = $props();

  let pct = $derived(telem.lastUpdate ? Math.max(0, Math.min(100, telem.batteryPercentage)) : 0);
  let barColor = $derived(pct >= 50 ? '#27ae60' : pct >= 20 ? '#f39c12' : '#e74c3c');
  let voltage = $derived(telem.lastUpdate ? `${telem.voltage.toFixed(1)}V` : '—V');
  let current = $derived(telem.lastUpdate ? `${telem.current.toFixed(1)}A` : '—A');
  let mah = $derived(telem.lastUpdate ? `${telem.mAhDrawn} mAh` : '—');
</script>

<div class="widget-card" style="--ws: {size}vmin">
  <span class="w-label">{$t('widgetLabels.bat')}</span>

  <!-- Voltage bar — only shown when battery % is available from FC -->
  {#if pct > 0}
  <div class="bat-bar-track">
    <div class="bat-bar-fill" style="width: {pct}%; background: {barColor}"></div>
  </div>
  {/if}

  <span class="w-value">{voltage}</span>
  <span class="w-secondary">{current}</span>
  <span class="w-tertiary">{mah}</span>
</div>

<style>
  .widget-card {
    width: var(--ws);
    height: var(--ws);
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: flex-start;
    background: rgba(30, 30, 30, 0.75);
    backdrop-filter: blur(10px);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: calc(var(--ws) * 0.08);
    gap: calc(var(--ws) * 0.02);
    box-sizing: border-box;
    padding: calc(var(--ws) * 0.05) calc(var(--ws) * 0.06) calc(var(--ws) * 0.04);
  }
  .w-label {
    font-size: calc(var(--ws) * 0.13);
    font-weight: 600;
    color: #37a8db;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .w-value {
    font-size: calc(var(--ws) * 0.24);
    font-weight: 700;
    color: #e0e0e0;
    font-variant-numeric: tabular-nums;
    line-height: 1.1;
    margin-top: calc(var(--ws) * 0.01);
  }
  .w-secondary {
    font-size: calc(var(--ws) * 0.13);
    color: #ccc;
    font-variant-numeric: tabular-nums;
  }
  .w-tertiary {
    font-size: calc(var(--ws) * 0.11);
    color: #888;
    font-variant-numeric: tabular-nums;
  }

  /* Battery bar */
  .bat-bar-track {
    width: 88%;
    height: calc(var(--ws) * 0.055);
    background: rgba(255, 255, 255, 0.1);
    border-radius: calc(var(--ws) * 0.03);
    overflow: hidden;
    margin: calc(var(--ws) * 0.01) 0 calc(var(--ws) * 0.02);
  }
  .bat-bar-fill {
    height: 100%;
    border-radius: calc(var(--ws) * 0.03);
    transition: width 0.5s ease;
  }
</style>
