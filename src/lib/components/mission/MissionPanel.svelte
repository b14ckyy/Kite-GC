<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- MissionPanel.svelte — thin switcher (panel framework)
     Delegates to the INAV or ArduPilot sub-panel (each owns its own PanelShell + the autopilot
     select in its header) and renders the system-switch confirmation dialog as a shared overlay.
-->
<script lang="ts">
  import { onDestroy } from 'svelte';
  import { get } from 'svelte/store';
  import { t } from 'svelte-i18n';
  import {
    autopilotSystem, pendingSystemSwitch,
    confirmSystemSwitch, cancelSystemSwitch,
    type AutopilotSystem, type SystemSwitchRequest,
  } from '$lib/stores/autopilotContext';
  import { connection } from '$lib/stores/connection';
  import { disconnectFC } from '$lib/controllers/connectionController';
  import InavMissionPanel from './InavMissionPanel.svelte';
  import ArduMissionPanel from './ArduMissionPanel.svelte';

  let currentSystem = $state<AutopilotSystem>(get(autopilotSystem));
  let currentSwitchReq = $state<SystemSwitchRequest | null>(get(pendingSystemSwitch));

  const unsubSystem = autopilotSystem.subscribe(s => { currentSystem = s; });
  const unsubSwitch = pendingSystemSwitch.subscribe(r => { currentSwitchReq = r; });
  onDestroy(() => { unsubSystem(); unsubSwitch(); });

  function systemLabel(system: AutopilotSystem): string {
    if (system === 'ardupilot') return 'ArduPilot';
    if (system === 'px4') return 'PX4';
    return 'INAV';
  }

  async function handleCancelSwitch() {
    const req = currentSwitchReq;
    cancelSystemSwitch();
    if (req?.trigger === 'connection') {
      const conn = get(connection);
      await disconnectFC(conn.baudRate);
    }
  }
</script>

{#if currentSystem === 'inav'}
  <InavMissionPanel />
{:else}
  <ArduMissionPanel />
{/if}

{#if currentSwitchReq}
  <div class="switch-overlay">
    <div class="switch-dialog">
      <div class="switch-title">{$t('mission.systemSwitchTitle')}</div>
      {#if currentSwitchReq.trigger === 'connection'}
        <p class="switch-body">
          {$t('mission.systemSwitchConnectBody', { values: { from: systemLabel(currentSwitchReq.from), to: systemLabel(currentSwitchReq.to) } })}
        </p>
        <div class="switch-actions">
          <button class="btn-switch-confirm" onclick={() => confirmSystemSwitch('clear')}>{$t('mission.systemSwitchConfirm')}</button>
          <button class="btn-switch-cancel" onclick={handleCancelSwitch}>{$t('mission.systemSwitchDisconnect')}</button>
        </div>
      {:else}
        <p class="switch-body">
          {$t('mission.systemSwitchChooseBody', { values: { from: systemLabel(currentSwitchReq.from), to: systemLabel(currentSwitchReq.to) } })}
        </p>
        <div class="switch-actions">
          <button class="btn-switch-convert" onclick={() => confirmSystemSwitch('convert')}>{$t('mission.switchConvert', { values: { to: systemLabel(currentSwitchReq.to) } })}</button>
          <button class="btn-switch-keep" onclick={() => confirmSystemSwitch('keep')}>{$t('mission.switchKeep')}</button>
          <button class="btn-switch-confirm" onclick={() => confirmSystemSwitch('clear')}>{$t('mission.switchClear')}</button>
          <button class="btn-switch-cancel" onclick={handleCancelSwitch}>{$t('mission.systemSwitchCancel')}</button>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .switch-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
  }
  .switch-dialog {
    background: #2e2e2e;
    border: 1px solid #37a8db;
    border-radius: 6px;
    padding: 16px;
    max-width: 280px;
    width: 90%;
  }
  .switch-title { font-size: 14px; font-weight: bold; color: #37a8db; margin-bottom: 8px; }
  .switch-body { font-size: 12px; color: #ccc; margin: 0 0 12px; line-height: 1.5; }
  .switch-actions { display: flex; flex-direction: column; gap: 6px; }
  .btn-switch-confirm { width: 100%; padding: 7px; background: #c0392b; border: none; border-radius: 4px; color: #fff; font-size: 13px; font-weight: 600; cursor: pointer; }
  .btn-switch-confirm:hover { background: #e74c3c; }
  .btn-switch-convert { width: 100%; padding: 7px; background: #1a3a5c; border: 1px solid #37a8db; border-radius: 4px; color: #37a8db; font-size: 13px; font-weight: 600; cursor: pointer; }
  .btn-switch-convert:hover { background: #37a8db; color: #fff; }
  .btn-switch-keep { width: 100%; padding: 7px; background: #2a3a2a; border: 1px solid #4caf50; border-radius: 4px; color: #81c784; font-size: 13px; cursor: pointer; }
  .btn-switch-keep:hover { background: #4caf50; color: #fff; }
  .btn-switch-cancel { width: 100%; padding: 7px; background: #333; border: 1px solid #555; border-radius: 4px; color: #ccc; font-size: 13px; cursor: pointer; }
  .btn-switch-cancel:hover { background: #444; }
</style>
