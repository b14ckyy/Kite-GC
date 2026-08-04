<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { t } from 'svelte-i18n';
  import { isMacOS } from '$lib/platform';

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

{#if isMacOS}
  <!-- macOS: traffic-light dots on the LEFT, native order close · minimize · zoom (red/yellow/green). -->
  <div class="mac-controls">
    <button class="mac-dot mac-close" onclick={close} title={$t('window.close')} aria-label={$t('window.close')}></button>
    <button class="mac-dot mac-min" onclick={minimize} title={$t('window.minimize')} aria-label={$t('window.minimize')}></button>
    <button
      class="mac-dot mac-zoom"
      onclick={toggleMaximize}
      title={isMaximized ? $t('window.restore') : $t('window.maximize')}
      aria-label={isMaximized ? $t('window.restore') : $t('window.maximize')}
    ></button>
  </div>
{:else}
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
{/if}

<style>
  .window-controls {
    display: flex;
    align-items: stretch;
    height: 100%;
    /* Never shrink: these are the only way to close the window, so they keep their size no matter
       how tight the toolbar around them gets (see the collapse logic in Toolbar.svelte). */
    flex: none;
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

  /* macOS traffic-light cluster, mirroring the native top-left placement + colours. */
  .mac-controls {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 100%;
    padding: 0 14px 0 12px;
    /* Same reasoning as `.window-controls`. Here it also matters that this is the toolbar's FIRST
       child — without it the dots would be squeezed under the logo as the bar fills up. */
    flex: none;
  }

  .mac-dot {
    width: 12px;
    height: 12px;
    padding: 0;
    border: none;
    border-radius: 50%;
    cursor: pointer;
    /* Non-drag hit target inside the draggable titlebar. */
    -webkit-app-region: no-drag;
    transition: filter 0.12s ease;
  }

  .mac-close {
    background: #ff5f57;
  }
  .mac-min {
    background: #febc2e;
  }
  .mac-zoom {
    background: #28c840;
  }

  /* Dim slightly until the window/titlebar is hovered — matches macOS idle traffic lights. */
  .mac-dot:hover {
    filter: brightness(1.1);
  }
  .mac-dot:active {
    filter: brightness(0.85);
  }
</style>
