<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // Connection status box (shown when connected, left of the Disconnect button): the primary protocol, a
  // protocol tunnelled inside it (ArduPilot passthrough → MAVLink) and a dot for the data flow.
  import { t } from 'svelte-i18n';
  import { connectionProtocol } from '$lib/stores/connection';
  import { linkActivity } from '$lib/helpers/linkActivity.svelte';
  import type { TelemetryData } from '$lib/stores/telemetry';

  let { telem }: { telem: TelemetryData } = $props();

  const flow = linkActivity(() => telem);
  const activity = $derived(flow());

  const dotTitle = $derived(
    activity === 'live'
      ? $t('statusBox.live')
      : activity === 'stale'
        ? $t('statusBox.noData')
        : $t('statusBox.waiting'),
  );
</script>

<div class="conn-status" class:live={activity === 'live'} class:stale={activity === 'stale'}>
  <span class="dot" title={dotTitle}></span>
  <div class="proto">
    <span class="primary">{$connectionProtocol.primary}</span>
    {#if $connectionProtocol.secondary}
      <span class="secondary">{$connectionProtocol.secondary}</span>
    {/if}
  </div>
</div>

<style>
  .conn-status {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 32px;
    padding: 0 10px;
    background: rgba(0, 0, 0, 0.25);
    border: 1px solid #272727;
    border-radius: 6px;
    font-family: 'Segoe UI', Tahoma, sans-serif;
  }
  .dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    flex: none;
    background: #949494; /* waiting */
    box-shadow: 0 0 0 0 transparent;
    transition: background 0.2s ease;
  }
  .conn-status.live .dot {
    background: #59aa29;
    box-shadow: 0 0 6px rgba(89, 170, 41, 0.7);
  }
  .conn-status.stale .dot {
    background: #d40000;
    box-shadow: 0 0 6px rgba(212, 0, 0, 0.7);
  }
  .proto {
    display: flex;
    flex-direction: column;
    line-height: 1.05;
  }
  .primary {
    font-size: 12px;
    font-weight: 600;
    color: #e0e0e0;
    letter-spacing: 0.02em;
  }
  .secondary {
    font-size: 10px;
    color: #949494;
  }
</style>
