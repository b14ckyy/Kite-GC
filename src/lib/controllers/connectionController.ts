// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { get } from 'svelte/store';
import type { FcInfo, PortInfo, BleDeviceInfo, TransportType, ProtocolType } from '$lib/stores/connection';
import type { InavStats } from '$lib/stores/flightlogTypes';
import { connection, connectionProtocol, fcLinkAlive, availablePorts, bleDevices, reconnectAttempt } from '$lib/stores/connection';
import { startTelemetryListeners, stopTelemetryListeners, resetTelemetry } from '$lib/stores/telemetry';
import { applyRelaysOnConnect, clearRelaysOnDisconnect } from '$lib/controllers/relayController';
import { loadSafehomeConfig, clearSafehome } from '$lib/stores/safehome';
import { loadGeozoneConfig, clearGeozones } from '$lib/stores/geozone';
import { loadFenceConfig, clearFence } from '$lib/stores/fence';
import { loadRallyConfig, clearRally } from '$lib/stores/rally';

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
  windEnabled: boolean;
  mavlinkFullTelemetry: boolean;
  flightLogEnabled: boolean;
  flightLogDbEnabled: boolean;
  flightLogPath: string;
  flightLogRawPath: string;
  flightLogRaw: boolean;
  flightLogRawAlways: boolean;
}

// ── Auto-reconnect state ────────────────────────────────────────────────────
// "Always try to get the link back unless the user said stop." A successful user connect records the
// params + sets userWantsConnected; a user disconnect clears it. On an unexpected drop (connection-lost),
// attemptReconnect() loops connectFC with the stored params until it succeeds or the user disconnects.
// See docs/active/AUTO_RECONNECT.md. (A settings toggle to disable it is a trivial follow-up — endless-on
// by default is what we want for the field.)
let intended: ConnectParams | null = null;
let userWantsConnected = false;
let reconnectActive = false;

/** Whether the user currently wants to be connected (i.e. an unexpected drop should auto-reconnect). */
export function reconnectWanted(): boolean {
  return userWantsConnected && intended != null;
}

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

/**
 * Reconnect loop after an unexpected drop. Tears the dead link down cleanly (without clearing the
 * user-intent), then retries the stored connection endlessly with a small capped backoff until it
 * reconnects or the user disconnects. Re-entrant-safe (a second connection-lost is ignored while a loop
 * runs). No-op if the user never connected / already disconnected.
 */
export async function attemptReconnect(): Promise<void> {
  if (reconnectActive || !userWantsConnected || !intended) return;
  reconnectActive = true;
  // The backend already tore the scheduler down on the drop; clean up the transport + frontend listeners
  // so the retry starts from a known state — but DON'T touch userWantsConnected/intended.
  try { await invoke('disconnect'); } catch { /* already gone */ }
  stopTelemetryListeners();
  resetTelemetry();
  await clearRelaysOnDisconnect().catch(() => {});

  let attempt = 0;
  while (userWantsConnected && intended) {
    attempt += 1;
    reconnectAttempt.set(attempt);
    connection.update((c) => ({ ...c, status: 'reconnecting', errorMessage: '' }));
    try {
      await connectFC(intended);
      break; // success → connectFC set status 'connected'
    } catch {
      if (!userWantsConnected) break;
      await sleep(attempt <= 2 ? 2000 : 5000); // 2s, 2s, then 5s cap
    }
  }
  reconnectAttempt.set(0);
  reconnectActive = false;
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
    windEnabled: params.windEnabled,
    mavlinkFullTelemetry: params.mavlinkFullTelemetry,
    flightLogEnabled: params.flightLogEnabled,
    flightLogDbEnabled: params.flightLogDbEnabled,
    flightLogPath: params.flightLogPath,
    flightLogRawPath: params.flightLogRawPath,
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
  // Seed the status-box protocol. MSP/MAVLink are known now; passive telemetry shows a placeholder
  // until the backend's `telemetry-protocol` event reports the locked sub-protocol.
  connectionProtocol.set({
    primary: params.protocolType === 'mavlink' ? 'MAVLink' : params.protocolType === 'msp' ? 'MSP' : 'Telemetry',
    secondary: null,
  });
  fcLinkAlive.set(true);
  // Record the intent so an unexpected drop auto-reconnects to the same endpoint (see attemptReconnect).
  intended = params;
  userWantsConnected = true;
  await startTelemetryListeners();
  // Auto-start the saved telemetry relays (push telemetry → no handshake needed).
  await applyRelaysOnConnect();
  // INAV/MSP: always download safehomes + autoland config for the map overlay (fire-and-forget; the
  // store updates when the ~18 MSP reads complete). See docs/active/AUTOLAND_SAFEHOME.md.
  if (params.protocolType === 'msp') {
    void loadSafehomeConfig();
    // Geozones (INAV ≥8.0; the backend returns has_geozones=false on older FCs). See docs/active/GEOZONES.md.
    void loadGeozoneConfig();
  }
  // ArduPilot/PX4 geofence + rally points over MAVLink (MAV_MISSION_TYPE_FENCE/RALLY). Both ride the
  // mission microprotocol (strict request→response) — run them SEQUENTIALLY so the two downloads don't
  // collide. See docs/active/GEOFENCE.md.
  if (params.protocolType === 'mavlink') {
    void (async () => { await loadFenceConfig(); await loadRallyConfig(); })();
  }
  return info;
}

/**
 * Disconnect from the flight controller, stop listeners, reset telemetry.
 */
export async function disconnectFC(baudRate: number): Promise<void> {
  // User-initiated (or recovery-driven) disconnect → stop wanting the link, so any in-flight reconnect
  // loop exits and a drop during teardown never re-triggers one.
  userWantsConnected = false;
  intended = null;
  reconnectAttempt.set(0);
  await clearRelaysOnDisconnect();
  clearSafehome();
  clearGeozones();
  clearFence();
  clearRally();
  stopTelemetryListeners();
  resetTelemetry();
  connectionProtocol.set({ primary: '', secondary: null });
  fcLinkAlive.set(true);
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

/** Write a craft name to the connected INAV FC (MSP_SET_NAME + EEPROM). INAV/MSP only — errors for
 *  other links. Used post-flight to push a newly chosen craft name so future flights auto-link. */
export async function setInavCraftName(name: string): Promise<void> {
  await invoke("inav_set_craft_name", { name });
}

/** Read the INAV lifetime flight statistics from the FC `stats` settings (INAV/MSP only). */
export async function readInavStats(): Promise<InavStats> {
  return invoke<InavStats>("inav_read_stats");
}
