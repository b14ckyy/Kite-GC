<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- Compass widget — rotating compass rose with heading display -->
<script lang="ts">
  import type { TelemetryData } from "$lib/stores/telemetry";
  import { t } from 'svelte-i18n';

  let { telem, size = 22.5 }: { telem: TelemetryData; size?: number } = $props();

  let rotation = $derived(-telem.yaw);

  // Course-over-ground (track) bug. COG is noise at standstill, so only show it while moving.
  const COG_MIN_SPEED = 1.5; // m/s
  let cogShown = $derived(!!telem.lastUpdate && telem.groundSpeed > COG_MIN_SPEED);

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

    <!-- Fixed heading pointer (top) -->
    <polygon points="100,2 92,20 108,20" fill="#37a8db" />

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
