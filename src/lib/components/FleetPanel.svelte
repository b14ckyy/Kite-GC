<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- FleetPanel — the multi-vehicle list on the nav rail: every known vehicle with its key state (mode,
     arming, battery, GPS, altitude, speed, link freshness), warnings floated to the top, a checkbox per
     row for the GROUP selection (stores/fleetSelection) and a row click to make a vehicle the ACTIVE
     one (the same arm-switch confirmation as LinkManager). Quick-select chips, a "Frame fleet" camera
     button, and the last group command's per-vehicle results with a retry for the failed ones.
     See docs/02-design/features/fleet-control.design.md §4.1. -->
<script lang="ts">
  import { t } from 'svelte-i18n';
  import { onMount } from 'svelte';
  import PanelShell from './panel/PanelShell.svelte';
  import Button from './panel/Button.svelte';
  import ConfirmDialog from './ConfirmDialog.svelte';
  import GroupCommandButtons from './GroupCommandButtons.svelte';
  import { vehicles, activeVehicleId, type VehicleSummary } from '$lib/stores/vehicles';
  import { telemetry, allTelemetry, type TelemetryData } from '$lib/stores/telemetry';
  import { selectedVehicleIds, toggleSelected, setSelected, clearSelection } from '$lib/stores/fleetSelection';
  import { switchVehicle } from '$lib/controllers/connectionController';
  import { groupRun, groupBusy, retryFailed } from '$lib/controllers/fleetControl';
  import { frameFleetOnMap } from '$lib/stores/mapCamera';
  import { arduMissionStatusByVehicle, type ArduVehicleMissionStatus } from '$lib/stores/missionArdupilot';
  import { activeWpNumber } from '$lib/stores/navStatus';
  import { requestNavTab } from '$lib/stores/navRequest';
  import { isArmed } from '$lib/helpers/telemetry';
  import { modeLabel, modeColor } from '$lib/helpers/flightModeRegistry';
  import {
    type LinkState, orderedVehicles, vehicleColorFor, vehicleActiveMode, isVehicleArmed, linkState, isFailsafe,
    isMavlinkVehicle, LOW_BATTERY_PCT,
  } from '$lib/helpers/fleetStatus';

  let confirmDialog = $state<ConfirmDialog>();
  let busy = $state(false);
  let errorMsg = $state('');

  // 1 Hz clock so stale/lost link states age even when no telemetry arrives (which is the point).
  let now = $state(Date.now());
  onMount(() => {
    const id = setInterval(() => { now = Date.now(); }, 1000);
    return () => clearInterval(id);
  });

  interface Row {
    v: VehicleSummary;
    tv: TelemetryData | undefined;
    color: string;
    armed: boolean;
    link: LinkState;
    modeText: string;
    modeColor: string;
    warnings: string[]; // i18n suffixes under fleet.warn.*
    active: boolean;
    selected: boolean;
    mavlink: boolean;
    mission: ArduVehicleMissionStatus | undefined;
  }

  /** Make `v` active and jump to the mission editor (its plan slot is restored on the switch). */
  async function editMission(v: VehicleSummary) {
    if (v.vehicleId !== $activeVehicleId) await pick(v);
    if ($activeVehicleId === v.vehicleId) requestNavTab('mission');
  }

  const rows = $derived.by((): Row[] => {
    const ordered = orderedVehicles($vehicles);
    const telem = $allTelemetry;
    const list: Row[] = ordered.map((v) => {
      const tv = telem.get(v.vehicleId);
      const link = linkState(tv, now);
      const armed = tv ? isVehicleArmed(tv) : false;
      const warnings: string[] = [];
      if (link === 'lost') warnings.push('linkLost');
      if (tv && tv.batteryPercentage > 0 && tv.batteryPercentage < LOW_BATTERY_PCT) warnings.push('lowBattery');
      if (tv && isFailsafe(tv)) warnings.push('failsafe');
      // Mode: the firmware table entry (ArduPilot/PX4) when known, else the canonical registry label.
      const m = tv ? vehicleActiveMode(v, tv) : undefined;
      const primary = tv?.flightMode?.primary ?? '';
      return {
        v, tv, color: vehicleColorFor(ordered, v.vehicleId), armed, link,
        modeText: m?.name ?? (primary ? modeLabel(primary) : '—'),
        modeColor: primary ? modeColor(primary) : '#9a9a9a',
        warnings,
        active: v.vehicleId === $activeVehicleId,
        selected: $selectedVehicleIds.has(v.vehicleId),
        mavlink: isMavlinkVehicle(v),
        mission: $arduMissionStatusByVehicle.get(v.vehicleId),
      };
    });
    // Warnings first (stable otherwise — keeps the (link, sysid) order and thus the colours readable).
    return list.sort((a, b) => Number(b.warnings.length > 0) - Number(a.warnings.length > 0));
  });

  const selCount = $derived(rows.filter((r) => r.selected).length);

  function quickSelect(which: 'all' | 'armed' | 'flying' | 'none') {
    if (which === 'none') { clearSelection(); return; }
    const ids = rows
      .filter((r) => r.mavlink)
      .filter((r) => which === 'all' || (r.armed && (which === 'armed' || (r.tv?.altitude ?? 0) > 2)))
      .map((r) => r.v.vehicleId);
    setSelected(ids);
  }

  async function pick(v: VehicleSummary) {
    if (busy || v.vehicleId === $activeVehicleId) return;
    // Switching away from an armed vehicle changes what the control panel commands — confirm (FR-16).
    const tnow = $telemetry;
    if (isArmed(tnow.armingFlags, tnow.lastUpdate) && confirmDialog) {
      const choice = await confirmDialog.show({
        title: $t('connection.switchArmedTitle'),
        message: $t('connection.switchArmedBody', { values: { name: v.name } }),
        buttons: [
          { label: $t('connection.switchArmedConfirm'), value: 'switch', danger: true },
        ],
      });
      if (choice !== 'switch') return;
    }
    busy = true;
    errorMsg = '';
    try {
      await switchVehicle(v.vehicleId);
    } catch (e) {
      errorMsg = String(e);
    } finally {
      busy = false;
    }
  }

  function onRowClick(e: MouseEvent, r: Row) {
    if (e.ctrlKey || e.metaKey) { toggleSelected(r.v.vehicleId); return; }
    void pick(r.v);
  }

  const lastRun = $derived($groupRun);
  const failCount = $derived(lastRun ? lastRun.items.filter((i) => i.status === 'fail').length : 0);

  function fmtAlt(tv: TelemetryData | undefined): string {
    return tv ? `${Math.round(tv.altitude)} m` : '—';
  }
  function fmtSpd(tv: TelemetryData | undefined): string {
    return tv ? `${tv.groundSpeed.toFixed(1)} m/s` : '—';
  }
  function fmtBat(tv: TelemetryData | undefined): string {
    if (!tv) return '—';
    if (tv.batteryPercentage > 0) return `${Math.round(tv.batteryPercentage)} %`;
    return tv.voltage > 0 ? `${tv.voltage.toFixed(1)} V` : '—';
  }
</script>

<PanelShell variant="compact" title={$t('fleet.title')}>
  {#snippet toolbar()}
    <div class="fp-toolbar">
      <div class="fp-chips" title={$t('fleet.col.select')}>
        <span class="fp-chips-label">{$t('fleet.selectLabel')}</span>
        <Button size="sm" variant="compact" onclick={() => quickSelect('all')}>{$t('fleet.selectAll')}</Button>
        <Button size="sm" variant="compact" onclick={() => quickSelect('armed')}>{$t('fleet.selectArmed')}</Button>
        <Button size="sm" variant="compact" onclick={() => quickSelect('flying')}>{$t('fleet.selectFlying')}</Button>
        <Button size="sm" variant="compact" disabled={selCount === 0} onclick={() => quickSelect('none')}>{$t('fleet.selectNone')}</Button>
      </div>
      <Button size="sm" variant="data" icon="map" disabled={rows.length === 0} title={$t('fleet.frameHint')} onclick={frameFleetOnMap}>
        {$t('fleet.frame')}
      </Button>
    </div>
  {/snippet}

  {#snippet body()}
    {#if rows.length === 0}
      <div class="panel-empty">
        <span class="panel-empty-icon">⊘</span>
        <span>{$t('fleet.empty')}</span>
      </div>
    {:else}
      <div class="fp-hint">{$t('fleet.switchHint')}</div>
      <ul class="fp-list">
        {#each rows as r (r.v.vehicleId)}
          <li class="fp-row" class:active={r.active} class:warn={r.warnings.length > 0} class:selected={r.selected} class:dim={!r.mavlink}>
            <label class="fp-check" title={$t('fleet.col.select')}>
              <input type="checkbox" checked={r.selected} disabled={!r.mavlink} onchange={() => toggleSelected(r.v.vehicleId)} />
            </label>
            <button class="fp-main" disabled={busy} onclick={(e) => onRowClick(e, r)}>
              <span class="fp-dot" style:background={r.color}></span>
              <span class="fp-name">{r.v.name}</span>
              {#if r.active}<span class="fp-active">{$t('fleet.active')}</span>{/if}
              <span class="fp-mode" style:--mc={r.modeColor}>{r.modeText}</span>
              <span class="fp-arm" class:armed={r.armed}>{r.armed ? $t('fleet.armed') : $t('fleet.disarmed')}</span>
              <span class="fp-link {r.link}" title={$t('fleet.link.' + r.link)}></span>
            </button>
            <div class="fp-stats">
              <span class="fp-stat" class:bad={r.warnings.includes('lowBattery')} title={$t('fleet.col.battery')}>🔋 {fmtBat(r.tv)}</span>
              <span class="fp-stat" title={$t('fleet.col.sats')}>🛰 {r.tv ? r.tv.numSat : '—'}</span>
              <span class="fp-stat" title={$t('fleet.col.alt')}>⇡ {fmtAlt(r.tv)}</span>
              <span class="fp-stat" title={$t('fleet.col.speed')}>➤ {fmtSpd(r.tv)}</span>
              <span class="fp-stat fp-meta">L{r.v.linkId}:S{r.v.sysid} · {r.v.fcVariant}</span>
            </div>
            {#if r.mavlink}
              <div class="fp-mission">
                {#if r.mission && r.mission.count > 0}
                  <span class="fp-mission-count">{$t('fleet.mission.wps', { values: { n: r.mission.locationCount } })}</span>
                  {#if r.mission.fcSynced}
                    <span class="fp-mbadge fc" title={$t('fleet.mission.fcTip')}>{$t('fleet.mission.fc')}</span>
                  {:else if r.mission.modified}
                    <span class="fp-mbadge mod" title={$t('fleet.mission.modifiedTip')}>{$t('fleet.mission.modified')}</span>
                  {/if}
                  {#if r.active && $activeWpNumber > 0}
                    <span class="fp-mbadge cur">{$t('fleet.mission.current', { values: { n: $activeWpNumber } })}</span>
                  {/if}
                {:else}
                  <span class="fp-mission-none">{$t('fleet.mission.none')}</span>
                {/if}
                <button class="fp-mission-edit" disabled={busy} onclick={() => { void editMission(r.v); }} title={$t('fleet.mission.edit')}>✎</button>
              </div>
            {/if}
            {#if r.warnings.length}
              <div class="fp-warns">
                {#each r.warnings as w (w)}
                  <span class="fp-warn">{$t('fleet.warn.' + w)}</span>
                {/each}
              </div>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}

    {#if lastRun}
      <div class="fp-section">
        <span>{$t('fleet.group.lastRun')} · {$t('fleet.kind.' + lastRun.kind)}</span>
        {#if failCount > 0 && !$groupBusy}
          <Button size="sm" variant="warning" onclick={() => { void retryFailed(); }}>{$t('fleet.group.retryFailed')} ({failCount})</Button>
        {/if}
      </div>
      <ul class="fp-results">
        {#each lastRun.items as it (it.vehicleId)}
          <li class="fp-res {it.status}">
            <span class="fp-dot" style:background={it.color}></span>
            <span class="fp-res-name">{it.name}</span>
            <span class="fp-res-status">{$t('fleet.status.' + it.status)}</span>
            {#if it.message}<span class="fp-res-msg" title={it.message}>{it.message}</span>{/if}
          </li>
        {/each}
      </ul>
    {/if}

    {#if errorMsg}
      <div class="fp-error">{errorMsg}</div>
    {/if}
  {/snippet}

  {#snippet footer()}
    <div class="fp-footer">
      <div class="fp-footer-head">
        <span class="fp-footer-title">{$t('fleet.group.section')}</span>
        <span class="fp-footer-sel" class:ready={selCount >= 2}>
          {selCount >= 2 ? $t('fleet.selectedCount', { values: { n: selCount } }) : $t('fleet.group.hint')}
        </span>
      </div>
      <GroupCommandButtons wrap />
    </div>
  {/snippet}
</PanelShell>

<ConfirmDialog bind:this={confirmDialog} />

<style>
  .fp-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    flex-wrap: wrap;
  }
  .fp-chips {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-wrap: wrap;
  }
  .fp-chips-label {
    margin-right: 2px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: #9a9a9a;
  }

  .fp-footer {
    display: flex;
    flex-direction: column;
    gap: 6px;
    width: 100%;
  }
  .fp-footer-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .fp-footer-title {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: #9a9a9a;
  }
  .fp-footer-sel { font-size: 11px; color: #9a9a9a; }
  .fp-footer-sel.ready { color: #37a8db; font-weight: 600; }
  .fp-hint {
    margin: 0 0 6px;
    font-size: 11px;
    color: #888;
  }

  .fp-list, .fp-results {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .fp-row {
    display: grid;
    grid-template-columns: 22px 1fr;
    grid-template-areas:
      "check main"
      ". stats"
      ". mission"
      ". warns";
    align-items: center;
    column-gap: 6px;
    row-gap: 2px;
    padding: 5px 8px 5px 6px;
    background: #363636;
    border: 1px solid #4a4a4a;
    border-radius: 4px;
    font-size: 12px;
    color: #e0e0e0;
  }
  .fp-row.active { border-color: #37a8db; background: rgba(55, 168, 219, 0.14); }
  .fp-row.selected { box-shadow: inset 3px 0 0 #37a8db; }
  .fp-row.warn { border-color: rgba(224, 108, 108, 0.7); background: rgba(120, 30, 30, 0.18); }
  .fp-row.dim { opacity: 0.6; }

  .fp-check { grid-area: check; display: flex; align-items: center; justify-content: center; }
  .fp-check input { margin: 0; cursor: pointer; }

  .fp-main {
    grid-area: main;
    display: flex;
    align-items: center;
    gap: 7px;
    min-width: 0;
    padding: 0;
    background: transparent;
    border: 0;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .fp-main:disabled { cursor: default; }
  .fp-dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    flex: 0 0 auto;
    box-shadow: 0 0 3px rgba(0, 0, 0, 0.6);
  }
  .fp-name { font-weight: 600; white-space: nowrap; }
  .fp-active {
    font-size: 10px;
    font-weight: 600;
    color: #37a8db;
    text-transform: uppercase;
  }
  .fp-mode {
    margin-left: auto;
    padding: 0 6px;
    border-radius: 3px;
    border: 1px solid var(--mc, #666);
    color: var(--mc, #cfcfcf);
    font-size: 11px;
    font-weight: 600;
    white-space: nowrap;
  }
  .fp-arm {
    font-size: 10.5px;
    font-weight: 600;
    color: #9a9a9a;
    text-transform: uppercase;
    white-space: nowrap;
  }
  .fp-arm.armed { color: #f5a623; }
  .fp-link {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: 0 0 auto;
    background: #59aa29;
    box-shadow: 0 0 4px rgba(89, 170, 41, 0.8);
  }
  .fp-link.stale { background: #f5a623; box-shadow: 0 0 4px rgba(245, 166, 35, 0.8); }
  .fp-link.lost { background: #e06c6c; box-shadow: 0 0 4px rgba(224, 108, 108, 0.8); }

  .fp-stats {
    grid-area: stats;
    display: flex;
    flex-wrap: wrap;
    gap: 2px 10px;
    font-size: 11px;
    color: #cfcfcf;
    font-variant-numeric: tabular-nums;
  }
  .fp-stat.bad { color: #f0a0a0; font-weight: 600; }
  .fp-meta { color: #888; margin-left: auto; }

  .fp-mission {
    grid-area: mission;
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    color: #cfcfcf;
  }
  .fp-mission-count { font-weight: 600; }
  .fp-mission-none { color: #888; }
  .fp-mbadge {
    padding: 0 5px;
    border-radius: 3px;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.03em;
    border: 1px solid #555;
    color: #9a9a9a;
  }
  .fp-mbadge.fc { border-color: #59aa29; color: #59aa29; }
  .fp-mbadge.mod { border-color: #f5a623; color: #f5a623; }
  .fp-mbadge.cur { border-color: #37a8db; color: #37a8db; font-variant-numeric: tabular-nums; }
  .fp-mission-edit {
    margin-left: auto;
    width: 22px;
    height: 20px;
    padding: 0;
    background: transparent;
    border: 1px solid #555;
    border-radius: 3px;
    color: #cfcfcf;
    font-size: 12px;
    line-height: 1;
    cursor: pointer;
  }
  .fp-mission-edit:hover:not(:disabled) { border-color: #37a8db; color: #37a8db; }
  .fp-mission-edit:disabled { opacity: 0.5; cursor: default; }

  .fp-warns { grid-area: warns; display: flex; gap: 4px; flex-wrap: wrap; }
  .fp-warn {
    padding: 1px 6px;
    border-radius: 3px;
    font-size: 10.5px;
    font-weight: 600;
    background: rgba(224, 108, 108, 0.2);
    color: #f0a0a0;
  }

  .fp-section {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin: 12px 0 4px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: #9a9a9a;
  }
  .fp-res {
    display: flex;
    align-items: center;
    gap: 7px;
    min-height: 24px;
    padding: 2px 8px;
    background: #363636;
    border: 1px solid #4a4a4a;
    border-radius: 4px;
    font-size: 11.5px;
  }
  .fp-res-name { font-weight: 600; white-space: nowrap; }
  .fp-res-status { font-weight: 600; white-space: nowrap; color: #9a9a9a; }
  .fp-res.ok .fp-res-status { color: #59aa29; }
  .fp-res.fail .fp-res-status { color: #e06c6c; }
  .fp-res.running .fp-res-status { color: #f5a623; }
  .fp-res-msg {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: #f0a0a0;
  }

  .fp-error {
    margin-top: 8px;
    padding: 6px 8px;
    border-radius: 4px;
    background: rgba(224, 108, 108, 0.15);
    border: 1px solid rgba(224, 108, 108, 0.5);
    color: #f0a0a0;
    word-break: break-word;
  }
</style>
