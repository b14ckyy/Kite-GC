// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// The weather widget's side of the backend weather service (`weather/service.rs`): says when a widget
// is on screen, hands over the fallback position and mirrors the service's state.

import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { derived, get, writable } from 'svelte/store';
import { homePosition } from '$lib/stores/home';
import { replayActive } from '$lib/stores/mission';
import type { WindAloft } from '$lib/helpers/windAloft';

export interface WindAloftState {
  data: WindAloft | null;
  /** When `data` arrived, ms. */
  fetchedAt: number;
  /** The last request failed (no internet, service down). */
  failed: boolean;
  noPosition: boolean;
}

export const windAloft = writable<WindAloftState>({ data: null, fetchedAt: 0, failed: false, noPosition: false });

let users = 0;
let unlisten: Promise<UnlistenFn> | null = null;
let unsubscribeHome: (() => void) | null = null;

const NO_REFERENCE = { lat: null, lon: null };
const warn = (e: unknown) => console.warn('[windAloft] service not reached:', e);

// The backend outlives a reload of the UI: without this it would go on polling for a widget that is gone.
void invoke('weather_watch', { on: false }).catch(warn);

function start(): void {
  unlisten = listen<WindAloftState>('weather-wind-aloft', (ev) => windAloft.set(ev.payload));
  // No reference while a replay is open: opening one moves the home store to the replayed flight.
  unsubscribeHome = derived([homePosition, replayActive], ([h, replay]) =>
    h.set && h.source !== 'replay' && !replay ? { lat: h.lat, lon: h.lon } : NO_REFERENCE,
  ).subscribe((ref) => void invoke('weather_set_reference', ref).catch(warn));
  void invoke<WindAloftState>('weather_watch', { on: true })
    .then((s) => windAloft.set(s))
    .catch(warn);
}

function stop(): void {
  void unlisten?.then((off) => off());
  unlisten = null;
  unsubscribeHome?.();
  unsubscribeHome = null;
  // The reference goes with the widget: a later recording without one asks for the aircraft only.
  void invoke('weather_set_reference', NO_REFERENCE).catch(warn);
  void invoke('weather_watch', { on: false }).catch(warn);
}

/** Keeps the service polling for the widget until the returned function is called. */
export function watchWindAloft(): () => void {
  users++;
  if (users === 1) start();
  return () => {
    users--;
    if (users === 0) stop();
  };
}

export interface ReplayWind {
  /** The answer that was current at the replay position; null = the flight has none. */
  data: WindAloft | null;
  /** The replayed moment, Unix ms, in steps of REPLAY_NOW_STEP_MS. */
  nowMs: number;
}

export const replayWind = writable<ReplayWind>({ data: null, nowMs: 0 });

/** The widget repaints its dock on every change: the replayed "now" only moves in these steps. */
const REPLAY_NOW_STEP_MS = 60_000;

let replayRows: { timestampMs: number; data: WindAloft }[] = [];
let replayLoad = 0;
/** The player's position: a paused replay does not seek again once the rows are in. */
let replayAt: { flightMs: number; startEpochMs: number } | null = null;

/** Load the weather answers of the replayed flight (`null` = no replay). */
export async function loadReplayWind(flightId: number | null, dbPath: string): Promise<void> {
  const load = ++replayLoad;
  replayRows = [];
  replayWind.set({ data: null, nowMs: 0 });
  if (flightId == null) {
    replayAt = null;
    return;
  }
  try {
    const rows = await invoke<{ timestamp_ms: number; data_json: string }[]>('flightlog_get_weather_records', {
      flightId,
      dbPath: dbPath || undefined,
    });
    if (load !== replayLoad) return;
    replayRows = rows.flatMap((r) => {
      try {
        return [{ timestampMs: r.timestamp_ms, data: JSON.parse(r.data_json) as WindAloft }];
      } catch {
        return [];
      }
    });
    if (replayAt) seekReplayWind(replayAt.flightMs, replayAt.startEpochMs);
  } catch (e) {
    console.warn('[windAloft] weather of the flight not read:', e);
  }
}

/** Move the replayed weather to `flightMs` on the flight's timeline; `startEpochMs` = the flight's start. */
export function seekReplayWind(flightMs: number, startEpochMs: number): void {
  replayAt = { flightMs, startEpochMs };
  if (replayRows.length === 0) return;
  // The newest answer at or before the position; before the first one, the first (it arrives seconds in).
  let row = replayRows[0];
  for (const r of replayRows) if (r.timestampMs <= flightMs) row = r;
  const nowMs = Math.floor((startEpochMs + flightMs) / REPLAY_NOW_STEP_MS) * REPLAY_NOW_STEP_MS;
  const cur = get(replayWind);
  if (cur.data !== row.data || cur.nowMs !== nowMs) replayWind.set({ data: row.data, nowMs });
}
