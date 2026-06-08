// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Connection state store
// Manages the reactive state of the FC connection across the UI

import { writable } from 'svelte/store';

export type ConnectionStatus = 'disconnected' | 'connecting' | 'connected' | 'error';

export type TransportType = 'serial' | 'tcp' | 'udp' | 'ble';

export type ProtocolType = 'msp' | 'mavlink';

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

export const availablePorts = writable<PortInfo[]>([]);

export const bleDevices = writable<BleDeviceInfo[]>([]);
