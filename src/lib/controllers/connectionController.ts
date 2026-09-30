// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { get } from 'svelte/store';
import { t } from 'svelte-i18n';
import type { FcInfo, PortInfo, BleDeviceInfo, TransportType, ProtocolType } from '$lib/stores/connection';
import type { InavStats } from '$lib/stores/flightlogTypes';
import { connection, connectionProtocol, fcLinkAlive, availablePorts, bleDevices, detectedPlatformType, hasMsp, isArduPilotLink } from '$lib/stores/connection';
import { startTelemetryListeners, stopTelemetryListeners, resetTelemetry, setFcVariant } from '$lib/stores/telemetry';
import { startVehicleListeners, activeVehicleId, resetVehicles, refreshLinks, selectVehicle, links, vehicles } from '$lib/stores/vehicles';
import { parseVehicleId } from '$lib/helpers/vehicleId';
import { homePosition, vehicleHomes, clearVehicleHomes } from '$lib/stores/home';
import { launchPoint } from '$lib/stores/mission';
import { applyRelaysOnConnect, clearRelaysOnDisconnect } from '$lib/controllers/relayController';
import { loadSafehomeConfig, clearSafehome } from '$lib/stores/safehome';
import { loadGeozoneConfig, clearGeozones } from '$lib/stores/geozone';
import { loadFenceConfig, clearFence } from '$lib/stores/fence';
import { loadRallyConfig, clearRally } from '$lib/stores/rally';
import { loadPx4RcInMode, px4RcInMode, px4RcInModeError } from '$lib/stores/rcFcConfig';
import { rcPlatform } from '$lib/stores/rcPlatform';

/** Session-only memory of the platform-type override, keyed by the FC hardware id (MSP / MAVLink).
 *  RAM only by design: a Kite restart forgets it; passive telemetry has no id and never lands here. */
const platformOverrides = new Map<string, number>();

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
  dropPendingBleDevices();
  bleDevices.set([]);
}

let bleUnlisten: UnlistenFn | null = null;

// Discovery events are batched before they reach the store. Devices arrive one event at a time —
// a burst of them in the first seconds of a scan, then RSSI/name updates from backends that report
// those (desktop) — and every store write re-renders the device <select>. On Android that tears the
// native picker popup down and rebuilds it per event, which reads as flicker while it is open; on
// desktop it is merely wasted renders. One write per window, and only when something changed.
const BLE_FLUSH_MS = 600;
let blePending = new Map<string, BleDeviceInfo>();
let bleFlushTimer: ReturnType<typeof setTimeout> | null = null;

function dropPendingBleDevices(): void {
  blePending = new Map();
  if (bleFlushTimer) {
    clearTimeout(bleFlushTimer);
    bleFlushTimer = null;
  }
}

function flushBleDevices(): void {
  bleFlushTimer = null;
  if (blePending.size === 0) return;
  const incoming = blePending;
  blePending = new Map();
  bleDevices.update((list) => {
    let changed = false;
    const next = [...list];
    for (const dev of incoming.values()) {
      const i = next.findIndex((d) => d.id === dev.id);
      if (i < 0) {
        next.push(dev);
        changed = true;
      } else if (next[i].name !== dev.name || next[i].profile !== dev.profile || next[i].rssi !== dev.rssi) {
        next[i] = dev;
        changed = true;
      }
    }
    return changed ? next : list;
  });
}

/** Subscribe to live BLE discovery events and upsert them into the bleDevices store — batched, see
 *  above. Idempotent. */
export async function startBleDeviceListener(): Promise<void> {
  if (bleUnlisten) return;
  bleUnlisten = await listen<BleDeviceInfo>("ble-device", (e) => {
    blePending.set(e.payload.id, e.payload);
    if (!bleFlushTimer) bleFlushTimer = setTimeout(flushBleDevices, BLE_FLUSH_MS);
  });
}

/** Tear down the BLE discovery listener. */
export function stopBleDeviceListener(): void {
  bleUnlisten?.();
  bleUnlisten = null;
  dropPendingBleDevices();
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

/**
 * Connect to the flight controller via Tauri, update stores, start listeners.
 */
/** What the backend's `connect` returns: registry ids of the new link / its primary vehicle + the
 *  handshake info. */
export interface ConnectResult {
  linkId: number;
  vehicleId: string;
  fcInfo: FcInfo;
}

/** The backend `connect` argument object for a set of connect params (shared by the first-link and
 *  additional-link paths). */
function connectArgs(params: ConnectParams): Record<string, unknown> {
  return {
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
  };
}

export async function connectFC(params: ConnectParams): Promise<FcInfo> {
  // Registry events first, so the backend's active-vehicle announcement (emitted as soon as the link is
  // registered, before `connect` returns) is not missed.
  await startVehicleListeners();
  let res: ConnectResult;
  try {
    res = await invoke<ConnectResult>("connect", connectArgs(params));
  } catch (e) {
    // Other links may still be up (multi-vehicle) — keep the mirror truthful for the caller's recovery.
    await refreshLinks().catch(() => {});
    throw e;
  }
  void refreshLinks().catch(() => {});
  const info = res.fcInfo;
  // Phase A: the singleton UI follows the newest link's primary vehicle (the backend only auto-selects
  // when nothing was active, so a second link must be selected explicitly here).
  activeVehicleId.set(res.vehicleId);
  // Re-apply the override chosen earlier this session for the same FC (see `platformOverrides`).
  detectedPlatformType.set(info.platform_type);
  const remembered = info.fc_uid ? platformOverrides.get(info.fc_uid) : undefined;
  if (remembered !== undefined && remembered !== info.platform_type) {
    try {
      await invoke('set_platform_type', { platformType: remembered });
      info.platform_type = remembered;
    } catch (e) {
      console.warn('[connect] restoring the platform-type override failed', e);
    }
  }
  // MAVLink link to INAV 10.0+ with MSP over MAVLink up (the backend probed it during connect).
  const mspTunnel = params.protocolType === 'mavlink' && (info.features?.msp_tunnel ?? false);
  connection.set({
    status: "connected",
    protocolType: params.protocolType,
    transportType: params.transportType,
    port: params.port ?? params.host ?? params.bleDeviceId ?? '',
    baudRate: params.baudRate ?? 0,
    errorMessage: "",
    fcInfo: info,
    mspTunnel,
  });
  // Seed the status-box protocol. MSP/MAVLink are known now; passive telemetry shows a placeholder
  // until the backend's `telemetry-protocol` event reports the locked sub-protocol.
  const tr = get(t);
  connectionProtocol.set({
    primary: mspTunnel
      ? tr('statusBox.protocolMspMav')
      : params.protocolType === 'mavlink'
        ? tr('statusBox.protocolMavlink')
        : params.protocolType === 'msp'
          ? tr('statusBox.protocolMsp')
          : tr('statusBox.protocolTelemetry'),
    secondary: null,
  });
  fcLinkAlive.set(true);
  setFcVariant(info.fc_variant);
  await startTelemetryListeners();
  // Auto-start the saved telemetry relays (push telemetry → no handshake needed).
  await applyRelaysOnConnect();
  // INAV/MSP (direct or MSP over MAVLink): always download safehomes + autoland config for the map
  // overlay (fire-and-forget; the store updates when the ~18 MSP reads complete). See
  // docs/active/AUTOLAND_SAFEHOME.md.
  if (get(hasMsp)) {
    void loadSafehomeConfig();
    // Geozones (INAV ≥8.0; the backend returns has_geozones=false on older FCs). See docs/active/GEOZONES.md.
    void loadGeozoneConfig();
  }
  // ArduPilot/PX4 geofence + rally points over MAVLink (MAV_MISSION_TYPE_FENCE/RALLY). Both ride the
  // mission microprotocol (strict request→response) — run them SEQUENTIALLY so the two downloads don't
  // collide. See docs/active/GEOFENCE.md. Not on an MSP-over-MAVLink link: that FC is INAV (geozones).
  // PX4's COM_RC_IN_MODE (RC panel banner) rides the same single param-receiver slot, so it comes last
  // in the sequence instead of racing the downloads from a component effect.
  if (get(isArduPilotLink)) {
    const px4 = get(rcPlatform) === 'px4';
    void (async () => {
      await loadFenceConfig();
      await loadRallyConfig();
      if (px4) await loadPx4RcInMode();
    })();
  }
  return info;
}

/**
 * Open an ADDITIONAL link while one is already up (multi-vehicle). Leaves the singleton connection
 * state and the active vehicle alone — the new vehicle shows up in the vehicle list (LinkManager) and
 * is switched to explicitly via `switchVehicle`.
 */
export async function connectLink(params: ConnectParams): Promise<ConnectResult> {
  await startVehicleListeners();
  const res = await invoke<ConnectResult>("connect", connectArgs(params));
  await refreshLinks();
  return res;
}

/**
 * Close one link. The last link goes through the full `disconnectFC` teardown; otherwise only that
 * link is stopped and, if the active vehicle lived on it, the singleton UI re-targets the vehicle the
 * backend moved to.
 */
export async function disconnectLink(linkId: number, baudRate: number): Promise<void> {
  const remaining = get(links).filter((l) => l.linkId !== linkId);
  if (remaining.length === 0) {
    await disconnectFC(baudRate);
    return;
  }
  const activeBefore = get(activeVehicleId);
  const wasActiveLink = activeBefore != null && parseVehicleId(activeBefore)?.link === linkId;
  await invoke("disconnect", { linkId });
  await refreshLinks();
  if (wasActiveLink) {
    // The backend already announced the new active vehicle (`active-vehicle-changed`).
    const now = get(activeVehicleId);
    if (now) await applyActiveVehicle(now);
  }
}

/** Make `vehicleId` the vehicle the widgets / control panel follow. */
export async function switchVehicle(vehicleId: string): Promise<void> {
  await selectVehicle(vehicleId);
  await applyActiveVehicle(vehicleId);
}

/** Re-target every singleton store at the (already selected) active vehicle: drop the previous
 *  vehicle's telemetry/config, describe the new link in the connection store, reload its FC-side
 *  config (fence/rally or safehome/geozone). */
async function applyActiveVehicle(vehicleId: string): Promise<void> {
  const link = get(links).find((l) => l.linkId === parseVehicleId(vehicleId)?.link);
  // (The telemetry store re-seeds itself from the new vehicle's cached state — see stores/telemetry.)
  // Home: the new vehicle's FC home if we have seen one; otherwise the previous vehicle's locked home
  // must not masquerade as this one's — demote it to a manual reference (same as on disconnect).
  const home = get(vehicleHomes).get(vehicleId);
  if (home) {
    homePosition.set({ lat: home.lat, lon: home.lon, alt: home.alt, set: true, source: 'fc' });
    launchPoint.set({ lat: home.lat, lng: home.lon });
  } else {
    const h = get(homePosition);
    if (h.source === 'fc') homePosition.set({ ...h, source: 'manual' });
  }
  clearSafehome();
  clearGeozones();
  clearFence();
  clearRally();
  if (!link) return;
  const protocolType: ProtocolType =
    link.protocol === 'MAVLink' ? 'mavlink' : link.protocol === 'MSP' ? 'msp' : 'telemetry';
  // A secondary vehicle on a shared MAVLink link has no handshake of its own — describe it from its
  // HEARTBEAT identity (variant / platform / MAV_TYPE) on top of the link's FcInfo.
  const v = get(vehicles).get(vehicleId);
  const fcInfo: FcInfo = v && !v.primary
    ? { ...link.fcInfo, fc_variant: v.fcVariant, platform_type: v.platformType, mav_type: v.mavType, craft_name: '' }
    : link.fcInfo;
  // MSP over MAVLink belongs to the link's handshake (primary) vehicle only — the backend's `with_msp`
  // refuses a secondary vehicle on the same link.
  const mspTunnel = protocolType === 'mavlink' && (v?.primary ?? true) && (fcInfo.features?.msp_tunnel ?? false);
  connection.update((c) => ({ ...c, status: 'connected', protocolType, port: link.transport, fcInfo, mspTunnel }));
  // Same status-box labels as connectFC.
  const tr = get(t);
  connectionProtocol.set({
    primary: mspTunnel
      ? tr('statusBox.protocolMspMav')
      : protocolType === 'mavlink'
        ? tr('statusBox.protocolMavlink')
        : protocolType === 'msp'
          ? tr('statusBox.protocolMsp')
          : tr('statusBox.protocolTelemetry'),
    secondary: null,
  });
  fcLinkAlive.set(true);
  setFcVariant(fcInfo.fc_variant);
  px4RcInMode.set(null);
  px4RcInModeError.set(null);
  // Same config loads as connectFC, for the new active vehicle: INAV (direct MSP or MSP over MAVLink)
  // → safehomes + geozones; ArduPilot/PX4 → fence, rally, then PX4's COM_RC_IN_MODE, SEQUENTIALLY.
  if (get(hasMsp)) {
    void loadSafehomeConfig();
    void loadGeozoneConfig();
  }
  if (get(isArduPilotLink)) {
    const px4 = get(rcPlatform) === 'px4';
    void (async () => {
      await loadFenceConfig();
      await loadRallyConfig();
      if (px4) await loadPx4RcInMode();
    })();
  }
}

/**
 * Bring the singleton UI in line with what the backend actually holds. Used after a failed connect
 * (other links may still be open) and at startup (a page reload while connected): when links exist
 * the UI must show "connected" and follow the active vehicle, not the failed attempt. Returns whether
 * any link is up.
 */
export async function recoverBackendLinks(): Promise<boolean> {
  await startVehicleListeners();
  const l = await refreshLinks().catch(() => [] as Awaited<ReturnType<typeof refreshLinks>>);
  if (l.length === 0) return false;
  // The vehicle list is built from live `vehicle-discovered` events; after a reload ask for a replay.
  await invoke('announce_vehicles').catch(() => {});
  let active = get(activeVehicleId);
  if (!active) {
    active = await invoke<string | null>('get_active_vehicle').catch(() => null);
    if (active) activeVehicleId.set(active);
  }
  // Always (re)arm the live feed: this runs after a page reload or a UI state that drifted, where the
  // module-level listeners may be gone even though the store still says "connected".
  await startTelemetryListeners();
  if (get(connection).status !== 'connected') await applyRelaysOnConnect();
  if (active) await applyActiveVehicle(active);
  void invoke('log_frontend', { level: 'info', area: 'ui', message: `recoverBackendLinks: ${l.length} link(s), active ${active ?? 'none'} — telemetry listeners armed` }).catch(() => {});
  return true;
}

/**
 * Disconnect from the flight controller, stop listeners, reset telemetry.
 */
export async function disconnectFC(baudRate: number): Promise<void> {
  await clearRelaysOnDisconnect();
  clearSafehome();
  clearGeozones();
  clearFence();
  clearRally();
  px4RcInMode.set(null);
  px4RcInModeError.set(null);
  stopTelemetryListeners();
  resetTelemetry();
  connectionProtocol.set({ primary: '', secondary: null });
  fcLinkAlive.set(true);
  // linkId null = every link (the single connect/disconnect toggle's semantics).
  await invoke("disconnect", { linkId: null });
  resetVehicles();
  clearVehicleHomes();
  connection.set({
    status: "disconnected",
    protocolType: 'msp',
    transportType: 'serial',
    port: "",
    baudRate,
    errorMessage: "",
    fcInfo: null,
    mspTunnel: false,
  });
}

/** Write a craft name to the connected INAV FC (MSP_SET_NAME + EEPROM). INAV/MSP only — errors for
 *  other links. Used post-flight to push a newly chosen craft name so future flights auto-link. */
export async function setInavCraftName(name: string): Promise<void> {
  await invoke("inav_set_craft_name", { name });
}

/** Live platform-type override (UAV Info panel dropdown). Applied to the backend (FC info + the flight
 *  recorder, so the flights of this link are stored with the chosen type), mirrored into the store, and
 *  remembered per FC hardware id for a reconnect within this session. Never persisted. */
export async function setLivePlatformType(platformType: number): Promise<void> {
  await invoke('set_platform_type', { platformType });
  connection.update((c) => (c.fcInfo ? { ...c, fcInfo: { ...c.fcInfo, platform_type: platformType } } : c));
  const uid = get(connection).fcInfo?.fc_uid;
  if (uid) platformOverrides.set(uid, platformType);
}

/** Read the INAV lifetime flight statistics from the FC `stats` settings (INAV/MSP only). */
export async function readInavStats(): Promise<InavStats> {
  return invoke<InavStats>("inav_read_stats");
}
