<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- Toolbar arming traffic light (left of the sensor bar): green = ready & disarmed, amber = armed,
     red = not ready. INAV lists the blocking ARMING_DISABLED_* reasons on hover; ArduPilot/PX4 show the
     latest "PreArm: …" STATUSTEXT. -->
<script lang="ts">
  import { t } from 'svelte-i18n';
  import type { TelemetryData } from '$lib/stores/telemetry';
  import { prearmReason } from '$lib/stores/statusText';
  import { armingStatus } from '$lib/helpers/arming';
  import { autopilotSystem } from '$lib/stores/autopilotContext';

  let { telem }: { telem: TelemetryData } = $props();

  const status = $derived(armingStatus(telem, $prearmReason, $autopilotSystem === 'inav'));

  const label = $derived(
    !status ? ''
      : status.level === 'armed' ? $t('arming.armed')
      : status.level === 'ready' ? $t('arming.ready')
      : $t('arming.notReady'),
  );

  const tooltip = $derived.by(() => {
    if (!status) return '';
    if (status.level === 'armed') return $t('arming.armed');
    if (status.level === 'ready') return $t('arming.readyHint');
    if (status.reasonKeys.length) {
      return `${$t('arming.notReady')}:\n` + status.reasonKeys.map((k) => `• ${$t(`arming.reasons.${k}`)}`).join('\n');
    }
    if (status.prearmText) {
      return `${$t('arming.notReady')}:\n` + status.prearmText.split('\n').map((l) => `• ${l}`).join('\n');
    }
    return $t('arming.notReady');
  });
</script>

{#if status}
  <div
    class="arming"
    class:armed={status.level === 'armed'}
    class:ready={status.level === 'ready'}
    class:notready={status.level === 'notReady'}
    title={tooltip}
  >
    <span class="dot"></span>{label}
  </div>
{/if}

<style>
  .arming {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    box-sizing: border-box;
    padding: 0 12px;
    border-radius: 4px;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.4px;
    text-transform: uppercase;
    white-space: nowrap;
    border: 1px solid transparent;
    cursor: default;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }
  .arming.ready {
    color: #7ec850;
    background: rgba(89, 170, 41, 0.12);
    border-color: rgba(89, 170, 41, 0.35);
  }
  .arming.ready .dot { background: #59aa29; box-shadow: 0 0 4px rgba(89, 170, 41, 0.6); }

  .arming.armed {
    color: #f0b443;
    background: rgba(232, 163, 23, 0.14);
    border-color: rgba(232, 163, 23, 0.45);
  }
  .arming.armed .dot { background: #e8a317; box-shadow: 0 0 5px rgba(232, 163, 23, 0.7); }

  .arming.notready {
    color: #ff6b6b;
    background: rgba(212, 0, 0, 0.14);
    border-color: rgba(212, 0, 0, 0.45);
  }
  .arming.notready .dot { background: #d40000; box-shadow: 0 0 5px rgba(212, 0, 0, 0.7); }
</style>
