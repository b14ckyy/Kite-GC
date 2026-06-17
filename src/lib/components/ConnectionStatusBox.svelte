<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // Connection status box (shown when connected, left of the Disconnect button).
  //  • Line 1: the primary protocol (MSP / MAVLink / SmartPort / CRSF / LTM).
  //  • Line 2 (optional): a secondary protocol tunneled inside it (ArduPilot passthrough → MAVLink,
  //    later MSP-over-telemetry → MSP).
  //  • A dot reflects data flow — green while valid parser data arrives, red after >5 s without any,
  //    grey until the first data. This is protocol-agnostic (any telemetry event bumps lastUpdate) and
  //    only asks "is ANY valid data reaching the parser", not whether every field is present.
  import { t } from 'svelte-i18n';
  import { connection, connectionProtocol, fcLinkAlive } from '$lib/stores/connection';
  import type { TelemetryData } from '$lib/stores/telemetry';

  let { telem }: { telem: TelemetryData } = $props();

  const STALE_MS = 5000;

  // 1 Hz ticker so the dot flips to red after the stale window without new telemetry.
  let now = $state(Date.now());
  $effect(() => {
    const id = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(id);
  });

  // "Any data" freshness — works for MSP/MAVLink (they only emit on real data). For passive telemetry
  // the decoder re-emits cached state + the RX keeps sending RSSI, so this never goes stale there;
  // instead the backend's fcLinkAlive tracks fresh FC-origin frames. Combine both.
  const dataFresh = $derived(!!telem.lastUpdate && now - telem.lastUpdate < STALE_MS);
  const isPassive = $derived($connection.protocolType === 'telemetry');
  const fcOk = $derived(!isPassive || $fcLinkAlive);

  const activity = $derived.by<'waiting' | 'live' | 'stale'>(() => {
    if (!telem.lastUpdate) return 'waiting';
    return dataFresh && fcOk ? 'live' : 'stale';
  });

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
