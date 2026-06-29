<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- RcControlPanel.svelte — GCS RC control over MSP for INAV (docs/archive/MSP_RC_CONTROL.md).
     Left (compact) stage = control surface (live channel view — coming with the mapping phase).
     Right (advanced) stage = Configuration: a collapsible raw-input monitor (default collapsed, just a
     wiring check) + the profile bar (dropdown + Save/New/Delete), and below it the channel mapping
     (next phase). Profiles are shareable files under Documents/KiteGC/HID-Profiles, not linked to any
     device/FC — the user manages the matching FC config themselves. -->
<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { get } from 'svelte/store';
  import { untrack } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { t } from 'svelte-i18n';
  import PanelShell, { type PanelVariant } from '$lib/components/panel/PanelShell.svelte';
  import Button from '$lib/components/panel/Button.svelte';
  import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
  import ChannelConfig from '$lib/components/control/ChannelConfig.svelte';
  import ChannelStates from '$lib/components/control/ChannelStates.svelte';
  import ManualConfig from '$lib/components/control/ManualConfig.svelte';
  import ManualStates from '$lib/components/control/ManualStates.svelte';
  import { settings } from '$lib/stores/settings';
  import {
    hidDevices,
    hidSnapshot,
    startHid,
    stopHid,
    selectHidDevice,
    type HidDevice,
  } from '$lib/stores/hid';
  import {
    rcProfiles,
    currentChannels,
    loadProfiles,
    saveProfile,
    deleteProfile,
    profilesDir,
    profileKind,
    type RcProfile,
    type RcProfileKind,
  } from '$lib/stores/rcProfiles';
  import { connection } from '$lib/stores/connection';
  import { telemetry } from '$lib/stores/telemetry';
  import { loadRcFcConfig, rcFcConfig, setOverrideBitmask } from '$lib/stores/rcFcConfig';
  import { rcEngaged, engage, disengage } from '$lib/stores/rcEngage';
  import { syncFromFc } from '$lib/stores/rcMirror';
  import { rcManual, defaultManualMap } from '$lib/stores/rcManual';
  import '$lib/stores/rcStream'; // self-initialising RC injection pump (engage-driven)
  import { rcLayout } from '$lib/stores/rcLayout';
  import { rcPlatform, rcPlatformLocked, setOfflinePlatform, type RcPlatform } from '$lib/stores/rcPlatform';
  import { evaluateRcSafety } from '$lib/helpers/rcSafety';

  // Configuration lives in the right (advanced) region; open it by default since that's the work area.
  let variant = $state<PanelVariant>('advanced');
  let rawOpen = $state(false); // raw monitor collapsed by default — only a wiring check
  let dirPath = $state('');

  let confirmDialog: ReturnType<typeof ConfirmDialog>;
  // Inline "New profile" name prompt.
  let newOpen = $state(false);
  let newName = $state('');

  // ── Device selection ────────────────────────────────────────────────────
  let selectedId = $state<number | null>(null);
  const selectedDevice = $derived(
    $hidDevices.find((d) => d.id === selectedId) ?? $hidDevices[0] ?? null,
  );

  $effect(() => {
    const list = $hidDevices;
    if (list.length === 0) {
      selectedId = null;
      return;
    }
    if (selectedId != null && list.some((d) => d.id === selectedId)) return;
    const savedUuid = $settings.rcControl.selectedUuid;
    const match = savedUuid ? list.find((d) => d.uuid === savedUuid) : null;
    void pickDevice(match ?? list[0]);
  });

  async function pickDevice(dev: HidDevice): Promise<void> {
    selectedId = dev.id;
    await selectHidDevice(dev.id);
    settings.patch({ rcControl: { ...$settings.rcControl, selectedUuid: dev.uuid } });
  }

  function onDevicePick(e: Event): void {
    const id = Number((e.currentTarget as HTMLSelectElement).value);
    const dev = $hidDevices.find((d) => d.id === id);
    if (dev) void pickDevice(dev);
  }

  // ── Live raw state of the selected device ───────────────────────────────
  const live = $derived($hidSnapshot && $hidSnapshot.id === selectedDevice?.id ? $hidSnapshot : null);
  const axes = $derived(live ? live.axes : []);
  const buttons = $derived(live ? live.buttons : []);
  const hats = $derived(live ? live.hats : []);
  const halfWidth = (v: number) => Math.min(50, Math.abs(v) * 50);

  // ── Profiles ────────────────────────────────────────────────────────────
  const activeProfile = $derived($settings.rcControl.activeProfile);
  // PX4 uses the manual map + editor/monitor instead of the channel grid; it also gets its own profile
  // group. Declared here (not with the other connection-derived flags below) because the profile helpers
  // above reference it.
  const isManualPlatform = $derived($rcPlatform === 'px4');

  function setActive(name: string | null): void {
    settings.patch({ rcControl: { ...$settings.rcControl, activeProfile: name } });
  }

  /** Snapshot the working config into a profile object (with the current device as metadata). The
   *  profile's kind follows the active platform — a PX4 profile holds only the manual map, an
   *  INAV/ArduPilot profile only the channel map (the two models aren't interchangeable). */
  function buildProfile(name: string): RcProfile {
    const existing = $rcProfiles.find((p) => p.name === name);
    const base = {
      name,
      deviceUuid: selectedDevice?.uuid ?? existing?.deviceUuid ?? null,
      deviceName: selectedDevice?.name ?? existing?.deviceName ?? null,
    };
    return isManualPlatform
      ? { ...base, kind: 'manual', manual: structuredClone($rcManual) }
      : { ...base, kind: 'channel', channels: structuredClone($currentChannels) };
  }

  /** Load a profile's working state. A channel profile seeds the channel grid; a manual (PX4) profile
   *  seeds the manual map. Each editor only reads its own model, so loading the irrelevant default for
   *  the other is harmless. */
  function loadWorking(p: RcProfile | null): void {
    currentChannels.set(p?.channels ? structuredClone(p.channels) : {});
    rcManual.set(p?.manual ? structuredClone(p.manual) : defaultManualMap());
  }

  function onProfilePick(e: Event): void {
    const name = (e.currentTarget as HTMLSelectElement).value || null;
    setActive(name);
    loadWorking(name ? $rcProfiles.find((x) => x.name === name) ?? null : null);
  }

  // Only profiles matching the active platform group are selectable — INAV/ArduPilot share the channel
  // model, PX4 is a separate manual model, and they're not cross-compatible.
  const currentKind = $derived<RcProfileKind>(isManualPlatform ? 'manual' : 'channel');
  const visibleProfiles = $derived($rcProfiles.filter((p) => profileKind(p) === currentKind));

  // When the platform group changes (offline pick or FC lock on connect), an active profile from the
  // other group is no longer valid — deselect it so the dropdown + working state stay consistent.
  $effect(() => {
    const name = activeProfile;
    const valid = visibleProfiles.some((p) => p.name === name);
    if (name && !valid) {
      untrack(() => {
        setActive(null);
        loadWorking(null);
      });
    }
  });

  async function onSave(): Promise<void> {
    if (!activeProfile) return;
    const ans = await confirmDialog.show({
      title: $t('rc.saveTitle'),
      message: $t('rc.saveMsg', { values: { name: activeProfile } }),
      buttons: [{ label: $t('rc.save'), value: 'ok', primary: true }],
    });
    if (ans === 'ok') await saveProfile(buildProfile(activeProfile));
  }

  function openNew(): void {
    newName = '';
    newOpen = true;
  }

  async function confirmNew(): Promise<void> {
    const name = newName.trim();
    if (!name) return;
    if ($rcProfiles.some((p) => p.name === name)) return; // guarded in the UI too
    newOpen = false;
    await saveProfile(buildProfile(name));
    setActive(name);
  }

  async function onDelete(): Promise<void> {
    if (!activeProfile) return;
    const ans = await confirmDialog.show({
      title: $t('rc.deleteTitle'),
      message: $t('rc.deleteMsg', { values: { name: activeProfile } }),
      buttons: [{ label: $t('rc.delete'), value: 'ok', danger: true }],
    });
    if (ans !== 'ok') return;
    await deleteProfile(activeProfile);
    setActive(null); // working config below stays loaded
  }

  const nameTaken = $derived($rcProfiles.some((p) => p.name === newName.trim()));

  // ── Safety evaluation (AUX-RC channels we control only — they latch on link loss) ──
  const safety = $derived(
    $rcFcConfig
      ? evaluateRcSafety($rcFcConfig.mode_ranges, Object.keys($currentChannels).map(Number), $rcLayout.rawMax)
      : { locked: false, blocks: [], warnings: [], manualConfigured: true },
  );
  const issueList = (arr: { channel: number; mode: string }[]) =>
    arr.map((i) => `CH${i.channel}: ${i.mode}`).join(', ');
  const hasCritical = $derived(safety.blocks.some((b) => b.reason === 'critical'));
  const hasNoManual = $derived(safety.blocks.some((b) => b.reason === 'gpsNoManual'));

  // ── Receiver-type hint + override-bitmask check (RAW_RC of a normal RX needs the bitmask) ──
  const rxType = $derived($rcFcConfig?.receiver_type ?? null);
  const overrideMask = $derived($rcFcConfig?.msp_override_channels ?? null);
  /** Bitmask the configured RAW_RC channels (CH1..rawMax) require. */
  const requiredMask = $derived(
    Object.keys($currentChannels)
      .map(Number)
      .filter((c) => c <= $rcLayout.rawMax)
      .reduce((m, ch) => m | (1 << (ch - 1)), 0),
  );
  // Only relevant when overriding a normal RX (receiver_type ≠ MSP) and the FC exposes the setting.
  const needsBitmaskFix = $derived(
    rxType != null && rxType !== 2 && requiredMask !== 0 && overrideMask != null &&
      (requiredMask & ~overrideMask) !== 0,
  );
  let fixingMask = $state(false);
  async function fixBitmask(): Promise<void> {
    fixingMask = true;
    try {
      await setOverrideBitmask(requiredMask);
    } catch (e) {
      console.warn('[rc] fixBitmask failed', e);
    } finally {
      fixingMask = false;
    }
  }

  const connectedMsp = $derived($connection.status === 'connected' && $connection.protocolType === 'msp');
  // ArduPilot over MAVLink: same engage/stream pipeline, RC_CHANNELS_OVERRIDE adapter. No MSP FC-config
  // read, no override-mode gate — the manual engage is the sole guard.
  const connectedArdu = $derived($connection.status === 'connected' && $rcPlatform === 'ardupilot');
  // PX4 over MAVLink: MANUAL_CONTROL adapter (4 sticks + buttons), separate manual mapping UI.
  const connectedPx4 = $derived($connection.status === 'connected' && $rcPlatform === 'px4');
  // Any FC we can inject RC to.
  const rcConnected = $derived(connectedMsp || connectedArdu || connectedPx4);

  // Armed state (protocol-agnostic: bit 2 of arming flags, set by both the MSP scheduler and the MAVLink
  // handler). Drives the "still armed" confirmation before a manual release.
  const vehicleArmed = $derived(($telemetry.armingFlags & 0x04) !== 0);

  // ── Platform selection (offline dropdown; locked to the FC when connected) ──
  const PLATFORMS: { id: RcPlatform; label: string }[] = [
    { id: 'inav', label: 'INAV' },
    { id: 'ardupilot', label: 'ArduPilot' },
    { id: 'px4', label: 'PX4' },
  ];
  function onPlatformPick(e: Event): void {
    setOfflinePlatform((e.currentTarget as HTMLSelectElement).value as RcPlatform);
  }

  // Read FC config + sync the FC channel state once whenever we (re)connect via MSP. The sync is a
  // read-only MSP_RC poll that seeds our state from INAV — so AUX starts from the FC's current values.
  $effect(() => {
    if (connectedMsp) {
      void loadRcFcConfig();
      void syncFromFc();
    } else {
      rcFcConfig.set(null);
    }
  });

  // Disengage automatically on involuntary loss of control authority: the FC link goes away, a safety
  // lock trips, or the input device disappears (USB unplugged / driver drop). Losing the gamepad MUST
  // stop the stream — otherwise the rcStream pump keeps re-sending the last channel values (the backend
  // deadman never trips) and the FC would hold frozen sticks instead of failsafing. This is an
  // *involuntary* stop (no explicit release frame): we just stop streaming and let the FC's
  // RC_OVERRIDE_TIME grace window run, which tolerates a brief device re-enumeration.
  $effect(() => {
    if ($rcEngaged.on && (!rcConnected || safety.locked || !selectedDevice)) disengage();
  });

  // RAW_RC only takes effect on the FC while MSP-RC-OVERRIDE is active. Re-seed at the moment it turns
  // on (while engaged) so the CH1–16 takeover continues from the FC's current state (no jump).
  let rawTakingOver = false;
  $effect(() => {
    const on = $rcEngaged.on && $telemetry.mspRcOverride;
    if (on && !rawTakingOver) void syncFromFc();
    rawTakingOver = on;
  });

  // Manual long-press toggle (default OFF, both RX types) — engaging starts AUX streaming + (once
  // override is active) the RAW takeover. Never auto-engages on connect/plug (anti-accidental).
  const LONG_PRESS_MS = 600;
  let lpTimer: ReturnType<typeof setTimeout> | null = null;
  function lpDown(): void {
    lpTimer = setTimeout(() => {
      lpTimer = null;
      void toggleEngage();
    }, LONG_PRESS_MS);
  }
  function lpCancel(): void {
    if (lpTimer) { clearTimeout(lpTimer); lpTimer = null; }
  }
  async function toggleEngage(): Promise<void> {
    if ($rcEngaged.on) {
      // Releasing control while the vehicle is ARMED hands flying back to the FC — with the GCS as the
      // sole RC source that means a failsafe (default config can disarm → crash). Make the user confirm.
      if (vehicleArmed) {
        const ans = await confirmDialog.show({
          title: $t('rc.disengageArmedTitle'),
          message: $t('rc.disengageArmedMsg'),
          buttons: [{ label: $t('rc.disengageArmedConfirm'), value: 'ok', danger: true }],
        });
        if (ans !== 'ok') return;
      }
      // Deliberate release: on ArduPilot (RC_CHANNELS_OVERRIDE) send an explicit release frame so the FC
      // hands control back immediately (revert to a real RX, or RC failsafe if we were the sole source)
      // instead of waiting out RC_OVERRIDE_TIME. PX4 (MANUAL_CONTROL) and INAV have no release frame —
      // they just stop streaming. The backend stops the stream as part of the release.
      if (connectedArdu) {
        try { await invoke('mav_rc_release'); } catch (e) { console.warn('[rc] release failed', e); }
      }
      disengage();
    }
    // INAV serial-RX auto-watches the override box ('serial'); everything else is a manual takeover.
    else if (!safety.locked) await engage(connectedMsp && rxType !== 2 ? 'serial' : 'msp');
  }

  // ── RAW_RC rate (user-selectable; AUX stays 5 Hz) ───────────────────────
  const RC_RATES = [10, 15, 20, 25];
  const rawRateHz = $derived($settings.rcControl.rawRateHz);
  function onRatePick(e: Event): void {
    const hz = Number((e.currentTarget as HTMLSelectElement).value);
    settings.patch({ rcControl: { ...$settings.rcControl, rawRateHz: hz } });
    void invoke('rc_stream_set_rate', { hz }); // applies live if currently engaged
  }

  // ── Link-speed probe verdict (emitted ~2 s after engage if the link is too slow) ──
  type RcLinkWarn = { badPct: number; avgLatencyMs: number; rawRateHz: number };
  let linkWarn = $state<RcLinkWarn | null>(null);
  let linkUnlisten: UnlistenFn | null = null;
  // Clear the warning on disengage so it doesn't linger into the next session.
  $effect(() => {
    if (!$rcEngaged.on) linkWarn = null;
  });

  onMount(() => {
    void startHid();
    void (async () => {
      await loadProfiles();
      dirPath = await profilesDir();
      const name = $settings.rcControl.activeProfile;
      const p = name ? get(rcProfiles).find((x) => x.name === name) : null;
      loadWorking(p ?? null);
    })();
    void (async () => {
      linkUnlisten = await listen<RcLinkWarn>('rc-link-slow', (e) => (linkWarn = e.payload));
    })();
  });
  onDestroy(() => {
    lpCancel();
    disengage();
    linkUnlisten?.();
    void stopHid();
  });
</script>

<PanelShell {variant} title={$t('rc.title')} detailTitle={$t('rc.config')}>
  {#snippet headerActions()}
    <button
      class="rc-expand"
      onclick={() => (variant = variant === 'advanced' ? 'compact' : 'advanced')}
      title={$t(variant === 'advanced' ? 'rc.collapse' : 'rc.expand')}
    >
      {variant === 'advanced' ? '‹' : '›'}
    </button>
  {/snippet}

  {#snippet toolbar()}
    <div class="rc-toolbar">
      <label class="rc-dev-label" for="rc-dev">{$t('rc.device')}</label>
      {#if $hidDevices.length === 0}
        <span class="rc-nodev">{$t('rc.noDevice')}</span>
      {:else}
        <select id="rc-dev" class="rc-dev" value={selectedDevice?.id ?? ''} onchange={onDevicePick}>
          {#each $hidDevices as dev (dev.id)}
            <option value={dev.id}>{dev.name}</option>
          {/each}
        </select>
      {/if}
      <label class="rc-dev-label" for="rc-platform">{$t('rc.platform')}</label>
      <select
        id="rc-platform"
        class="rc-platform"
        value={$rcPlatform}
        onchange={onPlatformPick}
        disabled={$rcPlatformLocked}
        title={$rcPlatformLocked ? $t('rc.platformLocked') : ''}
      >
        {#each PLATFORMS as p (p.id)}
          <option value={p.id}>{p.label}</option>
        {/each}
      </select>
    </div>
  {/snippet}

  <!-- Left (compact) stage: the live control surface — current channel outputs. -->
  {#snippet body()}
    {#if safety.blocks.length}
      <div class="rc-banner rc-banner-block">
        <div class="rc-banner-title">⛔ {$t('rc.lockTitle')}</div>
        <div class="rc-banner-list">{issueList(safety.blocks)}</div>
        {#if hasCritical}<div class="rc-banner-hint">{$t('rc.lockCriticalHint')}</div>{/if}
        {#if hasNoManual}<div class="rc-banner-hint">{$t('rc.lockNoManualHint')}</div>{/if}
      </div>
    {/if}
    {#if safety.warnings.length}
      <div class="rc-banner rc-banner-warn">
        <div class="rc-banner-title">⚠ {$t('rc.warnGpsTitle')}</div>
        <div class="rc-banner-hint">{$t('rc.warnGps', { values: { list: issueList(safety.warnings) } })}</div>
      </div>
    {/if}
    {#if $rcFcConfig && rxType != null}
      <div class="rc-banner rc-banner-info">
        <div class="rc-banner-hint">{rxType === 2 ? $t('rc.rxMspHint') : $t('rc.rxSerialHint')}</div>
      </div>
    {/if}
    {#if needsBitmaskFix}
      <div class="rc-banner rc-banner-warn">
        <div class="rc-banner-title">⚠ {$t('rc.bitmaskTitle')}</div>
        <div class="rc-banner-hint">{$t('rc.bitmaskHint')}</div>
        <button class="rc-fix-btn" disabled={fixingMask} onclick={fixBitmask}>{$t('rc.bitmaskFix')}</button>
      </div>
    {/if}
    {#if linkWarn}
      <div class="rc-banner rc-banner-warn">
        <div class="rc-banner-title">⚠ {$t('rc.linkSlowTitle')}</div>
        <div class="rc-banner-hint">
          {$t('rc.linkSlowHint', { values: { hz: linkWarn.rawRateHz, ms: linkWarn.avgLatencyMs, pct: linkWarn.badPct } })}
        </div>
      </div>
    {/if}
    {#if !selectedDevice}
      <div class="rc-empty">{$t('rc.connectHint')}</div>
    {:else}
      {#if rcConnected}
        <div class="rc-engage" class:on={$rcEngaged.on}>
          <button
            class="rc-engage-btn"
            class:on={$rcEngaged.on}
            disabled={!$rcEngaged.on && safety.locked}
            onpointerdown={lpDown}
            onpointerup={lpCancel}
            onpointerleave={lpCancel}
          >
            {$rcEngaged.on ? $t('rc.disengageBtn') : $t('rc.engageBtn')}
            <span class="rc-engage-hold">{$t('rc.engageHoldHint')}</span>
          </button>
        </div>
        {#if $rcEngaged.on && connectedMsp}
          <!-- MSP-RC-OVERRIDE box state — INAV only (ArduPilot has no override-mode gate). -->
          <div class="rc-engage-status" class:on={$telemetry.mspRcOverride}>
            {$telemetry.mspRcOverride ? $t('rc.overrideActive') : $t('rc.overrideInactive')}
          </div>
        {/if}
        {#if connectedPx4}
          <!-- PX4 ignores MANUAL_CONTROL unless COM_RC_IN_MODE allows a MAVLink/joystick source. -->
          <div class="rc-banner rc-banner-info">
            <div class="rc-banner-hint">{$t('rc.manual.comRcInModeHint')}</div>
          </div>
        {/if}
        <div class="rc-rate">
          <label class="rc-rate-label" for="rc-rate">{$t('rc.rawRate')}</label>
          <select id="rc-rate" class="rc-rate-sel" value={rawRateHz} onchange={onRatePick}>
            {#each RC_RATES as hz (hz)}<option value={hz}>{hz} Hz</option>{/each}
          </select>
        </div>
      {/if}
      {#if !$hidSnapshot && !$rcEngaged.on}<div class="rc-hint">{$t('rc.waitingInput')}</div>{/if}
      {#if isManualPlatform}<ManualStates />{:else}<ChannelStates />{/if}
    {/if}
  {/snippet}

  <!-- Right (advanced) stage: Configuration. -->
  {#snippet detail()}
    <!-- Collapsible raw-input monitor (wiring check). -->
    <div class="rc-collapse">
      <button class="rc-collapse-head" onclick={() => (rawOpen = !rawOpen)} aria-expanded={rawOpen}>
        <span class="rc-chevron" class:open={rawOpen}>▸</span>
        {$t('rc.rawMonitor')}
      </button>
      {#if rawOpen}
        <div class="rc-collapse-body">
          {#if !selectedDevice}
            <div class="rc-hint">{$t('rc.connectHint')}</div>
          {:else}
            <div class="rc-section-title">{$t('rc.axes')} · {axes.length}</div>
            {#if axes.length === 0}
              <div class="rc-hint">{$t('rc.moveHint')}</div>
            {:else}
              <div class="rc-axes">
                {#each axes as ax (ax.code)}
                  <div class="rc-axis">
                    <span class="rc-axis-code">0x{ax.code.toString(16)}</span>
                    <div class="rc-bar">
                      <span class="rc-bar-centre"></span>
                      <span
                        class="rc-bar-fill"
                        style="left:{ax.value >= 0 ? 50 : 50 - halfWidth(ax.value)}%; width:{halfWidth(ax.value)}%"
                      ></span>
                    </div>
                    <span class="rc-axis-val">{ax.value.toFixed(2)}</span>
                  </div>
                {/each}
              </div>
            {/if}

            {#if hats.length > 0}
              <div class="rc-section-title">{$t('rc.hats')} · {hats.length}</div>
              <div class="rc-hats">
                {#each hats as hat (hat.code)}
                  <div class="rc-hat" title={`0x${hat.code.toString(16)}`}>
                    <span class="rc-hat-dot" style="left:{50 + hat.x * 35}%; top:{50 - hat.y * 35}%"></span>
                  </div>
                {/each}
              </div>
            {/if}

            <div class="rc-section-title">{$t('rc.buttons')} · {buttons.length}</div>
            {#if buttons.length === 0}
              <div class="rc-hint">{$t('rc.pressHint')}</div>
            {:else}
              <div class="rc-buttons">
                {#each buttons as btn, i (btn.code)}
                  <span class="rc-btn" class:on={btn.pressed} title={`0x${btn.code.toString(16)}`}>{i + 1}</span>
                {/each}
              </div>
            {/if}
          {/if}
        </div>
      {/if}
    </div>

    <!-- Profile bar. -->
    <div class="rc-profiles">
      <div class="rc-profile-row">
        <label class="rc-prof-label" for="rc-prof">{$t('rc.profile')}</label>
        <select id="rc-prof" class="rc-prof" value={activeProfile ?? ''} onchange={onProfilePick}>
          <option value="">{$t('rc.noProfile')}</option>
          {#each visibleProfiles as p (p.name)}
            <option value={p.name}>{p.name}</option>
          {/each}
        </select>
      </div>
      <div class="rc-profile-actions">
        <Button variant="data" icon="save" disabled={!activeProfile} onclick={onSave}>{$t('rc.save')}</Button>
        <Button variant="standard" icon="add" onclick={openNew}>{$t('rc.new')}</Button>
        <Button variant="danger" icon="delete" disabled={!activeProfile} onclick={onDelete}>{$t('rc.delete')}</Button>
      </div>
      {#if dirPath}
        <div class="rc-dir" title={dirPath}>{$t('rc.profilesPathHint')} <span class="rc-dir-path">{dirPath}</span></div>
      {/if}
    </div>

    <!-- Mapping: channel grid (INAV/ArduPilot) or the PX4 manual map. -->
    <div class="rc-section-title">{isManualPlatform ? $t('rc.manual.title') : $t('rc.channels')}</div>
    {#if isManualPlatform}<ManualConfig />{:else}<ChannelConfig />{/if}
  {/snippet}
</PanelShell>

<ConfirmDialog bind:this={confirmDialog} />

{#if newOpen}
  <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
  <div class="rc-modal-backdrop" onclick={() => (newOpen = false)} onkeydown={(e) => { if (e.key === 'Escape') newOpen = false; }}>
    <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
    <div class="rc-modal" onclick={(e) => e.stopPropagation()}>
      <div class="rc-modal-title">{$t('rc.newTitle')}</div>
      <label class="rc-modal-label" for="rc-new-name">{$t('rc.newNameLabel')}</label>
      <!-- svelte-ignore a11y_autofocus -->
      <input
        id="rc-new-name"
        class="rc-modal-input"
        bind:value={newName}
        autofocus
        onkeydown={(e) => { if (e.key === 'Enter' && newName.trim() && !nameTaken) void confirmNew(); }}
      />
      {#if nameTaken}<div class="rc-modal-warn">{$t('rc.nameExists')}</div>{/if}
      <div class="rc-modal-buttons">
        <button class="rc-modal-btn cancel" onclick={() => (newOpen = false)}>{$t('dialog.cancel')}</button>
        <button class="rc-modal-btn primary" disabled={!newName.trim() || nameTaken} onclick={confirmNew}>{$t('rc.create')}</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .rc-expand {
    width: 24px; height: 24px; line-height: 1; font-size: 16px;
    border: 1px solid #444; border-radius: 4px; background: #2a2a2a; color: #cfcfcf; cursor: pointer;
  }
  .rc-expand:hover { border-color: #37a8db; color: #37a8db; }

  .rc-toolbar { display: flex; align-items: center; gap: 8px; }
  .rc-dev-label { color: #949494; font-size: 11px; }
  .rc-dev {
    flex: 1; min-width: 0; padding: 4px 6px; font-size: 12px;
    background: #2a2a2a; color: #e0e0e0; border: 1px solid #444; border-radius: 4px;
  }
  .rc-nodev { color: #949494; font-size: 12px; font-style: italic; }
  .rc-platform {
    padding: 4px 6px; font-size: 12px;
    background: #2a2a2a; color: #e0e0e0; border: 1px solid #444; border-radius: 4px;
  }
  .rc-platform:disabled { opacity: 0.7; cursor: default; }

  .rc-empty, .rc-hint {
    color: #949494; font-size: 12px; padding: 8px; font-style: italic;
  }

  .rc-banner { border-radius: 5px; padding: 7px 9px; margin-bottom: 8px; font-size: 11px; }
  .rc-banner-title { font-weight: 700; font-size: 11px; margin-bottom: 3px; }
  .rc-banner-list { font-variant-numeric: tabular-nums; margin-bottom: 3px; }
  .rc-banner-hint { color: #d8d8d8; line-height: 1.4; }
  .rc-banner-block {
    background: rgba(212, 0, 0, 0.16); border: 1px solid rgba(212, 0, 0, 0.5); color: #ff9a9a;
  }
  .rc-banner-warn {
    background: rgba(232, 163, 23, 0.14); border: 1px solid rgba(232, 163, 23, 0.45); color: #f0b443;
  }
  .rc-banner-info {
    background: rgba(55, 168, 219, 0.12); border: 1px solid rgba(55, 168, 219, 0.35); color: #9fd4ec;
  }
  .rc-fix-btn {
    margin-top: 6px; padding: 4px 10px; font-size: 11px; font-weight: 600; border-radius: 4px; cursor: pointer;
    background: rgba(232, 163, 23, 0.2); color: #f0b443; border: 1px solid rgba(232, 163, 23, 0.5);
  }
  .rc-fix-btn:hover:not(:disabled) { background: rgba(232, 163, 23, 0.32); }
  .rc-fix-btn:disabled { opacity: 0.5; cursor: not-allowed; }

  /* Engage gate */
  .rc-engage { margin-bottom: 8px; }
  .rc-engage-btn {
    display: flex; align-items: baseline; gap: 8px; width: 100%; justify-content: center;
    padding: 8px 12px; font-size: 12px; font-weight: 700; border-radius: 5px; cursor: pointer;
    background: #2a2a2a; color: #cfcfcf; border: 1px solid #444; user-select: none;
  }
  .rc-engage-btn:hover:not(:disabled) { border-color: #37a8db; color: #37a8db; }
  .rc-engage-btn.on {
    background: rgba(89, 170, 41, 0.2); color: #7ec850; border-color: #59aa29;
  }
  .rc-engage-btn:disabled { opacity: 0.5; cursor: not-allowed; }
  .rc-engage-hold { font-size: 10px; font-weight: 400; color: #949494; }
  .rc-engage-status {
    padding: 7px 10px; font-size: 11px; border-radius: 5px; line-height: 1.4;
    background: rgba(55, 168, 219, 0.1); border: 1px solid rgba(55, 168, 219, 0.3); color: #9fd4ec;
  }
  .rc-engage-status.on {
    background: rgba(89, 170, 41, 0.2); border-color: #59aa29; color: #7ec850; font-weight: 700;
  }
  .rc-rate { display: flex; align-items: center; gap: 8px; margin: 6px 0 2px; }
  .rc-rate-label { color: #949494; font-size: 11px; }
  .rc-rate-sel {
    padding: 3px 6px; font-size: 11px; background: #2a2a2a; color: #e0e0e0;
    border: 1px solid #444; border-radius: 4px;
  }

  /* Collapsible raw monitor */
  .rc-collapse { border: 1px solid #333; border-radius: 6px; margin-bottom: 12px; background: #262626; }
  .rc-collapse-head {
    display: flex; align-items: center; gap: 8px; width: 100%; text-align: left;
    padding: 7px 10px; background: none; border: none; cursor: pointer;
    color: #cfcfcf; font-size: 12px; font-weight: 600;
  }
  .rc-collapse-head:hover { color: #37a8db; }
  .rc-chevron { display: inline-block; transition: transform 0.15s; color: #949494; }
  .rc-chevron.open { transform: rotate(90deg); }
  .rc-collapse-body { padding: 4px 10px 10px; }

  .rc-section-title {
    color: #37a8db; font-size: 11px; text-transform: uppercase; letter-spacing: 0.4px;
    margin: 10px 2px 6px; font-weight: 700;
  }

  .rc-axes { display: flex; flex-direction: column; gap: 5px; }
  .rc-axis { display: flex; align-items: center; gap: 8px; }
  .rc-axis-code { color: #949494; font-size: 10px; width: 54px; font-family: 'Cascadia Code', monospace; }
  .rc-axis-val { color: #cfcfcf; font-size: 11px; width: 38px; text-align: right; font-variant-numeric: tabular-nums; }
  .rc-bar {
    position: relative; flex: 1; height: 12px; background: #232323;
    border: 1px solid #3a3a3a; border-radius: 3px; overflow: hidden;
  }
  .rc-bar-centre { position: absolute; left: 50%; top: 0; bottom: 0; width: 1px; background: #555; }
  .rc-bar-fill { position: absolute; top: 0; bottom: 0; background: #37a8db; }

  .rc-hats { display: flex; flex-wrap: wrap; gap: 8px; }
  .rc-hat { position: relative; width: 30px; height: 30px; border-radius: 50%; background: #232323; border: 1px solid #3a3a3a; }
  .rc-hat-dot {
    position: absolute; width: 7px; height: 7px; border-radius: 50%; background: #37a8db;
    transform: translate(-50%, -50%); transition: left 0.05s linear, top 0.05s linear;
  }

  .rc-buttons { display: flex; flex-wrap: wrap; gap: 4px; }
  .rc-btn {
    min-width: 22px; height: 20px; padding: 0 5px; display: inline-flex; align-items: center;
    justify-content: center; font-size: 10px; border-radius: 3px;
    background: #2a2a2a; color: #777; border: 1px solid #3a3a3a; font-variant-numeric: tabular-nums;
  }
  .rc-btn.on { background: rgba(89, 170, 41, 0.25); color: #7ec850; border-color: #59aa29; }

  /* Profile bar */
  .rc-profiles { display: flex; flex-direction: column; gap: 8px; }
  .rc-profile-row { display: flex; align-items: center; gap: 8px; }
  .rc-prof-label { color: #949494; font-size: 11px; }
  .rc-prof {
    flex: 1; min-width: 0; padding: 5px 6px; font-size: 12px;
    background: #2a2a2a; color: #e0e0e0; border: 1px solid #444; border-radius: 4px;
  }
  .rc-profile-actions { display: flex; gap: 6px; }
  .rc-dir { font-size: 10px; color: #6f6f6f; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .rc-dir-path { color: #888; }

  /* New-profile modal */
  .rc-modal-backdrop {
    position: fixed; inset: 0; z-index: 9999; background: rgba(0, 0, 0, 0.55);
    display: flex; align-items: center; justify-content: center;
  }
  .rc-modal {
    background: #2e2e2e; border: 1px solid rgba(55, 168, 219, 0.45); border-radius: 8px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5); padding: 20px 24px 16px; min-width: 340px;
  }
  .rc-modal-title { font-size: 14px; font-weight: 700; color: #e0e0e0; margin-bottom: 12px; }
  .rc-modal-label { display: block; font-size: 11px; color: #949494; margin-bottom: 5px; }
  .rc-modal-input {
    width: 100%; box-sizing: border-box; padding: 7px 9px; font-size: 13px;
    background: #232323; color: #e0e0e0; border: 1px solid #444; border-radius: 4px;
  }
  .rc-modal-input:focus { outline: none; border-color: #37a8db; }
  .rc-modal-warn { color: #d98a2b; font-size: 11px; margin-top: 6px; }
  .rc-modal-buttons { display: flex; justify-content: flex-end; gap: 8px; margin-top: 16px; }
  .rc-modal-btn {
    padding: 6px 14px; font-size: 12px; font-weight: 600; border-radius: 4px;
    border: 1px solid #555; background: #434343; color: #e0e0e0; cursor: pointer;
  }
  .rc-modal-btn.cancel { color: #999; }
  .rc-modal-btn.primary { background: #1a6b94; border-color: #2590c8; color: #fff; }
  .rc-modal-btn.primary:disabled { opacity: 0.45; cursor: not-allowed; }
  .rc-modal-btn:hover:not(:disabled) { background: #505050; }
  .rc-modal-btn.primary:hover:not(:disabled) { background: #237fae; }
</style>
