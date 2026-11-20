// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Recording of the native RTSP stream: runs from arming to disarming of the connected vehicle while the
// option is on. The backend writes the file and attaches it to the flight once that is saved.

import { get, writable } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { connection } from '$lib/stores/connection';
import { settings } from '$lib/stores/settings';
import { videoState } from '$lib/stores/video';
import { isArmed } from '$lib/controllers/vehicleControl';
import { hasVideoRecording } from '$lib/platform';

export interface VideoRecordingStatus {
  recording: boolean;
  /** The file being written, once the first keyframe opened it. */
  file: string | null;
  bytes: number;
  dropped: number;
  error: string | null;
}

const IDLE: VideoRecordingStatus = { recording: false, file: null, bytes: 0, dropped: 0, error: null };
const STATUS_MS = 2000;

export const videoRecording = writable<VideoRecordingStatus>({ ...IDLE });
/** `<Videos>/KiteGC` on this machine, for the panel's placeholder. */
export const videoRecordDefaultDir = writable('');

let started = false;
let enabled = false;
let recording = false;
let armed = false;
let statusTimer: ReturnType<typeof setInterval> | undefined;

/** Only the native client hands out the stream as it arrives; the engine path has nothing to record. */
function optionOn(): boolean {
  const v = get(videoState);
  return v.recordEnabled && v.kind === 'rtsp' && v.rtspNativeClient;
}

async function refreshStatus(): Promise<void> {
  try {
    videoRecording.set(await invoke<VideoRecordingStatus>('video_record_status'));
  } catch {
    /* the next tick tries again */
  }
}

async function startRecording(): Promise<void> {
  try {
    const vehicle = get(connection).fcInfo?.craft_name || null;
    await invoke('video_record_start', { dir: get(videoState).recordDir || null, vehicle });
    statusTimer ??= setInterval(() => void refreshStatus(), STATUS_MS);
    await refreshStatus();
  } catch (e) {
    recording = false;
    videoRecording.set({ ...IDLE, error: String(e) });
  }
}

async function stopRecording(): Promise<void> {
  clearInterval(statusTimer);
  statusTimer = undefined;
  try {
    await invoke('video_record_stop', { dbPath: get(settings).flightLogDbPath || null });
  } catch (e) {
    console.warn('[video] stopping the recording failed', e);
  }
  await refreshStatus();
}

// Start and stop are async commands on a thread pool: one after the other, or a quick disarm could reach
// the backend before its start.
let queue: Promise<void> = Promise.resolve();

function evaluate(): void {
  const on = optionOn();
  if (on !== enabled) {
    enabled = on;
    void invoke('video_record_enable', { on }).catch(() => {});
  }
  const should = on && armed && get(connection).status === 'connected';
  if (should === recording) return;
  recording = should;
  queue = queue.then(should ? startRecording : stopRecording);
}

export function startVideoRecording(): void {
  if (started || !hasVideoRecording) return;
  started = true;
  void invoke<string>('video_record_default_dir').then((d) => videoRecordDefaultDir.set(d)).catch(() => {});
  videoState.subscribe(evaluate);
  connection.subscribe(evaluate);
  isArmed.subscribe((a) => {
    armed = a;
    evaluate();
  });
}
