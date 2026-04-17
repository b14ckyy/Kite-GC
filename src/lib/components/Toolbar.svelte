<script lang="ts">
  import { t } from 'svelte-i18n';
  import type { PortInfo } from '$lib/stores/connection';
  import type { TelemetryData } from '$lib/stores/telemetry';

  let {
    appVersion,
    telem,
    ports,
    connStatus,
    isConnecting,
    selectedPort = $bindable(),
    selectedBaud = $bindable(),
    baudRates,
    onRefreshPorts,
    onConnect,
  }: {
    appVersion: string;
    telem: TelemetryData;
    ports: PortInfo[];
    connStatus: string;
    isConnecting: boolean;
    selectedPort: string;
    selectedBaud: number;
    baudRates: number[];
    onRefreshPorts: () => void;
    onConnect: () => void;
  } = $props();

  function getGpsFixLabel(): string {
    if (!telem.lastUpdate || telem.fixType === 0) return $t('gps.noFix');
    const types: Record<number, string> = { 1: $t('gps.fix2d'), 2: $t('gps.fix3d'), 3: $t('gps.fix3dDgps') };
    return types[telem.fixType] || `FIX:${telem.fixType}`;
  }
</script>

<header class="toolbar">
  <div class="toolbar-left">
    <div class="logo">{$t('app.brand')}</div>
    <span class="version">v{appVersion}</span>
  </div>
  <div class="toolbar-center">
    <div class="sensor-bar">
      <div class="sensor" class:active={telem.sensorGyro === 1} class:error={telem.sensorGyro === 3} title={$t('sensors.gyroTooltip')}>{$t('sensors.gyro')}</div>
      <div class="sensor" class:active={telem.sensorAcc === 1} class:error={telem.sensorAcc === 3} title={$t('sensors.accTooltip')}>{$t('sensors.acc')}</div>
      <div class="sensor" class:active={telem.sensorMag === 1} class:error={telem.sensorMag === 3} title={$t('sensors.magTooltip')}>{$t('sensors.mag')}</div>
      <div class="sensor" class:active={telem.sensorBaro === 1} class:error={telem.sensorBaro === 3} title={$t('sensors.baroTooltip')}>{$t('sensors.baro')}</div>
      <div class="sensor" class:active={telem.sensorGps === 1 && telem.fixType >= 2} class:warning={telem.sensorGps === 1 && telem.fixType < 2} class:error={telem.sensorGps === 3} title="GPS: {getGpsFixLabel()} {telem.numSat}S">{$t('sensors.gps')}</div>
    </div>
  </div>
  <div class="toolbar-right">
    <div class="port-controls">
      {#if connStatus !== "connected"}
        <select class="port-select" bind:value={selectedPort}>
          {#if ports.length === 0}
            <option value="">{$t('connection.noPortsFound')}</option>
          {:else}
            {#each ports as port}
              <option value={port.path}>{port.label}</option>
            {/each}
          {/if}
        </select>
        <select class="baud-select" bind:value={selectedBaud}>
          {#each baudRates as baud}
            <option value={baud}>{baud}</option>
          {/each}
        </select>
        <button class="refresh-btn" onclick={onRefreshPorts} title={$t('connection.refreshPorts')}>⟳</button>
      {/if}
    </div>
    <button
      class="connect-btn"
      class:connected={connStatus === "connected"}
      class:connecting={isConnecting}
      onclick={onConnect}
      disabled={isConnecting}
    >
      {#if isConnecting}
        {$t('connection.connecting')}
      {:else if connStatus === "connected"}
        {$t('connection.disconnect')}
      {:else}
        {$t('connection.connect')}
      {/if}
    </button>
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

  .port-select,
  .baud-select {
    padding: 4px 8px;
    background: #434343;
    border: 1px solid #555;
    border-radius: 3px;
    color: #e0e0e0;
    font-size: 12px;
  }

  .port-select {
    min-width: 160px;
  }

  .baud-select {
    min-width: 80px;
  }

  .refresh-btn {
    padding: 4px 8px;
    background: #434343;
    border: 1px solid #555;
    border-radius: 3px;
    color: #e0e0e0;
    font-size: 14px;
    cursor: pointer;
    transition: background-color 0.2s;
  }

  .refresh-btn:hover {
    background: #555;
  }

  .connect-btn {
    padding: 6px 16px;
    background: #37a8db;
    border: 1px solid #339cc1;
    border-radius: 3px;
    color: #fff;
    font-size: 12px;
    font-weight: 600;
    text-shadow: 0 1px rgba(0, 0, 0, 0.25);
    cursor: pointer;
    transition: background-color 0.2s ease;
    min-width: 90px;
  }

  .connect-btn:hover:not(:disabled) {
    background: #45bce5;
  }

  .connect-btn:disabled {
    opacity: 0.7;
    cursor: wait;
  }

  .connect-btn.connected {
    background: #e60000;
    border-color: #fe0000;
  }

  .connect-btn.connected:hover {
    background: #f21212;
  }

  .connect-btn.connecting {
    background: #f5a623;
    border-color: #e09a1e;
  }
</style>
