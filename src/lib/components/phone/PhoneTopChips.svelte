<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- PhoneTopChips — what survives of the desktop toolbar on the phone (Dev-Docs active/PHONE_UI.md
     D12), floating right of the nav-rail burger: the arming state, and a sensor chip that appears
     ONLY while a sensor is amber or red (a healthy airframe shows nothing — saves the space without
     losing the function). -->
<script lang="ts">
  import { t } from 'svelte-i18n';
  import ArmingIndicator from '$lib/components/ArmingIndicator.svelte';
  import { sensorProblems, ekfLabel } from '$lib/helpers/sensorHealth';
  import type { TelemetryData } from '$lib/stores/telemetry';

  let { telem }: { telem: TelemetryData } = $props();

  const problems = $derived(sensorProblems(telem, $t));
  const ekfProblem = $derived(telem.ekfStatus >= 2);
</script>

<div class="chips">
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
    display: flex;
    align-items: center;
    gap: 8px;
    pointer-events: auto;
  }

  .sensor-chip {
    display: flex;
    gap: 2px;
    height: 28px;
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
