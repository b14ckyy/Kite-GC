<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- GPS widget: fused position by default; with two receivers the title toggles to cycling their raw values. -->
<script lang="ts">
  import type { GpsReceiver, TelemetryData } from "$lib/stores/telemetry";
  import { settings } from '$lib/stores/settings';
  import { t } from 'svelte-i18n';

  let { telem, size = 9 }: { telem: TelemetryData; size?: number } = $props();

  const CYCLE_MS = 3000;

  function formatCoord(value: number, pos: string, neg: string): string {
    const hemi = value >= 0 ? pos : neg;
    return `${hemi} ${Math.abs(value).toFixed(5)}`;
  }

  let receivers = $derived(telem.gpsReceivers);
  let multi = $derived(receivers.length > 1);
  let cycling = $derived(multi && $settings.gpsWidgetMode === 'cycle');

  let slot = $state(0);
  $effect(() => {
    if (!cycling) return;
    const id = setInterval(() => { slot += 1; }, CYCLE_MS);
    return () => clearInterval(id);
  });
  let shown = $derived<GpsReceiver | null>(cycling ? receivers[slot % receivers.length] : null);

  // The receiver the fused view takes fix and sats from, by the backend's rule: higher fix (STATIC / PPP
  // rank as DGPS), then more sats.
  const rank = (raw: number) => (raw > 6 ? 4 : raw);
  let best = $derived(
    receivers.reduce<GpsReceiver | null>(
      (b, r) => (!b || rank(r.fixRaw) > rank(b.fixRaw) || (rank(r.fixRaw) === rank(b.fixRaw) && r.numSat > b.numSat) ? r : b),
      null
    )
  );

  function toggleMode() {
    if (!multi) return;
    settings.update((s) => ({ ...s, gpsWidgetMode: s.gpsWidgetMode === 'cycle' ? 'ekf' : 'cycle' }));
  }

  // Unified fix scale (backend `scheduler::telemetry::FIX_*`): 0 none · 1 2D · 2 3D · 3 DGPS/RTK.
  let fixType = $derived(shown ? shown.fixType : telem.fixType);
  let lat = $derived(shown ? shown.lat : telem.lat);
  let lon = $derived(shown ? shown.lon : telem.lon);
  let hdop = $derived(shown ? (shown.hdop ?? 0) : telem.gpsHdop);
  // MAVLink GPS_FIX_TYPE of the receiver behind the shown fix, null where only the unified scale exists.
  let fixRaw = $derived(shown ? shown.fixRaw : best && best.fixType === telem.fixType ? best.fixRaw : null);

  let latText = $derived(telem.lastUpdate && fixType > 0 ? formatCoord(lat, 'N', 'S') : 'N —');
  let lonText = $derived(telem.lastUpdate && fixType > 0 ? formatCoord(lon, 'E', 'W') : 'E —');

  let sats = $derived(telem.lastUpdate ? String(shown ? shown.numSat : telem.numSat) : '—');

  let fixLabel = $derived.by(() => {
    if (!telem.lastUpdate) return $t('gps.noFix');
    if (fixRaw === 0) return $t('gps.noGps');
    if (fixRaw === 5) return $t('gps.rtkFloat');
    if (fixRaw === 6) return $t('gps.rtkFixed');
    if (fixType === 0) return $t('gps.noFix');
    const types: Record<number, string> = { 1: $t('gps.fix2d'), 2: $t('gps.fix3d'), 3: $t('gps.fix3dDgps') };
    return types[fixType] || `FIX ${fixType}`;
  });

  let fixColor = $derived(
    !telem.lastUpdate || fixType < 1 ? '#e74c3c'
      : fixType === 1 ? '#f39c12'
      : '#27ae60'
  );

  let hdopText = $derived(telem.lastUpdate && hdop > 0 ? `HDOP ${hdop.toFixed(1)}` : 'HDOP -');

  let title = $derived(shown ? `${$t('sensors.gps')}${shown.id + 1}` : $t('sensors.gps'));
  let chip = $derived(cycling ? $t('widgetLabels.cyc') : $t('widgetLabels.ekf'));
</script>

<div class="widget-card" style="--ws: {size}px">
  <button class="w-label" class:clickable={multi} onclick={toggleMode} disabled={!multi} type="button">
    {title}{#if multi}<span class="chip">{chip}</span>{/if}
  </button>
  <span class="w-coord">{latText}</span>
  <span class="w-coord">{lonText}</span>
  <div class="w-meta-stack">
    <span class="w-meta w-meta-main" style="color: {fixColor}">
      {sats} SAT · {fixLabel}
    </span>
    <span class="w-meta w-meta-hdop">
      {hdopText}
    </span>
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
    backdrop-filter: var(--glass-blur, blur(6px));
    -webkit-backdrop-filter: var(--glass-blur, blur(6px));
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: calc(var(--ws) * 0.08);
    gap: calc(var(--ws) * 0.025);
    box-sizing: border-box;
    padding: calc(var(--ws) * 0.05) calc(var(--ws) * 0.05) calc(var(--ws) * 0.04);
  }
  .w-label {
    display: flex;
    align-items: center;
    gap: calc(var(--ws) * 0.03);
    font-size: calc(var(--ws) * 0.13);
    font-weight: 600;
    color: #37a8db;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    background: none;
    border: none;
    padding: 0;
    font-family: inherit;
  }
  .w-label.clickable { cursor: pointer; }
  .w-label.clickable:hover { color: #5bc0ef; }
  .chip {
    font-size: calc(var(--ws) * 0.09);
    font-weight: 700;
    color: #e0e0e0;
    background: rgba(55, 168, 219, 0.25);
    border-radius: calc(var(--ws) * 0.03);
    padding: 0 calc(var(--ws) * 0.04);
    letter-spacing: 0;
  }
  .w-coord {
    font-size: calc(var(--ws) * 0.14);
    font-weight: 600;
    color: #d8d8d8;
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.02em;
    line-height: 1.2;
    width: 100%;
    text-align: center;
  }
  .w-meta {
    font-size: calc(var(--ws) * 0.1);
    font-weight: 700;
    text-transform: uppercase;
    text-align: center;
    line-height: 1.15;
    width: 100%;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .w-meta-stack {
    margin-top: auto;
    width: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: calc(var(--ws) * 0.0125);
  }

  .w-meta-hdop {
    color: #9fb3bd;
    font-weight: 600;
  }
</style>
