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

  // Airspeed is the primary readout when available (it's what matters for flight); ground speed then
  // drops to the secondary line. Without airspeed, ground speed stays primary.
  let hasAir = $derived(telem.lastUpdate && telem.airspeed > 0);
  let gsConv = $derived(convertSpeed(telem.groundSpeed, interfaceSettings.speedUnit));
  let asConv = $derived(convertSpeed(telem.airspeed, interfaceSettings.speedUnit));

  let label = $derived(hasAir ? $t('widgetLabels.aspd') : $t('widgetLabels.spd'));
  let primaryConv = $derived(hasAir ? asConv : gsConv);
  let speed = $derived(telem.lastUpdate ? primaryConv.value.toFixed(0) : '—');
  let secondary = $derived(
    hasAir
      ? $t('widgetLabels.gs', { values: { value: `${gsConv.value.toFixed(0)} ${gsConv.unit}` } })
      : null
  );
</script>

<div class="widget-card" style="--ws: {size}px">
  <span class="w-label">{label}</span>
  <span class="w-value">{speed}</span>
  <span class="w-unit">{primaryConv.unit}</span>
  {#if secondary}
    <span class="w-secondary">{secondary}</span>
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
