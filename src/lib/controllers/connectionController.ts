// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { get } from 'svelte/store';
import type { FcInfo, PortInfo, BleDeviceInfo, TransportType, ProtocolType } from '$lib/stores/connection';
import { connection, availablePorts, bleDevices } from '$lib/stores/connection';
import { startTelemetryListeners, stopTelemetryListeners, resetTelemetry } from '$lib/stores/telemetry';

/**
 * Refresh the list of serial ports via Tauri and return the port that should be selected.
 *
 * Diffs against the previously known list so live polling behaves like a desktop configurator:
 *  - first population: keep the restored/last port if present, else the first one;
 *  - a newly appeared port (hotplug) is auto-selected;
 *  - if the selected port vanished (unplug) and nothing new appeared, it is deselected.
 */
export async function refreshSerialPorts(currentPort: string): Promise<string> {
  const prev = get(availablePorts);
  const result = await invoke<PortInfo[]>("list_serial_ports");
  availablePorts.set(result);

  const has = (path: string) => result.some((p) => p.path === path);

  // First population — don't treat everything as "new" (would hijack the restored last port).
  if (prev.length === 0) {
    if (currentPort && has(currentPort)) return currentPort;
    return result.length > 0 ? result[0].path : currentPort;
  }

  // Hotplug: select the freshly connected port.
  const prevPaths = new Set(prev.map((p) => p.path));
  const appeared = result.filter((p) => !prevPaths.has(p.path));
  if (appeared.length > 0) return appeared[appeared.length - 1].path;

  // Nothing new: keep the current port if it's still there, else deselect (it was unplugged).
  if (currentPort && has(currentPort)) return currentPort;
  return '';
}

/** Start a continuous BLE scan; discovered/updated devices arrive via the `ble-device` event
 *  (see startBleDeviceListener). The backend restarts any previous session. */
export async function startBleScan(): Promise<void> {
  await invoke("ble_scan_start");
}

/** Stop the continuous BLE scan session. */
export async function stopBleScan(): Promise<void> {
  await invoke("ble_scan_stop");
}

/** Clear the discovered-device list (e.g. before a fresh scan). */
export function clearBleDevices(): void {
  bleDevices.set([]);
}

let bleUnlisten: UnlistenFn | null = null;
/** Subscribe to live BLE discovery events and upsert them into the bleDevices store. Idempotent. */
export async function startBleDeviceListener(): Promise<void> {
  if (bleUnlisten) return;
  bleUnlisten = await listen<BleDeviceInfo>("ble-device", (e) => {
    const dev = e.payload;
    bleDevices.update((list) => {
      const i = list.findIndex((d) => d.id === dev.id);
      if (i >= 0) {
        const next = [...list];
        next[i] = dev;
        return next;
      }
      return [...list, dev];
    });
  });
}

/** Tear down the BLE discovery listener. */
export function stopBleDeviceListener(): void {
  bleUnlisten?.();
  bleUnlisten = null;
}

export interface ConnectParams {
  protocolType: ProtocolType;
  transportType: TransportType;
  // Serial
  port?: string;
  baudRate?: number;
  // TCP/UDP
  host?: string;
  tcpPort?: number;
  // BLE
  bleDeviceId?: string;
  // Telemetry config
  attitudeRateHz: number;
  positionRateHz: number;
  airspeedEnabled: boolean;
  flightLogEnabled: boolean;
  flightLogDbEnabled: boolean;
  flightLogPath: string;
  flightLogRaw: boolean;
  flightLogRawAlways: boolean;
}

/**
 * Connect to the flight controller via Tauri, update stores, start listeners.
 */
export async function connectFC(params: ConnectParams): Promise<FcInfo> {
  const info = await invoke<FcInfo>("connect", {
    protocol: params.protocolType,
    transportType: params.transportType,
    port: params.port ?? null,
    baudRate: params.baudRate ?? null,
    host: params.host ?? null,
    tcpPort: params.tcpPort ?? null,
    bleDeviceId: params.bleDeviceId ?? null,
    attitudeRateHz: params.attitudeRateHz,
    positionRateHz: params.positionRateHz,
    airspeedEnabled: params.airspeedEnabled,
    flightLogEnabled: params.flightLogEnabled,
    flightLogDbEnabled: params.flightLogDbEnabled,
    flightLogPath: params.flightLogPath,
    flightLogRaw: params.flightLogRaw,
    flightLogRawAlways: params.flightLogRawAlways,
  });
  connection.set({
    status: "connected",
    protocolType: params.protocolType,
    transportType: params.transportType,
    port: params.port ?? params.host ?? params.bleDeviceId ?? '',
    baudRate: params.baudRate ?? 0,
    errorMessage: "",
    fcInfo: info,
  });
  await startTelemetryListeners();
  return info;
}

/**
 * Disconnect from the flight controller, stop listeners, reset telemetry.
 */
export async function disconnectFC(baudRate: number): Promise<void> {
  stopTelemetryListeners();
  resetTelemetry();
  await invoke("disconnect");
  connection.set({
    status: "disconnected",
    protocolType: 'msp',
    transportType: 'serial',
    port: "",
    baudRate,
    errorMessage: "",
    fcInfo: null,
  });
}
