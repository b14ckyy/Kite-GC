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
  import { isMacOS, isMobile, isPhone } from '$lib/platform';
  import ConnectionStatusBox from '$lib/components/ConnectionStatusBox.svelte';
  import ArmingIndicator from '$lib/components/ArmingIndicator.svelte';
  import BatteryIndicator from '$lib/components/BatteryIndicator.svelte';
  import { rcEngaged } from '$lib/stores/rcEngage';
  import { isArmed } from '$lib/helpers/telemetry';
  import type { PortInfo, BleDeviceInfo, TransportType, ProtocolType } from '$lib/stores/connection';
  import type { TelemetryData } from '$lib/stores/telemetry';
  import { settings } from '$lib/stores/settings';

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
    onOpenRc,
    onRescanBle,
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
    /** Open the RC control panel (from the "RC control active" indicator). */
    onOpenRc?: () => void;
    /** Trigger a fresh bounded BLE scan window — called when the device dropdown is opened. */
    onRescanBle?: () => void;
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

  // Phone-only collapsible toolbar: the full bar is tall on a narrow screen, so on phones it collapses
  // to a slim status strip with a show/hide toggle, reclaiming map space. Starts expanded so the
  // connection controls are reachable; the toggle and collapsed strip only render on phones (the CSS
  // is width-gated, so tablets/desktop always show the full bar regardless of this flag).
  let toolbarCollapsed = $state(false);
  const connLabel = $derived(
    connStatus === 'connected'
      ? $t('connection.connected')
      : isConnecting
        ? $t('connection.connecting')
        : $t('connection.disconnected'),
  );
  // Protocol shown next to the status in the collapsed strip so the essentials (state + link type)
  // stay visible without expanding the bar.
  const protoLabel = $derived(
    selectedProtocol === 'mavlink' ? 'MAVLink' : selectedProtocol === 'telemetry' ? 'Telemetry' : 'MSP',
  );
  // Arming state for the collapsed strip (the bottom status bar is hidden on phone, so surface it here
  // when connected). Only meaningful with a live link + telemetry.
  const armedNow = $derived(connStatus === 'connected' && telem.lastUpdate > 0 && isArmed(telem.armingFlags, telem.lastUpdate));
  const showArm = $derived(connStatus === 'connected' && telem.lastUpdate > 0);
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<header class="toolbar" class:collapsed={toolbarCollapsed} data-tauri-drag-region ondblclick={onTitlebarDblClick}>
  {#if isMacOS}
    <!-- macOS: window controls live at top-left (native traffic-light placement). -->
    <WindowControls />
  {/if}
  <div class="toolbar-left" data-tauri-drag-region>
    <img class="logo" src="/branding/kitegc-wordmark-white.svg" alt={$t('app.brand')} draggable="false" data-tauri-drag-region />
    <span class="version" data-tauri-drag-region>v{appVersion}</span>
    <!-- Phone collapsed state: slim connection-status strip shown in place of the full controls. -->
    <span class="tb-collapsed-status">
      <span class="tb-status-dot" class:connected={connStatus === 'connected'}></span>
      {connLabel}
      <span class="tb-collapsed-proto">· {protoLabel}</span>
      {#if showArm}
        <span class="tb-collapsed-arm" class:armed={armedNow}>· {armedNow ? $t('arming.armed') : $t('arming.disarmed')}</span>
      {/if}
    </span>
  </div>
  <div class="toolbar-center" data-tauri-drag-region>
    {#if $rcEngaged.on}
      <button class="rc-active-pill" onclick={onOpenRc} title={$t('rc.activeBadgeHint')}>
        <span class="rc-active-dot"></span>{$t('rc.activeBadge')}
      </button>
    {/if}
    <ArmingIndicator {telem} />
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
    <BatteryIndicator {telem} />
  </div>
  <div class="toolbar-right" data-tauri-drag-region>
    <div class="port-controls">
      {#if connStatus !== "connected"}
        <!-- Protocol selector. The passive "Telemetry" mode is listen-only (auto-detect). -->
        <SegmentedToggle
          options={[{ value: 'msp', label: 'MSP' }, { value: 'mavlink', label: 'MAVLink' }, { value: 'telemetry', label: 'Telemetry' }]}
          value={selectedProtocol}
          onchange={(v) => (selectedProtocol = v as ProtocolType)}
        />

        <!-- Transport type selector. Switching between TCP/UDP flips the port between the two known
             defaults (TCP 5761 ⇄ UDP 14550 = the MAVLink convention) — a custom port (e.g. SITL 5762)
             is left untouched. Protocol-independent (MSP has no standard network port). -->
        <select class="tb-select transport-select" bind:value={selectedTransport}
          onchange={() => {
            if (selectedTransport === 'udp' && tcpPort === 5761) tcpPort = 14550;
            else if (selectedTransport === 'tcp' && tcpPort === 14550) tcpPort = 5761;
          }}>
          <!-- Serial is desktop-only (no raw serial access on iOS). BLE works on iOS via
               CoreBluetooth, so it stays available on mobile; TCP/UDP are cross-platform. -->
          {#if !isMobile}
            <option value="serial">Serial</option>
          {/if}
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
    {#if !isMacOS && !isPhone}
      <WindowControls />
    {/if}
  </div>
  <!-- Phone-only show/hide toggle (revealed by the phone media query in the style block). -->
  <button
    class="tb-collapse-toggle"
    onclick={() => (toolbarCollapsed = !toolbarCollapsed)}
    aria-label={toolbarCollapsed ? $t('connection.expandBar') : $t('connection.collapseBar')}
    title={toolbarCollapsed ? $t('connection.expandBar') : $t('connection.collapseBar')}
  >{toolbarCollapsed ? '⌄' : '⌃'}</button>
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
    gap: 10px;
  }

  .toolbar-right {
    display: flex;
    align-items: center;
    align-self: stretch;
    gap: 8px;
  }

  /* Mobile: keep the toolbar content clear of the landscape notch / Dynamic Island on the left edge
     (--safe-left is 0 in portrait, so this is a no-op there). */
  :global(html.is-mobile) .toolbar {
    padding-left: calc(16px + var(--safe-left, 0px));
  }

  /* Phone collapse toggle + collapsed status strip are hidden on desktop/tablet; the phone media
     query below reveals them. Everything here is gated to narrow (<=600px) mobile screens so tablets
     and desktop keep the normal single-row toolbar untouched. */
  .tb-collapse-toggle,
  .tb-collapsed-status {
    display: none;
  }

  @media (max-width: 600px) {
    /* Expanded phone bar: the controls do not fit one row, so wrap them onto extra lines instead of
       clipping the right side off-screen. The grid toolbar row is auto-sized to match (+page.svelte). */
    :global(html.is-mobile) .toolbar {
      flex-wrap: wrap;
      height: auto;
      row-gap: 6px;
      padding-top: 4px;
      padding-bottom: 8px;
      padding-right: 46px; /* clear the absolutely-positioned collapse toggle */
      align-items: center;
    }
    /* Push the connection controls onto their own line(s) below the logo/indicators row. */
    :global(html.is-mobile) .toolbar-right {
      flex-basis: 100%;
      justify-content: flex-start;
    }
    :global(html.is-mobile) .port-controls {
      flex-wrap: wrap;
    }

    /* Show/hide toggle pinned to the top-right of the bar in both states. */
    :global(html.is-mobile) .tb-collapse-toggle {
      display: flex;
      align-items: center;
      justify-content: center;
      position: absolute;
      top: 6px;
      right: 10px;
      width: 30px;
      height: 30px;
      padding: 0;
      font-size: 16px;
      line-height: 1;
      color: #e0e0e0;
      background: #3a3a3a;
      border: 1px solid #555;
      border-radius: 6px;
      cursor: pointer;
      z-index: 1;
    }

    /* Collapsed: drop the full controls, show only the slim status strip so the map gets the space. */
    :global(html.is-mobile) .toolbar.collapsed {
      flex-wrap: nowrap;
      padding-bottom: 4px;
    }
    :global(html.is-mobile) .toolbar.collapsed .toolbar-center,
    :global(html.is-mobile) .toolbar.collapsed .toolbar-right {
      display: none;
    }
  }

  /* Connection status chip in the toolbar-left: shown on iPhone in BOTH states — collapsed (it is the
     only content) and expanded (it fills the empty space on the logo row). */
  :global(html.is-phone) .tb-collapsed-status {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-left: 10px;
    font-size: 13px;
    color: #cfcfcf;
  }
  .tb-collapsed-proto {
    color: #37a8db;
    font-weight: 600;
  }
  .tb-collapsed-arm {
    font-weight: 700;
    color: #59aa29;
  }
  .tb-collapsed-arm.armed {
    color: #ff4444;
  }
  .tb-status-dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: #d40000;
    flex: none;
  }
  .tb-status-dot.connected {
    background: #3fbf5f;
  }

  .logo {
    display: block;
    height: 36px;
    width: auto;
    user-select: none;
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

  /* RC-control-active indicator — engage persists across the app, so it lives here in the top bar
     (left of the arming light) as an always-visible reminder + a click to jump back and release. */
  .rc-active-pill {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 26px; /* match the arming indicator so they sit on one line */
    box-sizing: border-box;
    padding: 0 12px;
    border: 1px solid #d40000;
    border-radius: 4px;
    background: rgba(60, 12, 12, 0.92);
    color: #ff5a5a;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.4px;
    white-space: nowrap;
    cursor: pointer;
  }
  .rc-active-pill:hover { border-color: #ff5a5a; color: #ff8080; }
  .rc-active-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #ff3030;
    box-shadow: 0 0 6px rgba(255, 48, 48, 0.9);
    animation: rc-active-pulse 1.1s ease-in-out infinite;
  }
  @keyframes rc-active-pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.25; } }
  /* WebKitGTK: the loop is replaced by the shared 1 Hz blink. There, a looping
     animation makes the compositor rebuild the entire window every frame — the cost is per frame
     produced, not per pixel changed — so this dot measured ~46 % of a core. See stores/pulseBlink.ts. */
  :global(html.kite-blink-mode) .rc-active-dot {
    animation: none;
    opacity: 0.25;
  }
  :global(html.kite-blink-mode.kite-blink) .rc-active-dot {
    opacity: 1;
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
