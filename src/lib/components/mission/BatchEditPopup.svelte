<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // Batch-edit popup for a multi-selection of waypoints. One APPLY button (no
  // live-apply → undo/redo-friendly). Fields use the shared UnitStepper/
  // NumberStepper; a field shows "---" (empty) when the selected WPs differ.
  // Editing a field applies it to ALL applicable WPs; after APPLY the popup
  // reloads fresh (relative-change resets, diff state recomputed).
  //   • Altitude (absolute) + Relative Change (delta, keeps relative differences)
  //   • Alt-mode toggle — blocked + warning when modes differ; clicking converts
  //     all selected to one mode (terrain/launch-aware)
  //   • Speed (Waypoint/Land), Hold time (PosHold-Time), User-Action bits
  // All values are unit-aware (steppers keep metric base internally).
  import { t } from 'svelte-i18n';
  import {
    mission, selectedWpIndices, missionUpdateWp, WpAction, hasLocation,
    ALT_MODE_REL, ALT_MODE_AMSL, beginUndoGroup, endUndoGroup, type Waypoint,
  } from '$lib/stores/mission';
  import { batchEdit, closeBatchEdit } from '$lib/stores/batchEdit';
  import { autopilotSystem } from '$lib/stores/autopilotContext';
  import { get } from 'svelte/store';
  import { convertAltCm } from '$lib/helpers/altConvert';
  import UnitStepper from '$lib/components/UnitStepper.svelte';
  import NumberStepper from '$lib/components/NumberStepper.svelte';
  import type { InterfaceSettings } from '$lib/stores/settings';

  let { interfaceSettings }: { interfaceSettings: InterfaceSettings } = $props();

  const UA_ACTIONS = [WpAction.Waypoint, WpAction.PosholdUnlim, WpAction.PosholdTime, WpAction.Land];

  // ── Selected WPs + applicable subsets (live) ───────────────────────
  const selWps = $derived(
    [...$selectedWpIndices]
      .sort((a, b) => a - b)
      .map((i) => ({ i, wp: $mission.waypoints[i] }))
      .filter((x): x is { i: number; wp: Waypoint } => !!x.wp),
  );
  const altWps = $derived(selWps.filter((x) => hasLocation(x.wp.action)));
  const speedWps = $derived(selWps.filter((x) => x.wp.action === WpAction.Waypoint || x.wp.action === WpAction.Land));
  const holdWps = $derived(selWps.filter((x) => x.wp.action === WpAction.PosholdTime));
  const uaWps = $derived(selWps.filter((x) => UA_ACTIONS.includes(x.wp.action)));

  const wpMode = (wp: Waypoint) => wp.alt_mode ?? ((wp.p3 & 1) ? ALT_MODE_AMSL : ALT_MODE_REL);
  const altModes = $derived(new Set(altWps.map((x) => wpMode(x.wp))));
  const altModeMixed = $derived(altModes.size > 1);
  const altModeCommon = $derived(altModes.size === 1 ? ([...altModes][0] as number) : null);
  const MODE_LABEL = ['REL', 'AMSL', 'AGL'];

  // ── Editable field state (metric base; NaN = "mixed/empty") ────────
  let altM = $state(NaN);
  let relM = $state(0);
  let speedMs = $state(NaN);
  let holdSec = $state(NaN);
  let ua = $state<('keep' | 'on' | 'off')[]>(['keep', 'keep', 'keep', 'keep']);
  let initAltCm = NaN;
  let initSpeedCms = NaN;
  let initHoldS = NaN;
  let busy = $state(false);
  let loaded = false;

  /** Common raw value across a list, or NaN if they differ / empty. */
  function commonRaw(vals: number[]): number {
    return vals.length > 0 && vals.every((v) => v === vals[0]) ? vals[0] : NaN;
  }

  function load() {
    initAltCm = commonRaw(altWps.map((x) => x.wp.altitude));
    altM = Number.isNaN(initAltCm) ? NaN : initAltCm / 100;
    relM = 0;
    initSpeedCms = commonRaw(speedWps.map((x) => x.wp.p1));
    speedMs = Number.isNaN(initSpeedCms) ? NaN : initSpeedCms / 100;
    initHoldS = commonRaw(holdWps.map((x) => x.wp.p1));
    holdSec = initHoldS;
    for (let b = 1; b <= 4; b++) {
      const bits = uaWps.map((x) => (x.wp.p3 >> b) & 1);
      ua[b - 1] = bits.length === 0 ? 'keep' : bits.every((v) => v === 1) ? 'on' : bits.every((v) => v === 0) ? 'off' : 'keep';
    }
  }

  // Load once when the popup opens.
  $effect(() => {
    if ($batchEdit && !loaded) {
      load();
      loaded = true;
    } else if (!$batchEdit) {
      loaded = false;
    }
  });

  // Close if the selection empties (e.g. leaving edit mode clears it).
  $effect(() => {
    if ($batchEdit && selWps.length === 0) closeBatchEdit();
  });

  // Close when the autopilot system switches out from under the popup.
  let lastSystem = get(autopilotSystem); // plain (not $state) → the effect only tracks the store
  $effect(() => {
    const s = $autopilotSystem;
    if (s !== lastSystem) { lastSystem = s; closeBatchEdit(); }
  });

  // Click / tap anywhere outside the popup closes it (the opening context-menu click already fired its
  // pointerdown before the popup existed, so it never self-closes).
  let popupEl: HTMLDivElement | undefined = $state();
  function onWinPointerDown(e: PointerEvent) {
    if (!$batchEdit) return;
    if (popupEl && e.target instanceof Node && popupEl.contains(e.target)) return;
    closeBatchEdit();
  }

  function cycleUa(b: number) {
    ua[b] = ua[b] === 'keep' ? 'on' : ua[b] === 'on' ? 'off' : 'keep';
  }

  // ── Alt-mode toggle: convert ALL selected to one mode (unifies if mixed) ──
  async function cycleAltMode() {
    if (busy || altWps.length === 0) return;
    busy = true;
    beginUndoGroup(); // batch alt-mode change = one undo step
    try {
      const target = ((altModeCommon ?? ALT_MODE_REL) + 1) % 3;
      for (const { i, wp } of altWps) {
        const from = wpMode(wp);
        const altitude = from === target ? wp.altitude : await convertAltCm(wp, from, target);
        await missionUpdateWp(i, { ...wp, alt_mode: target, altitude });
      }
      load();
    } finally {
      endUndoGroup();
      busy = false;
    }
  }

  async function apply() {
    if (busy) return;
    busy = true;
    try {
      const updates = new Map<number, Waypoint>();
      const draft = (i: number, wp: Waypoint) => {
        if (!updates.has(i)) updates.set(i, { ...wp });
        return updates.get(i) as Waypoint;
      };

      // Altitude: relative delta wins if given, else absolute set (modes uniform)
      const relCm = !Number.isNaN(relM) && relM !== 0 ? Math.round(relM * 100) : 0;
      const absCm = !altModeMixed && !Number.isNaN(altM) ? Math.round(altM * 100) : NaN;
      const absChanged = !Number.isNaN(absCm) && absCm !== initAltCm;
      for (const { i, wp } of altWps) {
        if (relCm !== 0) draft(i, wp).altitude = wp.altitude + relCm;
        else if (absChanged) draft(i, wp).altitude = absCm;
      }

      // Speed (Waypoint/Land → p1 cm/s)
      const spdCms = !Number.isNaN(speedMs) ? Math.round(speedMs * 100) : NaN;
      if (!Number.isNaN(spdCms) && spdCms !== initSpeedCms) {
        for (const { i, wp } of speedWps) draft(i, wp).p1 = spdCms;
      }

      // Hold time (PosHold-Time → p1 seconds)
      const hold = !Number.isNaN(holdSec) ? Math.max(0, Math.round(holdSec)) : NaN;
      if (!Number.isNaN(hold) && hold !== initHoldS) {
        for (const { i, wp } of holdWps) draft(i, wp).p1 = hold;
      }

      // User actions (p3 bits 1-4, read-modify-write so the alt-mode bit0 is kept)
      for (let b = 1; b <= 4; b++) {
        const st = ua[b - 1];
        if (st === 'keep') continue;
        for (const { i, wp } of uaWps) {
          const u = draft(i, wp);
          u.p3 = st === 'on' ? u.p3 | (1 << b) : u.p3 & ~(1 << b);
        }
      }

      if (updates.size > 0) {
        beginUndoGroup(); // whole batch apply = one undo step
        try {
          for (const [i, wp] of updates) await missionUpdateWp(i, wp);
        } finally {
          endUndoGroup();
        }
      }
      load(); // refresh diff state + reset relative change
    } finally {
      busy = false;
    }
  }
</script>

<svelte:window
  onkeydown={(e) => { if (e.key === 'Escape') closeBatchEdit(); }}
  onpointerdown={onWinPointerDown}
/>

{#if $batchEdit}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="batch-popup" bind:this={popupEl} style="left:{$batchEdit.x}px; top:{$batchEdit.y}px;">
    <div class="bp-header">
      <span class="bp-title">{$t('mission.batchEdit')} · {selWps.length}</span>
      <button class="bp-x" onclick={closeBatchEdit} title={$t('video.close')}>✕</button>
    </div>

    {#if altWps.length}
      <div class="bp-row">
        <span class="bp-label">{$t('missionLayer.alt')}</span>
        <UnitStepper
          kind="altitude"
          settings={interfaceSettings}
          bind:value={altM}
          decimals={0}
          allowEmpty
          placeholder="---"
          disabled={altModeMixed || busy}
        />
        <button class="bp-toggle" onclick={cycleAltMode} disabled={busy} title={$t('batch.altModeHint')}>
          {altModeMixed ? '---' : MODE_LABEL[altModeCommon ?? 0]}
        </button>
      </div>
      {#if altModeMixed}
        <div class="bp-warn">{$t('batch.mixedAltWarn')}</div>
      {/if}
      <div class="bp-row">
        <span class="bp-label">{$t('batch.relAlt')}</span>
        <UnitStepper kind="altitude" settings={interfaceSettings} bind:value={relM} decimals={0} disabled={busy} />
      </div>
    {/if}

    {#if speedWps.length}
      <div class="bp-row">
        <span class="bp-label">{$t('missionLayer.speed')}</span>
        <UnitStepper
          kind="speed"
          settings={interfaceSettings}
          bind:value={speedMs}
          min={0}
          decimals={1}
          displayStep={0.1}
          allowEmpty
          placeholder="---"
          disabled={busy}
        />
      </div>
    {/if}

    {#if holdWps.length}
      <div class="bp-row">
        <span class="bp-label">{$t('missionLayer.hold')}</span>
        <NumberStepper bind:value={holdSec} min={0} step={1} decimals={0} unit={$t('missionLayer.sec')} allowEmpty placeholder="---" disabled={busy} />
      </div>
    {/if}

    {#if uaWps.length}
      <div class="bp-row">
        <span class="bp-label">{$t('missionLayer.actions')}</span>
        <div class="bp-ua">
          {#each [0, 1, 2, 3] as b}
            <button
              class="bp-ua-btn"
              class:on={ua[b] === 'on'}
              class:off={ua[b] === 'off'}
              onclick={() => cycleUa(b)}
              disabled={busy}
              title="UA{b + 1}"
            >UA{b + 1}{ua[b] === 'keep' ? ' –' : ua[b] === 'on' ? ' ✓' : ' ✕'}</button>
          {/each}
        </div>
      </div>
    {/if}

    <div class="bp-actions">
      <button class="bp-apply" onclick={apply} disabled={busy}>{$t('batch.apply')}</button>
    </div>
  </div>
{/if}

<style>
  /* Mix of side-panel frame + widget background, like the context menu. */
  .batch-popup {
    position: fixed;
    z-index: 1900;
    width: 290px;
    padding: 8px;
    background: rgba(30, 30, 30, 0.82);
    backdrop-filter: blur(10px);
    border: 1px solid rgba(55, 168, 219, 0.35);
    border-radius: 8px;
    box-shadow: 0 6px 24px rgba(0, 0, 0, 0.5);
    color: #e0e0e0;
    font-size: 12px;
  }
  .bp-header {
    display: flex;
    align-items: center;
    margin-bottom: 8px;
    padding-bottom: 6px;
    border-bottom: 1px solid #444;
  }
  /* Wider stepper fields so the value never collides with the unit (e.g. km/h),
     kept uniform across all rows for symmetry. */
  .batch-popup :global(.ns-input) {
    width: 70px;
    padding-right: 30px;
  }
  .bp-title {
    flex: 1;
    font-weight: 600;
    color: #37a8db;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    font-size: 11px;
  }
  .bp-x {
    width: 18px;
    height: 18px;
    border: none;
    background: transparent;
    color: #aaa;
    cursor: pointer;
    border-radius: 3px;
  }
  .bp-x:hover {
    background: rgba(255, 255, 255, 0.12);
    color: #fff;
  }
  .bp-row {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 6px;
  }
  .bp-label {
    width: 46px;
    color: #999;
    flex-shrink: 0;
  }
  .bp-toggle {
    flex-shrink: 0;
    min-width: 46px;
    background: #333;
    color: #cfe8f5;
    border: 1px solid rgba(55, 168, 219, 0.4);
    border-radius: 4px;
    padding: 4px 6px;
    font-size: 11px;
    cursor: pointer;
  }
  .bp-toggle:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .bp-warn {
    color: #f1c40f;
    font-size: 10.5px;
    line-height: 1.25;
    margin: -2px 0 6px;
  }
  .bp-ua {
    display: flex;
    gap: 4px;
    flex: 1;
  }
  .bp-ua-btn {
    flex: 1;
    background: #2a2a2a;
    color: #999;
    border: 1px solid #555;
    border-radius: 4px;
    padding: 3px 2px;
    font-size: 10px;
    cursor: pointer;
    white-space: nowrap;
  }
  .bp-ua-btn.on {
    border-color: #27ae60;
    color: #2ecc71;
  }
  .bp-ua-btn.off {
    border-color: #c0392b;
    color: #e74c3c;
  }
  .bp-actions {
    display: flex;
    justify-content: flex-end;
    margin-top: 8px;
    padding-top: 8px;
    border-top: 1px solid #444;
  }
  .bp-apply {
    background: rgba(55, 168, 219, 0.18);
    color: #cfe8f5;
    border: 1px solid rgba(55, 168, 219, 0.5);
    border-radius: 5px;
    padding: 5px 14px;
    font-weight: 600;
    cursor: pointer;
  }
  .bp-apply:hover {
    background: rgba(55, 168, 219, 0.3);
  }
  .bp-apply:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
