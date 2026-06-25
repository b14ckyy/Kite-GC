<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- Compass widget — rotating compass rose with heading display -->
<script lang="ts">
  import type { TelemetryData } from "$lib/stores/telemetry";
  import type { InterfaceSettings } from "$lib/stores/settings";
  import { convertSpeed } from "$lib/utils/units";
  import { t } from 'svelte-i18n';

  let {
    telem,
    size = 22.5,
    interfaceSettings = { speedUnit: 'kmh', altitudeUnit: 'm', distanceUnit: 'metric', verticalSpeedUnit: 'ms', temperatureUnit: 'c' },
  }: {
    telem: TelemetryData;
    size?: number;
    interfaceSettings?: InterfaceSettings;
  } = $props();

  let rotation = $derived(-telem.yaw);

  // Course-over-ground (track) bug. COG is noise at standstill, so only show it while moving.
  const COG_MIN_SPEED = 1.5; // m/s
  let cogShown = $derived(!!telem.lastUpdate && telem.groundSpeed > COG_MIN_SPEED);

  // Wind arrow (translucent, pinned to the rose rim): fixed-size arrow whose tip sits at the rim,
  // pointing downwind (drift direction). Speed is shown as a number below the heading instead of via
  // length (kept legible at low wind). Hidden when calm / no estimate.
  const WIND_MIN_MS = 0.3;
  let windShown = $derived(!!telem.lastUpdate && telem.windSpeedMs > WIND_MIN_MS);
  let windDownwind = $derived((telem.windDirFrom + 180) % 360);
  let windSpeedConv = $derived(convertSpeed(telem.windSpeedMs, interfaceSettings.speedUnit));

  function cardinalLabel(heading: number): string {
    const dirs = ['N', 'NE', 'E', 'SE', 'S', 'SW', 'W', 'NW'];
    return dirs[Math.round(heading / 45) % 8];
  }

  // Tick marks every 10°
  const ticks: { deg: number; major: boolean }[] = [];
  for (let i = 0; i < 360; i += 10) {
    ticks.push({ deg: i, major: i % 30 === 0 });
  }
</script>

<div class="compass-container" style="--ws: {size}px">
  <svg viewBox="0 0 200 200" xmlns="http://www.w3.org/2000/svg">
    <!-- Rotating compass card -->
    <g transform="rotate({rotation}, 100, 100)">
      <!-- Wind arrow (behind the ticks/labels): fixed-size, tip pinned near the rim, pointing downwind.
           Nested in the rose + rotated by the downwind bearing → lands at the true wind direction
           relative to the nose. Speed is read out as a number below the heading. -->
      {#if windShown}
        <g transform="rotate({windDownwind}, 100, 100)" opacity="0.45">
          <line x1="100" y1="58" x2="100" y2="30" stroke="#7fd4ff" stroke-width="6" stroke-linecap="round" />
          <polygon points="100,16 90,34 110,34" fill="#7fd4ff" />
        </g>
      {/if}
      {#each ticks as t}
        {@const rad = (t.deg - 90) * Math.PI / 180}
        {@const inner = t.major ? 68 : 76}
        {@const x1 = 100 + 84 * Math.cos(rad)}
        {@const y1 = 100 + 84 * Math.sin(rad)}
        {@const x2 = 100 + inner * Math.cos(rad)}
        {@const y2 = 100 + inner * Math.sin(rad)}
        <line {x1} {y1} {x2} {y2}
              stroke="white" stroke-width={t.major ? 2 : 1}
              opacity={t.major ? 0.8 : 0.35} />
      {/each}

      <!-- Cardinal labels -->
      <text x="100" y="50" text-anchor="middle" dominant-baseline="middle"
            fill="#ff4444" font-size="25" font-weight="bold" font-family="sans-serif">{$t('compass.n')}</text>
      <text x="152" y="104" text-anchor="middle" dominant-baseline="middle"
            fill="white" font-size="22" font-weight="600" opacity="0.8" font-family="sans-serif">{$t('compass.e')}</text>
      <text x="100" y="155" text-anchor="middle" dominant-baseline="middle"
            fill="white" font-size="22" font-weight="600" opacity="0.8" font-family="sans-serif">{$t('compass.s')}</text>
      <text x="48" y="104" text-anchor="middle" dominant-baseline="middle"
            fill="white" font-size="22" font-weight="600" opacity="0.8" font-family="sans-serif">{$t('compass.w')}</text>

      <!-- Course-over-ground track bug: rides the rose rim at the COG bearing. Nested inside the rose
           (rotation = −yaw) and rotated by course, it lands at screen angle course−yaw; the gap to the
           fixed heading pointer at the top is the crab angle. Inward-pointing amber triangle on the rim,
           clear of the centre heading value. -->
      {#if cogShown}
        <g transform="rotate({telem.course}, 100, 100)">
          <polygon points="100,24 92,8 108,8" fill="#f5a623" opacity="0.85" />
        </g>
      {/if}
    </g>

    <!-- Fixed heading pointer (top) — white to match the heading text -->
    <polygon points="100,2 92,20 108,20" fill="#e0e0e0" />

    <!-- COG readout (amber, matches the track bug) — smaller, above the heading value -->
    {#if cogShown}
      <text x="100" y="80" text-anchor="middle" dominant-baseline="middle"
            fill="#f5a623" font-size="17" font-weight="600" font-family="sans-serif"
            style="font-variant-numeric: tabular-nums">{Math.round(telem.course)}°</text>
    {/if}

    <!-- Center heading value -->
    <text x="100" y="105" text-anchor="middle" dominant-baseline="middle"
          fill="#e0e0e0" font-size="28" font-weight="700" font-family="sans-serif"
          style="font-variant-numeric: tabular-nums">{telem.lastUpdate ? Math.round(telem.yaw) + '°' : '—'}</text>

    <!-- Wind speed readout (blue, matches the wind arrow) — below the heading. -->
    {#if windShown}
      <text x="100" y="128" text-anchor="middle" dominant-baseline="middle"
            fill="#7fd4ff" font-size="15" font-weight="600" font-family="sans-serif"
            style="font-variant-numeric: tabular-nums">{windSpeedConv.value.toFixed(0)} {windSpeedConv.unit}</text>
    {/if}

    <!-- Bezel ring -->
    <circle cx="100" cy="100" r="88" fill="none"
            stroke="rgba(255,255,255,0.12)" stroke-width="2" />
  </svg>

</div>

<style>
  .compass-container {
    display: flex;
    align-items: center;
    justify-content: center;
  }

  svg {
    width: var(--ws);
    height: var(--ws);
    border-radius: 50%;
    background: rgba(0, 0, 0, 0.5);
    box-shadow: 0 0 10px rgba(0, 0, 0, 0.5);
  }
</style>
