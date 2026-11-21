// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// FrSky Bluetooth trainer radios as RC control inputs (Dev-Docs active/FRSKY_BT_TRAINER.md). The
// backend task runs with the HID input (stores/hid.ts) and adds the streaming radio to the HID list.

import { writable, get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { settings, type TrainerRadio } from './settings';
import { bleDevices } from './connection';
import { hidActive } from './hid';

/** Matches the Rust `TrainerStatus`. */
export interface TrainerStatus {
  state: string;
  device_id: string;
  device: string;
  error: string | null;
  characteristic: string;
  channels: number[];
  frames: number;
  rejected: number;
  notifications: number;
  interval_last_ms: number;
  interval_mean_ms: number;
  interval_p95_ms: number;
  interval_max_ms: number;
  frame_age_ms: number | null;
  stalls: number;
  reconnects: number;
  last_raw: string;
  last_rejected_raw: string;
  pairing: boolean;
}

export const trainerStatus = writable<TrainerStatus | null>(null);
/** Ids of remembered radios that sent telemetry instead of trainer frames at the last look. */
export const trainerForeign = writable<string[]>([]);

let listening = false;
let retryTimer: ReturnType<typeof setTimeout> | null = null;
/** A radio that sent telemetry stays remembered; look again this often for trainer data. */
const RETRY_FOREIGN_MS = 15000;

const HOLDING = ['connecting', 'waiting', 'streaming', 'stalled'];

function radios(): TrainerRadio[] {
  return get(settings).rcControl.trainerRadios ?? [];
}

function saveRadios(list: TrainerRadio[]): void {
  settings.patch({ rcControl: { ...get(settings).rcControl, trainerRadios: list } });
}

/** BLE id of the radio while the trainer link holds it; it is not offered as a main link then. */
export function trainerRadioId(): string | null {
  const status = get(trainerStatus);
  return status && HOLDING.includes(status.state) ? status.device_id : null;
}

function clearRetry(): void {
  if (retryTimer) clearTimeout(retryTimer);
  retryTimer = null;
}

function onStatus(status: TrainerStatus): void {
  trainerStatus.set(status);
  const list = radios();
  const radio = list.find((r) => r.id === status.device_id);
  if (status.pairing && radio && !radio.pairing) {
    saveRadios(list.map((r) => (r.id === radio.id ? { ...r, pairing: true } : r)));
  }
  if (status.state === 'streaming') {
    trainerForeign.update((ids) => (ids.includes(status.device_id) ? ids.filter((d) => d !== status.device_id) : ids));
  }
  if (status.state !== 'foreign') return;
  trainerForeign.update((ids) => (ids.includes(status.device_id) ? ids : [...ids, status.device_id]));
  clearRetry();
  retryTimer = setTimeout(() => {
    retryTimer = null;
    // A restart looks at every remembered radio again; never interrupt a link that works.
    if (get(hidActive) && trainerRadioId() == null) void startTrainer();
  }, RETRY_FOREIGN_MS);
}

/** Look for the remembered radios (no-op without one). The backend retries until stopped. */
export async function startTrainer(): Promise<void> {
  const list = radios();
  if (!list.length) return;
  if (!listening) {
    listening = true;
    await listen<TrainerStatus>('trainer-status', (e) => onStatus(e.payload));
  }
  clearRetry();
  try {
    await invoke('trainer_start', { devices: list.map((r) => ({ id: r.id, pairing: r.pairing ?? false })) });
  } catch (e) {
    // Refused while the only radio serves the telemetry link; the handover from that link starts it.
    console.warn('[trainer] start refused:', e);
  }
}

export async function stopTrainer(): Promise<void> {
  clearRetry();
  await invoke('trainer_stop');
}

export async function rememberTrainer(radio: { id: string; name: string }): Promise<void> {
  const list = radios();
  if (!list.some((r) => r.id === radio.id)) saveRadios([...list, radio]);
  bleDevices.update((devs) => devs.filter((d) => d.id !== radio.id));
  // Without the HID input running, the next start of it connects.
  if (get(hidActive)) await startTrainer();
}

export async function forgetTrainer(id: string): Promise<void> {
  saveRadios(radios().filter((r) => r.id !== id));
  trainerForeign.update((ids) => ids.filter((d) => d !== id));
  await stopTrainer();
  trainerStatus.set(null);
  if (get(hidActive)) await startTrainer();
}
