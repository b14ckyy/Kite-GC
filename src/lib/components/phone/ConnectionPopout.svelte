<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- ConnectionPopout — the phone's replacement for the connection bar (Dev-Docs active/PHONE_UI.md
     D1): a chain-link button at the top-right of the map area opens an overlay panel with the
     connection controls in two rows (protocol + transport / the matching selection + Connect) and
     a Relay button that extends the panel downward with the relay entries. Tapping outside closes
     it. All selections are the same bindables the desktop Toolbar uses, owned by +page. -->
<script lang="ts">
  import { t } from 'svelte-i18n';
  import ConnectionControls from '$lib/components/ConnectionControls.svelte';
  import RelayPanel from '$lib/components/RelayPanel.svelte';
  import type { PortInfo, BleDeviceInfo, TransportType, ProtocolType } from '$lib/stores/connection';
  import type { TelemetryData } from '$lib/stores/telemetry';

  let {
    telem,
    ports,
    bleDeviceList = [],
    isBleScanning = false,
    connStatus,
    isConnecting,
    selectedTransport = $bindable(),
    selectedProtocol = $bindable(),
    selectedPort = $bindable(),
    selectedBaud = $bindable(),
    tcpHost = $bindable(),
    tcpPort = $bindable(),
    selectedBleDevice = $bindable(),
    baudRates,
    onConnect,
    onRescanBle,
  }: {
    telem: TelemetryData;
    ports: PortInfo[];
    bleDeviceList?: BleDeviceInfo[];
    isBleScanning?: boolean;
    connStatus: string;
    isConnecting: boolean;
    selectedTransport: TransportType;
    selectedProtocol: ProtocolType;
    selectedPort: string;
    selectedBaud: number;
    tcpHost: string;
    tcpPort: number;
    selectedBleDevice: string;
    baudRates: number[];
    onConnect: () => void;
    onRescanBle?: () => void;
  } = $props();

  let open = $state(false);
  let relayOpen = $state(false);
  let rootEl = $state<HTMLDivElement>();

  // Tap anywhere outside → close (capture phase, so a surface that stops propagation still counts).
  $effect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (rootEl && !rootEl.contains(e.target as Node)) {
        open = false;
        relayOpen = false;
      }
    };
    window.addEventListener('pointerdown', onDown, true);
    return () => window.removeEventListener('pointerdown', onDown, true);
  });
</script>

<div class="cp-root" bind:this={rootEl}>
  <button
    class="cp-btn"
    class:open
    class:connected={connStatus === 'connected'}
    onclick={() => (open = !open)}
    title={$t('connection.panelTitle')}
    aria-label={$t('connection.panelTitle')}
    aria-expanded={open}
  >
    <!-- chain link -->
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M10 13a5 5 0 0 0 7.1 0l2.8-2.8a5 5 0 0 0-7.1-7.1L11 4.9" />
      <path d="M14 11a5 5 0 0 0-7.1 0l-2.8 2.8a5 5 0 0 0 7.1 7.1L13 19.1" />
    </svg>
    <span class="cp-dot" class:connected={connStatus === 'connected'}></span>
  </button>

  {#if open}
    <div class="cp-panel">
      <div class="cp-main">
        <ConnectionControls
          stacked
          {telem}
          {ports}
          {bleDeviceList}
          {isBleScanning}
          {connStatus}
          {isConnecting}
          bind:selectedTransport
          bind:selectedProtocol
          bind:selectedPort
          bind:selectedBaud
          bind:tcpHost
          bind:tcpPort
          bind:selectedBleDevice
          {baudRates}
          {onConnect}
          {onRescanBle}
        />
        <button
          class="relay-toggle"
          class:open={relayOpen}
          onclick={() => (relayOpen = !relayOpen)}
          title={$t('relay.title')}
        >
          ⇅ {$t('relay.short')}
        </button>
      </div>
      {#if relayOpen}
        <!-- RelayPanel positions itself as a toolbar dropdown; inside the popout it flows inline. -->
        <div class="cp-relay">
          <RelayPanel open={true} />
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .cp-root {
    position: relative;
  }

  .cp-btn {
    position: relative;
    width: 42px;
    height: 42px;
    padding: 9px;
    background: rgba(46, 46, 46, 0.92);
    border: 1px solid rgba(55, 168, 219, 0.5);
    border-radius: 6px;
    color: #cfcfcf;
    cursor: pointer;
    backdrop-filter: blur(8px);
    -webkit-backdrop-filter: blur(8px);
    transition: background-color 0.2s, color 0.2s;
  }
  .cp-btn svg {
    width: 100%;
    height: 100%;
    fill: none;
    stroke: currentColor;
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .cp-btn.open {
    background: rgba(55, 168, 219, 0.22);
    border-color: #37a8db;
    color: #37a8db;
  }
  .cp-dot {
    position: absolute;
    right: 5px;
    top: 5px;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #d40000;
    box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.6);
  }
  .cp-dot.connected {
    background: #59aa29;
  }

  .cp-panel {
    position: absolute;
    top: 48px;
    right: 0;
    width: max-content;
    max-width: min(560px, calc(100vw - var(--phone-panel-w, 0px) + var(--phone-shift, 0px) - 24px));
    background: rgba(46, 46, 46, 0.97);
    border: 1px solid #272727;
    border-top: 2px solid #37a8db;
    border-radius: 8px;
    box-shadow: 0 6px 16px rgba(0, 0, 0, 0.45);
    z-index: 300;
  }

  .cp-main {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 10px;
  }

  .relay-toggle {
    height: 28px;
    box-sizing: border-box;
    padding: 0 10px;
    background: #434343;
    border: 1px solid #555;
    border-radius: 4px;
    color: #cfcfcf;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    white-space: nowrap;
    flex-shrink: 0;
  }
  .relay-toggle.open {
    background: rgba(55, 168, 219, 0.22);
    border-color: #37a8db;
    color: #37a8db;
  }

  .cp-relay {
    position: relative;
    border-top: 1px solid #272727;
  }
  /* RelayPanel is written as an absolutely positioned toolbar dropdown; flow it inside the popout. */
  .cp-relay :global(.relay-panel) {
    position: static;
    width: auto;
    max-width: none;
    border: none;
    border-radius: 0 0 8px 8px;
    box-shadow: none;
  }
</style>
