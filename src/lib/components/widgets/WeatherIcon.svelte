<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- Weather icon in a 40 × 40 box, for use inside an <svg>. -->
<svelte:options namespace="svg" />

<script lang="ts">
  import type { WeatherKind } from '$lib/helpers/windAloft';

  let { kind, day = true, stale = false, x = 0, y = 0 }: {
    kind: WeatherKind;
    day?: boolean;
    /** Grey: the data behind it is old. */
    stale?: boolean;
    x?: number;
    y?: number;
  } = $props();

  const GREY = '#6a6a6a';
  let sun = $derived(stale ? GREY : '#f2c94c');
  let moon = $derived(stale ? GREY : '#cfd8e3');
  let cloud = $derived(stale ? GREY : '#d6d6d6');
  let wet = $derived(stale ? GREY : '#7fd4ff');

  const CLOUD = 'M11,30 A7,7 0 0 1 11.5,16.2 A9,9 0 0 1 28.6,17.6 A6.2,6.2 0 0 1 29,30 Z';
  const RAYS = [0, 45, 90, 135, 180, 225, 270, 315];
  let precip = $derived(kind === 'rain' || kind === 'showers' || kind === 'snow' || kind === 'thunder' || kind === 'fog');
  let withSky = $derived(kind === 'partly' || kind === 'showers');
</script>

<g transform="translate({x}, {y})">
  {#if kind === 'clear'}
    {#if day}
      <circle cx="20" cy="20" r="7.5" fill={sun} />
      {#each RAYS as a}
        <line x1="20" y1="6" x2="20" y2="9.5" stroke={sun} stroke-width="2.4" stroke-linecap="round" transform="rotate({a}, 20, 20)" />
      {/each}
    {:else}
      <path d="M23,7 A13,13 0 1 0 33,24 A10,10 0 0 1 23,7 Z" fill={moon} />
    {/if}
  {:else}
    {#if withSky}
      {#if day}
        <circle cx="27" cy="12" r="6" fill={sun} />
        {#each RAYS as a}
          <line x1="27" y1="1.5" x2="27" y2="3.8" stroke={sun} stroke-width="2" stroke-linecap="round" transform="rotate({a}, 27, 12)" />
        {/each}
      {:else}
        <path d="M27,3 A8.5,8.5 0 1 0 35,15 A6.5,6.5 0 0 1 27,3 Z" fill={moon} />
      {/if}
    {/if}
    <path d={CLOUD} fill={cloud} transform="translate({withSky ? -3 : 0}, {precip ? -6 : withSky ? 2 : -2})" />
    {#if kind === 'rain' || kind === 'showers'}
      {#each [11, 19, 27] as rx}
        <line x1={rx + 2} y1="28" x2={rx - 1} y2="36" stroke={wet} stroke-width="2.4" stroke-linecap="round" />
      {/each}
    {:else if kind === 'snow'}
      {#each [[12, 30], [20, 34], [28, 30]] as [sx, sy]}
        <circle cx={sx} cy={sy} r="2.2" fill="#ffffff" opacity={stale ? 0.4 : 1} />
      {/each}
    {:else if kind === 'thunder'}
      <polygon points="21,24 14,33 19.5,33 17,39 26,29 20.5,29 23,24" fill={sun} />
    {:else if kind === 'fog'}
      {#each [29, 33.5, 38] as fy, i}
        <line x1={i === 1 ? 6 : 10} y1={fy} x2={i === 1 ? 30 : 34} y2={fy} stroke={cloud} stroke-width="2.2" stroke-linecap="round" opacity="0.75" />
      {/each}
    {/if}
  {/if}
</g>
