// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Pinned notice with a running timer while a connected link that had delivered data is quiet; an alarm when
// it goes quiet and once a minute after. The link itself stays up (a stall never disconnects): the notice
// ends with new data or a disconnect.

import { get } from 'svelte/store';
import { t } from 'svelte-i18n';
import { connection, fcLinkAlive } from '$lib/stores/connection';
import { telemetry } from '$lib/stores/telemetry';
import { clearPinnedStatus, playAlarm, setPinnedStatus } from '$lib/stores/statusText';
import { LINK_STALE_MS } from '$lib/helpers/linkActivity.svelte';

const KEY = 'telemetry-lost';
const ALARM_REPEAT_MS = 60_000;
const CHECK_MS = 1000;

let started = false;
let lostSince = 0;
let lastAlarm = 0;

function clock(ms: number): string {
  const sec = Math.floor(ms / 1000);
  const h = Math.floor(sec / 3600);
  const mm = Math.floor((sec % 3600) / 60);
  const ss = String(sec % 60).padStart(2, '0');
  return h ? `${h}:${String(mm).padStart(2, '0')}:${ss}` : `${mm}:${ss}`;
}

function check(): void {
  const c = get(connection);
  const last = get(telemetry).lastUpdate;
  const now = Date.now();
  // Passive telemetry never goes quiet by itself: the backend's FC-origin flag decides there.
  const fcGone = c.protocolType === 'telemetry' && !get(fcLinkAlive);
  const lost = c.status === 'connected' && last > 0 && (now - last >= LINK_STALE_MS || fcGone);
  if (!lost) {
    if (lostSince) clearPinnedStatus(KEY);
    lostSince = 0;
    return;
  }
  if (!lostSince) {
    lostSince = fcGone ? now : last;
    lastAlarm = 0;
  }
  setPinnedStatus(KEY, get(t)('connection.telemetryLost', { values: { time: clock(now - lostSince) } }));
  if (now - lastAlarm >= ALARM_REPEAT_MS) {
    lastAlarm = now;
    playAlarm();
  }
}

/** Idempotent. */
export function startTelemetryLostNotice(): void {
  if (started) return;
  started = true;
  setInterval(check, CHECK_MS);
}
