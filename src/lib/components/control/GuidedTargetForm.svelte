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
  import { guidedParams, fcLoiterRadius, type GuidedParams } from '$lib/controllers/vehicleControl';

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
  // Seed the radius from the FC's own default (WP_LOITER_RAD / NAV_LOITER_RAD, already read into
  // `fcLoiterRadius` for the loiter ring) when this session has no explicit value yet. The field used
  // to open blank, which reads as "no radius" while the vehicle will in fact use its configured one:
  // showing that number is both the honest default and the one the aircraft is about to fly.
  //
  // The seed is a *display* value only, never sent. `fcLoiterRadius` holds the magnitude, but on
  // ArduPilot the sign of WP_LOITER_RAD is the turn direction: with param3 = 0, `set_guided_WP`
  // takes radius and direction from the parameter, while a positive param3 plus param4 = NaN makes
  // `handle_command_int_do_reposition` clear `loiter_ccw` and force clockwise. Echoing the seed back
  // would therefore reverse the turn on an aircraft configured counter-clockwise. So while the field
  // still shows the untouched seed we send null and let the FC use its own value, sign included.
  const seeded = $guidedParams.loiterRadius == null && $fcLoiterRadius != null;
  let radius = $state<number>($guidedParams.loiterRadius ?? $fcLoiterRadius ?? NaN);

  function fly() {
    // Untouched seed: send nothing, and do not persist it as this session's explicit value either,
    // or it would stick across a reconnect to an aircraft with a different radius.
    const untouched = seeded && radius === $fcLoiterRadius;
    const p: GuidedParams = {
      alt,
      speed: $guidedParams.speed,
      yaw: multirotor ? (Number.isNaN(yaw) ? null : yaw) : $guidedParams.yaw,
      loiterRadius: multirotor
        ? $guidedParams.loiterRadius
        : (untouched || Number.isNaN(radius) ? null : radius),
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
    /* Wrap rather than overflow: the steppers are fixed-width, so if the popup is ever narrower
       than the pair needs, the second field drops to its own line instead of being clipped. */
    flex-wrap: wrap;
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
