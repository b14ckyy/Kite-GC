<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!--
  RC Link widget — adaptive RC link-quality readout. Shows whatever the active protocol provides and
  hides the rest: LQ (CRSF / SmartPort / INAV 9.1+), RSSI (% always, dBm for CRSF / INAV 9.1+), SNR
  (CRSF / INAV 9.1+). LTM / MAVLink / INAV pre-9.1 carry RSSI only.
-->
<script lang="ts">
  import type { TelemetryData } from "$lib/stores/telemetry";
  import { t } from 'svelte-i18n';

  let { telem, size = 9 }: { telem: TelemetryData; size?: number } = $props();

  let link = $derived(telem.link);
  let hasData = $derived(
    !!telem.lastUpdate &&
    (link.lq != null || link.rssiPercent != null || link.rssiDbm != null)
  );

  // Primary metric: LQ when available (the best single link-health indicator), else RSSI %.
  let primaryIsLq = $derived(link.lq != null);
  let primaryValue = $derived(primaryIsLq ? link.lq : link.rssiPercent);
  let primaryLabel = $derived(primaryIsLq ? $t('rcLink.lq') : $t('rcLink.rssi'));

  // Colour by the primary percentage (LQ or RSSI %).
  let primaryColor = $derived.by(() => {
    if (!hasData || primaryValue == null) return '#949494';
    if (primaryValue >= 70) return '#59aa29';
    if (primaryValue >= 40) return '#e8c820';
    return '#d40000';
  });

  let primaryText = $derived(
    hasData && primaryValue != null ? `${Math.round(primaryValue)} %` : '—'
  );

  // Secondary RSSI line — only when LQ is the primary (otherwise RSSI is already the big number).
  // Prefer dBm (more meaningful) over the derived %.
  let rssiText = $derived.by(() => {
    if (!hasData || !primaryIsLq) return null;
    if (link.rssiDbm != null) return `${$t('rcLink.rssi')} ${link.rssiDbm} dBm`;
    if (link.rssiPercent != null) return `${$t('rcLink.rssi')} ${Math.round(link.rssiPercent)} %`;
    return null;
  });

  let snrText = $derived(
    hasData && link.snrDb != null ? `${$t('rcLink.snr')} ${link.snrDb} dB` : null
  );
</script>

<div class="widget-card" style="--ws: {size}px">
  <span class="w-label">{$t('rcLink.title')}</span>
  <span class="w-value" style="color: {primaryColor}">{primaryText}</span>
  <span class="w-sub">{hasData ? primaryLabel : $t('rcLink.noLink')}</span>
  <div class="w-meta-stack">
    {#if rssiText}<span class="w-meta">{rssiText}</span>{/if}
    {#if snrText}<span class="w-meta">{snrText}</span>{/if}
  </div>
</div>

<style>
  .widget-card {
    width: var(--ws);
    height: var(--ws);
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: flex-start;
    background: rgba(30, 30, 30, 0.75);
    backdrop-filter: blur(10px);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: calc(var(--ws) * 0.08);
    gap: calc(var(--ws) * 0.02);
    box-sizing: border-box;
    padding: calc(var(--ws) * 0.05) calc(var(--ws) * 0.05) calc(var(--ws) * 0.04);
  }
  .w-label {
    font-size: calc(var(--ws) * 0.13);
    font-weight: 600;
    color: #37a8db;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .w-value {
    font-size: calc(var(--ws) * 0.26);
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    line-height: 1.15;
    margin-top: calc(var(--ws) * 0.02);
  }
  .w-sub {
    font-size: calc(var(--ws) * 0.1);
    font-weight: 600;
    color: #9fb3bd;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .w-meta-stack {
    margin-top: auto;
    width: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: calc(var(--ws) * 0.0125);
  }
  .w-meta {
    font-size: calc(var(--ws) * 0.1);
    font-weight: 600;
    color: #d8d8d8;
    text-align: center;
    line-height: 1.15;
    width: 100%;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    font-variant-numeric: tabular-nums;
  }
</style>
