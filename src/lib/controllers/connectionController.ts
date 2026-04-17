import { invoke } from "@tauri-apps/api/core";
import type { FcInfo, PortInfo } from '$lib/stores/connection';
import { connection, availablePorts } from '$lib/stores/connection';
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

export interface ConnectParams {
  port: string;
  baudRate: number;
  attitudeRateHz: number;
  positionRateHz: number;
  airspeedEnabled: boolean;
  flightLogEnabled: boolean;
  flightLogPath: string;
  flightLogRaw: boolean;
}

/**
 * Connect to the flight controller via Tauri, update stores, start listeners.
 */
export async function connectFC(params: ConnectParams): Promise<FcInfo> {
  const info = await invoke<FcInfo>("connect", {
    port: params.port,
    baudRate: params.baudRate,
    attitudeRateHz: params.attitudeRateHz,
    positionRateHz: params.positionRateHz,
    airspeedEnabled: params.airspeedEnabled,
    flightLogEnabled: params.flightLogEnabled,
    flightLogPath: params.flightLogPath,
    flightLogRaw: params.flightLogRaw,
  });
  connection.set({
    status: "connected",
    port: params.port,
    baudRate: params.baudRate,
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
    port: "",
    baudRate,
    errorMessage: "",
    fcInfo: null,
  });
}
