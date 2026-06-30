<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // Batch-edit popup for a multi-selection of ArduPilot/PX4 waypoints — the cross-autopilot counterpart
  // of BatchEditPopup (INAV). One APPLY button (no live-apply → undo/redo-friendly). A field shows
  // "---" (empty) when the selected location WPs differ. Mirrors the INAV mapping:
  //   • Altitude (absolute, metres) + Relative Change (delta) — matches the metres-based Ardu editor
  //   • Frame toggle (REL/AMSL/TRN) — clicking unifies all selected to one frame; mixed blocks the
  //     absolute-alt field (the number means different things per frame), like INAV's alt-mode rule
  //   • Add action — appends a DO_ command to every selected waypoint (the cross-stack equivalent of
  //     INAV's User-Action bits; "just like single edit's Add modifier")
  import { t } from 'svelte-i18n';
  import { get } from 'svelte/store';
  import {
    arduMission, arduSelectedWpIndices, arduVehicleClass, arduUpdateWp, arduClearWpSelection,
    arduBeginUndoGroup, arduEndUndoGroup,
    groupArduMission, groupEndIndex,
    MAV_FRAME_GLOBAL, MAV_FRAME_GLOBAL_RELATIVE_ALT, MAV_FRAME_GLOBAL_TERRAIN_ALT,
    type ArduWaypoint, type MavFrame,
  } from '$lib/stores/missionArdupilot';
  import { arduBatchEdit, closeArduBatchEdit } from '$lib/stores/arduBatchEdit';
  import {
    CMD, cmdHasLocation, cmdStandaloneCoordinate, cmdDefaultCoordParams, modifierCommandsByCategory,
  } from '$lib/helpers/arduCommandCatalog';
  import { autopilotSystem } from '$lib/stores/autopilotContext';
  import type { ArduAction } from '$lib/stores/surveyPattern.svelte';
  import NumberStepper from '$lib/components/NumberStepper.svelte';
  import SurveyArduAction from '$lib/components/mission/SurveyArduAction.svelte';

  const firmware = $derived($autopilotSystem === 'px4' ? 'px4' : 'ardupilot');
  // Same modifier set as the single-waypoint editor's "+ Add modifier" (includes Change Speed, servo,
  // relay, camera, delay …) — flattened to ids for the picker dropdown.
  const actionCmds = $derived(
    modifierCommandsByCategory($arduVehicleClass, firmware).flatMap((g) => g.cmds.map((c) => c.id)),
  );

  // ── Selected WPs + the location subset (alt/frame/action apply to located WPs) ──
  const selWps = $derived(
    [...$arduSelectedWpIndices]
      .sort((a, b) => a - b)
      .map((i) => ({ i, wp: $arduMission[i] }))
      .filter((x): x is { i: number; wp: ArduWaypoint } => !!x.wp),
  );
  const locWps = $derived(selWps.filter((x) => cmdHasLocation(x.wp.command)));
  const frames = $derived(new Set(locWps.map((x) => x.wp.frame)));
  const frameMixed = $derived(frames.size > 1);
  const frameCommon = $derived(frames.size === 1 ? ([...frames][0] as MavFrame) : null);

  function frameLabel(frame: MavFrame): string {
    if (frame === MAV_FRAME_GLOBAL) return 'AMSL';
    if (frame === MAV_FRAME_GLOBAL_TERRAIN_ALT) return 'TRN';
    return 'REL';
  }
  function nextFrame(frame: MavFrame): MavFrame {
    if (frame === MAV_FRAME_GLOBAL_RELATIVE_ALT) return MAV_FRAME_GLOBAL;
    if (frame === MAV_FRAME_GLOBAL) return MAV_FRAME_GLOBAL_TERRAIN_ALT;
    return MAV_FRAME_GLOBAL_RELATIVE_ALT;
  }

  // ── Editable field state (metres; NaN = "mixed/empty") ──────────────
  let altM = $state(NaN);
  let relM = $state(0);
  let pendingAction = $state<ArduAction | null>(null);
  let initAlt = NaN;
  let busy = $state(false);
  let loaded = false;

  /** Common raw value across a list, or NaN if they differ / empty. */
  function commonRaw(vals: number[]): number {
    return vals.length > 0 && vals.every((v) => v === vals[0]) ? vals[0] : NaN;
  }

  function load() {
    initAlt = commonRaw(locWps.map((x) => x.wp.alt));
    altM = Number.isNaN(initAlt) ? NaN : initAlt;
    relM = 0;
    pendingAction = null;
  }

  // Load once when the popup opens.
  $effect(() => {
    if ($arduBatchEdit && !loaded) {
      load();
      loaded = true;
    } else if (!$arduBatchEdit) {
      loaded = false;
    }
  });

  // Close if the selection empties (e.g. leaving edit mode clears it).
  $effect(() => {
    if ($arduBatchEdit && selWps.length === 0) closeArduBatchEdit();
  });

  // Close when the autopilot system switches out from under the popup (stale selection on the new stack).
  let lastSystem = get(autopilotSystem); // plain (not $state) → the effect only tracks the store
  $effect(() => {
    const s = $autopilotSystem;
    if (s !== lastSystem) { lastSystem = s; closeArduBatchEdit(); }
  });

  // Click / tap anywhere outside the popup closes it (the opening context-menu click already fired its
  // pointerdown before the popup existed, so it never self-closes).
  let popupEl: HTMLDivElement | undefined = $state();
  function onWinPointerDown(e: PointerEvent) {
    if (!$arduBatchEdit) return;
    if (popupEl && e.target instanceof Node && popupEl.contains(e.target)) return;
    closeArduBatchEdit();
  }

  // ── Frame toggle: unify ALL selected location WPs to one frame ──────
  function cycleFrame() {
    if (busy || locWps.length === 0) return;
    busy = true;
    arduBeginUndoGroup(); // batch frame change = one undo step
    try {
      const target: MavFrame = frameMixed
        ? MAV_FRAME_GLOBAL_RELATIVE_ALT // mixed → unify to REL first; further clicks cycle
        : nextFrame(frameCommon ?? MAV_FRAME_GLOBAL_RELATIVE_ALT);
      for (const { i, wp } of locWps) arduUpdateWp(i, { ...wp, frame: target });
    } finally {
      arduEndUndoGroup();
      busy = false;
    }
    load();
  }

  /** Build a DO_ action item for `anchor` — standalone-coordinate actions (e.g. ROI) take the anchor's
   *  position; others keep their catalog coordinate defaults. Mirrors the layer's add-modifier insert. */
  function buildActionWp(a: ArduAction, anchor: ArduWaypoint): ArduWaypoint {
    const standalone = cmdStandaloneCoordinate(a.command);
    const coord = cmdDefaultCoordParams(a.command);
    return {
      command: a.command, frame: MAV_FRAME_GLOBAL_RELATIVE_ALT,
      param1: a.param1, param2: a.param2, param3: a.param3, param4: a.param4,
      lat: standalone ? anchor.lat : (coord.x ?? 0),
      lon: standalone ? anchor.lon : (coord.y ?? 0),
      alt: coord.z ?? 0, autocontinue: true,
    };
  }

  /** Insert the action into each selected location WP's group (descending so insert indices stay valid).
   *  A JUMP_TAG leads its waypoint (the FC resumes at the next nav WP after the tag), every other
   *  modifier trails it — matching the single-editor's add-modifier placement. */
  function appendAction(a: ArduAction, selAnchors: Set<number>) {
    const wps = get(arduMission);
    const groups = groupArduMission(wps);
    const leading = a.command === CMD.JUMP_TAG;
    const inserts: { at: number; mod: ArduWaypoint }[] = [];
    for (const g of groups) {
      if (g.anchor && selAnchors.has(g.anchorIdx)) {
        inserts.push({ at: leading ? g.anchorIdx : groupEndIndex(g), mod: buildActionWp(a, g.anchor) });
      }
    }
    inserts.sort((x, y) => y.at - x.at);
    const out = [...wps];
    for (const { at, mod } of inserts) out.splice(at, 0, mod);
    arduMission.set(out);
  }

  function apply() {
    if (busy) return;
    busy = true;
    const targets = locWps; // snapshot the selection before mutating
    const selAnchors = new Set(targets.map((x) => x.i));
    let structural = false; // an action append inserts items → selection indices shift
    arduBeginUndoGroup(); // whole batch apply = one undo step
    try {
      // Altitude: relative delta wins if given, else absolute set (frames must be uniform).
      const relDelta = !Number.isNaN(relM) && relM !== 0 ? relM : 0;
      const absChanged = !frameMixed && !Number.isNaN(altM) && altM !== initAlt;
      for (const { i, wp } of targets) {
        if (relDelta !== 0) arduUpdateWp(i, { ...wp, alt: wp.alt + relDelta });
        else if (absChanged) arduUpdateWp(i, { ...wp, alt: altM });
      }
      // Action append (after the alt edits → indices unchanged; append shifts indices last).
      if (pendingAction) { appendAction(pendingAction, selAnchors); structural = true; }
    } finally {
      arduEndUndoGroup();
      busy = false;
    }
    // An action append shifts every later index, so the stored selection is stale → drop it (which
    // closes the popup). Value-only edits keep the selection so the popup stays open for further tweaks.
    if (structural) arduClearWpSelection();
    else load(); // refresh diff state + reset relative change
  }
</script>

<svelte:window
  onkeydown={(e) => { if (e.key === 'Escape') closeArduBatchEdit(); }}
  onpointerdown={onWinPointerDown}
/>

{#if $arduBatchEdit}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="batch-popup" bind:this={popupEl} style="left:{$arduBatchEdit.x}px; top:{$arduBatchEdit.y}px;">
    <div class="bp-header">
      <span class="bp-title">{$t('mission.batchEdit')} · {selWps.length}</span>
      <button class="bp-x" onclick={closeArduBatchEdit} title={$t('video.close')}>✕</button>
    </div>

    {#if locWps.length}
      <div class="bp-row">
        <span class="bp-label">{$t('missionLayer.alt')}</span>
        <NumberStepper
          bind:value={altM}
          min={0}
          step={1}
          decimals={0}
          unit="m"
          allowEmpty
          placeholder="---"
          disabled={frameMixed || busy}
        />
        <button class="bp-toggle" onclick={cycleFrame} disabled={busy} title={$t('batch.frameHint')}>
          {frameMixed ? '---' : frameLabel(frameCommon ?? MAV_FRAME_GLOBAL_RELATIVE_ALT)}
        </button>
      </div>
      {#if frameMixed}
        <div class="bp-warn">{$t('batch.mixedFrameWarn')}</div>
      {/if}
      <div class="bp-row">
        <span class="bp-label">{$t('batch.relAlt')}</span>
        <NumberStepper bind:value={relM} step={1} decimals={0} unit="m" disabled={busy} />
      </div>

      <div class="bp-action">
        <SurveyArduAction
          label={$t('batch.addAction')}
          value={pendingAction}
          {firmware}
          commands={actionCmds}
          onchange={(v) => (pendingAction = v)}
        />
      </div>
    {/if}

    <div class="bp-actions">
      <button class="bp-apply" onclick={apply} disabled={busy}>{$t('batch.apply')}</button>
    </div>
  </div>
{/if}

<style>
  /* Mirrors BatchEditPopup (INAV) — side-panel frame + widget background, like the context menu. */
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
  /* The action picker sits in its own block (label + dropdown + param rows). */
  .bp-action {
    margin-top: 4px;
    padding-top: 6px;
    border-top: 1px dashed #444;
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
