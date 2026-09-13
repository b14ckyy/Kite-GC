<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // An FC hardware id (24–36 hex chars) shown shortened — the first 10 characters and an ellipsis,
  // the full value in the tooltip — with an icon-only copy button. Used wherever the id appears (UAV
  // Info panel, flight detail, vehicle build sheet): the copy exists so a swapped flight controller's
  // id can be pasted into the existing vehicle entry.
  import { t } from 'svelte-i18n';
  import { copyToClipboard } from '$lib/helpers/clipboard';

  let { uid }: { uid: string } = $props();

  const SHOWN = 10;
  const short = $derived(uid.length > SHOWN ? `${uid.slice(0, SHOWN)}…` : uid);

  let copied = $state(false);
  let copiedTimer: ReturnType<typeof setTimeout> | null = null;

  async function copy() {
    if (!(await copyToClipboard(uid))) return;
    copied = true;
    if (copiedTimer) clearTimeout(copiedTimer);
    copiedTimer = setTimeout(() => { copied = false; }, 1500);
  }
</script>

<span class="uid-chip">
  <span class="uid-text" title={uid}>{short}</span>
  <button class="uid-copy" class:copied onclick={copy} title={copied ? $t('fcUid.copied') : $t('fcUid.copy')} aria-label={$t('fcUid.copy')}>
    {#if copied}
      <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true"><path d="M3 8.5l3 3 7-7" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>
    {:else}
      <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true"><rect x="5.5" y="5.5" width="8" height="8" rx="1.2" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M10.5 5.5V3.7A1.2 1.2 0 0 0 9.3 2.5H3.7A1.2 1.2 0 0 0 2.5 3.7v5.6a1.2 1.2 0 0 0 1.2 1.2h1.8" fill="none" stroke="currentColor" stroke-width="1.4"/></svg>
    {/if}
  </button>
</span>

<style>
  .uid-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
  }

  .uid-text {
    font-weight: 400;
    font-size: 11px;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .uid-copy {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    padding: 0;
    background: none;
    border: 1px solid transparent;
    border-radius: 3px;
    color: #949494;
    cursor: pointer;
    flex-shrink: 0;
  }
  .uid-copy:hover {
    color: #37a8db;
    border-color: rgba(55, 168, 219, 0.4);
  }
  .uid-copy.copied {
    color: #59aa29;
  }
</style>
