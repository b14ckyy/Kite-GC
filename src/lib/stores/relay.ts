// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Telemetry Relay ("forwarding") config + live status. Re-encodes the live inbound telemetry into a
// chosen wire protocol and emits it out a second link (antenna trackers / GCS / monitoring apps). See
// docs/active/TELEMETRY_FORWARDING.md. Configs persist in the settings store; runtime status arrives via
// the backend `relay-stats` event.

import { writable } from 'svelte/store';

/** Output protocol to encode into. `json` is the read-only export for external services (served by the
 *  `http` output); the rest are binary FC protocols for trackers/GCS. */
export type RelayProtocol = 'ltm' | 'mavlink' | 'crsf' | 'smartport' | 'json';

/** Output transport kind: serial (covers HC-05/BT-SPP virtual COM) / ble / tcp (server) / udp /
 *  http (server: JSON snapshot + SSE stream). */
export type RelayOutputKind = 'serial' | 'ble' | 'tcp' | 'udp' | 'http';

export interface RelayOutput {
  kind: RelayOutputKind;
  /** serial */
  port?: string;
  baud?: number;
  /** ble — device id */
  bleDeviceId?: string;
  /** tcp + http — server listen port */
  listenPort?: number;
  /** udp — send target */
  host?: string;
  udpPort?: number;
  /** http — bind 0.0.0.0 (LAN-reachable) instead of loopback. Off by default. */
  lan?: boolean;
}

/** One configured relay (persisted). */
export interface RelayConfig {
  id: string;
  enabled: boolean;
  protocol: RelayProtocol;
  /** Stamped into every frame of the `json` export so a consumer can attribute the samples. REQUIRED for
   *  `json` — the backend refuses to start the relay without one, so no port is bound. Unused by the FC
   *  protocols, which have no field to carry it. */
  missionId?: string;
  /** Output rate for `json`, in Hz. Absent → backend default (5 Hz). Ignored by the FC protocols. */
  rateHz?: number;
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

/** Default emit rate for the JSON export (Hz) — mirrors the backend's fallback in encoders/json.rs. */
export const DEFAULT_JSON_RATE_HZ = 5;

/** Default listen port for the HTTP export server. */
export const DEFAULT_HTTP_PORT = 8080;

/** A `json` relay with no mission ID is refused by the backend (no port is bound), so the UI flags it. */
export function missionIdMissing(r: RelayConfig): boolean {
  return r.protocol === 'json' && !r.missionId?.trim();
}

/** Create a fresh default relay config row. */
export function newRelay(): RelayConfig {
  return {
    id: crypto.randomUUID(),
    enabled: true,
    protocol: 'ltm',
    missionId: '',
    rateHz: DEFAULT_JSON_RATE_HZ,
    output: { kind: 'serial', port: '', baud: 115200, bleDeviceId: '', listenPort: 5760, host: '', udpPort: 14550, lan: false },
  };
}
