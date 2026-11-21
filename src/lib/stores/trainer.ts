// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// FrSky Bluetooth trainer radio as an RC control input (Dev-Docs active/FRSKY_BT_TRAINER.md). The
// backend task runs with the HID input (stores/hid.ts) and adds the radio to the HID device list.

import { writable, get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { settings } from './settings';
import { bleDevices } from './connection';
import { hidActive } from './hid';

/** Matches the Rust `TrainerStatus`. */
export interface TrainerStatus {
  state: string;
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
}

export const trainerStatus = writable<TrainerStatus | null>(null);

let listening = false;
let retryTimer: ReturnType<typeof setTimeout> | null = null;
/** A radio that sent telemetry stays remembered; look again this often for trainer data. */
const RETRY_FOREIGN_MS = 15000;

const HOLDING = ['connecting', 'waiting', 'streaming', 'stalled'];

/** BLE id of the radio while the trainer link holds it; it is not offered as a main link then. */
export function trainerRadioId(): string | null {
  const status = get(trainerStatus);
  if (!status || !HOLDING.includes(status.state)) return null;
  return get(settings).rcControl.trainerRadio?.id ?? null;
}

function clearRetry(): void {
  if (retryTimer) clearTimeout(retryTimer);
  retryTimer = null;
}

function onStatus(status: TrainerStatus): void {
  trainerStatus.set(status);
  if (status.state !== 'foreign') return;
  clearRetry();
  retryTimer = setTimeout(() => {
    retryTimer = null;
    if (get(hidActive)) void startTrainer();
  }, RETRY_FOREIGN_MS);
}

/** Connect to the remembered radio (no-op without one). The backend retries until stopped. */
export async function startTrainer(): Promise<void> {
  const radio = get(settings).rcControl.trainerRadio;
  if (!radio) return;
  if (!listening) {
    listening = true;
    await listen<TrainerStatus>('trainer-status', (e) => onStatus(e.payload));
  }
  clearRetry();
  try {
    await invoke('trainer_start', { deviceId: radio.id });
  } catch (e) {
    // Refused while the radio serves the telemetry link; the handover from that link starts it.
    console.warn('[trainer] start refused:', e);
  }
}

export async function stopTrainer(): Promise<void> {
  clearRetry();
  await invoke('trainer_stop');
}

export async function rememberTrainer(radio: { id: string; name: string }): Promise<void> {
  settings.patch({ rcControl: { ...get(settings).rcControl, trainerRadio: radio } });
  bleDevices.update((list) => list.filter((d) => d.id !== radio.id));
  // Without the HID input running, the next start of it connects.
  if (get(hidActive)) await startTrainer();
}

export async function forgetTrainer(): Promise<void> {
  settings.patch({ rcControl: { ...get(settings).rcControl, trainerRadio: null } });
  await stopTrainer();
  trainerStatus.set(null);
}
