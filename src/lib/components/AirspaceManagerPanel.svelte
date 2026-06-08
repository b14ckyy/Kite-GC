<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // Airspace Manager panel (aeronautical data) on the panel framework — the static counterpart to the
  // Radar panel. `advanced` two-column: left = per-layer 2D/3D visibility + cache; right = the per-layer
  // grouped nearby list (Obstacles · Airspaces · Airfields). See docs/active/AIRSPACE_MANAGER.md.
  import { onMount } from 'svelte';
  import { t } from 'svelte-i18n';
  import PanelShell from '$lib/components/panel/PanelShell.svelte';
  import Toggle from '$lib/components/panel/Toggle.svelte';
  import Button from '$lib/components/panel/Button.svelte';
  import {
    aeroData, aeroLayers, aeroCacheStats, clearAeroCache, refreshAeroCacheStats, focusAero,
    type AeroLayerKey, type Airspace,
  } from '$lib/stores/airspace';
  import { haversineDistance } from '$lib/utils/geo';
  import { aeroPointInfo } from '$lib/helpers/airspaceStyle';
  import type { DistanceUnit } from '$lib/stores/settings';
  import { convertDistance, formatConverted } from '$lib/utils/units';

  let { reference = null, distanceUnit = 'metric' }: {
    reference?: { lat: number; lon: number } | null;
    distanceUnit?: DistanceUnit;
  } = $props();

  const variant = 'advanced' as const;
  const LIST_CAP = 25;

  const LAYERS: { key: AeroLayerKey; label: () => string }[] = [
    { key: 'airspaces', label: () => $t('airspace.layerAirspaces') },
    { key: 'obstacles', label: () => $t('airspace.layerObstacles') },
    { key: 'airports', label: () => $t('airspace.layerAirports') },
    { key: 'rc', label: () => $t('airspace.layerRc') },
  ];

  function setVis(key: AeroLayerKey, dim: 'd2' | 'd3', on: boolean) {
    aeroLayers.update((l) => ({ ...l, [key]: { ...l[key], [dim]: on } }));
  }

  function distM(lat: number, lon: number): number {
    return reference ? haversineDistance(reference.lat, reference.lon, lat, lon) : Number.POSITIVE_INFINITY;
  }
  function fmtDist(m: number): string {
    return m === Number.POSITIVE_INFINITY ? '—' : formatConverted(convertDistance(m, distanceUnit), m < 10000 ? 1 : 0);
  }

  // Nearest vertex distance to an airspace (cheap approximation for sorting/list).
  function airspaceDist(a: Airspace): number {
    let best = Number.POSITIVE_INFINITY;
    for (const ring of a.outlines) {
      for (const [lon, lat] of ring) {
        const d = distM(lat, lon);
        if (d < best) best = d;
      }
    }
    return best;
  }

  const nearAirspaces = $derived(
    $aeroData.airspaces
      .map((a) => ({ a, d: airspaceDist(a) }))
      .sort((x, y) => x.d - y.d)
      .slice(0, LIST_CAP),
  );
  const nearObstacles = $derived(
    $aeroData.obstacles
      .map((p) => ({ p, d: distM(p.lat, p.lon) }))
      .sort((x, y) => x.d - y.d)
      .slice(0, LIST_CAP),
  );
  const nearAirfields = $derived(
    [...$aeroData.airports, ...$aeroData.rcAirfields]
      .map((p) => ({ p, d: distM(p.lat, p.lon) }))
      .sort((x, y) => x.d - y.d)
      .slice(0, LIST_CAP),
  );

  onMount(() => { void refreshAeroCacheStats(); });
</script>

<PanelShell {variant} title={$t('airspace.title')} detailTitle={$t('airspace.nearby')} body={controls} detail={nearbyList} />

{#snippet controls()}
  <div class="am-controls">
    <div class="am-head-row">
      <span class="am-col-label"></span>
      <span class="am-dim">2D</span>
      <span class="am-dim">3D</span>
    </div>
    {#each LAYERS as layer}
      <div class="am-row">
        <span class="am-layer">{layer.label()}</span>
        <Toggle checked={$aeroLayers[layer.key].d2} onchange={(c) => setVis(layer.key, 'd2', c)} />
        <Toggle checked={$aeroLayers[layer.key].d3} onchange={(c) => setVis(layer.key, 'd3', c)} />
      </div>
    {/each}

    <div class="am-cache">
      <span class="am-cache-label">{$t('airspace.cache')}</span>
      {#if $aeroCacheStats}
        <span class="am-cache-val">
          {$aeroCacheStats.total} · {($aeroCacheStats.approxBytes / 1024).toFixed(0)} KB
        </span>
      {:else}
        <span class="am-cache-val">—</span>
      {/if}
      <Button variant="standard" size="sm" onclick={() => clearAeroCache()}>{$t('settings.clear')}</Button>
    </div>
  </div>
{/snippet}

{#snippet nearbyList()}
  <div class="am-list">
    <!-- Obstacles -->
    <div class="am-group-head">{$t('airspace.layerObstacles')} <span class="am-group-count">{$aeroData.obstacles.length}</span></div>
    {#each nearObstacles as { p, d }}
      <button class="am-item" onclick={() => focusAero(p.lat, p.lon)}>
        <span class="am-item-name">{p.name || p.subtype}</span>
        <span class="am-item-sub">{p.subtype}{p.heightM != null ? ` · ${Math.round(p.heightM)} m` : ''}</span>
        <span class="am-item-dist">{fmtDist(d)}</span>
      </button>
    {/each}

    <!-- Airspaces -->
    <div class="am-group-head">{$t('airspace.layerAirspaces')} <span class="am-group-count">{$aeroData.airspaces.length}</span></div>
    {#each nearAirspaces as { a, d }}
      <button class="am-item" onclick={() => { const o = a.outlines[0]?.[0]; if (o) focusAero(o[1], o[0]); }}>
        <span class="am-item-name">{a.name}</span>
        <span class="am-item-sub">{a.typeName} · {a.lower.label}–{a.upper.label}</span>
        <span class="am-item-dist">{fmtDist(d)}</span>
      </button>
    {/each}

    <!-- Airfields -->
    <div class="am-group-head">{$t('airspace.layerAirfields')} <span class="am-group-count">{$aeroData.airports.length + $aeroData.rcAirfields.length}</span></div>
    {#each nearAirfields as { p, d }}
      <button class="am-item" onclick={() => focusAero(p.lat, p.lon)}>
        <span class="am-item-name">{p.name || p.subtype}</span>
        <span class="am-item-sub">{p.kind === 'rc' ? 'RC' : p.subtype}{aeroPointInfo(p) ? ` · ${aeroPointInfo(p)}` : ''}</span>
        <span class="am-item-dist">{fmtDist(d)}</span>
      </button>
    {/each}
  </div>
{/snippet}

<style>
  .am-controls { display: flex; flex-direction: column; gap: 4px; }
  .am-head-row, .am-row {
    display: grid;
    grid-template-columns: 1fr 36px 36px;
    align-items: center;
    gap: 8px;
  }
  .am-head-row { padding: 0 2px 2px; }
  .am-dim { font-size: 11px; color: #949494; text-align: center; }
  .am-layer { font-size: 13px; color: #e0e0e0; }
  .am-row { padding: 4px 2px; }

  .am-cache {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 10px;
    padding-top: 8px;
    border-top: 1px solid #272727;
  }
  .am-cache-label { font-size: 12px; color: #949494; }
  .am-cache-val { font-size: 12px; color: #e0e0e0; flex: 1; }

  .am-list { display: flex; flex-direction: column; }
  .am-group-head {
    font-size: 11px; font-weight: 700; color: #37a8db; text-transform: uppercase; letter-spacing: 0.5px;
    padding: 8px 4px 3px; position: sticky; top: 0; background: #2e2e2e; z-index: 1;
  }
  .am-group-count { color: #949494; font-weight: 600; }
  .am-item {
    display: grid;
    grid-template-columns: 1fr auto;
    grid-template-areas: "name dist" "sub dist";
    gap: 0 8px;
    width: 100%;
    text-align: left;
    background: none;
    border: none;
    border-bottom: 1px solid #272727;
    padding: 5px 4px;
    cursor: pointer;
    color: inherit;
  }
  .am-item:hover { background: rgba(55, 168, 219, 0.08); }
  .am-item-name { grid-area: name; font-size: 12px; color: #e0e0e0; }
  .am-item-sub { grid-area: sub; font-size: 10.5px; color: #949494; }
  .am-item-dist { grid-area: dist; align-self: center; font-size: 11px; color: #37a8db; white-space: nowrap; }
</style>
