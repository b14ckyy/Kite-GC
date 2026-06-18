// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Telemetry Relay controller — pushes the persisted relay configs to the backend and tracks live status.
// Relays auto-connect with the primary path (push telemetry needs no handshake); on disconnect they are
// torn down. See docs/active/TELEMETRY_FORWARDING.md.

import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { get } from 'svelte/store';
import { settings } from '$lib/stores/settings';
import { connection } from '$lib/stores/connection';
import { relayStats, relayResults, type RelayStatusInfo, type RelayResult } from '$lib/stores/relay';

let statsUnlisten: UnlistenFn | null = null;

/** Subscribe to live per-relay status (byte rate / ok / errors). Idempotent. */
export async function startRelayStatsListener(): Promise<void> {
  if (statsUnlisten) return;
  statsUnlisten = await listen<RelayStatusInfo[]>('relay-stats', (e) => relayStats.set(e.payload));
}

function stopRelayStatsListener(): void {
  statsUnlisten?.();
  statsUnlisten = null;
}

/** Hand the saved configs to the backend, which opens the available output devices. Returns per-relay
 *  results so the UI can flag device-missing / errors. */
async function pushConfigs(): Promise<void> {
  const configs = get(settings).relays ?? [];
  const results = await invoke<RelayResult[]>('relay_configure', { configs });
  relayResults.set(Object.fromEntries(results.map((r) => [r.id, r])));
}

/** Called on primary connect — start status listener + apply the saved relays. */
export async function applyRelaysOnConnect(): Promise<void> {
  await startRelayStatsListener();
  await pushConfigs();
}

/** Re-apply after the user edits a relay while connected, so changes take effect live. No-op when
 *  disconnected — the edited config is persisted and applied on the next connect. */
export async function reconfigureRelays(): Promise<void> {
  if (get(connection).status !== 'connected') return;
  await pushConfigs();
}

/** Called on primary disconnect — tear down all relays + stop listening. */
export async function clearRelaysOnDisconnect(): Promise<void> {
  try {
    await invoke('relay_clear');
  } finally {
    stopRelayStatsListener();
    relayStats.set([]);
    relayResults.set({});
  }
}
