// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Poller behind the weather widget: wind by height at the UAV position, while a widget is mounted and not in replay.
// Faster than the model's 15-min step: a fast aircraft changes model cell within minutes.

import { invoke } from '@tauri-apps/api/core';
import { get, writable } from 'svelte/store';
import { telemetry } from '$lib/stores/telemetry';
import { homePosition } from '$lib/stores/home';
import { replayActive } from '$lib/stores/mission';
import { haversineDistance } from '$lib/utils/geo';
import type { WindAloft } from '$lib/helpers/windAloft';

const REFRESH_MS = 5 * 60_000;
const RETRY_MS = 60_000;
const TICK_MS = 3_000;
/** A position this far from the last request's is asked for at once (launch point → aircraft, vehicle switch). */
const MOVED_M = 5_000;

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
let timer: ReturnType<typeof setInterval> | null = null;
let nextAt = 0;
let busy = false;
let asked: { lat: number; lon: number } | null = null;

function position(): { lat: number; lon: number } | null {
  const t = get(telemetry);
  if (t.lastUpdate && t.fixType >= 1 && (t.lat !== 0 || t.lon !== 0)) return { lat: t.lat, lon: t.lon };
  const h = get(homePosition);
  return h.set && h.source !== 'replay' ? { lat: h.lat, lon: h.lon } : null;
}

async function tick(): Promise<void> {
  if (busy || get(replayActive)) return;
  const pos = position();
  if (!pos) {
    windAloft.update((s) => (s.noPosition ? s : { ...s, noPosition: true }));
    return;
  }
  const moved = !!asked && haversineDistance(asked.lat, asked.lon, pos.lat, pos.lon) > MOVED_M;
  if (Date.now() < nextAt && !moved) return;
  busy = true;
  asked = pos;
  try {
    const data = await invoke<WindAloft>('weather_wind_aloft', pos);
    windAloft.set({ data, fetchedAt: Date.now(), failed: false, noPosition: false });
    nextAt = Date.now() + REFRESH_MS;
  } catch (e) {
    console.warn('[windAloft] request failed:', e);
    windAloft.update((s) => ({ ...s, failed: true, noPosition: false }));
    nextAt = Date.now() + RETRY_MS;
  } finally {
    busy = false;
  }
}

/** Keeps the poller running until the returned function is called. */
export function watchWindAloft(): () => void {
  users++;
  if (!timer) {
    timer = setInterval(() => void tick(), TICK_MS);
    void tick();
  }
  return () => {
    users--;
    if (users === 0 && timer) {
      clearInterval(timer);
      timer = null;
    }
  };
}
