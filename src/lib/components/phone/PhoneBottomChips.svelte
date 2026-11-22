<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- PhoneBottomChips — the phone's bottom-left corner (Dev-Docs archive/PHONE_UI.md D18): the arming
     state and, right of it, the sensor chip, shown only while a sensor is amber or red. Below the panel
     layer. Publishes the row's right edge as `--phone-bottom-w` (Debug button, attribution) and the
     arming pill's as `--phone-arming-w`: the bottom widget tiles start there, so the chip may cover
     their lower edge. -->
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
  // Right edges in viewport px (left offset + width), re-read on every size change: the arming
  // label and the sensor chip come and go with the telemetry.
  $effect(() => {
    void widthPx;
    const root = document.documentElement.style;
    const right = rowEl ? Math.ceil(rowEl.getBoundingClientRect().right) : 0;
    root.setProperty('--phone-bottom-w', `${right}px`);
    const pill = rowEl?.querySelector('.arming');
    root.setProperty('--phone-arming-w', `${pill ? Math.ceil(pill.getBoundingClientRect().right) : right}px`);
    return () => {
      root.removeProperty('--phone-bottom-w');
      root.removeProperty('--phone-arming-w');
    };
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
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    pointer-events: auto;
  }
  /* The arming pill is styled for the toolbar's solid background; over the map it gets a backing. */
  .chips > :global(.arming) {
    background-color: rgba(46, 46, 46, 0.92);
    backdrop-filter: var(--glass-blur, blur(6px));
    -webkit-backdrop-filter: var(--glass-blur, blur(6px));
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
    backdrop-filter: var(--glass-blur, blur(6px));
    -webkit-backdrop-filter: var(--glass-blur, blur(6px));
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
