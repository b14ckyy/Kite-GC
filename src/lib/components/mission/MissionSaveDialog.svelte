<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts" module>
  export interface MissionSaveResult { name: string; notes: string; }
  export interface MissionSaveOptions { defaultName?: string; defaultNotes?: string; }
</script>

<script lang="ts">
  import { t } from 'svelte-i18n';

  let open = $state(false);
  let name = $state('');
  let notes = $state('');
  let resolver: ((value: MissionSaveResult | null) => void) | null = null;

  /** Show the name + notes dialog; resolves to the entered values or null on cancel/Escape. */
  export function show(opts: MissionSaveOptions = {}): Promise<MissionSaveResult | null> {
    name = opts.defaultName ?? '';
    notes = opts.defaultNotes ?? '';
    open = true;
    return new Promise((resolve) => { resolver = resolve; });
  }

  function close(value: MissionSaveResult | null) {
    open = false;
    if (resolver) { resolver(value); resolver = null; }
  }
  function confirm() { close({ name: name.trim(), notes: notes.trim() }); }
  function handleKeydown(e: KeyboardEvent) { if (e.key === 'Escape') close(null); }
</script>

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="dialog-backdrop" onclick={() => close(null)} onkeydown={handleKeydown}>
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="dialog-box" onclick={(e) => e.stopPropagation()}>
      <div class="dialog-title">{$t('mission.saveLibTitle')}</div>
      <label class="fld">
        <span class="fld-label">{$t('mission.saveLibName')}</span>
        <!-- svelte-ignore a11y_autofocus -->
        <input class="fld-input" type="text" bind:value={name} autofocus />
      </label>
      <label class="fld">
        <span class="fld-label">{$t('mission.saveLibNotes')}</span>
        <textarea class="fld-input fld-area" bind:value={notes} rows="3"></textarea>
      </label>
      <div class="dialog-buttons">
        <button class="dialog-btn dialog-btn-cancel" onclick={() => close(null)}>{$t('dialog.cancel')}</button>
        <button class="dialog-btn dialog-btn-primary" onclick={confirm}>{$t('mission.saveLibSave')}</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .dialog-backdrop {
    position: fixed;
    inset: 0;
    z-index: 9999;
    background: rgba(0, 0, 0, 0.55);
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .dialog-box {
    background: #2e2e2e;
    border: 1px solid rgba(55, 168, 219, 0.45);
    border-radius: 8px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
    padding: 20px 24px 16px;
    min-width: 340px;
    max-width: 480px;
  }
  .dialog-title {
    font-size: 14px;
    font-weight: 700;
    color: #e0e0e0;
    margin-bottom: 14px;
  }
  .fld { display: block; margin-bottom: 12px; }
  .fld-label {
    display: block;
    font-size: 11px;
    font-weight: 600;
    color: #949494;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    margin-bottom: 4px;
  }
  .fld-input {
    box-sizing: border-box;
    width: 100%;
    padding: 6px 8px;
    font-size: 13px;
    color: #e0e0e0;
    background: #1f1f1f;
    border: 1px solid #444;
    border-radius: 4px;
    font-family: 'Segoe UI', Tahoma, sans-serif;
  }
  .fld-input:focus { outline: none; border-color: #37a8db; }
  .fld-area { resize: vertical; }
  .dialog-buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 4px;
  }
  .dialog-btn {
    padding: 6px 14px;
    font-size: 12px;
    font-weight: 600;
    border-radius: 4px;
    border: 1px solid #555;
    background: #434343;
    color: #e0e0e0;
    cursor: pointer;
    transition: background 0.15s;
  }
  .dialog-btn:hover { background: #505050; }
  .dialog-btn-cancel { color: #999; }
  .dialog-btn-primary {
    background: #1a6b94;
    border-color: #2590c8;
    color: #fff;
  }
  .dialog-btn-primary:hover { background: #237fae; }
</style>
