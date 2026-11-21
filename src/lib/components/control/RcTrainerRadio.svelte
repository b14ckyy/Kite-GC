<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- Copyright (C) 2026 Marc Hoffmann (b14ckyy) -->
<!-- Pick, show and remove the FrSky radio used as the Bluetooth trainer input of RC control. -->
<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { t } from 'svelte-i18n';
  import { settings } from '$lib/stores/settings';
  import type { BleDeviceInfo } from '$lib/stores/connection';
  import { trainerStatus, rememberTrainer, forgetTrainer } from '$lib/stores/trainer';

  let { disabled = false }: { disabled?: boolean } = $props();

  let found = $state<BleDeviceInfo[]>([]);
  let scanning = $state(false);
  let scanned = $state(false);
  let error = $state('');

  const radio = $derived($settings.rcControl.trainerRadio ?? null);
  const STATES = ['searching', 'connecting', 'waiting', 'streaming', 'stalled', 'foreign'];
  const stateKey = $derived(
    $trainerStatus && STATES.includes($trainerStatus.state) ? $trainerStatus.state : 'searching',
  );

  async function scan() {
    scanning = true;
    error = '';
    try {
      found = await invoke<BleDeviceInfo[]>('scan_ble_devices');
      scanned = true;
    } catch (e) {
      error = String(e);
    } finally {
      scanning = false;
    }
  }
</script>

<div class="trainer">
  <div class="head">{$t('rc.trainer.title')}</div>
  {#if radio}
    <div class="line">
      <span class="name">{radio.name}</span>
      <span class="state state-{stateKey}">
        {$t(`rc.trainer.state.${stateKey}`, { values: { count: $trainerStatus?.channels.length ?? 0 } })}
      </span>
      <button class="btn" onclick={forgetTrainer} {disabled}>{$t('rc.trainer.remove')}</button>
    </div>
  {:else}
    <div class="hint">{$t('rc.trainer.hint')}</div>
    <div class="line">
      <button class="btn" onclick={scan} disabled={scanning || disabled}>
        {scanning ? $t('rc.trainer.scanning') : $t('rc.trainer.scan')}
      </button>
      {#if error}<span class="state state-stalled">{error}</span>{/if}
      {#if scanned && !scanning && found.length === 0}<span class="state">{$t('rc.trainer.none')}</span>{/if}
    </div>
    {#each found as dev (dev.id)}
      <button class="dev" onclick={() => rememberTrainer({ id: dev.id, name: dev.name })} {disabled}>
        <span class="name">{dev.name}</span>
        <span class="state">{dev.rssi ?? '—'} dBm</span>
      </button>
    {/each}
  {/if}
</div>

<style>
  .trainer {
    margin-top: 8px;
    padding: 8px;
    border: 1px solid #444;
    border-radius: 4px;
    background: rgba(255, 255, 255, 0.03);
    font-size: 12px;
  }
  .head { color: #949494; font-size: 11px; font-weight: 600; margin-bottom: 6px; }
  .hint { color: #949494; font-size: 11px; margin-bottom: 6px; }
  .line { display: flex; align-items: center; gap: 8px; }
  .name { color: #e0e0e0; font-weight: 600; }
  .state { color: #949494; font-size: 11px; flex: 1; min-width: 0; }
  .state-streaming { color: #59aa29; }
  .state-stalled, .state-foreign { color: #f5a623; }
  .btn {
    padding: 3px 10px; font-size: 12px; cursor: pointer;
    background: #2a2a2a; color: #e0e0e0; border: 1px solid #444; border-radius: 4px;
  }
  .btn:hover:not(:disabled) { border-color: #37a8db; color: #37a8db; }
  .btn:disabled { opacity: 0.6; cursor: default; }
  .dev {
    display: flex; align-items: baseline; gap: 8px; width: 100%; margin-top: 4px; padding: 4px 6px;
    background: none; border: 1px solid transparent; border-radius: 4px; cursor: pointer; text-align: left;
  }
  .dev:hover:not(:disabled) { border-color: #37a8db; }
</style>
