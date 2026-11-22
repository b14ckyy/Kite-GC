// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Mini view, the main window's half: hands the Settings toggle to the backend and, on desktop, feeds the
// mini window snapshots while it is shown. The mini window has no telemetry listeners of its own.

import { get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { emitTo, listen, type UnlistenFn } from '@tauri-apps/api/event';
import { settings } from '$lib/stores/settings';
import { telemetry, type GpsReceiver } from '$lib/stores/telemetry';
import { connection, fcLinkAlive } from '$lib/stores/connection';
import { homePosition } from '$lib/stores/home';
import { prearmReason } from '$lib/stores/statusText';
import { autopilotSystem } from '$lib/stores/autopilotContext';
import { bearing, haversineDistance } from '$lib/utils/geo';
import { batteryLevel } from '$lib/helpers/battery';
import { armingStatus } from '$lib/helpers/arming';
import { LINK_STALE_MS } from '$lib/helpers/linkActivity.svelte';
import { isMobile } from '$lib/platform';
import {
  MINI_LABEL,
  MINI_SNAPSHOT_EVENT,
  MINI_SNAPSHOT_MS,
  MINI_VISIBLE_EVENT,
  type MiniLink,
  type MiniSnapshot,
} from '$lib/helpers/miniSnapshot';

let unsubs: (() => void)[] = [];
let unlisteners: UnlistenFn[] = [];
let lastEnabled: boolean | null = null;
let toggleChain: Promise<unknown> = Promise.resolve();
let timer: ReturnType<typeof setInterval> | undefined;
let lastPush = 0;

/** GPS widget's fused-view rule: the receiver with the higher fix (STATIC / PPP rank as DGPS), then more sats. */
const fixRank = (raw: number) => (raw > 6 ? 4 : raw);

/** The active vehicle's live values. Each rule is the one of the matching widget / indicator. */
export function buildMiniSnapshot(): MiniSnapshot {
  const c = get(connection);
  const t = get(telemetry);
  const s = get(settings);
  const now = Date.now();
  let link: MiniLink;
  if (c.status !== 'connected') link = 'off';
  else if (!t.lastUpdate) link = 'waiting';
  else {
    const fcOk = c.protocolType !== 'telemetry' || get(fcLinkAlive);
    link = now - t.lastUpdate < LINK_STALE_MS && fcOk ? 'live' : 'stale';
  }
  const has = link === 'live' || link === 'stale';
  const batt = has ? batteryLevel(t) : null;
  const home = get(homePosition);
  const homeOk = has && home.set;
  // ArmingIndicator: the INAV bitfield only arrives over MSP and INAV's passive decoders, not over MAVLink.
  const inavFlags = get(autopilotSystem) === 'inav' && c.protocolType !== 'mavlink';
  const arming = has && t.statusSeen ? (armingStatus(t, get(prearmReason), inavFlags)?.level ?? null) : null;
  const best = t.gpsReceivers.reduce<GpsReceiver | null>(
    (b, r) => (!b || fixRank(r.fixRaw) > fixRank(b.fixRaw) || (fixRank(r.fixRaw) === fixRank(b.fixRaw) && r.numSat > b.numSat) ? r : b),
    null,
  );
  const speedIsAir = has && s.airspeedEnabled && t.airspeed > 0;
  const rcIsLq = t.link.lq != null;
  return {
    at: now,
    link,
    arming,
    batteryPct: batt?.percent ?? null,
    voltage: batt?.voltage ?? null,
    mode: has ? t.flightMode.primary : '',
    homeDistM: homeOk ? haversineDistance(t.lat, t.lon, home.lat, home.lon) : null,
    homeRelDeg: homeOk ? (bearing(t.lat, t.lon, home.lat, home.lon) - t.yaw + 360) % 360 : null,
    altM: has ? t.altitude : null,
    varioMs: has ? t.vario : null,
    speedMs: has ? (speedIsAir ? t.airspeed : t.groundSpeed) : null,
    speedIsAir,
    sats: has ? t.numSat : null,
    fixType: has ? t.fixType : 0,
    fixRaw: has && best && best.fixType === t.fixType ? best.fixRaw : null,
    rcPct: has ? (rcIsLq ? t.link.lq : t.link.rssiPercent) : null,
    rcIsLq,
    units: s.interface,
  };
}

/** Main window only; idempotent. */
export function startMiniView(): void {
  if (unsubs.length) return;
  unsubs.push(
    settings.subscribe((s) => {
      const on = s.miniView;
      if (on === lastEnabled) return;
      lastEnabled = on;
      // Chained: async commands may run out of order, and the last flip must win.
      toggleChain = toggleChain
        .then(() => invoke('mini_view_set_enabled', { enabled: on }))
        .catch((e) => console.warn('[mini-view] toggle failed:', e));
    }),
  );
  // Android renders the mini view in this page itself (MiniViewPip).
  if (isMobile) return;
  void listen<boolean>(MINI_VISIBLE_EVENT, (e) => setShown(e.payload)).then((u) => unlisteners.push(u));
  // Telemetry-driven as well: a minimized WebView may throttle the interval, the backend's events still land.
  unsubs.push(
    telemetry.subscribe(() => {
      if (timer !== undefined && Date.now() - lastPush >= MINI_SNAPSHOT_MS) push();
    }),
  );
}

export function stopMiniView(): void {
  setShown(false);
  for (const u of unsubs) u();
  for (const u of unlisteners) u();
  unsubs = [];
  unlisteners = [];
  lastEnabled = null;
}

function setShown(shown: boolean): void {
  clearInterval(timer);
  timer = undefined;
  if (!shown) return;
  push();
  timer = setInterval(push, MINI_SNAPSHOT_MS);
}

function push(): void {
  lastPush = Date.now();
  void emitTo(MINI_LABEL, MINI_SNAPSHOT_EVENT, buildMiniSnapshot()).catch(() => {});
}
