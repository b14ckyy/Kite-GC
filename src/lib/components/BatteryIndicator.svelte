<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- Toolbar battery indicator (right of the sensor bar): the primary battery (instance 0). When the FC
     reports a native charge % the glyph fills by it and the % is shown; otherwise the voltage is shown
     (no guessed %). Voltage / cell count / mAh drawn on hover. -->
<script lang="ts">
  import type { TelemetryData } from '$lib/stores/telemetry';
  import { batteryLevel, batteryColor } from '$lib/helpers/battery';

  let { telem }: { telem: TelemetryData } = $props();

  const batt = $derived(batteryLevel(telem));
  const pct = $derived(batt && batt.percent != null ? batt.percent : null);
  const fillW = $derived(pct != null ? (16 * pct) / 100 : 0);
  const color = $derived(pct != null ? batteryColor(pct) : '#cfcfcf');
  const tooltip = $derived(
    batt ? `${batt.voltage.toFixed(1)} V · ${batt.cells}S · ${Math.round(batt.mAhDrawn)} mAh` : '',
  );
</script>

{#if batt}
  <div class="battery" title={tooltip}>
    <svg viewBox="0 0 24 12" class="batt-glyph" aria-hidden="true">
      <rect x="0.5" y="1.5" width="20" height="9" rx="1.5" fill="none" stroke="#888" stroke-width="1" />
      <rect x="21.2" y="4" width="2.3" height="4" rx="0.6" fill="#888" />
      {#if pct != null}
        <rect x="2" y="3" width={fillW} height="6" rx="0.6" fill={color} />
      {/if}
    </svg>
    <span class="pct" style="color: {color}">
      {#if pct != null}{pct}%{:else}{batt.voltage.toFixed(1)}V{/if}
    </span>
  </div>
{/if}

<style>
  .battery {
    display: flex;
    align-items: center;
    gap: 7px;
    height: 26px;
    cursor: default;
  }
  .batt-glyph {
    width: 48px;
    height: 24px;
    flex: none;
  }
  .pct {
    font-size: 15px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    min-width: 42px;
    text-align: right;
  }
</style>
