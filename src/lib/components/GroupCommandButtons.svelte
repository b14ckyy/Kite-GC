<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- GroupCommandButtons — the group command set (ARM … MODE + the red RTL ALL) with its confirm
     dialog and the run summary. Shared by the map overlay bar (GroupCommandBar) and the fleet panel
     footer, so both surfaces behave identically: every button opens GroupConfirmDialog, nothing is
     sent without the hold-to-confirm. Needs 2+ SELECTED MAVLink vehicles for the command buttons;
     RTL ALL targets every armed vehicle regardless of selection. -->
<script lang="ts">
  import { t } from 'svelte-i18n';
  import Button from '$lib/components/panel/Button.svelte';
  import GroupConfirmDialog from '$lib/components/GroupConfirmDialog.svelte';
  import { get } from 'svelte/store';
  import { vehicles } from '$lib/stores/vehicles';
  import { allTelemetry } from '$lib/stores/telemetry';
  import { arduMission } from '$lib/stores/missionArdupilot';
  import { selectedVehicleIds } from '$lib/stores/fleetSelection';
  import {
    type GroupCommandKind, type GroupCommandParams,
    groupRun, groupBusy, runGroupCommand, rtlAllTargets, targetsFor,
  } from '$lib/controllers/fleetControl';
  import { orderedVehicles, vehicleColorFor, isMavlinkVehicle, vehicleSystem } from '$lib/helpers/fleetStatus';

  let { wrap = false }: {
    /** Let the buttons wrap onto several rows (panel footer) instead of one long row (map bar). */
    wrap?: boolean;
  } = $props();

  let dialog = $state<GroupConfirmDialog>();

  // Remembered between dialogs (module-scope, like MavCommandPanel's savedInputs).
  const remembered: GroupCommandParams = { altitude: 50, staggerMs: 1000, speed: 10, intent: 'hold' };

  const mavVehicles = $derived(orderedVehicles($vehicles).filter((v) => isMavlinkVehicle(v) && vehicleSystem(v) != null));
  const selectedMav = $derived(mavVehicles.filter((v) => $selectedVehicleIds.has(v.vehicleId)));
  const selCount = $derived(selectedMav.length);
  const canGroup = $derived(selCount >= 2 && !$groupBusy);
  // `rtlAllTargets` reads the stores imperatively — re-evaluate on the registry and the (10 Hz) fleet
  // telemetry flush so the badge tracks arming changes.
  const armedCount = $derived.by(() => { void $vehicles; void $allTelemetry; return rtlAllTargets().length; });

  const runSummary = $derived.by(() => {
    const r = $groupRun;
    if (!r) return null;
    const ok = r.items.filter((i) => i.status === 'ok').length;
    const fail = r.items.filter((i) => i.status === 'fail').length;
    return { kind: r.kind, ok, fail, total: r.items.length, done: r.finishedAt != null };
  });

  const planCount = $derived($arduMission.length);

  async function open(kind: GroupCommandKind, ids?: Iterable<string>) {
    if (!dialog || $groupBusy) return;
    const ordered = orderedVehicles($vehicles);
    const targets = ids
      ? targetsFor(ids)
      : selectedMav.map((v) => ({ vehicle: v, color: vehicleColorFor(ordered, v.vehicleId) }));
    if (!targets.length) return;
    // Upload sends whatever the mission editor holds right now (the active vehicle's plan).
    const params: GroupCommandParams = kind === 'missionUpload'
      ? { ...remembered, waypoints: get(arduMission) }
      : { ...remembered };
    const res = await dialog.show({ kind, targets, params });
    if (!res) return;
    const { waypoints: _wps, ...keep } = res.params;
    void _wps;
    Object.assign(remembered, keep);
    await runGroupCommand(kind, targetsFor(res.vehicleIds), res.params);
  }

  function openRtlAll() {
    const targets = rtlAllTargets();
    if (!targets.length) return;
    void open('rtl', targets.map((tg) => tg.vehicle.vehicleId));
  }

  const kinds: { kind: GroupCommandKind; variant: 'standard' | 'danger' | 'warning' | 'data' }[] = [
    { kind: 'arm', variant: 'danger' },
    { kind: 'disarm', variant: 'danger' },
    { kind: 'takeoff', variant: 'warning' },
    { kind: 'land', variant: 'warning' },
    { kind: 'rtl', variant: 'warning' },
    { kind: 'hold', variant: 'standard' },
    { kind: 'missionStart', variant: 'standard' },
    { kind: 'changeSpeed', variant: 'data' },
    { kind: 'setMode', variant: 'data' },
  ];
</script>

<div class="gcbtns" class:wrap>
  <div class="gcbtns-row">
    {#each kinds as k (k.kind)}
      <Button size="sm" variant={k.variant} disabled={!canGroup} onclick={() => open(k.kind)} title={selCount < 2 ? $t('fleet.group.hint') : ''}>
        {$t('fleet.kind.' + k.kind)}
      </Button>
    {/each}
    <Button size="sm" variant="data" icon="upload" disabled={!canGroup || planCount === 0} onclick={() => open('missionUpload')}
            title={planCount === 0 ? $t('fleet.check.emptyPlan') : selCount < 2 ? $t('fleet.group.hint') : $t('fleet.mission.uploadHint')}>
      {$t('fleet.kind.missionUpload')}
    </Button>
  </div>

  {#if runSummary}
    <div class="gcbtns-run" class:busy={!runSummary.done} class:has-fail={runSummary.fail > 0}>
      <span class="gcbtns-run-kind">{$t('fleet.kind.' + runSummary.kind)}</span>
      {#if !runSummary.done}
        <span>{$t('fleet.group.busy')}</span>
      {:else}
        <span class="gcbtns-ok">{runSummary.ok}✓</span>
        {#if runSummary.fail > 0}<span class="gcbtns-fail">{runSummary.fail}✗</span>{/if}
        <span class="gcbtns-total">/ {runSummary.total}</span>
      {/if}
    </div>
  {/if}

  <button class="gcbtns-rtl-all" disabled={$groupBusy || armedCount === 0} onclick={openRtlAll}
          title={armedCount === 0 ? $t('fleet.group.noArmed') : $t('fleet.group.rtlAllHint')}>
    {$t('fleet.group.rtlAll')}{#if armedCount > 0}<span class="gcbtns-rtl-n">{armedCount}</span>{/if}
  </button>
</div>

<GroupConfirmDialog bind:this={dialog} />

<style>
  .gcbtns {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
    font-family: 'Segoe UI', Tahoma, sans-serif;
    font-size: 12px;
    color: #e0e0e0;
  }
  .gcbtns.wrap { flex-wrap: wrap; }
  .gcbtns-row {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .gcbtns.wrap .gcbtns-row { flex: 1 1 100%; }

  .gcbtns-run {
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 2px 8px;
    border-radius: 4px;
    background: rgba(255, 255, 255, 0.06);
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }
  .gcbtns-run.busy { color: #f6e3b0; }
  .gcbtns-run-kind { font-weight: 600; color: #cfcfcf; }
  .gcbtns-ok { color: #59aa29; font-weight: 600; }
  .gcbtns-fail { color: #e06c6c; font-weight: 600; }
  .gcbtns-total { color: #9a9a9a; }

  .gcbtns-rtl-all {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 12px;
    margin-left: auto;
    background: rgba(192, 57, 43, 0.85);
    border: 1px solid #e74c3c;
    border-radius: 4px;
    color: #fff;
    font-weight: 700;
    font-size: 12px;
    letter-spacing: 0.04em;
    cursor: pointer;
    white-space: nowrap;
  }
  .gcbtns-rtl-all:hover:not(:disabled) { background: #e74c3c; }
  .gcbtns-rtl-all:disabled { opacity: 0.45; cursor: default; }
  .gcbtns-rtl-n {
    padding: 0 5px;
    border-radius: 8px;
    background: rgba(0, 0, 0, 0.35);
    font-size: 11px;
  }
</style>
