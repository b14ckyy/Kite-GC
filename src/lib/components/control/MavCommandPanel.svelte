<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // Vehicle-control panel (compact) — direct GCS command of a MAVLink vehicle (ArduPilot + PX4):
  // arm/disarm, flight-mode, takeoff/land/RTL, Guided toggle, mission start/restart. MAVLink-only in
  // V1 (INAV guided + joystick are later phases). See docs/active/VEHICLE_CONTROL.md.
  import { t } from 'svelte-i18n';
  import PanelShell from '$lib/components/panel/PanelShell.svelte';
  import Button from '$lib/components/panel/Button.svelte';
  import HoldToConfirm from '$lib/components/panel/HoldToConfirm.svelte';
  import ArmSlider from '$lib/components/panel/ArmSlider.svelte';
  import NumberStepper from '$lib/components/NumberStepper.svelte';
  import { autopilotSystem } from '$lib/stores/autopilotContext';
  import { arduVehicleClass, arduMission, arduMissionFcSynced, MAV_CMD_SHORT } from '$lib/stores/missionArdupilot';
  import { modesFor, type MavMode } from '$lib/helpers/mavModes';
  import {
    controlAvailable, rcLinkPresent, isArmed, activeMode, busyAction, lastFeedback,
    guidedActive,
    setMode, arm, disarm, takeoff, land, rtl, missionStart, missionRestart, missionSetCurrent, setGuided,
    changeAlt, changeSpeed, setLoiterRadius, setHomeHere, abortLanding, setHeading, vtolTransition,
  } from '$lib/controllers/vehicleControl';

  const sys = $derived($autopilotSystem);
  const cls = $derived($arduVehicleClass);
  const allModes = $derived(modesFor(sys, cls));
  const safeModes = $derived(allModes.filter((m) => !m.stick));
  const stickModes = $derived(allModes.filter((m) => m.stick));

  // Modes with their own dedicated control (Guided toggle, RTL/Land hold buttons, Takeoff command,
  // PX4 mission/hold modes) are not repeated as quick mode buttons.
  const DEDICATED = new Set(['guided', 'hold', 'rtl', 'return', 'land', 'takeoff', 'mission']);
  const quickModes = $derived(safeModes.filter((m) => !DEDICATED.has(m.key)));

  let showAllModes = $state(false);
  const stickUnlocked = $derived($rcLinkPresent); // stick modes selectable only with an RC link

  let takeoffAlt = $state(50);

  // Takeoff via NAV_TAKEOFF only makes sense for multirotors / VTOL / PX4. A fixed-wing plane takes
  // off through an AUTO mission (takeoff WP) or manually — the command is rejected, so hide it.
  const takeoffSupported = $derived(sys === 'px4' ? true : (cls === 'copter' || cls === 'quadplane'));

  // Vehicle-capability gates for the active-flight adjustments. Fixed-wing covers ArduPlane and PX4
  // fixed-wing/VTOL alike (the vehicle class comes from MAV_TYPE for PX4, fc_variant for ArduPilot) —
  // loiter radius + landing go-around are fixed-wing concepts on both firmwares (the loiter-radius
  // parameter name differs; the controller picks it). Copter/Rover don't get these.
  const isFixedWing = $derived(cls === 'plane' || cls === 'quadplane');
  const hasAltitude = $derived(sys === 'px4' || cls === 'copter' || cls === 'quadplane' || cls === 'plane');
  // Plain ArduPlane fixed-wing has no land-now command (landing = AUTO sequence / RTL) → hide Land.
  // Copter, QuadPlane (→ QLAND) and PX4 (Land mode) all support it.
  const landSupported = $derived(!(sys === 'ardupilot' && cls === 'plane'));
  // Change Alt is a reposition (DO_REPOSITION) → only works while the FC is in the guided/reposition-
  // ready mode. In Loiter/Cruise/Auto the altitude is held / pilot- / mission-controlled, so hide it.
  const inGuided = $derived($activeMode?.guided === true);

  let changeAltVal = $state(50);
  let changeSpeedVal = $state(10);
  let loiterRadVal = $state(80);
  let headingVal = $state(0);
  // Set Heading is Copter-only (CONDITION_YAW). The fixed-wing GUIDED_CHANGE_HEADING path is removed:
  // ArduPlane's guided heading slew is buggy — it locks heading + bypasses waypoint nav and isn't
  // cleared by reposition (it even rejects descent), so it's unsafe to expose. PX4 has no equivalent.
  const canSetHeading = $derived(sys === 'ardupilot' && cls === 'copter' && inGuided);

  const activeKey = $derived($activeMode?.key ?? '');

  // ── Set active WP ──────────────────────────────────────────────────────────
  // The FC item index differs from the displayed WP number by the home slot: ArduPilot reserves
  // item 0 for home (so displayed WP n → FC seq n), PX4 has no home slot (displayed WP n → FC seq n-1).
  let setWpNum = $state(1); // 1-based displayed WP number
  const wpCount = $derived($arduMission.length);
  const homeSlot = $derived(sys === 'px4' ? 0 : 1);
  function setActiveWp() {
    const idx = setWpNum - 1;
    if (idx < 0 || idx >= wpCount) return;
    void missionSetCurrent(idx + homeSlot);
  }
  function wpLabel(i: number): string {
    const wp = $arduMission[i];
    const short = wp ? (MAV_CMD_SHORT[wp.command] ?? '') : '';
    return short ? `WP ${i + 1} · ${short}` : `WP ${i + 1}`;
  }

  // Auto-off the Guided toggle when the FC's active mode is no longer the guided mode (user picked
  // another mode here or on the transmitter), keeping the map interaction in sync with the FC.
  $effect(() => {
    if ($guidedActive && $activeMode && !$activeMode.guided) {
      guidedActive.set(false);
    }
  });

  function busy(action: string): boolean { return $busyAction === action; }

  function pickMode(m: MavMode) { void setMode(m); }

  const feedback = $derived($lastFeedback);
  function actionLabel(action: string): string { return $t(`control.action.${action}`, { default: action }); }
</script>

<PanelShell variant="compact" title={$t('control.title')}>
  {#snippet body()}
    {#if !$controlAvailable}
      <div class="cc-notice">{$t('control.notConnected')}</div>
    {:else}
      <div class="cc">
        <!-- 1. Arm / disarm -->
        {#if !$isArmed}
          <ArmSlider label={$t('control.action.arm')} busy={busy('arm')} onconfirm={() => arm()} />
        {:else}
          <HoldToConfirm variant="danger" busy={busy('disarm')} onconfirm={() => disarm()}>
            {$t('control.action.disarm')}
          </HoldToConfirm>
        {/if}

        <!-- 2. Active flight mode (large, centred) -->
        <div class="cc-mode-readout" title={$t('control.mode.current')}>
          <span class="cc-mode-name">{$activeMode?.name ?? '—'}</span>
        </div>

        <!-- 3. Takeoff + target altitude (multirotor / VTOL / PX4 only) -->
        {#if takeoffSupported}
          <div class="cc-takeoff">
            <div class="cc-takeoff-btn">
              <HoldToConfirm busy={busy('takeoff')} onconfirm={() => takeoff(takeoffAlt)}>
                {$t('control.action.takeoff')}
              </HoldToConfirm>
            </div>
            <NumberStepper bind:value={takeoffAlt} min={1} max={1000} step={5} unit="m" />
          </div>
        {/if}

        <!-- 4. RTL / Land (+ Abort Landing / go-around for fixed-wing) -->
        <div class="cc-row">
          <HoldToConfirm variant="warning" busy={busy('rtl')} onconfirm={() => rtl()}>
            {$t('control.action.rtl')}
          </HoldToConfirm>
          {#if landSupported}
            <HoldToConfirm variant="warning" busy={busy('land')} onconfirm={() => land()}>
              {$t('control.action.land')}
            </HoldToConfirm>
          {/if}
        </div>
        {#if isFixedWing}
          <HoldToConfirm variant="warning" busy={busy('abortLanding')} onconfirm={() => abortLanding()}>
            {$t('control.action.abortLanding')}
          </HoldToConfirm>
        {/if}

        <!-- 4b. VTOL transition (QuadPlane / VTOL only) -->
        {#if cls === 'quadplane'}
          <section class="cc-sec">
            <span class="cc-sec-title">{$t('control.section.transition')}</span>
            <div class="cc-row">
              <HoldToConfirm variant="warning" busy={busy('vtolTransition')} onconfirm={() => vtolTransition(false)}>
                {$t('control.action.toHover')}
              </HoldToConfirm>
              <HoldToConfirm variant="warning" busy={busy('vtolTransition')} onconfirm={() => vtolTransition(true)}>
                {$t('control.action.toForward')}
              </HoldToConfirm>
            </div>
          </section>
        {/if}

        <!-- 5. Guided toggle -->
        <div class="cc-guided-btn">
          <Button variant="mode" active={$guidedActive} full onclick={() => setGuided(!$guidedActive)} title={$t('control.guidedHint')}>
            {$t('control.action.guided')}
          </Button>
        </div>
        {#if $guidedActive}
          <div class="cc-guided-hint">{$t('control.guidedClickHint')}</div>
        {/if}

        <!-- 5b. Active-flight adjustments -->
        <section class="cc-sec">
          <span class="cc-sec-title">{$t('control.section.adjust')}</span>
          {#if hasAltitude && inGuided}
            <div class="cc-adjust">
              <NumberStepper bind:value={changeAltVal} min={1} max={1000} step={5} unit="m" />
              <Button variant="standard" size="sm" disabled={busy('changeAlt')} onclick={() => changeAlt(changeAltVal)}>{$t('control.action.changeAlt')}</Button>
            </div>
          {/if}
          {#if canSetHeading}
            <div class="cc-adjust">
              <NumberStepper bind:value={headingVal} min={0} max={359} step={5} unit="°" />
              <Button variant="standard" size="sm" disabled={busy('setHeading')} onclick={() => setHeading(headingVal)}>{$t('control.action.setHeading')}</Button>
            </div>
          {/if}
          <div class="cc-adjust">
            <NumberStepper bind:value={changeSpeedVal} min={1} max={100} step={1} unit="m/s" />
            <Button variant="standard" size="sm" disabled={busy('changeSpeed')} onclick={() => changeSpeed(changeSpeedVal, isFixedWing)}>{$t('control.action.changeSpeed')}</Button>
          </div>
          {#if isFixedWing}
            <div class="cc-adjust">
              <NumberStepper bind:value={loiterRadVal} min={10} max={2000} step={10} unit="m" />
              <Button variant="standard" size="sm" disabled={busy('setLoiterRadius')} onclick={() => setLoiterRadius(loiterRadVal)}>{$t('control.action.setLoiterRadius')}</Button>
            </div>
          {/if}
          <Button variant="standard" size="sm" full disabled={busy('setHome')} onclick={() => setHomeHere()}>{$t('control.action.setHome')}</Button>
        </section>

        <!-- 6. Other modes (quick switch) -->
        <section class="cc-sec">
          <span class="cc-sec-title">{$t('control.section.modes')}</span>
          <div class="cc-modegrid">
            {#each quickModes as m}
              <Button variant="mode" size="sm" active={activeKey === m.key} onclick={() => pickMode(m)}>{m.name}</Button>
            {/each}
          </div>
          {#if stickModes.length}
            <label class="cc-allmodes" title={stickUnlocked ? '' : $t('control.rcLockHint')}>
              <input type="checkbox" bind:checked={showAllModes} />
              <span>{$t('control.showAllModes')}</span>
              {#if showAllModes && !stickUnlocked}<span class="cc-warn">{$t('control.noRcLink')}</span>{/if}
            </label>
            {#if showAllModes}
              <div class="cc-modegrid">
                {#each stickModes as m}
                  <Button variant="mode" size="sm" active={activeKey === m.key} disabled={!stickUnlocked} onclick={() => pickMode(m)}>{m.name}</Button>
                {/each}
              </div>
            {/if}
          {/if}
        </section>

        <!-- 7. Mission control -->
        <section class="cc-sec">
          <span class="cc-sec-title">{$t('control.section.mission')}</span>
          <div class="cc-row">
            <Button variant="standard" full onclick={() => missionStart()}>{$t('control.action.missionStart')}</Button>
            <Button variant="standard" full onclick={() => missionRestart()}>{$t('control.action.missionRestart')}</Button>
          </div>

          <!-- Set active WP — only when the mission is in sync with the FC (no edits since
               download/upload), so the chosen WP number maps to the FC's actual item. -->
          <div class="cc-setwp" class:disabled={!$arduMissionFcSynced}>
            <select class="cc-select" bind:value={setWpNum} disabled={!$arduMissionFcSynced || wpCount === 0}>
              {#each Array(wpCount) as _, i}
                <option value={i + 1}>{wpLabel(i)}</option>
              {/each}
            </select>
            <Button variant="standard" size="sm" disabled={!$arduMissionFcSynced || wpCount === 0 || busy('setWp')} onclick={setActiveWp}>
              {$t('control.action.setWp')}
            </Button>
          </div>
          {#if !$arduMissionFcSynced}
            <div class="cc-setwp-hint">{$t('control.setWpHint')}</div>
          {/if}
        </section>
      </div>
    {/if}
  {/snippet}

  {#snippet footer()}
    {#if feedback}
      <div class="cc-feedback" class:ok={feedback.ok} class:err={!feedback.ok}>
        <span class="cc-fb-action">{actionLabel(feedback.action)}</span>
        <span class="cc-fb-msg">{feedback.ok ? $t('control.ack.ok') : feedback.message}</span>
      </div>
    {/if}
  {/snippet}
</PanelShell>

<style>
  .cc {
    display: flex;
    flex-direction: column;
    gap: 11px;
    padding: 4px 2px;
  }
  .cc-notice {
    padding: 18px 12px;
    text-align: center;
    color: #949494;
    font-size: 12.5px;
  }

  .cc-mode-readout {
    text-align: center;
    padding: 4px 0 2px;
  }
  .cc-mode-name {
    font-size: 20px;
    font-weight: 700;
    letter-spacing: 0.5px;
    color: #37a8db;
  }

  .cc-takeoff {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .cc-takeoff-btn { flex: 1; }

  .cc-adjust {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  /* Let the button take the remaining width next to the fixed-size stepper. */
  .cc-adjust :global(button) { flex: 1; }

  .cc-row {
    display: flex;
    gap: 6px;
  }

  /* Make the Guided mode-toggle button as tall as the hold-to-confirm actions. */
  .cc-guided-btn :global(button) {
    min-height: 38px;
    font-size: 13px;
    font-weight: 700;
  }

  .cc-guided-hint {
    font-size: 11px;
    color: #37a8db;
    font-style: italic;
    text-align: center;
  }

  .cc-sec {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .cc-sec-title {
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: #949494;
  }
  .cc-modegrid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 5px;
  }
  .cc-allmodes {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    color: #949494;
    cursor: pointer;
  }
  .cc-warn { color: #d4a017; }

  .cc-setwp {
    display: flex;
    gap: 6px;
    align-items: stretch;
  }
  .cc-setwp .cc-select { flex: 1; }
  .cc-setwp.disabled { opacity: 0.5; }
  .cc-setwp-hint {
    font-size: 10.5px;
    color: #949494;
    font-style: italic;
  }
  .cc-select {
    width: 100%;
    height: 30px;
    padding: 0 8px;
    background: #2e2e2e;
    color: #e0e0e0;
    border: 1px solid #272727;
    border-radius: 5px;
    font-size: 12.5px;
  }

  .cc-feedback {
    display: flex;
    gap: 8px;
    align-items: baseline;
    font-size: 11.5px;
    padding: 2px 2px;
  }
  .cc-feedback.ok .cc-fb-msg { color: #59aa29; }
  .cc-feedback.err .cc-fb-msg { color: #d40000; }
  .cc-fb-action { font-weight: 700; color: #cfd2d1; }
  .cc-fb-msg { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
