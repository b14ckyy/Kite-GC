<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- Weather widget: model wind at the flight height, relative to the nose. Wide: plus the next 90 min. -->
<script lang="ts">
  import { untrack } from 'svelte';
  import { t } from 'svelte-i18n';
  import type { TelemetryData } from '$lib/stores/telemetry';
  import type { InterfaceSettings } from '$lib/stores/settings';
  import { homePosition } from '$lib/stores/home';
  import { replayActive } from '$lib/stores/mission';
  import { bearing } from '$lib/utils/geo';
  import { convertSpeed, speedDigits } from '$lib/utils/units';
  import { watchWindAloft, windAloft } from '$lib/controllers/windAloft';
  import { currentStep, weatherKind, windAt } from '$lib/helpers/windAloft';
  import WeatherIcon from './WeatherIcon.svelte';

  let { telem, interfaceSettings, width, height, small = false }: {
    telem: TelemetryData;
    interfaceSettings: InterfaceSettings;
    width: number;
    height: number;
    /** The small square: rose and speed only. */
    small?: boolean;
  } = $props();

  $effect(() => watchWindAloft());

  let mode = $derived<'S' | 'L' | 'W'>(small ? 'S' : width > height * 1.5 ? 'W' : 'L');

  let now = $state(Date.now());
  $effect(() => {
    const id = setInterval(() => (now = Date.now()), 30_000);
    return () => clearInterval(id);
  });

  // Every change repaints the whole dock: the rose follows the nose in steps, the height in 2 m.
  const YAW_STEP_DEG = 5;
  let yawShown = $state(0);
  $effect(() => {
    const y = telem.yaw;
    untrack(() => {
      if (Math.abs(((y - yawShown + 540) % 360) - 180) > YAW_STEP_DEG) yawShown = y;
    });
  });
  // Height above home stands in for height above ground: the model cell is kilometres wide.
  let heightM = $derived(telem.lastUpdate ? Math.max(0, Math.round(telem.altitude / 2) * 2) : 0);

  const STALE_MS = 30 * 60_000;
  let data = $derived($replayActive ? null : $windAloft.data);
  let stale = $derived(!!data && now - $windAloft.fetchedAt > STALE_MS);
  let stepIdx = $derived(data ? currentStep(data.steps, now) : 0);
  let step = $derived(data ? data.steps[stepIdx] : null);
  let wind = $derived(step ? windAt(step.levels, heightM) : null);
  let kind = $derived(step ? weatherKind(step.weatherCode) : null);

  const CALM_MS = 0.3;
  const rel = (dirFrom: number) => Math.round((dirFrom + 180 - yawShown + 720) % 360);
  const speedText = (ms: number) => convertSpeed(ms, interfaceSettings.speedUnit).value.toFixed(speedDigits(interfaceSettings.speedUnit));
  let unit = $derived(convertSpeed(0, interfaceSettings.speedUnit).unit);

  /** The steps 30 / 60 / 90 minutes on, at the same height. */
  let forecast = $derived(
    data
      ? [2, 4, 6].flatMap((o) => {
          const s = data.steps[stepIdx + o];
          const w = s ? windAt(s.levels, heightM) : null;
          return w ? [{ min: o * 15, w }] : [];
        })
      : [],
  );

  let homeRel = $derived(
    $homePosition.set && telem.lastUpdate && (telem.lat !== 0 || telem.lon !== 0)
      ? Math.round((bearing(telem.lat, telem.lon, $homePosition.lat, $homePosition.lon) - yawShown + 720) % 360)
      : null,
  );

  // Height scale: ground at the bottom, SCALE_TOP_M at the tip of its arrow head.
  const SCALE_TOP_M = 200;
  const SCALE_X = 20;
  const SCALE_Y0 = 184;
  const SCALE_Y1 = 30;
  const DOTS_M = [30, 80, 130, 180];
  const scaleY = (m: number) => SCALE_Y0 - (m / SCALE_TOP_M) * (SCALE_Y0 - SCALE_Y1);
  let above = $derived(heightM >= SCALE_TOP_M);

  // Rose: centre and radius per layout.
  let rose = $derived(mode === 'S' ? { cx: 100, cy: 100, r: 84, k: 1.4 } : { cx: 118, cy: 122, r: 60, k: 1 });
  const TICKS = [0, 45, 90, 135, 180, 225, 270, 315];
  const ARROW = '-8,42 -8,-10 -20,-10 0,-46 20,-10 8,-10 8,42';
  const UAV = '0,-40 -22,34 0,20 22,34';

  let note = $derived(
    $replayActive || (!data && $windAloft.noPosition)
      ? $t('widgetLabels.na')
      : !data
        ? $windAloft.failed
          ? $t('widgetLabels.offline')
          : '…'
        : null,
  );
</script>

<div class="widget-card" style="width:{width}px; height:{height}px;">
  <svg {width} {height} viewBox="0 0 {mode === 'W' ? 400 : 200} 200">
    <!-- Rose: ring, the aircraft nose up, wind arrow pointing downwind, home on the rim -->
    <g transform="translate({rose.cx}, {rose.cy})">
      <circle r={rose.r} class="ring" />
      {#each TICKS as a}
        {@const major = a % 90 === 0}
        <line x1="0" y1={-rose.r - (major ? 7 : 4)} x2="0" y2={-rose.r + (major ? 7 : 4)} class="tick" transform="rotate({a})" />
      {/each}
      <polygon points={UAV} class="uav" transform="scale({rose.k})" />
      {#if wind && wind.speedMs > CALM_MS}
        <polygon points={ARROW} class="wind" class:under-text={mode === 'S'} transform="rotate({rel(wind.dirDeg)}) scale({rose.k})" />
      {/if}
      {#if homeRel != null}
        <polygon points="0,{-rose.r - 11} -7,{-rose.r + 2} 7,{-rose.r + 2}" class="home" transform="rotate({homeRel})" />
      {/if}
      {#if mode === 'S' && wind}
        <text class="speed centre" x="0" y="4" text-anchor="middle" dominant-baseline="middle">{speedText(wind.speedMs)}</text>
        <text class="unit centre" x="0" y="36" text-anchor="middle" dominant-baseline="middle">{unit}</text>
      {/if}
    </g>

    {#if mode !== 'S'}
      <!-- Height scale 0 … 200 m with the aircraft's height -->
      <line x1={SCALE_X} y1={SCALE_Y0} x2={SCALE_X} y2="33" class="scale" />
      <line x1={SCALE_X - 8} y1={SCALE_Y0} x2={SCALE_X + 8} y2={SCALE_Y0} class="scale" />
      <polygon points="{SCALE_X},20 {SCALE_X - 7},33 {SCALE_X + 7},33" class="scale-head" class:above />
      {#each DOTS_M as m}
        <circle cx={SCALE_X} cy={scaleY(m)} r="4.5" class="scale-dot" />
      {/each}
      {#if telem.lastUpdate && !above}
        <polygon points="3,-6 3,6 13,0" class="marker" transform="translate(0, {scaleY(heightM)})" />
      {/if}

      {#if wind}
        <text x="36" y="32" class="speed">{speedText(wind.speedMs)}<tspan class="unit" dx="4">{unit}</tspan></text>
        {#if step?.gustMs != null}
          <text x="37" y="49" class="gust">{$t('widgetLabels.gust', { values: { value: speedText(step.gustMs) } })}</text>
        {/if}
      {/if}
      {#if kind && step}
        <WeatherIcon {kind} day={step.isDay} {stale} x={mode === 'W' ? 150 : 154} y={5} />
      {/if}
    {/if}

    {#if mode === 'W'}
      <line x1="200" y1="14" x2="200" y2="186" class="divider" />
      {#each forecast as f, i}
        <g transform="translate({234 + i * 66}, 0)">
          <text x="0" y="30" text-anchor="middle" class="fc-time">{$t('widgetLabels.inMin', { values: { n: f.min } })}</text>
          <circle cy="96" r="27" class="ring thin" />
          {#if f.w.speedMs > CALM_MS}
            <polygon points={ARROW} class="wind" transform="translate(0, 96) rotate({rel(f.w.dirDeg)}) scale(0.52)" />
          {/if}
          <text x="0" y="158" text-anchor="middle" class="fc-speed">{speedText(f.w.speedMs)}</text>
          <text x="0" y="177" text-anchor="middle" class="fc-unit">{unit}</text>
        </g>
      {/each}
    {/if}

    {#if note}
      <text x={rose.cx} y={rose.cy} text-anchor="middle" dominant-baseline="middle" class="note">{note}</text>
    {/if}
  </svg>
</div>

<style>
  .widget-card {
    box-sizing: border-box;
    background: rgba(30, 30, 30, 0.75);
    backdrop-filter: var(--glass-blur, blur(6px));
    -webkit-backdrop-filter: var(--glass-blur, blur(6px));
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 8px;
    overflow: hidden;
  }
  svg {
    display: block;
    font-family: sans-serif;
  }
  .ring {
    fill: none;
    stroke: rgba(255, 255, 255, 0.4);
    stroke-width: 2.5;
  }
  .ring.thin {
    stroke: rgba(255, 255, 255, 0.2);
    stroke-width: 1.5;
  }
  .tick {
    stroke: rgba(255, 255, 255, 0.55);
    stroke-width: 2.5;
    stroke-linecap: round;
  }
  .uav {
    fill: rgba(255, 255, 255, 0.16);
  }
  .wind {
    fill: #37a8db;
  }
  .wind.under-text {
    opacity: 0.75;
  }
  .home {
    fill: #f5a623;
  }
  .scale {
    stroke: #e0e0e0;
    stroke-width: 2.5;
    stroke-linecap: round;
  }
  .scale-head {
    fill: #2a2a2a;
    stroke: #e0e0e0;
    stroke-width: 2.5;
    stroke-linejoin: round;
  }
  .scale-head.above {
    fill: #37a8db;
    stroke: #37a8db;
  }
  .scale-dot {
    fill: #2a2a2a;
    stroke: #e0e0e0;
    stroke-width: 2.5;
  }
  .marker {
    fill: #37a8db;
  }
  .speed {
    fill: #e0e0e0;
    font-size: 27px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }
  .speed.centre {
    font-size: 46px;
    paint-order: stroke;
    stroke: rgba(20, 20, 20, 0.85);
    stroke-width: 5px;
    stroke-linejoin: round;
  }
  .unit {
    fill: #949494;
    font-size: 14px;
    font-weight: 600;
  }
  .unit.centre {
    fill: #e0e0e0;
    font-size: 19px;
    paint-order: stroke;
    stroke: rgba(20, 20, 20, 0.85);
    stroke-width: 4px;
    stroke-linejoin: round;
  }
  .gust {
    fill: #949494;
    font-size: 13px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .divider {
    stroke: rgba(255, 255, 255, 0.12);
    stroke-width: 1;
  }
  .fc-time {
    fill: #949494;
    font-size: 13px;
    font-weight: 600;
  }
  .fc-speed {
    fill: #e0e0e0;
    font-size: 23px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }
  .fc-unit {
    fill: #949494;
    font-size: 12px;
    font-weight: 600;
  }
  .note {
    fill: #777;
    font-size: 22px;
    font-weight: 600;
    paint-order: stroke;
    stroke: rgba(20, 20, 20, 0.85);
    stroke-width: 4px;
  }
</style>
