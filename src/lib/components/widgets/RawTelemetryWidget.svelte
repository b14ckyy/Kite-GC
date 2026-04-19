<!-- Raw Telemetry widget — compact numeric readouts -->
<script lang="ts">
  import type { TelemetryData } from "$lib/stores/telemetry";
  import type { InterfaceSettings } from "$lib/stores/settings";
  import { convertAltitude, convertSpeed, convertVerticalSpeed, formatConverted } from "$lib/utils/units";
  import { t } from 'svelte-i18n';

  let {
    telem,
    size = 9,
    interfaceSettings = { speedUnit: 'kmh', altitudeUnit: 'm', distanceUnit: 'metric', verticalSpeedUnit: 'ms', temperatureUnit: 'c' },
  }: {
    telem: TelemetryData;
    size?: number;
    interfaceSettings?: InterfaceSettings;
  } = $props();

  let altText = $derived(formatConverted(convertAltitude(telem.altitude, interfaceSettings.altitudeUnit), 1));
  let speedText = $derived(formatConverted(convertSpeed(telem.groundSpeed, interfaceSettings.speedUnit), 0));
  let varioText = $derived(() => {
    const converted = convertVerticalSpeed(telem.vario, interfaceSettings.verticalSpeedUnit);
    return `${telem.vario >= 0 ? '+' : ''}${converted.value.toFixed(1)} ${converted.unit}`;
  });

  function getGpsFixLabel(): string {
    if (!telem.lastUpdate || telem.fixType === 0) return $t('gps.noFix');
    const types: Record<number, string> = { 1: $t('gps.fix2d'), 2: $t('gps.fix3d'), 3: $t('gps.fix3dDgps') };
    return types[telem.fixType] || `FIX ${telem.fixType}`;
  }
</script>

<div class="widget-card" style="--ws: {size}vmin">
  {#if telem.lastUpdate > 0}
    <div class="rt-row"><span class="rtk">ALT</span><span class="rtv">{altText}</span></div>
    <div class="rt-row"><span class="rtk">SPD</span><span class="rtv">{speedText}</span></div>
    <div class="rt-row"><span class="rtk">VRT</span><span class="rtv">{varioText()}</span></div>
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
    <span class="rt-nodata">{$t('rawTelemetry.noData')}</span>
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
