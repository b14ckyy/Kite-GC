<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
<!-- Copyright (C) 2026 Marc Hoffmann (b14ckyy) -->
<!-- Debug Monitor tab: FrSky Bluetooth trainer link, frame timing and raw bytes
     (Dev-Docs active/FRSKY_BT_TRAINER.md). -->
<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { onMount, onDestroy } from 'svelte';
  import { t } from 'svelte-i18n';
  import type { BleDeviceInfo } from '$lib/stores/connection';

  /** Matches the Rust `TrainerStatus`. */
  interface TrainerStatus {
    state: string;
    device: string;
    error: string | null;
    characteristic: string;
    channels: number[];
    frames: number;
    rejected: number;
    notifications: number;
    interval_last_ms: number;
    interval_mean_ms: number;
    interval_p95_ms: number;
    interval_max_ms: number;
    frame_age_ms: number | null;
    stalls: number;
    reconnects: number;
    last_raw: string;
    last_rejected_raw: string;
  }

  let devices = $state<BleDeviceInfo[]>([]);
  let scanning = $state(false);
  let scanErr = $state('');
  let status = $state<TrainerStatus | null>(null);
  let unlisten: UnlistenFn | null = null;

  const running = $derived(status != null && status.state !== 'stopped' && status.state !== 'foreign');
  const rate = $derived(status && status.interval_mean_ms > 0 ? 1000 / status.interval_mean_ms : 0);

  onMount(async () => {
    unlisten = await listen<TrainerStatus>('trainer-status', (e) => (status = e.payload));
  });
  // The link outlives the tab on purpose: a long-run measurement must survive a tab change.
  onDestroy(() => unlisten?.());

  async function scan() {
    scanning = true;
    scanErr = '';
    try {
      devices = await invoke<BleDeviceInfo[]>('scan_ble_devices');
    } catch (e) {
      scanErr = String(e);
    } finally {
      scanning = false;
    }
  }

  async function start(id: string) {
    scanErr = '';
    try {
      await invoke('trainer_start', { deviceId: id });
    } catch (e) {
      scanErr = String(e);
    }
  }

  const stop = () => invoke('trainer_stop');
  const ms = (v: number) => v.toFixed(1);
  /** Bar position of a channel on the 988..2012 µs span. */
  const pct = (us: number) => Math.max(0, Math.min(100, ((us - 988) / 1024) * 100));
</script>

<div class="trainer">
  <div class="row">
    <button class="btn" onclick={scan} disabled={scanning}>
      {scanning ? $t('debug.trainer.scanning') : $t('debug.trainer.scan')}
    </button>
    {#if running}
      <button class="btn" onclick={stop}>{$t('debug.trainer.stop')}</button>
    {/if}
    {#if scanErr}<span class="warn">{scanErr}</span>{/if}
  </div>

  {#if devices.length}
    <div class="devices">
      {#each devices as dev (dev.id)}
        <button class="dev" onclick={() => start(dev.id)}>
          <span class="name">{dev.name}</span>
          <span class="dim">{dev.rssi ?? '—'} dBm · {dev.id}</span>
        </button>
      {/each}
    </div>
  {/if}

  {#if status}
    <div class="row">
      <span class="label">{$t('debug.trainer.state')}</span>
      <span class="value state-{status.state}">{status.state}</span>
      <span class="value">{status.device}</span>
      {#if status.error}<span class="warn">{status.error}</span>{/if}
    </div>
    <div class="row">
      <span class="label">{$t('debug.trainer.interval')}</span>
      <span class="value">
        {ms(status.interval_last_ms)} / {ms(status.interval_mean_ms)} / {ms(status.interval_p95_ms)} / {ms(status.interval_max_ms)} ms
      </span>
      <span class="label">{$t('debug.trainer.rate')}</span>
      <span class="value">{rate.toFixed(1)} Hz</span>
      <span class="label">{$t('debug.trainer.age')}</span>
      <span class="value">{status.frame_age_ms ?? '—'} ms</span>
    </div>
    <div class="row">
      <span class="label">{$t('debug.trainer.frames')}</span>
      <span class="value">{status.frames}</span>
      <span class="label">{$t('debug.trainer.rejected')}</span>
      <span class="value" class:bad={status.rejected > 0}>{status.rejected}</span>
      <span class="label">{$t('debug.trainer.notifications')}</span>
      <span class="value">{status.notifications}</span>
      <span class="label">{$t('debug.trainer.stalls')}</span>
      <span class="value" class:bad={status.stalls > 0}>{status.stalls}</span>
      <span class="label">{$t('debug.trainer.reconnects')}</span>
      <span class="value" class:bad={status.reconnects > 0}>{status.reconnects}</span>
    </div>
    <div class="row">
      <span class="label">{$t('debug.trainer.characteristic')}</span>
      <span class="value mono">{status.characteristic || '—'}</span>
    </div>
    <div class="row">
      <span class="label">{$t('debug.trainer.raw')}</span>
      <span class="value mono">{status.last_raw || '—'}</span>
    </div>
    {#if status.last_rejected_raw}
      <div class="row">
        <span class="label">{$t('debug.trainer.rejectedRaw')}</span>
        <span class="value mono bad">{status.last_rejected_raw}</span>
      </div>
    {/if}
    {#if status.channels.length}
      <div class="channels">
        {#each status.channels as us, i (i)}
          <div class="ch">
            <span class="label">CH{i + 1}</span>
            <div class="bar"><div class="fill" style:width="{pct(us)}%"></div></div>
            <span class="value">{us}</span>
          </div>
        {/each}
      </div>
    {/if}
  {/if}
</div>

<style>
  .trainer {
    font-size: 11px;
    overflow-y: auto;
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 8px;
    padding: 5px 12px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.06);
  }
  .btn {
    background: rgba(55, 168, 219, 0.15);
    border: 1px solid rgba(55, 168, 219, 0.3);
    border-radius: 3px;
    color: #37a8db;
    cursor: pointer;
    font-size: 12px;
    padding: 4px 10px;
  }
  .btn:hover {
    background: rgba(55, 168, 219, 0.3);
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .devices {
    display: flex;
    flex-direction: column;
    border-bottom: 1px solid rgba(255, 255, 255, 0.06);
  }
  .dev {
    display: flex;
    gap: 10px;
    align-items: baseline;
    padding: 4px 12px;
    background: none;
    border: none;
    color: #e0e0e0;
    cursor: pointer;
    font-size: 11px;
    text-align: left;
  }
  .dev:hover {
    background: rgba(55, 168, 219, 0.15);
  }
  .name {
    font-weight: 600;
  }
  .dim,
  .label {
    color: #949494;
  }
  .label {
    font-weight: 600;
  }
  .value {
    color: #e0e0e0;
    font-variant-numeric: tabular-nums;
  }
  .mono {
    font-family: Consolas, monospace;
    word-break: break-all;
  }
  .warn,
  .bad,
  .state-stalled,
  .state-searching {
    color: #f5a623;
  }
  .state-streaming {
    color: #59aa29;
  }
  .channels {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 3px 16px;
    padding: 6px 12px;
  }
  .ch {
    display: grid;
    grid-template-columns: 34px 1fr 34px;
    align-items: center;
    gap: 6px;
  }
  .bar {
    height: 6px;
    background: rgba(255, 255, 255, 0.08);
    border-radius: 3px;
    overflow: hidden;
  }
  .fill {
    height: 100%;
    background: #37a8db;
  }
</style>
