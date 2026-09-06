<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- PhoneBottomChips — the phone's bottom-left corner (Dev-Docs archive/PHONE_UI.md D18): the
     arming state and the sensor chip (ONLY while a sensor is amber or red — it is an arming-block
     indicator as much as a health one). They moved down from the top row so the replay player and
     its compact strip own the space between the burger and the chain-link button. The old status
     strip (connection text) is gone: the chain-link button already carries the connection dot.
     The chips sit BELOW the panel layer (a panel may cover them; they still peek out left of it),
     the dev Debug button (PhoneDebugButton) floats right of them above the panels. The row
     publishes its right edge as `--phone-bottom-w` on the root, so the Debug button and the
     Leaflet attribution can line up after it. -->
<script lang="ts">
  import { t } from 'svelte-i18n';
  import ArmingIndicator from '$lib/components/ArmingIndicator.svelte';
  import { sensorProblems, ekfLabel } from '$lib/helpers/sensorHealth';
  import type { TelemetryData } from '$lib/stores/telemetry';

  let { telem }: { telem: TelemetryData } = $props();

  const problems = $derived(sensorProblems(telem, $t));
  const ekfProblem = $derived(telem.ekfStatus >= 2);

  let rowEl = $state<HTMLDivElement>();
  let widthPx = $state(0);
  // Right edge in viewport px (left offset + width), re-read on every size change: the arming
  // label and the sensor chip come and go with the telemetry.
  $effect(() => {
    void widthPx;
    const right = rowEl ? Math.ceil(rowEl.getBoundingClientRect().right) : 0;
    document.documentElement.style.setProperty('--phone-bottom-w', `${right}px`);
    return () => document.documentElement.style.removeProperty('--phone-bottom-w');
  });
</script>

<div class="chips" bind:this={rowEl} bind:clientWidth={widthPx}>
  <ArmingIndicator {telem} />
  {#if problems.length > 0 || ekfProblem}
    <div class="sensor-chip">
      {#each problems as s (s.key)}
        <span class="sensor" class:warning={s.warn && s.state < 2} class:error={s.state >= 2} title={s.tooltip}>{s.label}</span>
      {/each}
      {#if ekfProblem}
        <span class="sensor" class:warning={telem.ekfStatus === 2} class:error={telem.ekfStatus === 3} title={$t('sensors.ekfTooltip')}>{ekfLabel(telem)}</span>
      {/if}
    </div>
  {/if}
</div>

<style>
  .chips {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    pointer-events: auto;
  }
  /* The arming pill is styled for the toolbar's solid background; over the map it gets a backing. */
  .chips > :global(.arming) {
    background-color: rgba(46, 46, 46, 0.92);
    backdrop-filter: blur(8px);
    -webkit-backdrop-filter: blur(8px);
  }

  .sensor-chip {
    display: flex;
    gap: 2px;
    height: 26px;
    padding: 0 4px;
    align-items: center;
    background: rgba(46, 46, 46, 0.92);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 6px;
    backdrop-filter: blur(8px);
    -webkit-backdrop-filter: blur(8px);
  }
  .sensor {
    padding: 2px 6px;
    border-radius: 3px;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.04em;
    color: #1a1a1a;
  }
  .sensor.warning {
    background: #f5a623;
  }
  .sensor.error {
    background: #d40000;
    color: #fff;
  }
</style>
