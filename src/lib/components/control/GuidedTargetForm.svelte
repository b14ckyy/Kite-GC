<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // Guided "fly here" form — mounted into the Leaflet map popup (vehicle control). Vehicle-aware
  // fields (Copter/VTOL: alt + heading · Plane: alt + loiter radius), backed by the shared
  // `guidedParams` store so the last values persist for the next click. See VEHICLE_CONTROL.md.
  import { t } from 'svelte-i18n';
  import NumberStepper from '$lib/components/NumberStepper.svelte';
  import { guidedParams, type GuidedParams } from '$lib/controllers/vehicleControl';

  let {
    lat,
    lon,
    multirotor,
    onfly,
  }: {
    lat: number;
    lon: number;
    multirotor: boolean;
    /** Fired with the resolved params when "Fly Here" is pressed. */
    onfly: (lat: number, lon: number, p: GuidedParams) => void;
  } = $props();

  let alt = $state($guidedParams.alt);
  let yaw = $state<number>($guidedParams.yaw ?? NaN);
  let radius = $state<number>($guidedParams.loiterRadius ?? NaN);

  function fly() {
    const p: GuidedParams = {
      alt,
      speed: $guidedParams.speed,
      yaw: multirotor ? (Number.isNaN(yaw) ? null : yaw) : $guidedParams.yaw,
      loiterRadius: multirotor ? $guidedParams.loiterRadius : (Number.isNaN(radius) ? null : radius),
    };
    guidedParams.set(p);
    onfly(lat, lon, p);
  }
</script>

<div class="gtf">
  <div class="gtf-coords">{lat.toFixed(6)}, {lon.toFixed(6)}</div>
  <div class="gtf-fields">
    <NumberStepper bind:value={alt} min={1} max={1000} step={5} label={$t('control.alt')} unit="m" />
    {#if multirotor}
      <NumberStepper bind:value={yaw} min={0} max={359} step={5} label={$t('control.heading')} unit="°" allowEmpty placeholder="—" />
    {:else}
      <NumberStepper bind:value={radius} min={0} max={2000} step={10} label={$t('control.loiterRadius')} unit="m" allowEmpty placeholder="—" />
    {/if}
  </div>
  <button class="gtf-fly" onclick={fly}>{$t('control.flyHere')}</button>
</div>

<style>
  .gtf {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 168px;
  }
  .gtf-coords {
    font-size: 11px;
    color: #949494;
  }
  .gtf-fields {
    display: flex;
    gap: 10px;
  }
  .gtf-fly {
    width: 100%;
    height: 30px;
    border: 1px solid #37a8db;
    border-radius: 5px;
    background: rgba(55, 168, 219, 0.18);
    color: #cfe8f4;
    font-weight: 700;
    font-size: 12.5px;
    cursor: pointer;
  }
  .gtf-fly:hover { background: rgba(55, 168, 219, 0.32); }
</style>
