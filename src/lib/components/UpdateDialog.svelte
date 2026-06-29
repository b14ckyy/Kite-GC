<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- UpdateDialog.svelte — one-shot "new version available" prompt driven by the update-check controller.
     No auto-download (by design): the user opens the release page to read the notes. Three explicit
     choices: open the page, remind me later, or skip this version. Styled like ConfirmDialog. -->
<script lang="ts">
  import { t } from 'svelte-i18n';
  import {
    pendingUpdate, currentVersion, openReleasePage, remindLater, skipVersion,
  } from '$lib/controllers/updateCheck';

  const info = $derived($pendingUpdate);

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') remindLater(); // Escape = remind me later (non-destructive)
  }
</script>

{#if info}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="dialog-backdrop" onclick={remindLater} onkeydown={onKeydown}>
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="dialog-box" onclick={(e) => e.stopPropagation()}>
      <div class="dialog-title">{$t('update.title')}</div>
      <div class="dialog-message">
        {$t('update.body')}
        <div class="upd-versions">
          <div><span class="upd-k">{$t('update.latest')}</span> <span class="upd-v">{info.version}</span>{#if info.prerelease} <span class="upd-pre">{$t('update.prerelease')}</span>{/if}</div>
          <div><span class="upd-k">{$t('update.current')}</span> <span class="upd-v">{currentVersion}</span></div>
        </div>
      </div>
      <div class="dialog-buttons">
        <button class="dialog-btn dialog-btn-cancel" onclick={skipVersion}>{$t('update.skip')}</button>
        <button class="dialog-btn" onclick={remindLater}>{$t('update.later')}</button>
        <button class="dialog-btn dialog-btn-primary" onclick={openReleasePage}>{$t('update.openPage')}</button>
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
    margin-bottom: 10px;
  }

  .dialog-message {
    font-size: 12px;
    color: #bbb;
    line-height: 1.5;
    margin-bottom: 16px;
  }

  .upd-versions {
    margin-top: 12px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .upd-k { color: #949494; }
  .upd-v { color: #e0e0e0; font-weight: 600; }
  .upd-pre { color: #f5a623; font-size: 11px; }

  .dialog-buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    flex-wrap: wrap;
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
