<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- GroupConfirmDialog — the safety gate in front of every group command. Shows the command's
     parameters (takeoff altitude + stagger, speed, abstract mode), then every target vehicle with its
     pre-flight verdict: `fail` rows start unticked, `warn` rows ticked but flagged; the operator can
     override either. The confirm button is hold-to-confirm, like every single-vehicle control action.
     See docs/02-design/features/fleet-control.design.md §4.3. -->
<script lang="ts" module>
  import type { VehicleSummary } from '$lib/stores/vehicles';
  import type { GroupCommandKind, GroupCommandParams } from '$lib/controllers/fleetControl';

  export interface GroupConfirmTarget {
    vehicle: VehicleSummary;
    color: string;
  }
  export interface GroupConfirmRequest {
    kind: GroupCommandKind;
    targets: GroupConfirmTarget[];
    params: GroupCommandParams;
  }
  export interface GroupConfirmResult {
    vehicleIds: string[];
    params: GroupCommandParams;
  }
</script>

<script lang="ts">
  import { t } from 'svelte-i18n';
  import { get } from 'svelte/store';
  import Button from '$lib/components/panel/Button.svelte';
  import HoldToConfirm from '$lib/components/panel/HoldToConfirm.svelte';
  import NumberStepper from '$lib/components/NumberStepper.svelte';
  import { allTelemetry } from '$lib/stores/telemetry';
  import type { ArduWaypoint } from '$lib/stores/missionArdupilot';
  import {
    type FleetModeIntent, type PreflightResult, FLEET_MODE_INTENTS, preflightChecks,
  } from '$lib/helpers/fleetStatus';

  let open = $state(false);
  let kind = $state<GroupCommandKind>('rtl');
  let targets = $state<GroupConfirmTarget[]>([]);
  let included = $state<Set<string>>(new Set());
  let altitude = $state(50);
  let staggerS = $state(1);
  let speed = $state(10);
  let intent = $state<FleetModeIntent>('hold');
  let force = $state(false);
  let offsetM = $state(0);
  let offsetBearing = $state(90);
  let plan = $state<ArduWaypoint[]>([]);
  let resolver: ((value: GroupConfirmResult | null) => void) | null = null;

  // Verdicts depend on the intent (setMode), so they are derived, not captured at open time.
  const checks = $derived.by(() => {
    const telem = $allTelemetry;
    const now = Date.now();
    const out = new Map<string, PreflightResult>();
    for (const tg of targets) {
      out.set(tg.vehicle.vehicleId, preflightChecks(kind, tg.vehicle, telem.get(tg.vehicle.vehicleId), kind === 'setMode' ? intent : undefined, now, kind === 'missionUpload' ? plan : undefined));
    }
    return out;
  });
  const includedCount = $derived(targets.filter((tg) => included.has(tg.vehicle.vehicleId)).length);

  const hasParams = $derived(kind === 'takeoff' || kind === 'changeSpeed' || kind === 'setMode' || kind === 'arm' || kind === 'disarm' || kind === 'missionUpload');

  /** Open for `req`; resolves with the included ids + final params, or null on cancel. */
  export function show(req: GroupConfirmRequest): Promise<GroupConfirmResult | null> {
    kind = req.kind;
    targets = req.targets;
    altitude = req.params.altitude ?? altitude;
    staggerS = Math.round(((req.params.staggerMs ?? staggerS * 1000) / 1000) * 10) / 10;
    speed = req.params.speed ?? speed;
    intent = req.params.intent ?? intent;
    force = !!req.params.force;
    offsetM = req.params.offsetM ?? offsetM;
    offsetBearing = req.params.offsetBearingDeg ?? offsetBearing;
    plan = req.params.waypoints ?? [];
    // Default inclusion: everything that isn't a hard `fail`.
    const telem = get(allTelemetry);
    const now = Date.now();
    const inc = new Set<string>();
    for (const tg of targets) {
      const c = preflightChecks(kind, tg.vehicle, telem.get(tg.vehicle.vehicleId), kind === 'setMode' ? intent : undefined, now, kind === 'missionUpload' ? plan : undefined);
      if (c.level !== 'fail') inc.add(tg.vehicle.vehicleId);
    }
    included = inc;
    open = true;
    return new Promise<GroupConfirmResult | null>((resolve) => { resolver = resolve; });
  }

  function close(value: GroupConfirmResult | null) {
    open = false;
    if (resolver) { resolver(value); resolver = null; }
  }

  function confirm() {
    if (includedCount === 0) return;
    const params: GroupCommandParams = {};
    if (kind === 'takeoff') { params.altitude = altitude; params.staggerMs = Math.round(staggerS * 1000); }
    if (kind === 'changeSpeed') params.speed = speed;
    if (kind === 'setMode') params.intent = intent;
    if (kind === 'arm' || kind === 'disarm') params.force = force;
    if (kind === 'missionUpload') { params.waypoints = plan; params.offsetM = offsetM; params.offsetBearingDeg = offsetBearing; }
    close({ vehicleIds: targets.filter((tg) => included.has(tg.vehicle.vehicleId)).map((tg) => tg.vehicle.vehicleId), params });
  }

  function toggleInclude(id: string) {
    const next = new Set(included);
    if (next.has(id)) next.delete(id); else next.add(id);
    included = next;
  }

  // Re-default inclusion when the intent changes (a vehicle may lose/gain support for the mode).
  function onIntentChange(v: FleetModeIntent) {
    intent = v;
    const telem = get(allTelemetry);
    const inc = new Set<string>();
    for (const tg of targets) {
      if (preflightChecks(kind, tg.vehicle, telem.get(tg.vehicle.vehicleId), intent).level !== 'fail') inc.add(tg.vehicle.vehicleId);
    }
    included = inc;
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') close(null);
  }

  const dangerKinds: GroupCommandKind[] = ['arm', 'disarm', 'takeoff', 'land'];
</script>

{#if open}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="gc-backdrop" onclick={() => close(null)} onkeydown={handleKeydown}>
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="gc-box" role="dialog" aria-modal="true" tabindex="-1" onclick={(e) => e.stopPropagation()}>
      <div class="gc-title">
        <span class="gc-title-kind">{$t('fleet.kind.' + kind)}</span>
        <span class="gc-title-sub">{$t('fleet.group.title')}</span>
      </div>

      {#if hasParams}
        <div class="gc-params">
          {#if kind === 'takeoff'}
            <label class="gc-param">
              <span>{$t('fleet.group.altitude')}</span>
              <NumberStepper bind:value={altitude} min={1} max={1000} step={5} unit="m" />
            </label>
            <label class="gc-param">
              <span>{$t('fleet.group.stagger')}</span>
              <NumberStepper bind:value={staggerS} min={0} max={10} step={0.5} decimals={1} unit="s" />
            </label>
          {:else if kind === 'changeSpeed'}
            <label class="gc-param">
              <span>{$t('fleet.group.speed')}</span>
              <NumberStepper bind:value={speed} min={0.5} max={100} step={0.5} decimals={1} unit="m/s" />
            </label>
          {:else if kind === 'setMode'}
            <label class="gc-param">
              <span>{$t('fleet.group.mode')}</span>
              <select class="gc-select" value={intent} onchange={(e) => onIntentChange((e.currentTarget as HTMLSelectElement).value as FleetModeIntent)}>
                {#each FLEET_MODE_INTENTS as it (it)}
                  <option value={it}>{$t('fleet.intent.' + it)}</option>
                {/each}
              </select>
            </label>
          {:else if kind === 'missionUpload'}
            <div class="gc-note">{$t('fleet.mission.uploadHint')} · {$t('fleet.mission.wps', { values: { n: plan.length } })}</div>
            <label class="gc-param">
              <span>{$t('fleet.mission.offset')}</span>
              <NumberStepper bind:value={offsetM} min={0} max={5000} step={5} unit="m" />
            </label>
            <label class="gc-param">
              <span>{$t('fleet.mission.offsetBearing')}</span>
              <NumberStepper bind:value={offsetBearing} min={0} max={359} step={5} unit="°" disabled={offsetM === 0} />
            </label>
          {:else if kind === 'arm' || kind === 'disarm'}
            <label class="gc-check">
              <input type="checkbox" checked={force} onchange={(e) => (force = (e.currentTarget as HTMLInputElement).checked)} />
              <span>{kind === 'arm' ? $t('control.action.forceArmHint') : $t('control.action.forceDisarmHint')}</span>
            </label>
          {/if}
        </div>
      {/if}

      <div class="gc-section">{$t('fleet.group.targets')} · {includedCount}/{targets.length}</div>
      <ul class="gc-list">
        {#each targets as tg (tg.vehicle.vehicleId)}
          {@const id = tg.vehicle.vehicleId}
          {@const c = checks.get(id) ?? { level: 'ok', reasons: [] }}
          {@const on = included.has(id)}
          <li class="gc-row {c.level}" class:off={!on}>
            <label class="gc-row-main">
              <input type="checkbox" checked={on} onchange={() => toggleInclude(id)} title={$t('fleet.group.include')} />
              <span class="gc-dot" style:background={tg.color}></span>
              <span class="gc-name">{tg.vehicle.name}</span>
              <span class="gc-meta">L{tg.vehicle.linkId}:S{tg.vehicle.sysid} · {tg.vehicle.fcVariant}</span>
            </label>
            {#if c.reasons.length}
              <div class="gc-reasons">
                {#each c.reasons as r (r)}
                  <span class="gc-reason">{$t('fleet.check.' + r)}</span>
                {/each}
              </div>
            {/if}
          </li>
        {/each}
      </ul>

      <div class="gc-actions">
        <Button size="sm" onclick={() => close(null)}>{$t('connection.btNameCancel')}</Button>
        <div class="gc-confirm">
          <HoldToConfirm variant={dangerKinds.includes(kind) ? 'danger' : 'warning'} disabled={includedCount === 0} onconfirm={confirm}>
            {includedCount === 0 ? $t('fleet.group.sendNone') : $t('fleet.group.send', { values: { n: includedCount } })}
          </HoldToConfirm>
        </div>
      </div>
    </div>
  </div>
{/if}

<style>
  .gc-backdrop {
    position: fixed;
    inset: 0;
    z-index: 9999;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(0, 0, 0, 0.55);
    font-family: 'Segoe UI', Tahoma, sans-serif;
  }
  .gc-box {
    width: 460px;
    max-width: 94vw;
    max-height: 86vh;
    display: flex;
    flex-direction: column;
    padding: 14px 16px 14px;
    background: #2e2e2e;
    border: 1px solid #555;
    border-radius: 8px;
    box-shadow: 0 12px 36px rgba(0, 0, 0, 0.55);
    color: #e0e0e0;
    font-size: 12px;
  }
  .gc-title {
    display: flex;
    align-items: baseline;
    gap: 8px;
    margin-bottom: 10px;
  }
  .gc-title-kind {
    font-size: 15px;
    font-weight: 700;
    color: #37a8db;
  }
  .gc-title-sub {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: #9a9a9a;
  }

  .gc-params {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-bottom: 10px;
    padding-bottom: 10px;
    border-bottom: 1px solid #444;
  }
  .gc-param {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }
  .gc-param > span { color: #cfcfcf; }
  .gc-select {
    min-width: 160px;
    height: 26px;
    padding: 0 8px;
    background: #434343;
    border: 1px solid #555;
    border-radius: 4px;
    color: #e0e0e0;
    font-size: 12px;
  }
  .gc-check {
    display: flex;
    align-items: center;
    gap: 8px;
    color: #f6e3b0;
  }
  .gc-note {
    font-size: 11px;
    color: #9a9a9a;
    line-height: 1.4;
  }

  .gc-section {
    margin: 2px 0 4px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: #9a9a9a;
  }
  .gc-list {
    list-style: none;
    margin: 0 0 10px;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    overflow-y: auto;
    min-height: 0;
    scrollbar-width: thin;
    scrollbar-color: #555 transparent;
  }
  .gc-row {
    padding: 5px 8px;
    background: #363636;
    border: 1px solid #4a4a4a;
    border-radius: 4px;
  }
  .gc-row.warn { border-color: rgba(245, 166, 35, 0.6); }
  .gc-row.fail { border-color: rgba(224, 108, 108, 0.6); }
  .gc-row.off { opacity: 0.55; }
  .gc-row-main {
    display: flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
  }
  .gc-dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    flex: 0 0 auto;
    box-shadow: 0 0 3px rgba(0, 0, 0, 0.6);
  }
  .gc-name { font-weight: 600; white-space: nowrap; }
  .gc-meta {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: #9a9a9a;
    font-size: 11px;
  }
  .gc-reasons {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin: 4px 0 0 24px;
  }
  .gc-reason {
    padding: 1px 6px;
    border-radius: 3px;
    font-size: 10.5px;
    background: rgba(255, 255, 255, 0.06);
    color: #f6e3b0;
  }
  .gc-row.fail .gc-reason { color: #f0a0a0; }

  .gc-actions {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }
  .gc-confirm { flex: 1 1 auto; max-width: 280px; }
</style>
