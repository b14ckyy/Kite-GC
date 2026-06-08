<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- Speed widget — ground speed + optional airspeed -->
<script lang="ts">
  import type { TelemetryData } from "$lib/stores/telemetry";
  import type { InterfaceSettings } from "$lib/stores/settings";
  import { convertSpeed } from "$lib/utils/units";
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

  let speedConv = $derived(convertSpeed(telem.groundSpeed, interfaceSettings.speedUnit));
  let speed = $derived(telem.lastUpdate ? speedConv.value.toFixed(0) : '—');
  let airspeed = $derived(
    telem.lastUpdate && telem.airspeed > 0
      ? $t('widgetLabels.airspeed', {
          values: {
            value: `${convertSpeed(telem.airspeed, interfaceSettings.speedUnit).value.toFixed(0)} ${speedConv.unit}`,
          },
        })
      : null
  );
</script>

<div class="widget-card" style="--ws: {size}px">
  <span class="w-label">{$t('widgetLabels.spd')}</span>
  <span class="w-value">{speed}</span>
  <span class="w-unit">{speedConv.unit}</span>
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
  .w-secondary {
    font-size: calc(var(--ws) * 0.12);
    color: #aaa;
  }
</style>
