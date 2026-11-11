<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- LinkManager — the multi-vehicle surface in the toolbar: a "Links" toggle (shown while connected)
     that drops down a panel listing every known vehicle (click = make it the one the widgets and the
     control panel follow), every open link (with a per-link close), and an "Add link" form that opens
     an additional connection without touching the existing ones. Reuses ConnectionControls in its
     stacked layout for the form. See docs/02-design/features/multi-vehicle.design.md §6.5 / §6.6. -->
<script lang="ts">
  import { t } from 'svelte-i18n';
  import Button from '$lib/components/panel/Button.svelte';
  import SegmentedToggle from '$lib/components/panel/SegmentedToggle.svelte';
  import type { FleetMarkerStyle } from '$lib/stores/settings';
  import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
  import ConnectionControls from '$lib/components/ConnectionControls.svelte';
  import { settings } from '$lib/stores/settings';
  import { telemetry } from '$lib/stores/telemetry';
  import { vehicles, activeVehicleId, links, type VehicleSummary } from '$lib/stores/vehicles';
  import { netHostOrLocal } from '$lib/stores/connection';
  import type { PortInfo, BleDeviceInfo, TransportType, ProtocolType } from '$lib/stores/connection';
  import { connectLink, disconnectLink, switchVehicle } from '$lib/controllers/connectionController';
  import { isArmed } from '$lib/helpers/telemetry';

  let {
    ports,
    bleDeviceList = [],
    isBleScanning = false,
    baudRates,
    onRescanBle,
  }: {
    ports: PortInfo[];
    bleDeviceList?: BleDeviceInfo[];
    isBleScanning?: boolean;
    baudRates: number[];
    onRescanBle?: () => void;
  } = $props();

  let open = $state(false);
  let adding = $state(false);
  let busy = $state(false);
  let errorMsg = $state('');
  let root = $state<HTMLElement>();
  let confirmDialog = $state<ConfirmDialog>();

  // Add-link form — seeded from the last-used connection, independent of the toolbar's own selection.
  let selectedTransport = $state<TransportType>('udp');
  let selectedProtocol = $state<ProtocolType>('mavlink');
  let selectedPort = $state('');
  let selectedBaud = $state(115200);
  let tcpHost = $state('');
  let tcpPort = $state(14550);
  let selectedBleDevice = $state('');

  function seedForm() {
    const s = $settings;
    selectedTransport = (s.lastTransport as TransportType) || 'udp';
    selectedProtocol = (s.lastProtocol as ProtocolType) || 'mavlink';
    selectedPort = s.lastPort;
    selectedBaud = s.lastBaud || 115200;
    tcpHost = s.lastHost;
    tcpPort = s.lastTcpPort || 14550;
    selectedBleDevice = s.lastBleDevice;
  }

  const vehicleList = $derived(
    [...$vehicles.values()].sort((a, b) => a.linkId - b.linkId || a.sysid - b.sysid),
  );
  const count = $derived(vehicleList.length);

  function toggle() {
    open = !open;
    if (open) { errorMsg = ''; adding = false; }
  }

  // Close on outside click / Escape.
  $effect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (root && !root.contains(e.target as Node)) open = false;
    };
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') open = false; };
    document.addEventListener('mousedown', onDown, true);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('mousedown', onDown, true);
      document.removeEventListener('keydown', onKey);
    };
  });

  async function pick(v: VehicleSummary) {
    if (busy || v.vehicleId === $activeVehicleId) return;
    // Switching away from an armed vehicle changes what the control panel commands — confirm (FR-16).
    const tnow = $telemetry;
    if (isArmed(tnow.armingFlags, tnow.lastUpdate) && confirmDialog) {
      const choice = await confirmDialog.show({
        title: $t('connection.switchArmedTitle'),
        message: $t('connection.switchArmedBody', { values: { name: v.name } }),
        buttons: [
          { label: $t('connection.switchArmedConfirm'), value: 'switch', danger: true },
        ],
      });
      if (choice !== 'switch') return;
    }
    busy = true;
    errorMsg = '';
    try {
      await switchVehicle(v.vehicleId);
    } catch (e) {
      errorMsg = String(e);
    } finally {
      busy = false;
    }
  }

  async function closeLink(linkId: number) {
    if (busy) return;
    busy = true;
    errorMsg = '';
    try {
      await disconnectLink(linkId, $settings.lastBaud);
    } catch (e) {
      errorMsg = String(e);
    } finally {
      busy = false;
    }
  }

  async function addLink() {
    if (busy) return;
    if (selectedTransport === 'serial' && !selectedPort) { errorMsg = $t('connection.noPortSelected'); return; }
    if (selectedTransport === 'ble' && !selectedBleDevice) { errorMsg = $t('connection.noBleDeviceSelected'); return; }
    const s = $settings;
    busy = true;
    errorMsg = '';
    try {
      await connectLink({
        protocolType: selectedProtocol,
        transportType: selectedTransport,
        port: selectedTransport === 'serial' ? selectedPort : undefined,
        baudRate: selectedTransport === 'serial' ? selectedBaud : undefined,
        host: selectedTransport === 'tcp' || selectedTransport === 'udp' ? netHostOrLocal(tcpHost) : undefined,
        tcpPort: selectedTransport === 'tcp' || selectedTransport === 'udp' ? tcpPort : undefined,
        bleDeviceId: selectedTransport === 'ble' ? selectedBleDevice : undefined,
        attitudeRateHz: s.attitudeRateHz,
        positionRateHz: s.positionRateHz,
        airspeedEnabled: s.airspeedEnabled,
        windEnabled: s.windEnabled,
        mavlinkFullTelemetry: s.mavlinkFullTelemetry,
        flightLogEnabled: s.flightRecordingEnabled,
        flightLogDbEnabled: s.flightLoggingEnabled && s.flightRecordingEnabled,
        flightLogPath: s.flightLogDbPath,
        flightLogRawPath: s.flightLogRawPath,
        flightLogRaw: s.flightRecordingEnabled && (!s.flightLoggingEnabled || s.flightLogRawEnabled),
        flightLogRawAlways: s.flightRecordingEnabled && s.flightLogRawAlways,
      });
      adding = false;
    } catch (e) {
      errorMsg = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="lm-root" bind:this={root}>
  <button class="lm-toggle" class:open onclick={toggle} title={$t('connection.linksTip')}>
    ⛓ {count > 1 ? count : ''} {$t('connection.links')}
  </button>

  {#if open}
    <div class="lm-pop" role="dialog" aria-label={$t('connection.linksTitle')}>
      <div class="lm-section-title">{$t('connection.vehicles')}</div>
      {#if vehicleList.length === 0}
        <div class="lm-empty">{$t('connection.noVehicles')}</div>
      {:else}
        <ul class="lm-list">
          {#each vehicleList as v (v.vehicleId)}
            {@const active = v.vehicleId === $activeVehicleId}
            <li>
              <button class="lm-vehicle" class:active disabled={busy} onclick={() => pick(v)}>
                <span class="lm-dot" class:active></span>
                <span class="lm-name">{v.name}</span>
                <span class="lm-meta">{v.protocol} · L{v.linkId}{v.sysid ? `:S${v.sysid}` : ''}</span>
                {#if active}<span class="lm-active">{$t('connection.activeVehicle')}</span>{/if}
              </button>
            </li>
          {/each}
        </ul>
      {/if}

      <div class="lm-style-row">
        <span class="lm-style-label">{$t('connection.fleetStyle')}</span>
        <SegmentedToggle
          size="sm"
          options={[{ value: 'model', label: $t('connection.fleetStyleModel') }, { value: 'symbol', label: $t('connection.fleetStyleSymbol') }]}
          value={$settings.fleetMarkerStyle}
          onchange={(v) => settings.patch({ fleetMarkerStyle: v as FleetMarkerStyle })}
        />
      </div>

      <div class="lm-section-title">{$t('connection.linksTitle')}</div>
      <ul class="lm-list">
        {#each $links as l (l.linkId)}
          <li class="lm-link">
            <span class="lm-name">L{l.linkId} · {l.protocol}</span>
            <span class="lm-meta">{l.transport}</span>
            <button class="lm-close" disabled={busy} onclick={() => closeLink(l.linkId)} title={$t('connection.closeLink')}>✕</button>
          </li>
        {/each}
      </ul>

      {#if adding}
        <div class="lm-add">
          <ConnectionControls
            telem={$telemetry}
            {ports}
            {bleDeviceList}
            {isBleScanning}
            connStatus="disconnected"
            isConnecting={busy}
            bind:selectedTransport
            bind:selectedProtocol
            bind:selectedPort
            bind:selectedBaud
            bind:tcpHost
            bind:tcpPort
            bind:selectedBleDevice
            {baudRates}
            onConnect={addLink}
            {onRescanBle}
            stacked
          />
          <div class="lm-row-end">
            <Button size="sm" onclick={() => (adding = false)}>{$t('connection.btNameCancel')}</Button>
          </div>
        </div>
      {:else}
        <div class="lm-row-end">
          <Button variant="data" size="sm" disabled={busy} onclick={() => { seedForm(); adding = true; }}>
            + {$t('connection.addLink')}
          </Button>
        </div>
      {/if}

      {#if errorMsg}
        <div class="lm-error">{errorMsg}</div>
      {/if}
    </div>
  {/if}
</div>

<ConfirmDialog bind:this={confirmDialog} />

<style>
  .lm-root {
    position: relative;
    display: inline-flex;
  }

  /* Same look as the toolbar's other toggles (.relay-toggle) so it reads as one control family. */
  .lm-toggle {
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
  .lm-toggle:hover {
    background: rgba(55, 168, 219, 0.18);
    color: #e0e0e0;
  }
  .lm-toggle.open {
    background: rgba(55, 168, 219, 0.22);
    border-color: #37a8db;
    color: #37a8db;
  }

  .lm-pop {
    position: absolute;
    top: calc(100% + 6px);
    right: 0;
    z-index: 1200;
    width: 400px;
    max-width: 92vw;
    padding: 10px 12px 12px;
    background: #2b2b2b;
    border: 1px solid #555;
    border-radius: 6px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.45);
    color: #e0e0e0;
    font-size: 12px;
    text-align: left;
    cursor: default;
  }

  .lm-section-title {
    margin: 6px 0 4px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: #9a9a9a;
  }
  .lm-empty {
    padding: 6px 2px;
    color: #888;
  }

  .lm-list {
    list-style: none;
    margin: 0 0 6px;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .lm-vehicle,
  .lm-link {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    min-height: 28px;
    box-sizing: border-box;
    padding: 3px 8px;
    background: #363636;
    border: 1px solid #4a4a4a;
    border-radius: 4px;
    color: #e0e0e0;
    font-size: 12px;
  }
  .lm-vehicle {
    cursor: pointer;
    text-align: left;
    transition: background-color 0.15s, border-color 0.15s;
  }
  .lm-vehicle:hover:not(:disabled) {
    background: rgba(55, 168, 219, 0.14);
    border-color: #37a8db;
  }
  .lm-vehicle.active {
    border-color: #37a8db;
    background: rgba(55, 168, 219, 0.2);
  }
  .lm-vehicle:disabled {
    opacity: 0.6;
    cursor: default;
  }

  .lm-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #666;
    flex: 0 0 auto;
  }
  .lm-dot.active {
    background: #37a8db;
    box-shadow: 0 0 6px rgba(55, 168, 219, 0.8);
  }
  .lm-name {
    font-weight: 600;
    white-space: nowrap;
  }
  .lm-meta {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: #9a9a9a;
  }
  .lm-active {
    font-size: 10px;
    font-weight: 600;
    color: #37a8db;
    text-transform: uppercase;
  }

  .lm-close {
    width: 22px;
    height: 22px;
    padding: 0;
    background: transparent;
    border: 1px solid #555;
    border-radius: 4px;
    color: #cfcfcf;
    font-size: 11px;
    cursor: pointer;
  }
  .lm-close:hover:not(:disabled) {
    border-color: #e06c6c;
    color: #e06c6c;
  }

  .lm-style-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin: 4px 0 8px;
  }
  .lm-style-label {
    color: #9a9a9a;
    font-size: 11px;
  }

  .lm-add {
    margin-top: 6px;
    padding-top: 8px;
    border-top: 1px solid #444;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .lm-row-end {
    display: flex;
    justify-content: flex-end;
    margin-top: 6px;
  }

  .lm-error {
    margin-top: 8px;
    padding: 6px 8px;
    border-radius: 4px;
    background: rgba(224, 108, 108, 0.15);
    border: 1px solid rgba(224, 108, 108, 0.5);
    color: #f0a0a0;
    word-break: break-word;
  }
</style>
