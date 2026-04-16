<!-- Speed widget — ground speed + optional airspeed -->
<script lang="ts">
  import type { TelemetryData } from "$lib/stores/telemetry";

  let { telem, size = 9 }: { telem: TelemetryData; size?: number } = $props();

  let speed = $derived(telem.lastUpdate ? (telem.groundSpeed * 3.6).toFixed(0) : '—');
  let airspeed = $derived(
    telem.lastUpdate && telem.airspeed > 0
      ? `AS ${(telem.airspeed * 3.6).toFixed(0)}`
      : null
  );
</script>

<div class="widget-card" style="--ws: {size}vmin">
  <span class="w-label">SPD</span>
  <span class="w-value">{speed}</span>
  <span class="w-unit">km/h</span>
  {#if airspeed}
    <span class="w-secondary">{airspeed}</span>
  {/if}
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
  .w-secondary {
    font-size: calc(var(--ws) * 0.12);
    color: #aaa;
  }
</style>
