<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- RcControlPanel.svelte — GCS RC control over MSP for INAV (docs/active/RC_CONTROL.md).
     Left (compact) stage = control surface (live channel view — coming with the mapping phase).
     Right (advanced) stage = Configuration: a collapsible raw-input monitor (default collapsed, just a
     wiring check) + the profile bar (dropdown + Save/New/Delete), and below it the channel mapping
     (next phase). Profiles are shareable files under Documents/KiteGC/HID-Profiles, not linked to any
     device/FC — the user manages the matching FC config themselves. -->
<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { get } from 'svelte/store';
  import { t } from 'svelte-i18n';
  import PanelShell, { type PanelVariant } from '$lib/components/panel/PanelShell.svelte';
  import Button from '$lib/components/panel/Button.svelte';
  import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
  import ChannelConfig from '$lib/components/control/ChannelConfig.svelte';
  import ChannelStates from '$lib/components/control/ChannelStates.svelte';
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
    type RcProfile,
  } from '$lib/stores/rcProfiles';

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

  function setActive(name: string | null): void {
    settings.patch({ rcControl: { ...$settings.rcControl, activeProfile: name } });
  }

  /** Snapshot the working config into a profile object (with the current device as metadata). */
  function buildProfile(name: string): RcProfile {
    const existing = $rcProfiles.find((p) => p.name === name);
    return {
      name,
      deviceUuid: selectedDevice?.uuid ?? existing?.deviceUuid ?? null,
      deviceName: selectedDevice?.name ?? existing?.deviceName ?? null,
      channels: structuredClone($currentChannels),
    };
  }

  function onProfilePick(e: Event): void {
    const name = (e.currentTarget as HTMLSelectElement).value || null;
    setActive(name);
    const p = name ? $rcProfiles.find((x) => x.name === name) : null;
    currentChannels.set(p ? structuredClone(p.channels) : {});
  }

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

  onMount(() => {
    void startHid();
    void (async () => {
      await loadProfiles();
      dirPath = await profilesDir();
      const name = $settings.rcControl.activeProfile;
      const p = name ? get(rcProfiles).find((x) => x.name === name) : null;
      currentChannels.set(p ? structuredClone(p.channels) : {});
    })();
  });
  onDestroy(() => void stopHid());
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
    </div>
  {/snippet}

  <!-- Left (compact) stage: the live control surface — current channel outputs. -->
  {#snippet body()}
    {#if !selectedDevice}
      <div class="rc-empty">{$t('rc.connectHint')}</div>
    {:else}
      <ChannelStates />
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
          {#each $rcProfiles as p (p.name)}
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

    <!-- Channel mapping. -->
    <div class="rc-section-title">{$t('rc.channels')}</div>
    <ChannelConfig />
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

  .rc-empty, .rc-hint {
    color: #949494; font-size: 12px; padding: 8px; font-style: italic;
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
