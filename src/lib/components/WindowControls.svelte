<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { t } from 'svelte-i18n';

  // Track the maximized state so the middle button can switch between
  // "maximize" and "restore" glyphs. onResized fires on maximize/unmaximize/resize.
  let isMaximized = $state(false);

  $effect(() => {
    const win = getCurrentWindow();
    let unlisten: (() => void) | undefined;
    const sync = () => win.isMaximized().then((v) => (isMaximized = v));
    sync();
    win.onResized(sync).then((fn) => (unlisten = fn));
    return () => unlisten?.();
  });

  const minimize = () => getCurrentWindow().minimize();
  const toggleMaximize = () => getCurrentWindow().toggleMaximize();
  const close = () => getCurrentWindow().close();
</script>

<div class="window-controls">
  <button class="win-btn" onclick={minimize} title={$t('window.minimize')} aria-label={$t('window.minimize')}>
    <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
      <line x1="1" y1="5" x2="9" y2="5" stroke="currentColor" stroke-width="1" />
    </svg>
  </button>

  <button
    class="win-btn"
    onclick={toggleMaximize}
    title={isMaximized ? $t('window.restore') : $t('window.maximize')}
    aria-label={isMaximized ? $t('window.restore') : $t('window.maximize')}
  >
    {#if isMaximized}
      <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
        <rect x="1" y="2.5" width="6" height="6" fill="none" stroke="currentColor" stroke-width="1" />
        <path d="M3 2.5 V1 H8.5 V6.5 H7" fill="none" stroke="currentColor" stroke-width="1" />
      </svg>
    {:else}
      <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
        <rect x="1" y="1" width="8" height="8" fill="none" stroke="currentColor" stroke-width="1" />
      </svg>
    {/if}
  </button>

  <button class="win-btn win-close" onclick={close} title={$t('window.close')} aria-label={$t('window.close')}>
    <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
      <line x1="1" y1="1" x2="9" y2="9" stroke="currentColor" stroke-width="1" />
      <line x1="9" y1="1" x2="1" y2="9" stroke="currentColor" stroke-width="1" />
    </svg>
  </button>
</div>

<style>
  .window-controls {
    display: flex;
    align-items: stretch;
    height: 100%;
    margin-left: 4px;
    /* Pull into the toolbar's right padding so the buttons sit flush in the corner. */
    margin-right: -16px;
  }

  .win-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 42px;
    height: 100%;
    padding: 0;
    border: none;
    background: transparent;
    color: #c0c0c0;
    cursor: pointer;
    transition: background 0.12s ease, color 0.12s ease;
  }

  .win-btn:hover {
    background: rgba(255, 255, 255, 0.1);
    color: #ffffff;
  }

  .win-btn:active {
    background: rgba(255, 255, 255, 0.05);
  }

  .win-close:hover {
    background: #d40000;
    color: #ffffff;
  }

  .win-close:active {
    background: #a30000;
  }
</style>
