<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // Shown on the first entry into the 3D view when no Cesium Ion token is configured. Lets the user
  // paste + save a key (enabling world terrain), defer ("remind later"), or dismiss it for good.
  import { t } from 'svelte-i18n';

  let {
    open = $bindable(false),
    onSave,
    onRemindLater,
    onIgnore,
  }: {
    open: boolean;
    /** Save the entered token (non-empty, trimmed). */
    onSave: (token: string) => void;
    /** Close for now — will show again next time the 3D view is entered. */
    onRemindLater: () => void;
    /** Never show again. */
    onIgnore: () => void;
  } = $props();

  let token = $state('');

  function save() {
    const v = token.trim();
    if (!v) return;
    onSave(v);
    token = '';
  }
</script>

{#if open}
  <div class="dialog-backdrop">
    <div class="dialog-box">
      <div class="dialog-title">{$t('cesiumKey.title')}</div>
      <p class="ck-text">{$t('cesiumKey.body')}</p>
      <a class="ck-link" href="https://ion.cesium.com/tokens" target="_blank" rel="noopener noreferrer">
        {$t('cesiumKey.getKey')}
      </a>
      <input
        class="ck-input"
        type="text"
        placeholder={$t('cesiumKey.placeholder')}
        bind:value={token}
        onkeydown={(e) => { if (e.key === 'Enter') save(); }}
      />
      <div class="dialog-buttons">
        <button class="dialog-btn" onclick={onIgnore}>{$t('cesiumKey.ignore')}</button>
        <span class="ck-spacer"></span>
        <button class="dialog-btn" onclick={onRemindLater}>{$t('cesiumKey.later')}</button>
        <button class="dialog-btn dialog-btn-primary" disabled={!token.trim()} onclick={save}>{$t('cesiumKey.save')}</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .dialog-backdrop { position: fixed; inset: 0; z-index: 9999; background: rgba(0, 0, 0, 0.55); display: flex; align-items: center; justify-content: center; }
  .dialog-box { background: #2e2e2e; border: 1px solid rgba(55, 168, 219, 0.45); border-radius: 8px; box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5); padding: 20px 24px 16px; min-width: 360px; max-width: 480px; }
  .dialog-title { font-size: 14px; font-weight: 700; color: #e0e0e0; margin-bottom: 10px; }
  .ck-text { font-size: 13px; color: #c4c4c4; line-height: 1.5; margin: 0 0 8px; }
  .ck-link { display: inline-block; font-size: 12px; color: #37a8db; margin-bottom: 12px; }
  .ck-link:hover { text-decoration: underline; }
  .ck-input { box-sizing: border-box; width: 100%; padding: 6px 8px; font-size: 13px; color: #e0e0e0; background: #1f1f1f; border: 1px solid #444; border-radius: 4px; font-family: 'Segoe UI', Tahoma, sans-serif; margin-bottom: 14px; }
  .ck-input:focus { outline: none; border-color: #37a8db; }

  .dialog-buttons { display: flex; justify-content: flex-end; align-items: center; gap: 8px; }
  .ck-spacer { flex: 1 1 auto; }
  .dialog-btn { padding: 6px 14px; font-size: 12px; font-weight: 600; border-radius: 4px; border: 1px solid #555; background: #434343; color: #e0e0e0; cursor: pointer; transition: background 0.15s; }
  .dialog-btn:hover { background: #505050; }
  .dialog-btn:disabled { opacity: 0.45; cursor: not-allowed; }
  .dialog-btn-primary { background: #1a6b94; border-color: #2590c8; color: #fff; }
  .dialog-btn-primary:not(:disabled):hover { background: #237fae; }
</style>
