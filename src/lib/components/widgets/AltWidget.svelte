<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- Altitude widget — altitude + vario indicator -->
<script lang="ts">
  import type { TelemetryData } from "$lib/stores/telemetry";
  import type { InterfaceSettings } from "$lib/stores/settings";
  import { convertAltitude, convertVerticalSpeed } from "$lib/utils/units";
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

  let altConv = $derived(convertAltitude(telem.altitude, interfaceSettings.altitudeUnit));
  let varioConv = $derived(convertVerticalSpeed(telem.vario, interfaceSettings.verticalSpeedUnit));
  let alt = $derived(telem.lastUpdate ? altConv.value.toFixed(1) : '—');
  let varioText = $derived(() => {
    if (!telem.lastUpdate) return `— ${varioConv.unit}`;
    const arrow = telem.vario > 0.1 ? '▲' : telem.vario < -0.1 ? '▼' : '•';
    const sign = varioConv.value >= 0 ? '+' : '';
    return `${arrow} ${sign}${varioConv.value.toFixed(1)} ${varioConv.unit}`;
  });
  let varioPositive = $derived(telem.vario > 0.1);
  let varioNegative = $derived(telem.vario < -0.1);
</script>

<div class="widget-card" style="--ws: {size}px">
  <span class="w-label">{$t('widgetLabels.alt')}</span>
  <span class="w-value">{alt}</span>
  <span class="w-unit">{altConv.unit}</span>
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
    font-size: calc(var(--ws) * 0.3);
    font-weight: 700;
    color: #e0e0e0;
    font-variant-numeric: tabular-nums;
    line-height: 1.1;
    margin-top: calc(var(--ws) * 0.02);
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
