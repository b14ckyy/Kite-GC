<!-- Raw Telemetry widget — compact numeric readouts -->
<script lang="ts">
  import type { TelemetryData } from "$lib/stores/telemetry";

  let { telem, size = 9 }: { telem: TelemetryData; size?: number } = $props();

  function getGpsFixLabel(): string {
    if (!telem.lastUpdate || telem.fixType === 0) return 'NO FIX';
    const types: Record<number, string> = { 1: '2D', 2: '3D', 3: '3D DGPS' };
    return types[telem.fixType] || `FIX ${telem.fixType}`;
  }
</script>

<div class="widget-card" style="--ws: {size}vmin">
  {#if telem.lastUpdate > 0}
    <div class="rt-row"><span class="rtk">ALT</span><span class="rtv">{telem.altitude.toFixed(1)} m</span></div>
    <div class="rt-row"><span class="rtk">SPD</span><span class="rtv">{(telem.groundSpeed * 3.6).toFixed(0)} km/h</span></div>
    <div class="rt-row"><span class="rtk">VRT</span><span class="rtv">{telem.vario >= 0 ? '+' : ''}{telem.vario.toFixed(1)} m/s</span></div>
    <div class="rt-row"><span class="rtk">HDG</span><span class="rtv">{Math.round(telem.yaw)}°</span></div>
    <div class="rt-row"><span class="rtk">ROL</span><span class="rtv">{telem.roll.toFixed(1)}°</span></div>
    <div class="rt-row"><span class="rtk">PIT</span><span class="rtv">{telem.pitch.toFixed(1)}°</span></div>
    <div class="rt-row"><span class="rtk">BAT</span><span class="rtv">{telem.voltage.toFixed(1)}V</span></div>
    <div class="rt-row"><span class="rtk">CUR</span><span class="rtv">{telem.current.toFixed(1)}A</span></div>
    <div class="rt-row"><span class="rtk">MAH</span><span class="rtv">{telem.mAhDrawn}</span></div>
    <div class="rt-row"><span class="rtk">SAT</span><span class="rtv">{telem.numSat} {getGpsFixLabel()}</span></div>
    <div class="rt-row"><span class="rtk">RSSI</span><span class="rtv">{telem.rssi}</span></div>
    <div class="rt-row"><span class="rtk">CPU</span><span class="rtv">{telem.cpuLoad}%</span></div>
  {:else}
    <span class="rt-nodata">NO DATA</span>
  {/if}
</div>

<style>
  .widget-card {
    width: var(--ws);
    min-height: var(--ws);
    display: flex;
    flex-direction: column;
    justify-content: center;
    padding: calc(var(--ws) * 0.06);
    background: rgba(30, 30, 30, 0.75);
    backdrop-filter: blur(10px);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: calc(var(--ws) * 0.08);
    gap: calc(var(--ws) * 0.01);
    box-sizing: border-box;
  }
  .rt-row {
    display: flex;
    justify-content: space-between;
    padding: calc(var(--ws) * 0.01) calc(var(--ws) * 0.03);
  }
  .rtk {
    font-size: calc(var(--ws) * 0.09);
    font-weight: 600;
    color: #37a8db;
  }
  .rtv {
    font-size: calc(var(--ws) * 0.09);
    color: #ccc;
    font-variant-numeric: tabular-nums;
  }
  .rt-nodata {
    font-size: calc(var(--ws) * 0.12);
    color: #555;
    font-weight: 600;
    text-align: center;
    padding: calc(var(--ws) * 0.2) 0;
  }
</style>
