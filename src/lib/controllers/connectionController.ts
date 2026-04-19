import { invoke } from "@tauri-apps/api/core";
import type { FcInfo, PortInfo, BleDeviceInfo, TransportType, ProtocolType } from '$lib/stores/connection';
import { connection, availablePorts, bleDevices } from '$lib/stores/connection';
import { startTelemetryListeners, stopTelemetryListeners, resetTelemetry } from '$lib/stores/telemetry';

/**
 * Refresh the list of serial ports via Tauri.
 * Returns the port that should be selected (preserves current if still valid).
 */
export async function refreshSerialPorts(currentPort: string): Promise<string> {
  const result = await invoke<PortInfo[]>("list_serial_ports");
  availablePorts.set(result);
  if (result.length > 0 && !currentPort) return result[0].path;
  if (currentPort && result.some((p) => p.path === currentPort)) return currentPort;
  if (result.length > 0) return result[0].path;
  return currentPort;
}

/**
 * Scan for BLE devices matching known serial profiles.
 * Updates the bleDevices store and returns the list.
 */
export async function scanBleDevices(): Promise<BleDeviceInfo[]> {
  const result = await invoke<BleDeviceInfo[]>("scan_ble_devices");
  bleDevices.set(result);
  return result;
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
