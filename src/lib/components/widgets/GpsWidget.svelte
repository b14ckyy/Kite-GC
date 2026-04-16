<!-- GPS widget — satellite count + fix type -->
<script lang="ts">
  import type { TelemetryData } from "$lib/stores/telemetry";

  let { telem, size = 9 }: { telem: TelemetryData; size?: number } = $props();

  let sats = $derived(telem.lastUpdate ? telem.numSat : '—');

  let fixLabel = $derived(() => {
    if (!telem.lastUpdate || telem.fixType === 0) return 'NO FIX';
    const types: Record<number, string> = { 1: '2D', 2: '3D', 3: '3D DGPS' };
    return types[telem.fixType] || `FIX ${telem.fixType}`;
  });

  let fixColor = $derived(
    !telem.lastUpdate || telem.fixType < 2 ? '#e74c3c'
      : telem.fixType === 2 ? '#f39c12'
      : '#27ae60'
  );
</script>

<div class="widget-card" style="--ws: {size}vmin">
  <span class="w-label">GPS</span>
  <span class="w-value">{sats}</span>
  <span class="w-fix" style="color: {fixColor}">{fixLabel()}</span>
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
  .w-fix {
    font-size: calc(var(--ws) * 0.12);
    font-weight: 700;
    text-transform: uppercase;
  }
</style>
