// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Connection state store
// Manages the reactive state of the FC connection across the UI

import { writable } from 'svelte/store';

export type ConnectionStatus = 'disconnected' | 'connecting' | 'connected' | 'error';

export type TransportType = 'serial' | 'tcp' | 'udp' | 'ble';

export type ProtocolType = 'msp' | 'mavlink' | 'telemetry';

export interface InavVersion {
  major: number;
  minor: number;
  patch: number;
}

export interface FeatureSet {
  version: InavVersion;
  autoland_config: boolean;
  geozones: boolean;
  msp_rc: boolean;
  aux_rc: boolean;
  adsb_msp: boolean;
}

export interface FcInfo {
  msp_protocol: number;
  api_version: string;
  fc_variant: string;
  fc_version: string;
  craft_name: string;
  board_id: string;
  hardware_revision: number;
  platform_type: number;
  mixer_preset: number;
  features: FeatureSet | null;
  /** MAVLink HEARTBEAT MAV_TYPE (ArduPilot/PX4 only; 0 for MSP). The only reliable QuadPlane signal
   *  (a QuadPlane reports fc_variant "ArduPlane" but a VTOL_* MAV_TYPE). */
  mav_type: number;
}

export interface PortInfo {
  path: string;
  label: string;
  port_type: string;
}

export interface BleDeviceInfo {
  id: string;
  name: string;
  profile: string;
  rssi: number | null;
}

export interface ConnectionInfo {
  status: ConnectionStatus;
  protocolType: ProtocolType;
  transportType: TransportType;
  port: string;
  baudRate: number;
  errorMessage: string;
  fcInfo: FcInfo | null;
}

export const connection = writable<ConnectionInfo>({
  status: 'disconnected',
  protocolType: 'msp',
  transportType: 'serial',
  port: '',
  baudRate: 115200,
  errorMessage: '',
  fcInfo: null,
});

/// Active protocol for the connection status box: the primary protocol name (MSP / MAVLink / SmartPort /
/// CRSF / LTM) and an optional secondary tunneled inside it (e.g. ArduPilot passthrough → "MAVLink").
/// Set from `protocolType` on connect for MSP/MAVLink; for passive telemetry the backend's
/// `telemetry-protocol` event fills it once the sub-protocol locks.
export const connectionProtocol = writable<{ primary: string; secondary: string | null }>({
  primary: '',
  secondary: null,
});

/// FC-link liveness for passive telemetry. The decoder re-emits cached state and the receiver/TX keeps
/// sending housekeeping (RSSI) after the FC drops, so the generic "any data" heartbeat never goes stale;
/// the backend's `telemetry-fc-link` tracks fresh FC-origin frames instead. true = FC data flowing.
export const fcLinkAlive = writable<boolean>(true);

export const availablePorts = writable<PortInfo[]>([]);

export const bleDevices = writable<BleDeviceInfo[]>([]);
