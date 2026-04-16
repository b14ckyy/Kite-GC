<!-- Altitude widget — altitude + vario indicator -->
<script lang="ts">
  import type { TelemetryData } from "$lib/stores/telemetry";

  let { telem, size = 9 }: { telem: TelemetryData; size?: number } = $props();

  let alt = $derived(telem.lastUpdate ? telem.altitude.toFixed(1) : '—');
  let varioText = $derived(() => {
    if (!telem.lastUpdate) return '— m/s';
    const arrow = telem.vario > 0.1 ? '▲' : telem.vario < -0.1 ? '▼' : '•';
    const sign = telem.vario >= 0 ? '+' : '';
    return `${arrow} ${sign}${telem.vario.toFixed(1)}`;
  });
  let varioPositive = $derived(telem.vario > 0.1);
  let varioNegative = $derived(telem.vario < -0.1);
</script>

<div class="widget-card" style="--ws: {size}vmin">
  <span class="w-label">ALT</span>
  <span class="w-value">{alt}</span>
  <span class="w-unit">m</span>
  <span class="w-vario" class:vario-up={varioPositive} class:vario-down={varioNegative}>
    {varioText()}
  </span>
</div>

<style>
  .widget-card {
    width: var(--ws);
    height: var(--ws);
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    background: rgba(30, 30, 30, 0.75);
    backdrop-filter: blur(10px);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: calc(var(--ws) * 0.08);
    gap: calc(var(--ws) * 0.02);
    box-sizing: border-box;
  }
  .w-label {
    font-size: calc(var(--ws) * 0.13);
    font-weight: 600;
    color: #37a8db;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .w-value {
    font-size: calc(var(--ws) * 0.3);
    font-weight: 700;
    color: #e0e0e0;
    font-variant-numeric: tabular-nums;
    line-height: 1.1;
  }
  .w-unit {
    font-size: calc(var(--ws) * 0.11);
    color: #888;
  }
  .w-vario {
    font-size: calc(var(--ws) * 0.13);
    font-weight: 600;
    color: #aaa;
    font-variant-numeric: tabular-nums;
  }
  .vario-up {
    color: #27ae60;
  }
  .vario-down {
    color: #e74c3c;
  }
</style>
