<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- Hi-res replay parse popup (Dev-Docs active/HIRES_REPLAY.md): shown while the archived log is
     re-parsed at full resolution. Deliberately modal and self-dismissing — the parse can take a
     while on slower machines, and the user must see that the wait is expected. No buttons. -->
<script lang="ts">
  import { t } from 'svelte-i18n';
  import type { BlackboxImportProgress } from '$lib/stores/flightlog';

  let {
    progress = null,
    estimateBytes = null,
  }: {
    progress?: BlackboxImportProgress | null;
    estimateBytes?: number | null;
  } = $props();

  const pct = $derived(progress?.progress ?? 0);

  const estimateLabel = $derived.by(() => {
    if (estimateBytes == null || estimateBytes <= 0) return null;
    const mb = estimateBytes / (1024 * 1024);
    return mb >= 1024 ? `~${(mb / 1024).toFixed(1)} GB` : `~${Math.max(1, Math.round(mb))} MB`;
  });
</script>

<div class="dialog-backdrop">
  <div class="dialog-box">
    <div class="dialog-title">{$t('player.hiresParsingTitle')}</div>
    <div class="dialog-message">
      {$t('player.hiresParsingHint')}
      {#if estimateLabel}
        {$t('player.hiresParsingSize', { values: { size: estimateLabel } })}
      {/if}
    </div>
    <div class="hires-progress">
      <div class="hires-progress-fill" style="width: {pct}%"></div>
    </div>
    <div class="hires-progress-msg">{progress?.message ?? ''}</div>
  </div>
</div>

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
    margin-bottom: 14px;
  }

  .hires-progress {
    height: 8px;
    background: #434343;
    border-radius: 4px;
    overflow: hidden;
  }

  .hires-progress-fill {
    height: 100%;
    background: #37a8db;
    border-radius: 4px;
    transition: width 0.2s ease;
  }

  .hires-progress-msg {
    margin-top: 6px;
    font-size: 11px;
    color: #949494;
    min-height: 14px;
  }
</style>
