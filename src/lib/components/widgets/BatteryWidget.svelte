<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- Battery widget — voltage bar, voltage, current + power, mAh. Multi-battery aware (ArduPilot/PX4):
     shows one monitor instance with an AUTO selector (highest current draw, with a low-battery safety
     override) and manual pinning. Single-battery setups (INAV) show the one battery. See
     docs/active/MULTI_BATTERY.md. -->
<script lang="ts">
  import type { TelemetryData, BatteryInstance } from "$lib/stores/telemetry";
  import { settings } from "$lib/stores/settings";
  import { t } from 'svelte-i18n';

  let { telem, size = 9, widgetId = 'battery' }:
    { telem: TelemetryData; size?: number; widgetId?: string } = $props();

  // AUTO switching tuning (margin hardcoded by design; the alert % is user-configurable under Alerts).
  const MARGIN_A = 1.0;     // a new candidate must lead the shown pack by this many amps…
  const DWELL_MS = 5000;    // …continuously for this long before AUTO switches (anti-flap hysteresis)

  // The battery list: live per-instance array, else a single synthesised instance from the primary
  // fields (INAV / single-battery ArduPilot). Empty before the first telemetry frame.
  let instances = $derived<BatteryInstance[]>(
    telem.batteries.length > 0
      ? telem.batteries
      : telem.lastUpdate
        ? [{
            id: 0,
            voltage: telem.voltage,
            current: telem.current,
            mahDrawn: telem.mAhDrawn,
            percentage: telem.batteryPercentage,
            cellCount: telem.cellCount,
            temperature: null,
          }]
        : []
  );
  let multi = $derived(instances.length > 1);
  let alertPct = $derived($settings.batteryAlertPct);

  // ── AUTO selection (highest current draw, 5 s margin hysteresis; low-battery safety override) ──
  // curAuto/cand* are plain (non-reactive) so the $effect never reads the $state it writes (that
  // read+write-same-state pattern freezes the main thread — see the speed widget's ACC bar).
  let autoShownId = $state(0);
  let curAuto = -1;
  let candId = -1;
  let candSince = 0;

  $effect(() => {
    const list = instances;
    const now = telem.lastUpdate;
    const thr = alertPct;
    if (list.length === 0) { curAuto = -1; autoShownId = -1; return; }
    if (list.length === 1) { curAuto = list[0].id; autoShownId = list[0].id; return; }

    // Safety override: any pack below the alert threshold → show the lowest-% one immediately.
    const low = list.filter((b) => b.percentage > 0 && thr > 0 && b.percentage < thr);
    if (low.length > 0) {
      curAuto = low.reduce((a, b) => (b.percentage < a.percentage ? b : a)).id;
      candId = -1;
      autoShownId = curAuto;
      return;
    }

    // Default: highest absolute current draw, with margin + dwell hysteresis.
    const best = list.reduce((a, b) => (Math.abs(b.current) > Math.abs(a.current) ? b : a));
    const cur = list.find((b) => b.id === curAuto);
    if (!cur) {
      curAuto = best.id;
      candId = -1;
    } else if (best.id !== curAuto && Math.abs(best.current) > Math.abs(cur.current) + MARGIN_A) {
      if (candId !== best.id) { candId = best.id; candSince = now; }
      else if (now - candSince >= DWELL_MS) { curAuto = best.id; candId = -1; }
    } else {
      candId = -1;
    }
    autoShownId = curAuto;
  });

  // Manual pin ('auto' or an instance id string) per widget, persisted in settings.
  let selection = $derived($settings.batterySelect?.[widgetId] ?? 'auto');
  let shownId = $derived.by(() => {
    if (selection !== 'auto') {
      const n = Number(selection);
      if (instances.some((b) => b.id === n)) return n;
    }
    return autoShownId;
  });
  let shown = $derived(instances.find((b) => b.id === shownId) ?? instances[0]);

  // Cycle: AUTO → instance 0 → instance 1 → … → AUTO.
  function cycle() {
    if (!multi) return;
    const ids = instances.map((b) => b.id);
    let next: string;
    if (selection === 'auto') {
      next = String(ids[0]);
    } else {
      const i = ids.indexOf(Number(selection));
      next = i >= 0 && i < ids.length - 1 ? String(ids[i + 1]) : 'auto';
    }
    settings.update((s) => ({ ...s, batterySelect: { ...s.batterySelect, [widgetId]: next } }));
  }

  let pct = $derived(shown ? Math.max(0, Math.min(100, shown.percentage)) : 0);
  let barColor = $derived(pct >= 50 ? '#27ae60' : pct >= 20 ? '#f39c12' : '#e74c3c');
  let voltage = $derived(shown ? `${shown.voltage.toFixed(1)}V` : '—V');
  let current = $derived(
    shown
      ? `${Math.abs(shown.current) >= 100 ? shown.current.toFixed(0) : shown.current.toFixed(1)}A`
      : '—A'
  );
  let watt = $derived(shown ? `${Math.round(shown.voltage * shown.current)}W` : '—W');
  let mah = $derived(shown ? `${shown.mahDrawn} mAh` : '—');
  // Alert state: shown pack below the configured threshold.
  let alert = $derived(!!shown && shown.percentage > 0 && alertPct > 0 && shown.percentage < alertPct);
  // Mode chip: in AUTO show the auto-selected pack number too (so you know which is shown); when
  // manually pinned the number alone is enough (you chose it).
  let chip = $derived(
    selection === 'auto'
      ? (multi ? `${$t('widgetLabels.auto')} ${shownId + 1}` : '')
      : `${shownId + 1}`
  );
</script>

<div class="widget-card" class:alert style="--ws: {size}px">
  <button class="w-label" class:clickable={multi} onclick={cycle} disabled={!multi} type="button">
    {$t('widgetLabels.bat')}{#if multi}<span class="chip">{chip}</span>{/if}
  </button>

  <!-- Voltage bar — only shown when battery % is available from FC -->
  {#if pct > 0}
  <div class="bat-bar-track">
    <div class="bat-bar-fill" style="width: {pct}%; background: {barColor}"></div>
  </div>
  {/if}

  <span class="w-value">{voltage}</span>
  <div class="w-secondary-row">
    <span class="w-secondary">{current}</span>
    <span class="w-secondary">{watt}</span>
  </div>
  <span class="w-tertiary">{mah}</span>
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
    padding: calc(var(--ws) * 0.05) calc(var(--ws) * 0.06) calc(var(--ws) * 0.04);
    transition: border-color 0.3s, box-shadow 0.3s;
  }
  /* Low-battery alert state for the shown pack. */
  .widget-card.alert {
    border-color: #d40000;
    box-shadow: 0 0 calc(var(--ws) * 0.06) rgba(212, 0, 0, 0.55);
    animation: bat-alert 1.2s ease-in-out infinite;
  }
  @keyframes bat-alert {
    50% { box-shadow: 0 0 calc(var(--ws) * 0.1) rgba(212, 0, 0, 0.85); }
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
  .w-value {
    font-size: calc(var(--ws) * 0.24);
    font-weight: 700;
    color: #e0e0e0;
    font-variant-numeric: tabular-nums;
    line-height: 1.1;
    margin-top: calc(var(--ws) * 0.01);
  }
  /* Current + power sit side by side, evenly spaced, sharing the same type. */
  .w-secondary-row {
    display: flex;
    width: 100%;
    justify-content: space-around;
    align-items: baseline;
    gap: calc(var(--ws) * 0.06);
  }
  .w-secondary {
    flex: 1;
    text-align: center;
    font-size: calc(var(--ws) * 0.13);
    color: #ccc;
    font-variant-numeric: tabular-nums;
  }
  .w-tertiary {
    font-size: calc(var(--ws) * 0.11);
    color: #888;
    font-variant-numeric: tabular-nums;
  }

  /* Battery bar */
  .bat-bar-track {
    width: 88%;
    height: calc(var(--ws) * 0.055);
    background: rgba(255, 255, 255, 0.1);
    border-radius: calc(var(--ws) * 0.03);
    overflow: hidden;
    margin: calc(var(--ws) * 0.01) 0 calc(var(--ws) * 0.02);
  }
  .bat-bar-fill {
    height: 100%;
    border-radius: calc(var(--ws) * 0.03);
    transition: width 0.5s ease;
  }
</style>
