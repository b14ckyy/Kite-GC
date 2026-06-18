<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  import { t } from 'svelte-i18n';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import Button from '$lib/components/panel/Button.svelte';
  import SegmentedToggle from '$lib/components/panel/SegmentedToggle.svelte';
  import WindowControls from '$lib/components/WindowControls.svelte';
  import ConnectionStatusBox from '$lib/components/ConnectionStatusBox.svelte';
  import type { PortInfo, BleDeviceInfo, TransportType, ProtocolType } from '$lib/stores/connection';
  import type { TelemetryData } from '$lib/stores/telemetry';

  let {
    appVersion,
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
    relayOpen = false,
    onToggleRelay,
  }: {
    appVersion: string;
    telem: TelemetryData;
    ports: PortInfo[];
    bleDeviceList: BleDeviceInfo[];
    isBleScanning: boolean;
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
    relayOpen?: boolean;
    onToggleRelay?: () => void;
  } = $props();

  function getGpsFixLabel(): string {
    if (!telem.lastUpdate || telem.fixType === 0) return $t('gps.noFix');
    const types: Record<number, string> = { 1: $t('gps.fix2d'), 2: $t('gps.fix3d'), 3: $t('gps.fix3dDgps') };
    return types[telem.fixType] || `FIX:${telem.fixType}`;
  }

  // Sensor-health bar: one tile per sensor, shown only when present (state !== 0), so the bar adapts
  // to the airframe (rangefinder/pitot appear only when equipped). State 0=NONE / 1=OK / 2|3=fault.
  // GPS additionally goes amber while the fix is below 3D. Fed by SYS_STATUS (MAVLink) or
  // MSP_SENSOR_STATUS (INAV) — both land in the same telemetry fields.
  type SensorTile = { key: string; state: number; label: string; tooltip: string; warn: boolean };
  const sensorTiles = $derived<SensorTile[]>(
    [
      { key: 'gyro', state: telem.sensorGyro, label: $t('sensors.gyro'), tooltip: $t('sensors.gyroTooltip'), warn: false },
      { key: 'acc', state: telem.sensorAcc, label: $t('sensors.acc'), tooltip: $t('sensors.accTooltip'), warn: false },
      { key: 'mag', state: telem.sensorMag, label: $t('sensors.mag'), tooltip: $t('sensors.magTooltip'), warn: false },
      { key: 'baro', state: telem.sensorBaro, label: $t('sensors.baro'), tooltip: $t('sensors.baroTooltip'), warn: false },
      { key: 'gps', state: telem.sensorGps, label: $t('sensors.gps'), tooltip: `GPS: ${getGpsFixLabel()} ${telem.numSat}S`, warn: telem.sensorGps === 1 && telem.fixType < 2 },
      { key: 'rangefinder', state: telem.sensorRangefinder, label: $t('sensors.rangefinder'), tooltip: $t('sensors.rangefinderTooltip'), warn: false },
      { key: 'pitot', state: telem.sensorPitot, label: $t('sensors.pitot'), tooltip: $t('sensors.pitotTooltip'), warn: false },
    ].filter((s) => s.state !== 0)
  );

  // EKF estimator tile (ArduPilot only — INAV never sets ekfStatus, so it stays hidden). Label shows
  // the active core; colour follows the estimator health.
  const ekfLabel = $derived(telem.ekfType === 2 ? 'EKF2' : telem.ekfType === 3 ? 'EKF3' : 'EKF');

  // Double-click the title bar to maximize/restore. Windows/macOS drag regions already do this
  // natively, so only Linux/GTK needs the manual handler (otherwise it would toggle twice).
  const isLinux = typeof navigator !== 'undefined' && navigator.userAgent.includes('Linux');
  function onTitlebarDblClick(e: MouseEvent) {
    if (!isLinux) return;
    // Ignore double-clicks that land on interactive controls (buttons, selects, the window buttons).
    if ((e.target as HTMLElement).closest('button, select, input, a')) return;
    void getCurrentWindow().toggleMaximize();
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<header class="toolbar" data-tauri-drag-region ondblclick={onTitlebarDblClick}>
  <div class="toolbar-left" data-tauri-drag-region>
    <div class="logo" data-tauri-drag-region>{$t('app.brand')}</div>
    <span class="version" data-tauri-drag-region>v{appVersion}</span>
  </div>
  <div class="toolbar-center" data-tauri-drag-region>
    <div class="sensor-bar">
      {#each sensorTiles as s (s.key)}
        <div class="sensor"
          class:active={s.state === 1 && !s.warn}
          class:warning={s.warn}
          class:error={s.state >= 2}
          title={s.tooltip}>{s.label}</div>
      {/each}
      {#if telem.ekfStatus !== 0}
        <div class="sensor"
          class:active={telem.ekfStatus === 1}
          class:warning={telem.ekfStatus === 2}
          class:error={telem.ekfStatus === 3}
          title={$t('sensors.ekfTooltip')}>{ekfLabel}</div>
      {/if}
    </div>
  </div>
  <div class="toolbar-right" data-tauri-drag-region>
    <div class="port-controls">
      {#if connStatus !== "connected"}
        <!-- Protocol selector. The dev-only passive "Telemetry" mode (listen-only, auto-detect) is
             added only in dev builds until the pipeline is real. -->
        <SegmentedToggle
          options={import.meta.env.DEV
            ? [{ value: 'msp', label: 'MSP' }, { value: 'mavlink', label: 'MAVLink' }, { value: 'telemetry', label: 'Telemetry' }]
            : [{ value: 'msp', label: 'MSP' }, { value: 'mavlink', label: 'MAVLink' }]}
          value={selectedProtocol}
          onchange={(v) => (selectedProtocol = v as ProtocolType)}
        />

        <!-- Transport type selector -->
        <select class="tb-select transport-select" bind:value={selectedTransport}>
          <option value="serial">Serial</option>
          <option value="tcp">TCP</option>
          <option value="udp">UDP</option>
          <option value="ble">BLE</option>
        </select>

        {#if selectedTransport === 'serial'}
          <select class="tb-select port-select" bind:value={selectedPort}>
            {#if ports.length === 0}
              <option value="">{$t('connection.noPortsFound')}</option>
            {:else}
              {#each ports as port}
                <option value={port.path}>{port.label}</option>
              {/each}
            {/if}
          </select>
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
          <select class="tb-select ble-select" bind:value={selectedBleDevice}>
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
      {/if}
      {#if isConnecting}
        <Button variant="warning" disabled>{$t('connection.connecting')}</Button>
      {:else if connStatus === "connected"}
        <ConnectionStatusBox {telem} />
        <Button variant="danger" onclick={onConnect}>{$t('connection.disconnect')}</Button>
      {:else}
        <Button variant="data" onclick={onConnect}>{$t('connection.connect')}</Button>
      {/if}
    </div>
    <button
      class="relay-toggle"
      class:open={relayOpen}
      onclick={() => onToggleRelay?.()}
      title={$t('relay.title')}
    >
      ⇅ {$t('relay.short')}
    </button>
    <WindowControls />
  </div>
</header>

<style>
  .toolbar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0 16px;
    height: 50px;
    background: #2e2e2e;
    border-bottom: 3px solid #37a8db;
    box-shadow: 0 2px 4px rgba(0, 0, 0, 0.3);
    position: relative;
    z-index: 200;
  }

  .toolbar-left {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .toolbar-center {
    display: flex;
    align-items: center;
  }

  .toolbar-right {
    display: flex;
    align-items: center;
    align-self: stretch;
    gap: 8px;
  }

  .logo {
    font-size: 20px;
    font-weight: 700;
    color: #37a8db;
    letter-spacing: 0.5px;
  }

  .version {
    font-size: 11px;
    color: #949494;
  }

  .sensor-bar {
    display: flex;
    gap: 1px;
    background: #434343;
    border-radius: 5px;
    border: 1px solid #272727;
    box-shadow: 0 2px 0 rgba(92, 92, 92, 0.5);
    overflow: hidden;
  }

  .sensor {
    padding: 6px 12px;
    font-size: 10px;
    font-weight: 600;
    color: #4f4f4f;
    text-shadow: 0 1px rgba(0, 0, 0, 1.0);
    background: #434343 linear-gradient(to bottom, transparent, rgba(0, 0, 0, 0.45));
    border-right: 1px solid #373737;
    text-align: center;
    min-width: 36px;
  }

  .sensor:last-child {
    border-right: none;
  }

  .sensor.active {
    color: #59aa29;
    text-shadow: 0 0 4px rgba(89, 170, 41, 0.3);
  }

  .sensor.warning {
    color: #f5a623;
    text-shadow: 0 0 4px rgba(245, 166, 35, 0.3);
  }

  .sensor.error {
    color: #d40000;
    text-shadow: 0 0 4px rgba(212, 0, 0, 0.3);
  }

  .port-controls {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  /* Relay dropdown toggle — always visible, right of the connection controls. */
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
    transition: background-color 0.2s, color 0.2s, border-color 0.2s;
  }
  .relay-toggle:hover {
    background: rgba(55, 168, 219, 0.18);
    color: #e0e0e0;
  }
  .relay-toggle.open {
    background: rgba(55, 168, 219, 0.22);
    border-color: #37a8db;
    color: #37a8db;
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
