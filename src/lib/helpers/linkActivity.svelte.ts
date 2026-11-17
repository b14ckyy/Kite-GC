// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

import { fromStore } from 'svelte/store';
import { connection, fcLinkAlive } from '$lib/stores/connection';
import type { TelemetryData } from '$lib/stores/telemetry';

export type LinkActivity = 'waiting' | 'live' | 'stale';

/** Without telemetry for this long a connected link counts as quiet. */
export const LINK_STALE_MS = 5000;

/**
 * Data flow on a connected link: `waiting` until the first telemetry, `stale` after 5 s without any.
 * Passive telemetry never goes quiet (the decoder re-emits cached state, the receiver keeps sending
 * RSSI), so there the backend's FC-origin flag decides. Call during component initialisation.
 */
export function linkActivity(telem: () => TelemetryData): () => LinkActivity {
  let now = $state(Date.now());
  $effect(() => {
    const id = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(id);
  });
  const conn = fromStore(connection);
  const fcAlive = fromStore(fcLinkAlive);
  const activity = $derived.by<LinkActivity>(() => {
    const last = telem().lastUpdate;
    if (!last) return 'waiting';
    const fcOk = conn.current.protocolType !== 'telemetry' || fcAlive.current;
    return now - last < LINK_STALE_MS && fcOk ? 'live' : 'stale';
  });
  return () => activity;
}
