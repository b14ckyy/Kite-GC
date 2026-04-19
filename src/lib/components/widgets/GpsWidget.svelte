<!-- GPS widget — satellite count + fix type -->
<script lang="ts">
  import type { TelemetryData } from "$lib/stores/telemetry";
  import { t } from 'svelte-i18n';

  let { telem, size = 9 }: { telem: TelemetryData; size?: number } = $props();

  function formatCoord(value: number, pos: string, neg: string): string {
    const hemi = value >= 0 ? pos : neg;
    return `${hemi} ${Math.abs(value).toFixed(5)}`;
  }

  let latText = $derived(
    telem.lastUpdate && telem.fixType > 0 ? formatCoord(telem.lat, 'N', 'S') : 'N —'
  );
  let lonText = $derived(
    telem.lastUpdate && telem.fixType > 0 ? formatCoord(telem.lon, 'E', 'W') : 'E —'
  );

  let sats = $derived(telem.lastUpdate ? String(telem.numSat) : '—');

  let fixLabel = $derived.by(() => {
    if (!telem.lastUpdate || telem.fixType === 0) return $t('gps.noFix');
    const types: Record<number, string> = { 1: $t('gps.fix2d'), 2: $t('gps.fix3d'), 3: $t('gps.fix3dDgps') };
    return types[telem.fixType] || `FIX ${telem.fixType}`;
  });

  let fixColor = $derived(
    !telem.lastUpdate || telem.fixType < 2 ? '#e74c3c'
      : telem.fixType === 2 ? '#f39c12'
      : '#27ae60'
  );

  let hdopText = $derived(
    telem.lastUpdate && telem.gpsHdop > 0 ? `HDOP ${telem.gpsHdop.toFixed(1)}` : 'HDOP -'
  );
</script>

<div class="widget-card" style="--ws: {size}vmin">
  <span class="w-label">{$t('sensors.gps')}</span>
  <span class="w-coord">{latText}</span>
  <span class="w-coord">{lonText}</span>
  <div class="w-meta-stack">
    <span class="w-meta w-meta-main" style="color: {fixColor}">
      {sats} SAT · {fixLabel}
    </span>
    <span class="w-meta w-meta-hdop">
      {hdopText}
    </span>
  </div>
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
    gap: calc(var(--ws) * 0.025);
    box-sizing: border-box;
    padding: calc(var(--ws) * 0.05) calc(var(--ws) * 0.05) calc(var(--ws) * 0.04);
  }
  .w-label {
    font-size: calc(var(--ws) * 0.13);
    font-weight: 600;
    color: #37a8db;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .w-coord {
    font-size: calc(var(--ws) * 0.11);
    font-weight: 600;
    color: #d8d8d8;
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.02em;
    line-height: 1.2;
    width: 100%;
    text-align: center;
  }
  .w-meta {
    font-size: calc(var(--ws) * 0.1);
    font-weight: 700;
    text-transform: uppercase;
    text-align: center;
    line-height: 1.15;
    width: 100%;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .w-meta-stack {
    margin-top: auto;
    width: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: calc(var(--ws) * 0.0125);
  }

  .w-meta-hdop {
    color: #9fb3bd;
    font-weight: 600;
  }
</style>
