<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- RcControlPanel.svelte — GCS RC control over MSP for INAV (docs/active/RC_CONTROL.md).
     Phase 1 (local): pick a HID device and watch its RAW axes/buttons live (calibration view). The
     compact stage is the control surface; the advanced stage (right column) will host the channel
     mapping + per-channel config — stubbed for now. No MSP yet: this validates the input pipeline. -->
<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { t } from 'svelte-i18n';
  import PanelShell, { type PanelVariant } from '$lib/components/panel/PanelShell.svelte';
  import { settings } from '$lib/stores/settings';
  import {
    hidDevices,
    hidSnapshot,
    startHid,
    stopHid,
    selectHidDevice,
    type HidDevice,
  } from '$lib/stores/hid';

  let variant = $state<PanelVariant>('compact');

  // Currently selected device id (numeric, from the backend). Resolved from the persisted UUID once
  // the device list arrives; falls back to the first connected device.
  let selectedId = $state<number | null>(null);

  const selectedDevice = $derived(
    $hidDevices.find((d) => d.id === selectedId) ?? $hidDevices[0] ?? null,
  );

  // Re-resolve the selection whenever the device list changes (hotplug / first enumeration).
  $effect(() => {
    const list = $hidDevices;
    if (list.length === 0) {
      selectedId = null;
      return;
    }
    if (selectedId != null && list.some((d) => d.id === selectedId)) return; // still valid
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

  // Live axes/buttons/hats of the selected device (only when it's the one being streamed).
  const live = $derived($hidSnapshot && $hidSnapshot.id === selectedDevice?.id ? $hidSnapshot : null);
  const axes = $derived(live ? live.axes : []);
  const buttons = $derived(live ? live.buttons : []);
  const hats = $derived(live ? live.hats : []);

  /** −1..1 axis value → 0..100 % fill width from centre (for the bipolar bar). */
  const halfWidth = (v: number) => Math.min(50, Math.abs(v) * 50);

  onMount(() => void startHid());
  onDestroy(() => void stopHid());
</script>

<PanelShell
  {variant}
  title={$t('rc.title')}
  detailTitle={$t('rc.config')}
>
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
    </div>
  {/snippet}

  {#snippet body()}
    {#if !selectedDevice}
      <div class="rc-empty">{$t('rc.connectHint')}</div>
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
            <span class="rc-btn" class:on={btn.pressed} title={`0x${btn.code.toString(16)}`}>
              {i + 1}
            </span>
          {/each}
        </div>
      {/if}
    {/if}
  {/snippet}

  {#snippet detail()}
    <div class="rc-stub">{$t('rc.mappingSoon')}</div>
  {/snippet}
</PanelShell>

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

  .rc-empty, .rc-hint {
    color: #949494; font-size: 12px; padding: 8px; font-style: italic;
  }
  .rc-section-title {
    color: #37a8db; font-size: 11px; text-transform: uppercase; letter-spacing: 0.4px;
    margin: 10px 2px 6px; font-weight: 700;
  }
  .rc-section-title:first-child { margin-top: 2px; }

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
  .rc-hat {
    position: relative; width: 30px; height: 30px; border-radius: 50%;
    background: #232323; border: 1px solid #3a3a3a;
  }
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

  .rc-stub { color: #949494; font-size: 12px; font-style: italic; padding: 10px; }
</style>
