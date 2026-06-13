<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- Artificial Horizon Indicator — circular SVG with pitch/roll animation -->
<script lang="ts">
  import type { TelemetryData } from "$lib/stores/telemetry";

  let { telem, size = 22.5 }: { telem: TelemetryData; size?: number } = $props();

  const PITCH_SCALE = 3; // SVG units per degree of pitch

  // INAV: negative pitch = nose up, so negate for display
  let pitchPx = $derived(-telem.pitch * PITCH_SCALE);
  let rollDeg = $derived(-telem.roll);

  // Pitch ladder marks: major every 10°, minor every 5°
  const pitchMarks: { deg: number; hw: number; label: boolean }[] = [];
  for (let d = -30; d <= 30; d += 5) {
    if (d === 0) continue;
    pitchMarks.push({
      deg: d,
      hw: d % 10 === 0 ? 22 : 12,
      label: d % 10 === 0,
    });
  }

  // Roll scale ticks (fixed on bezel)
  const rollTicks = [
    { angle: -60, len: 8 }, { angle: -45, len: 8 },
    { angle: -30, len: 12 }, { angle: -20, len: 8 },
    { angle: -10, len: 8 },
    { angle: 10, len: 8 }, { angle: 20, len: 8 },
    { angle: 30, len: 12 }, { angle: 45, len: 8 },
    { angle: 60, len: 8 },
  ];
</script>

<div class="ahi-container" style="--ahi-size: {size}">
  <svg viewBox="0 0 200 200" xmlns="http://www.w3.org/2000/svg">
    <defs>
      <clipPath id="ahi-clip">
        <circle cx="100" cy="100" r="88" />
      </clipPath>
      <linearGradient id="ahi-sky" x1="0" y1="0" x2="0" y2="1">
        <stop offset="0%" stop-color="#0a2e52" />
        <stop offset="100%" stop-color="#2980b9" />
      </linearGradient>
      <linearGradient id="ahi-gnd" x1="0" y1="0" x2="0" y2="1">
        <stop offset="0%" stop-color="#8B6914" />
        <stop offset="100%" stop-color="#3d2b1f" />
      </linearGradient>
    </defs>

    <!-- Horizon: clipped circle, rotated by roll, shifted by pitch -->
    <g clip-path="url(#ahi-clip)">
      <g transform="rotate({rollDeg}, 100, 100)">
        <!-- Sky -->
        <rect x="-200" y={-400 + pitchPx} width="600" height="500"
              fill="url(#ahi-sky)" />
        <!-- Ground -->
        <rect x="-200" y={100 + pitchPx} width="600" height="500"
              fill="url(#ahi-gnd)" />
        <!-- Horizon line -->
        <line x1="-200" y1={100 + pitchPx} x2="400" y2={100 + pitchPx}
              stroke="rgba(255,255,255,0.8)" stroke-width="1.5" />

        <!-- Pitch ladder -->
        {#each pitchMarks as pm}
          {@const y = 100 - pm.deg * PITCH_SCALE + pitchPx}
          <line x1={100 - pm.hw} y1={y} x2={100 + pm.hw} y2={y}
                stroke="white" stroke-width={pm.label ? 1.2 : 0.8}
                opacity={pm.label ? 0.8 : 0.5} />
          {#if pm.label}
            <text x={100 + pm.hw + 4} y={y + 4}
                  fill="white" font-size="14" opacity="0.7"
                  font-family="sans-serif">{Math.abs(pm.deg)}</text>
            <text x={100 - pm.hw - 4} y={y + 4}
                  fill="white" font-size="14" opacity="0.7"
                  font-family="sans-serif" text-anchor="end">{Math.abs(pm.deg)}</text>
          {/if}
        {/each}
      </g>
    </g>

    <!-- Roll scale ticks (fixed on bezel) -->
    {#each rollTicks as rt}
      {@const rad = (rt.angle - 90) * Math.PI / 180}
      {@const x1 = 100 + 88 * Math.cos(rad)}
      {@const y1 = 100 + 88 * Math.sin(rad)}
      {@const x2 = 100 + (88 - rt.len) * Math.cos(rad)}
      {@const y2 = 100 + (88 - rt.len) * Math.sin(rad)}
      <line {x1} {y1} {x2} {y2} stroke="white" stroke-width="1.5" opacity="0.5" />
    {/each}

    <!-- Fixed roll index (zero-bank reference) — points DOWN at the roll pointer,
         so it reads distinctly from the live sky pointer (they converge when level). -->
    <polygon points="100,13 96,5 104,5" fill="white" />

    <!-- Roll pointer (rotates with aircraft) -->
    <g transform="rotate({rollDeg}, 100, 100)">
      <polygon points="100,14 97,22 103,22" fill="white" />
    </g>

    <!-- Bezel ring -->
    <circle cx="100" cy="100" r="90" fill="none"
            stroke="rgba(255,255,255,0.12)" stroke-width="3" />

    <!-- Center aircraft symbol (fixed) -->
    <line x1="50" y1="100" x2="82" y2="100"
          stroke="#f0c040" stroke-width="3" stroke-linecap="round" />
    <line x1="118" y1="100" x2="150" y2="100"
          stroke="#f0c040" stroke-width="3" stroke-linecap="round" />
    <circle cx="100" cy="100" r="4" fill="none"
            stroke="#f0c040" stroke-width="2.5" />
    <line x1="100" y1="104" x2="100" y2="112"
          stroke="#f0c040" stroke-width="2.5" stroke-linecap="round" />
  </svg>
</div>

<style>
  .ahi-container {
    width: calc(var(--ahi-size, 150) * 1px);
    height: calc(var(--ahi-size, 150) * 1px);
  }

  svg {
    width: 100%;
    height: 100%;
    border-radius: 50%;
    background: rgba(0, 0, 0, 0.3);
    box-shadow: 0 0 10px rgba(0, 0, 0, 0.5);
  }
</style>
