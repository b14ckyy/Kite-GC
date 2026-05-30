<script lang="ts">
  /**
   * UnitStepper.svelte
   *
   * NumberStepper that keeps its bound `value` in **metric base units**
   * (metres for altitude/length, m/s for speed) while displaying + editing in
   * the user's selected interface units. min/max/step are given in metric too.
   */
  import NumberStepper from './NumberStepper.svelte';
  import {
    convertAltitude,
    toAltitudeM,
    convertLength,
    toLengthM,
    convertSpeed,
    toSpeedMs,
    type ConvertedValue,
  } from '$lib/utils/units';
  import type { InterfaceSettings } from '$lib/stores/settings';

  type Kind = 'altitude' | 'length' | 'speed';

  let {
    value = $bindable(0),
    kind,
    settings,
    label = '',
    min,
    max,
    step = 1,
    decimals = 0,
    disabled = false,
    onchange,
  }: {
    /** Value in metric base units (m for altitude/length, m/s for speed) */
    value?: number;
    kind: Kind;
    settings: InterfaceSettings;
    label?: string;
    /** min/max/step in metric base units */
    min?: number;
    max?: number;
    step?: number;
    decimals?: number;
    disabled?: boolean;
    onchange?: (e: Event) => void;
  } = $props();

  function toDisplay(metric: number): ConvertedValue {
    if (kind === 'altitude') return convertAltitude(metric, settings.altitudeUnit);
    if (kind === 'length') return convertLength(metric, settings.distanceUnit);
    return convertSpeed(metric, settings.speedUnit);
  }
  function toMetric(display: number): number {
    if (kind === 'altitude') return toAltitudeM(display, settings.altitudeUnit);
    if (kind === 'length') return toLengthM(display, settings.distanceUnit);
    return toSpeedMs(display, settings.speedUnit);
  }

  const round = (v: number, d: number) => {
    const f = 10 ** d;
    return Math.round(v * f) / f;
  };

  const unitLabel = $derived(toDisplay(0).unit);
  const dMin = $derived(min != null ? round(toDisplay(min).value, decimals) : undefined);
  const dMax = $derived(max != null ? round(toDisplay(max).value, decimals) : undefined);
  const dStep = $derived(Math.max(1 / 10 ** decimals, round(toDisplay(step).value, decimals)));

  // Display value mirrors the metric value (and the active unit). Resyncs on
  // external changes; our own onChange round-trips back to the same display.
  // svelte-ignore state_referenced_locally
  let display = $state(round(toDisplay(value).value, decimals));
  $effect(() => {
    display = round(toDisplay(value).value, decimals);
  });

  function onChange(e: Event) {
    value = toMetric(display);
    onchange?.(e);
  }
</script>

<NumberStepper
  bind:value={display}
  {label}
  min={dMin}
  max={dMax}
  step={dStep}
  {decimals}
  unit={unitLabel}
  {disabled}
  onchange={onChange}
/>
