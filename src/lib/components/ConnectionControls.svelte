<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- ConnectionControls — the protocol / transport / device selection + Connect button. One markup
     for every place the connection is configured: the desktop Toolbar (inline in its right-hand
     group, or on its floating second row) and the phone's Connection popout (`stacked`: row 1 =
     protocol + transport, row 2 = the matching selection + Connect). Extracted from Toolbar.svelte
     for the phone UI (Dev-Docs active/PHONE_UI.md D1). All selections are bindables owned by +page. -->
<script lang="ts">
  import { t } from 'svelte-i18n';
  import Button from '$lib/components/panel/Button.svelte';
  import SegmentedToggle from '$lib/components/panel/SegmentedToggle.svelte';
  import ConnectionStatusBox from '$lib/components/ConnectionStatusBox.svelte';
  import { hasSerialPorts } from '$lib/platform';
  import { settings } from '$lib/stores/settings';
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
    stacked = false,
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
    /** Trigger a fresh bounded BLE scan window — called when the device dropdown is opened. */
    onRescanBle?: () => void;
    /** Two rows (phone popout) instead of one inline run of controls. */
    stacked?: boolean;
  } = $props();

  // ── Bluetooth SPP port custom names ────────────────────────────────
  // Outgoing BT SPP ports (tagged `bluetooth-spp` by the backend) get no useful OS descriptor, so the
  // user can rename them; the name is stored per COM path in settings and appended to "COMx".
  function portLabel(p: PortInfo): string {
    if (p.port_type !== 'bluetooth-spp') return p.label;
    const name = $settings.btPortNames[p.path];
    return name ? `${p.path} — ${name}` : `${p.path} — ${$t('connection.bluetooth')}`;
  }
  const selectedIsBtSpp = $derived(
    ports.some((p) => p.path === selectedPort && p.port_type === 'bluetooth-spp'),
  );

  let editingBt = $state(false);
  let btNameDraft = $state('');

  function openBtEdit() {
    btNameDraft = $settings.btPortNames[selectedPort] ?? '';
    editingBt = true;
  }
  function saveBtName() {
    const name = btNameDraft.trim();
    settings.update((s) => {
      const map = { ...s.btPortNames };
      if (name) map[selectedPort] = name;
      else delete map[selectedPort];
      return { ...s, btPortNames: map };
    });
    editingBt = false;
  }
  // Close the editor when the selection changes (or leaves BT SPP / serial).
  $effect(() => {
    void selectedPort;
    void selectedTransport;
    editingBt = false;
  });
</script>

<!-- Protocol selector + transport type (row 1 when stacked). -->
{#snippet protocolAndTransport()}
  <!-- The passive "Telemetry" mode is listen-only (auto-detect). -->
  <SegmentedToggle
    options={[{ value: 'msp', label: 'MSP' }, { value: 'mavlink', label: 'MAVLink' }, { value: 'telemetry', label: 'Telemetry' }]}
    value={selectedProtocol}
    onchange={(v) => (selectedProtocol = v as ProtocolType)}
  />

  <!-- Switching between TCP/UDP flips the port between the two known defaults (TCP 5761 ⇄ UDP 14550
       = the MAVLink convention) — a custom port (e.g. SITL 5762) is left untouched.
       Protocol-independent (MSP has no standard network port). -->
  <select class="tb-select transport-select" bind:value={selectedTransport}
    onchange={() => {
      if (selectedTransport === 'udp' && tcpPort === 5761) tcpPort = 14550;
      else if (selectedTransport === 'tcp' && tcpPort === 14550) tcpPort = 5761;
    }}>
    <!-- Serial is a capability, not a form factor: desktop and Android (USB host / OTG) have
         it, iOS does not. BLE and TCP/UDP exist everywhere. -->
    {#if hasSerialPorts}
      <option value="serial">Serial</option>
    {/if}
    <option value="tcp">TCP</option>
    <option value="udp">UDP</option>
    <option value="ble">BLE</option>
  </select>
{/snippet}

<!-- The transport-specific selection (row 2 when stacked). -->
{#snippet selection()}
  {#if selectedTransport === 'serial'}
    <select class="tb-select port-select" bind:value={selectedPort}>
      {#if ports.length === 0}
        <option value="">{$t('connection.noPortsFound')}</option>
      {:else}
        {#each ports as port}
          <option value={port.path}>{portLabel(port)}</option>
        {/each}
      {/if}
    </select>
    {#if selectedIsBtSpp}
      {#if editingBt}
        <input
          class="tb-input bt-name-input"
          type="text"
          bind:value={btNameDraft}
          placeholder={$t('connection.btNamePlaceholder')}
          onkeydown={(e) => {
            if (e.key === 'Enter') saveBtName();
            else if (e.key === 'Escape') (editingBt = false);
          }}
        />
        <button class="bt-edit" onclick={saveBtName} title={$t('connection.btNameSave')}>✓</button>
        <button class="bt-edit" onclick={() => (editingBt = false)} title={$t('connection.btNameCancel')}>✕</button>
      {:else}
        <button class="bt-edit" onclick={openBtEdit} title={$t('connection.renameBtPort')}>✎</button>
      {/if}
    {/if}
    <select class="tb-select baud-select" bind:value={selectedBaud}>
      {#each baudRates as baud}
        <option value={baud}>{baud}</option>
      {/each}
    </select>
  {:else if selectedTransport === 'tcp' || selectedTransport === 'udp'}
    <input
      class="tb-input host-input"
      type="text"
      bind:value={tcpHost}
      placeholder="Host (z.B. 192.168.1.1)"
    />
    <input
      class="tb-input port-input"
      type="number"
      bind:value={tcpPort}
      placeholder="Port"
      min="1"
      max="65535"
    />
  {:else if selectedTransport === 'ble'}
    <select class="tb-select ble-select" bind:value={selectedBleDevice}
      onmousedown={() => onRescanBle?.()} onfocus={() => onRescanBle?.()}>
      {#if bleDeviceList.length === 0}
        <option value="">{isBleScanning ? $t('connection.bleScanning') : $t('connection.noBleDevices')}</option>
      {:else}
        {#each bleDeviceList as device}
          <option value={device.id}>
            {device.name} ({device.profile}{device.rssi != null ? `, ${device.rssi} dBm` : ''})
          </option>
        {/each}
      {/if}
    </select>
  {/if}
{/snippet}

{#snippet connectButton()}
  {#if isConnecting}
    <Button variant="warning" disabled>{$t('connection.connecting')}</Button>
  {:else if connStatus === "connected"}
    <ConnectionStatusBox {telem} />
    <Button variant="danger" onclick={onConnect}>{$t('connection.disconnect')}</Button>
  {:else}
    <Button variant="data" onclick={onConnect}>{$t('connection.connect')}</Button>
  {/if}
{/snippet}

{#if stacked}
  <div class="cc-stacked">
    {#if connStatus !== "connected"}
      <div class="cc-row">{@render protocolAndTransport()}</div>
    {/if}
    <div class="cc-row">
      {#if connStatus !== "connected"}{@render selection()}{/if}
      {@render connectButton()}
    </div>
  </div>
{:else}
  <!-- Inline: `display: contents`, so the controls lay out as direct flex children of the caller's
       row (the Toolbar measures and wraps them itself). -->
  <div class="cc-inline">
    {#if connStatus !== "connected"}
      {@render protocolAndTransport()}
      {@render selection()}
    {/if}
    {@render connectButton()}
  </div>
{/if}

<style>
  .cc-inline {
    display: contents;
  }

  .cc-stacked {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .cc-row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }

  /* Unified toolbar form controls — match the control-library height (28px), so selects, inputs,
     the SegmentedToggle and the <Button> all align on one line (see docs/active/PANEL_FRAMEWORK.md). */
  .tb-select,
  .tb-input {
    height: 28px;
    box-sizing: border-box;
    padding: 0 8px;
    background: #434343;
    border: 1px solid #555;
    border-radius: 4px;
    color: #e0e0e0;
    font-size: 12px;
  }

  /* Fixed widths + ellipsis so long device names never stretch the bar. */
  .transport-select { width: 90px; }
  .baud-select { width: 92px; }
  .port-select {
    width: 180px;
    text-overflow: ellipsis;
  }
  .ble-select {
    width: 220px;
    text-overflow: ellipsis;
  }
  .host-input { width: 150px; }
  .port-input { width: 72px; }
  .bt-name-input { width: 130px; }

  /* Small square icon button for the Bluetooth-port rename (✎ / ✓ / ✕). */
  .bt-edit {
    height: 28px;
    width: 28px;
    box-sizing: border-box;
    padding: 0;
    background: #434343;
    border: 1px solid #555;
    border-radius: 4px;
    color: #cfcfcf;
    font-size: 13px;
    cursor: pointer;
    transition: background-color 0.2s, color 0.2s, border-color 0.2s;
  }
  .bt-edit:hover {
    background: rgba(55, 168, 219, 0.18);
    color: #37a8db;
    border-color: #37a8db;
  }

  /* Drop the native number spinner — the up/down arrows are clutter in the toolbar. */
  .port-input {
    appearance: textfield;
    -moz-appearance: textfield;
  }
  .port-input::-webkit-inner-spin-button,
  .port-input::-webkit-outer-spin-button {
    -webkit-appearance: none;
    margin: 0;
  }

  .tb-input::placeholder {
    color: #777;
  }
</style>
