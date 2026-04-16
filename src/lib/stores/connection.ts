// Connection state store
// Manages the reactive state of the FC connection across the UI

import { writable } from 'svelte/store';

export type ConnectionStatus = 'disconnected' | 'connecting' | 'connected' | 'error';

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

export interface ConnectionInfo {
  status: ConnectionStatus;
  port: string;
  baudRate: number;
  errorMessage: string;
  fcInfo: FcInfo | null;
}

export const connection = writable<ConnectionInfo>({
  status: 'disconnected',
  port: '',
  baudRate: 115200,
  errorMessage: '',
  fcInfo: null,
});

export const availablePorts = writable<PortInfo[]>([]);
