// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Telemetry Relay ("forwarding") config + live status. Re-encodes the live inbound telemetry into a
// chosen wire protocol and emits it out a second link (antenna trackers / GCS / monitoring apps). See
// docs/active/TELEMETRY_FORWARDING.md. Configs persist in the settings store; runtime status arrives via
// the backend `relay-stats` event.

import { writable } from 'svelte/store';

/** Output protocol to encode into. */
export type RelayProtocol = 'ltm' | 'mavlink' | 'crsf' | 'smartport';

/** Output transport kind: serial (covers HC-05/BT-SPP virtual COM) / ble / tcp (server) / udp. */
export type RelayOutputKind = 'serial' | 'ble' | 'tcp' | 'udp';

export interface RelayOutput {
  kind: RelayOutputKind;
  /** serial */
  port?: string;
  baud?: number;
  /** ble — device id */
  bleDeviceId?: string;
  /** tcp — server listen port */
  listenPort?: number;
  /** udp — send target */
  host?: string;
  udpPort?: number;
}

/** One configured relay (persisted). */
export interface RelayConfig {
  id: string;
  enabled: boolean;
  protocol: RelayProtocol;
  output: RelayOutput;
}

/** Per-relay runtime status pushed from the backend (`relay-stats` event, camelCase). */
export interface RelayStatusInfo {
  id: string;
  protocol: string;
  target: string;
  ok: boolean;
  waiting: boolean;
  bytesPerSec: number;
  framesOut: number;
  errors: number;
}

/** Result of (re)configuring a relay (returned by `relay_configure`). */
export interface RelayResult {
  id: string;
  ok: boolean;
  error: string | null;
  target: string | null;
}

/** Live per-relay status, keyed off the `relay-stats` event. */
export const relayStats = writable<RelayStatusInfo[]>([]);

/** Last configure result per relay id (so the UI can show "device missing" / errors). */
export const relayResults = writable<Record<string, RelayResult>>({});

/** Create a fresh default relay config row. */
export function newRelay(): RelayConfig {
  return {
    id: crypto.randomUUID(),
    enabled: true,
    protocol: 'ltm',
    output: { kind: 'serial', port: '', baud: 115200, bleDeviceId: '', listenPort: 5760, host: '', udpPort: 14550 },
  };
}
