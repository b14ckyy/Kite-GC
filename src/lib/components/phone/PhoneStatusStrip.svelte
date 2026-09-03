<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- PhoneStatusStrip — the phone's stand-in for the status bar (Dev-Docs active/PHONE_UI.md D5):
     a cut-off strip in the bottom-left corner with the connection status and the Debug toggle
     (dev builds); the rest of the width stays map. The strip publishes its width as
     `--phone-strip-w` on the root, so the Leaflet attribution can sit right next to it. -->
<script lang="ts">
  import { t } from 'svelte-i18n';
  import { fcLinkAlive, type FcInfo } from '$lib/stores/connection';

  let {
    connStatus,
    fcInfo,
    connectionPort,
    devMode = false,
    debugOpen = $bindable(false),
  }: {
    connStatus: string;
    fcInfo: FcInfo | null;
    connectionPort: string;
    devMode?: boolean;
    debugOpen?: boolean;
  } = $props();

  let widthPx = $state(0);
  $effect(() => {
    document.documentElement.style.setProperty('--phone-strip-w', `${Math.ceil(widthPx)}px`);
    return () => document.documentElement.style.removeProperty('--phone-strip-w');
  });
</script>

<div class="strip" bind:clientWidth={widthPx}>
  <span
    class="dot"
    class:connected={connStatus === 'connected' && $fcLinkAlive}
    class:reconnecting={connStatus === 'connected' && !$fcLinkAlive}
  ></span>
  <span class="label">
    {#if connStatus === 'connected' && !$fcLinkAlive}
      {$t('connection.reconnecting')}
    {:else if connStatus === 'connected' && fcInfo}
      {$t('connection.connectedOn', { values: { variant: fcInfo.fc_variant, version: fcInfo.fc_version, port: connectionPort } })}
    {:else if connStatus === 'connecting'}
      {$t('connection.connecting')}
    {:else}
      {$t('connection.disconnected')}
    {/if}
  </span>
  {#if devMode}
    <button class="debug-btn" class:open={debugOpen} onclick={() => (debugOpen = !debugOpen)} title="MSP Debug Monitor">
      🔧 {$t('statusBar.debug')}
    </button>
  {/if}
</div>

<style>
  .strip {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    height: 22px;
    padding: 0 10px 0 calc(8px + var(--safe-left, 0px));
    background: rgba(46, 46, 46, 0.85);
    border-top: 1px solid #272727;
    border-right: 1px solid #272727;
    border-top-right-radius: 6px;
    backdrop-filter: blur(6px);
    -webkit-backdrop-filter: blur(6px);
    font-size: 11px;
    color: #949494;
    white-space: nowrap;
    pointer-events: auto;
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #d40000;
    flex-shrink: 0;
  }
  .dot.connected {
    background: #59aa29;
  }
  .dot.reconnecting {
    background: #f5a623;
  }

  .label {
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 40vw;
  }

  .debug-btn {
    height: 18px;
    padding: 0 6px;
    background: transparent;
    border: 1px solid #555;
    border-radius: 3px;
    color: #949494;
    font-size: 10px;
    cursor: pointer;
  }
  .debug-btn.open {
    border-color: #37a8db;
    color: #37a8db;
  }
</style>
