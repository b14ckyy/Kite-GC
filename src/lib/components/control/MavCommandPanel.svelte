<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script module lang="ts">
  // Module scope outlives the component, so the last-entered command values and the RC-modes fold state
  // survive a panel close/reopen.
  const savedInputs = { takeoffAlt: 50, changeAlt: 50, changeSpeed: 10, loiterRad: 80, heading: 0, rcModesOpen: false };
</script>

<script lang="ts">
  // Vehicle-control panel (compact) — direct GCS command of a MAVLink vehicle (ArduPilot + PX4):
  // arm/disarm, flight-mode, takeoff/land/RTL, Guided toggle, mission start/restart. MAVLink-only in
  // V1 (INAV guided + joystick are later phases). See docs/active/VEHICLE_CONTROL.md.
  import { t } from 'svelte-i18n';
  import { onDestroy } from 'svelte';
  import { get } from 'svelte/store';
  import PanelShell from '$lib/components/panel/PanelShell.svelte';
  import Button from '$lib/components/panel/Button.svelte';
  import HoldToConfirm from '$lib/components/panel/HoldToConfirm.svelte';
  import ArmSlider from '$lib/components/panel/ArmSlider.svelte';
  import NumberStepper from '$lib/components/NumberStepper.svelte';
  import { autopilotSystem } from '$lib/stores/autopilotContext';
  import { arduVehicleClass, arduMission, arduMissionFcSynced, MAV_CMD_SHORT, MAV_CMD_NAV_TAKEOFF } from '$lib/stores/missionArdupilot';
  import { CMD } from '$lib/helpers/arduCommandCatalog';
  import { offeredModes, guidedModeFor, type MavMode } from '$lib/helpers/mavModes';
  import {
    controlAvailable, stickModesUnlocked, isArmed, airborne, activeMode, busyAction, lastFeedback,
    guidedActive, activeFcModesChecked, modeTableMatchesVehicle,
    setMode, arm, disarm, takeoff, planeTakeoff, land, rtl, missionStart, missionRestart, missionDownload, missionSetCurrent, setGuided,
    changeAlt, changeSpeed, setLoiterRadius, setHomeHere, abortLanding, setHeading, vtolTransition,
  } from '$lib/controllers/vehicleControl';
  import { frameMissionOnMap } from '$lib/stores/mapCamera';
  import { requestMapAccess } from '$lib/stores/panelYield';

  const sys = $derived($autopilotSystem);
  const cls = $derived($arduVehicleClass);
  // The FC's own mode list (ArduPilot 4.7+) decides which modes exist; without one, Kite's static table.
  // Checked: during a vehicle switch (system/class not caught up yet) the list is held back.
  const allModes = $derived(offeredModes(sys, cls, $activeFcModesChecked));
  const safeModes = $derived(allModes.filter((m) => !m.stick));
  const stickModes = $derived(allModes.filter((m) => m.stick));

  // Modes with a dedicated control (Guided toggle, RTL/Land, Takeoff, PX4 mission) get no quick mode button. The
  // Guided mode is matched by its flag, not its key (PX4's Hold is it, ArduRover's Hold is ordinary). ArduPlane's
  // return/land modes and ArduSub's Surface sit beside RTL — see `landModes`.
  const DEDICATED = new Set(['guided', 'rtl', 'return', 'land', 'takeoff', 'mission', 'qrtl', 'qland', 'loiterqland', 'autoland', 'surface']);
  // Hold-to-confirm modes (return / land) go last, as one block under the ordinary flight modes.
  const quickModes = $derived.by(() => {
    const grid = safeModes.filter((m) => !m.guided && !DEDICATED.has(m.key));
    return [...grid.filter((m) => !m.confirm), ...grid.filter((m) => m.confirm)];
  });

  let rcModesOpen = $state(savedInputs.rcModesOpen);
  const stickUnlocked = $derived($stickModesUnlocked); // RC transmitter OR engaged RC control

  let takeoffAlt = $state(savedInputs.takeoffAlt);

  // Takeoff via NAV_TAKEOFF only makes sense for multirotors / VTOL / PX4 — on a plain ArduPlane it is the
  // QuadPlane VTOL takeoff and the FC rejects it.
  const takeoffSupported = $derived(sys === 'px4' ? true : (cls === 'copter' || cls === 'quadplane'));
  // A plain ArduPlane takes off in TAKEOFF mode instead (climbs to the FC's TKOFF_ALT) — its own row,
  // hidden when the FC's mode list does not offer the mode, and while a vehicle switch has not caught up
  // (mode 13 of another vehicle type is something else — Copter SPORT).
  const planeTakeoffMode = $derived(sys === 'ardupilot' && cls === 'plane' && $modeTableMatchesVehicle
    ? allModes.find((m) => m.key === 'takeoff') : undefined);
  // ArduPlane return/land modes beside RTL: QuadPlane → QRTL + Loiter QLand + QLand; plain plane → AutoLand
  // (4.7+). Driven by the FC's offered list, so an undetected QuadPlane still gets them; empty while a vehicle
  // switch hasn't caught up. ArduSub: Surface is the only button.
  const landModes = $derived.by(() => {
    if (sys !== 'ardupilot' || !$modeTableMatchesVehicle) return { qrtl: undefined, qland: undefined, loiterQland: undefined, autoland: undefined, surface: undefined };
    const by = (key: string) => allModes.find((m) => m.key === key);
    return { qrtl: by('qrtl'), qland: by('qland'), loiterQland: by('loiterqland'), autoland: by('autoland'), surface: by('surface') };
  });
  // The Guided toggle's mode; with the FC's mode list only when the FC offers GUIDED.
  const guidedMode = $derived(guidedModeFor(sys, cls, $activeFcModesChecked));

  // Fixed-wing (ArduPlane and PX4 fixed-wing/VTOL alike) gets loiter radius + landing go-around; the
  // controller picks the firmware's parameter name. Copter/Rover don't.
  const isFixedWing = $derived(cls === 'plane' || cls === 'quadplane');
  const hasAltitude = $derived(sys === 'px4' || cls === 'copter' || cls === 'quadplane' || cls === 'plane');
  // Plain ArduPlane fixed-wing has no land-now command (landing = AUTO sequence / RTL) → hide Land.
  // ArduRover / Boat have nothing to land, ArduSub surfaces instead. Copter, QuadPlane (→ QLAND) and
  // PX4 (Land mode) all support it.
  const landSupported = $derived(!(sys === 'ardupilot' && (cls === 'plane' || cls === 'rover' || cls === 'boat' || cls === 'sub')));
  // ArduSub has no RTL mode (mode numbers 6 / 11 are refused).
  const rtlSupported = $derived(!(sys === 'ardupilot' && cls === 'sub'));
  // The plain Land command (NAV_LAND) is not a QuadPlane's landing — QLand is, on its own button.
  const landCommand = $derived(landSupported && !(sys === 'ardupilot' && cls === 'quadplane'));
  // Change Alt is a reposition (DO_REPOSITION) → only works while the FC is in the guided/reposition-
  // ready mode. In Loiter/Cruise/Auto the altitude is held / pilot- / mission-controlled, so hide it.
  const inGuided = $derived($activeMode?.guided === true);

  let changeAltVal = $state(savedInputs.changeAlt);
  let changeSpeedVal = $state(savedInputs.changeSpeed);
  let loiterRadVal = $state(savedInputs.loiterRad);
  let headingVal = $state(savedInputs.heading);
  // Mirror the live input values back into module scope so they survive the panel closing/reopening.
  $effect(() => {
    savedInputs.takeoffAlt = takeoffAlt;
    savedInputs.changeAlt = changeAltVal;
    savedInputs.changeSpeed = changeSpeedVal;
    savedInputs.loiterRad = loiterRadVal;
    savedInputs.heading = headingVal;
    savedInputs.rcModesOpen = rcModesOpen;
  });
  // Set Heading is Copter-only (CONDITION_YAW): ArduPlane's GUIDED_CHANGE_HEADING locks heading, bypasses WP nav
  // and isn't cleared by reposition — unsafe to expose. PX4 has no equivalent.
  const canSetHeading = $derived(sys === 'ardupilot' && cls === 'copter' && inGuided);

  const activeKey = $derived($activeMode?.key ?? '');
  // In TAKEOFF mode the plane runs its motor up the moment it is armed — say so before arming.
  const takeoffArmWarning = $derived(sys === 'ardupilot' && (cls === 'plane' || cls === 'quadplane') && $modeTableMatchesVehicle
    && activeKey === 'takeoff' && !$isArmed);

  // ── Set active WP ──────────────────────────────────────────────────────────
  // The FC item index differs from the displayed WP number by the home slot: ArduPilot reserves
  // item 0 for home (so displayed WP n → FC seq n), PX4 has no home slot (displayed WP n → FC seq n-1).
  let setWpNum = $state(1); // 1-based displayed WP number
  const wpCount = $derived($arduMission.length);
  const homeSlot = $derived(sys === 'px4' ? 0 : 1);

  // ── Takeoff ⇄ mission mode ─────────────────────────────────────────────────
  // AUTO (PX4: Mission) replaces the takeoff in flight (the FC may hold a mission Kite has not seen) and on
  // the ground when the FC-synced mission starts with a takeoff; the takeoff moves into the mode grid.
  const autoMode = $derived(safeModes.find((m) => m.key === (sys === 'px4' ? 'mission' : 'auto')));
  const firstCmd = $derived($arduMission[0]?.command);
  const missionOnFc = $derived($arduMissionFcSynced && wpCount > 0);
  const missionTakesOff = $derived(missionOnFc && (firstCmd === MAV_CMD_NAV_TAKEOFF || firstCmd === CMD.NAV_VTOL_TAKEOFF));
  // Mirrored by hand: a store first read inside a $derived that short-circuits is not followed reliably.
  let flying = $state(get(airborne));
  onDestroy(airborne.subscribe((v) => { flying = v; }));
  const autoOnTop = $derived((takeoffSupported || planeTakeoffMode != null) && autoMode != null
    && (flying || missionTakesOff));
  // The takeoff takes the mission mode's cell in the grid; PX4's Mission has none, so it goes last.
  const autoInGrid = $derived(quickModes.some((m) => m.key === autoMode?.key));
  // AUTO on a NAV_TAKEOFF item releases the throttle the way TAKEOFF mode does.
  const autoTakeoffArmWarning = $derived(sys === 'ardupilot' && (cls === 'plane' || cls === 'quadplane') && $modeTableMatchesVehicle
    && activeKey === 'auto' && missionOnFc && firstCmd === MAV_CMD_NAV_TAKEOFF && !$isArmed);
  function setActiveWp() {
    const idx = setWpNum - 1;
    if (idx < 0 || idx >= wpCount) return;
    void missionSetCurrent(idx + homeSlot);
  }

  // Pull the FC's mission into the working mission so it can be commanded here (Start/Restart/Set WP).
  // Frame it on the map on success, mirroring the mission panel's download.
  async function handleMissionDownload() {
    if (await missionDownload()) frameMissionOnMap();
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

  // Force arm/disarm: only offered after the FC *rejected* a normal arm/disarm (e.g. failed pre-arm
  // checks). The force command sets the ArduPilot bypass magic (param2 = 21196). Long-press +
  // danger styling keeps it a deliberate, double-guarded action.
  const armRejected = $derived(feedback != null && feedback.action === 'arm' && !feedback.ok);
  const disarmRejected = $derived(feedback != null && feedback.action === 'disarm' && !feedback.ok);
</script>

<!-- Takeoff + target altitude (multirotor / VTOL / PX4), or TAKEOFF mode on a plain ArduPlane. -->
{#snippet takeoffControl()}
  {#if takeoffSupported}
    <div class="cc-takeoff">
      <div class="cc-takeoff-btn">
        <HoldToConfirm busy={busy('takeoff')} onconfirm={() => takeoff(takeoffAlt)}>
          {$t('control.action.holdTakeoff')}
        </HoldToConfirm>
      </div>
      <NumberStepper bind:value={takeoffAlt} min={1} max={1000} step={5} unit="m" />
    </div>
  {:else if planeTakeoffMode}
    {@const mode = planeTakeoffMode}
    <HoldToConfirm busy={busy('takeoff')} title={$t('control.planeTakeoffHint')} onconfirm={() => planeTakeoff(mode)}>
      {$t('control.action.holdTakeoff')}
    </HoldToConfirm>
  {/if}
{/snippet}

<!-- The takeoff as a cell of the mode grid, to the altitude last set in the takeoff row. -->
{#snippet takeoffCell()}
  <div class="cc-confirm">
    {#if takeoffSupported}
      <HoldToConfirm busy={busy('takeoff')} title={$t('control.takeoffToHint', { values: { alt: takeoffAlt } })} onconfirm={() => takeoff(takeoffAlt)}>
        {$t('control.action.takeoff')}
      </HoldToConfirm>
    {:else if planeTakeoffMode}
      {@const mode = planeTakeoffMode}
      <HoldToConfirm busy={busy('takeoff')} title={$t('control.planeTakeoffHint')} onconfirm={() => planeTakeoff(mode)}>
        {$t('control.action.takeoff')}
      </HoldToConfirm>
    {/if}
  </div>
{/snippet}

<PanelShell variant="compact" title={$t('control.title')}>
  {#snippet body()}
    {#if !$controlAvailable}
      <div class="cc-notice">{$t('control.notConnected')}</div>
    {:else}
      <div class="cc">
        <!-- 1. Arm / disarm -->
        {#if !$isArmed}
          <ArmSlider label={$t('control.action.arm')} busy={busy('arm')} onconfirm={() => arm()} />
          {#if takeoffArmWarning}
            <div class="cc-takeoff-warn">{$t('control.takeoffArmWarning')}</div>
          {:else if autoTakeoffArmWarning}
            <div class="cc-takeoff-warn">{$t('control.autoTakeoffArmWarning')}</div>
          {/if}
          {#if armRejected}
            <HoldToConfirm variant="danger" duration={2000} busy={busy('arm')} title={$t('control.action.forceArmHint')} onconfirm={() => arm(true)}>
              {$t('control.action.forceArm')}
            </HoldToConfirm>
          {/if}
        {:else}
          <HoldToConfirm variant="danger" busy={busy('disarm')} onconfirm={() => disarm()}>
            {$t('control.action.disarm')}
          </HoldToConfirm>
          {#if disarmRejected}
            <HoldToConfirm variant="danger" duration={2000} busy={busy('disarm')} title={$t('control.action.forceDisarmHint')} onconfirm={() => disarm(true)}>
              {$t('control.action.forceDisarm')}
            </HoldToConfirm>
          {/if}
        {/if}

        <!-- 2. Active flight mode (large, centred) -->
        <div class="cc-mode-readout" title={$t('control.mode.current')}>
          <span class="cc-mode-name">{$activeMode?.name ?? '—'}</span>
        </div>

        <!-- 3. Takeoff, or the mission mode in its place (see autoOnTop) -->
        {#if autoOnTop && autoMode}
          {@const mode = autoMode}
          {#if flying}
            <Button variant="mode" full active={activeKey === mode.key} onclick={() => pickMode(mode)}>{mode.name}</Button>
          {:else}
            <!-- From the ground the mode starts the flight: held, like the takeoff. -->
            <div class="cc-hold-mode" class:active={activeKey === mode.key}>
              <HoldToConfirm busy={busy('setMode')} onconfirm={() => pickMode(mode)}>
                {$t('control.action.holdMode', { values: { mode: mode.name.toUpperCase() } })}
              </HoldToConfirm>
            </div>
          {/if}
        {:else}
          {@render takeoffControl()}
        {/if}

        <!-- 4. RTL / Land, Surface on a Sub (+ Abort Landing / go-around for fixed-wing) -->
        <div class="cc-row">
          {#if rtlSupported}
            <HoldToConfirm variant="warning" busy={busy('rtl')} onconfirm={() => rtl()}>
              {$t('control.action.rtl')}
            </HoldToConfirm>
          {:else if landModes.surface}
            {@const mode = landModes.surface}
            <HoldToConfirm variant="warning" busy={busy('setMode')} onconfirm={() => pickMode(mode)}>
              {mode.name}
            </HoldToConfirm>
          {/if}
          {#if landModes.qrtl}
            {@const mode = landModes.qrtl}
            <HoldToConfirm variant="warning" busy={busy('setMode')} title={$t('control.qrtlHint')} onconfirm={() => pickMode(mode)}>
              {mode.name}
            </HoldToConfirm>
          {:else if landModes.autoland}
            {@const mode = landModes.autoland}
            <HoldToConfirm variant="warning" busy={busy('setMode')} title={$t('control.autolandHint')} onconfirm={() => pickMode(mode)}>
              {mode.name}
            </HoldToConfirm>
          {:else if landCommand}
            <HoldToConfirm variant="warning" busy={busy('land')} onconfirm={() => land()}>
              {$t('control.action.land')}
            </HoldToConfirm>
          {/if}
        </div>
        {#if landModes.loiterQland || landModes.qland}
          <div class="cc-row">
            {#if landModes.loiterQland}
              {@const mode = landModes.loiterQland}
              <HoldToConfirm variant="warning" busy={busy('setMode')} title={$t('control.loiterQlandHint')} onconfirm={() => pickMode(mode)}>
                {mode.name}
              </HoldToConfirm>
            {/if}
            {#if landModes.qland}
              {@const mode = landModes.qland}
              <HoldToConfirm variant="warning" busy={busy('setMode')} title={$t('control.qlandHint')} onconfirm={() => pickMode(mode)}>
                {mode.name}
              </HoldToConfirm>
            {/if}
          </div>
        {/if}
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
          <Button variant="mode" active={$guidedActive} full disabled={!guidedMode && !$guidedActive} onclick={() => { const on = !$guidedActive; setGuided(on); if (on) requestMapAccess(); }}
                  title={guidedMode ? $t('control.guidedHint') : $t('control.guidedUnavailable')}>
            {$t('control.action.guided')}
          </Button>
        </div>
        <!-- No Fly-Here on ArduSub (Map.svelte guidedClickArmed) — so no "click the map" hint either. -->
        {#if $guidedActive && !(sys === 'ardupilot' && cls === 'sub')}
          <div class="cc-guided-hint">{$t('control.guidedClickHint')}</div>
        {/if}

        <!-- 5b. Active-flight adjustments -->
        <section class="cc-sec">
          <span class="cc-sec-title">{$t('control.section.adjust')}</span>
          {#if hasAltitude && inGuided}
            <div class="cc-adjust">
              <NumberStepper bind:value={changeAltVal} min={1} max={5000} step={5} unit="m" />
              <Button variant="standard" disabled={busy('changeAlt')} onclick={() => changeAlt(changeAltVal)}>{$t('control.action.changeAlt')}</Button>
            </div>
          {/if}
          {#if canSetHeading}
            <div class="cc-adjust">
              <NumberStepper bind:value={headingVal} min={0} max={359} step={5} unit="°" />
              <Button variant="standard" disabled={busy('setHeading')} onclick={() => setHeading(headingVal)}>{$t('control.action.setHeading')}</Button>
            </div>
          {/if}
          <div class="cc-adjust">
            <NumberStepper bind:value={changeSpeedVal} min={1} max={100} step={1} unit="m/s" />
            <Button variant="standard" disabled={busy('changeSpeed')} onclick={() => changeSpeed(changeSpeedVal, isFixedWing)}>{$t('control.action.changeSpeed')}</Button>
          </div>
          {#if isFixedWing}
            <div class="cc-adjust">
              <NumberStepper bind:value={loiterRadVal} min={10} max={2000} step={10} unit="m" />
              <Button variant="standard" disabled={busy('setLoiterRadius')} onclick={() => setLoiterRadius(loiterRadVal)}>{$t('control.action.setLoiterRadius')}</Button>
            </div>
          {/if}
          <Button variant="standard" full disabled={busy('setHome')} onclick={() => setHomeHere()}>{$t('control.action.setHome')}</Button>
        </section>

        <!-- 6. Other modes (quick switch) -->
        <section class="cc-sec">
          <span class="cc-sec-title">{$t('control.section.modes')}</span>
          <div class="cc-modegrid">
            {#each quickModes as m (m.key)}
              {#if autoOnTop && m.key === autoMode?.key}
                {@render takeoffCell()}
              {:else if m.confirm}
                <!-- Landing modes: hold-to-confirm like the RTL / Land buttons, not a single click. -->
                <div class="cc-confirm" class:active={activeKey === m.key}>
                  <HoldToConfirm variant="warning" onconfirm={() => pickMode(m)}>{m.name}</HoldToConfirm>
                </div>
              {:else}
                <Button variant="mode" active={activeKey === m.key} onclick={() => pickMode(m)}>{m.name}</Button>
              {/if}
            {/each}
            {#if autoOnTop && !autoInGrid}{@render takeoffCell()}{/if}
          </div>
          {#if stickModes.length}
            <!-- Fold-out bar for the stick-flown modes: closed by default, because they need an RC source. -->
            <button type="button" class="cc-fold" aria-expanded={rcModesOpen} onclick={() => (rcModesOpen = !rcModesOpen)}
                    title={stickUnlocked ? '' : $t('control.rcLockHint')}>
              <span class="cc-fold-chevron" class:open={rcModesOpen} aria-hidden="true">▸</span>
              <span class="cc-fold-label">{$t('control.rcModes')}</span>
              {#if !stickUnlocked}<span class="cc-warn">{$t('control.noRcLink')}</span>{/if}
            </button>
            {#if rcModesOpen}
              <div class="cc-modegrid">
                {#each stickModes as m}
                  <Button variant="mode" active={activeKey === m.key} disabled={!stickUnlocked} onclick={() => pickMode(m)}>{m.name}</Button>
                {/each}
              </div>
            {/if}
          {/if}
        </section>

        <!-- 7. Mission control -->
        <section class="cc-sec">
          <span class="cc-sec-title">{$t('control.section.mission')}</span>
          <Button variant="data" icon="download" full disabled={busy('missionDownload')} onclick={handleMissionDownload}>
            {$t('control.action.missionDownload')}
          </Button>
          <div class="cc-row">
            <!-- Start turns amber when the plan on screen isn't what the FC holds — the vehicle flies the FC's copy. -->
            <Button variant={!$arduMissionFcSynced && wpCount > 0 ? 'warning' : 'standard'} full onclick={() => missionStart()}
                    title={!$arduMissionFcSynced && wpCount > 0 ? $t('control.startNotSyncedHint') : ''}>
              {$t('control.action.missionStart')}
            </Button>
            <Button variant="standard" full onclick={() => missionRestart()}>{$t('control.action.missionRestart')}</Button>
          </div>
          {#if !$arduMissionFcSynced && wpCount > 0}
            <div class="cc-setwp-hint">{$t('control.startNotSyncedHint')}</div>
          {/if}

          <!-- Set active WP — only when the mission is in sync with the FC (no edits since
               download/upload), so the chosen WP number maps to the FC's actual item. -->
          <div class="cc-setwp" class:disabled={!$arduMissionFcSynced}>
            <select class="cc-select" bind:value={setWpNum} disabled={!$arduMissionFcSynced || wpCount === 0}>
              {#each Array(wpCount) as _, i}
                <option value={i + 1}>{wpLabel(i)}</option>
              {/each}
            </select>
            <Button variant="standard" disabled={!$arduMissionFcSynced || wpCount === 0 || busy('setWp')} onclick={setActiveWp}>
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
  .cc-takeoff-warn {
    font-size: 11px;
    color: #d4a017;
    text-align: center;
  }

  .cc-adjust {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  /* 26 px + the stepper's 1 px borders = the 28 px Button beside it. */
  .cc-adjust :global(.ns-input) { min-height: 26px; }
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
  /* A hold-to-confirm mode in the quick grid: Button metrics, the active mode outlined. */
  .cc-confirm :global(.htc) {
    min-height: 28px;
    height: 28px;
    padding: 0 10px;
    border-radius: 4px;
    font-size: 12px;
  }
  .cc-confirm.active :global(.htc),
  .cc-hold-mode.active :global(.htc) { box-shadow: inset 0 0 0 1px #37a8db; }
  .cc-modegrid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 5px;
  }
  /* Mode names come from the vehicle: a long one is cut, it must not widen its column. */
  .cc-modegrid :global(.kbtn),
  .cc-modegrid :global(.htc) {
    padding: 0 6px;
    overflow: hidden;
  }
  /* Fold-out bar: a full-width, finger-sized divider that opens the stick-flown modes. */
  .cc-fold {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    min-height: 34px;
    padding: 0 10px;
    border: 1px solid #333;
    border-radius: 6px;
    background: rgba(255, 255, 255, 0.04);
    color: #c8c8c8;
    font: inherit;
    font-size: 12px;
    font-weight: 600;
    text-align: left;
    cursor: pointer;
  }
  .cc-fold:hover { border-color: #37a8db; color: #fff; }
  .cc-fold-chevron { display: inline-block; width: 16px; font-size: 20px; line-height: 1; text-align: center; color: #949494; transition: transform 0.15s; }
  .cc-fold-chevron.open { transform: rotate(90deg); }
  .cc-fold-label { flex: 1; }
  .cc-warn { color: #d4a017; font-weight: 400; }

  .cc-setwp {
    display: flex;
    gap: 6px;
    align-items: stretch;
  }
  .cc-setwp .cc-select { flex: 1; }
  .cc-setwp :global(.kbtn) { height: 30px; }
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
