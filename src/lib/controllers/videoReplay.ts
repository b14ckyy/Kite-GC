// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// A replayed flight's recordings play in the video surfaces, in step with the replay. The backend follows
// the flight time it is told (video/record/replay.rs); this side tells it and owns the hand-over between
// the live stream and the recording: loading a flight with a recording ends the live stream.

import { get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { flightVideos } from '$lib/stores/flightlog';
import { enterVideoReplay, leaveVideoReplay, setVideoReplayGap, stopVideo, videoState } from '$lib/stores/video';
import { hasVideoRecording } from '$lib/platform';

export interface VideoReplayPosition {
  /** The flight shown on the map, or null when none is (no replay, or a vehicle is connected). */
  flightId: number | null;
  dbPath: string;
  /** Replay time in ms from the flight's start. */
  flightMs: number;
  playing: boolean;
  rate: number;
}

/** How often a running replay re-reports its time; a change of state or a jump goes out at once. */
const REPORT_MS = 250;

let openFlight: number | null = null;
let wanted: number | null = null;
let last: VideoReplayPosition | null = null;
let lastSent = 0;
let lastKey = '';
// Open / close run one after the other: a quick change of flight must not close the newer one.
let queue: Promise<void> = Promise.resolve();

async function open(flightId: number, dbPath: string): Promise<void> {
  try {
    const videos = await flightVideos(flightId, dbPath);
    if (wanted !== flightId || !videos.some((v) => v.exists)) return;
    // The recording takes the decode sink: the live stream ends, and stays off.
    if (get(videoState).enabled) stopVideo();
    const info = await invoke<{ codec: string; width: number; height: number }>('video_replay_open', {
      flightId,
      dbPath: dbPath || null,
    });
    openFlight = flightId;
    inGap = false;
    enterVideoReplay(info.codec, info.width, info.height);
    lastKey = '';
    if (last) send(last);
  } catch (e) {
    console.warn('[video] replay of the recording failed', e);
  }
}

async function close(): Promise<void> {
  if (openFlight == null) return;
  openFlight = null;
  // Stopped by hand meanwhile (Stop video): the store has left replay already.
  if (!get(videoState).replay) return;
  leaveVideoReplay();
  await invoke('video_rtsp_native_stop').catch(() => {});
}

/** Leaving a gap, the cover stays until the new picture is there: below it the sink still holds the last
 *  picture from before the gap. Asked again in this rhythm, for at most this long. */
const READY_POLL_MS = 100;
const READY_WAIT_MS = 3000;
const DECODE_MARGIN_MS = 200;
let inGap = false;
let readyTimer: ReturnType<typeof setTimeout> | undefined;

function report(p: VideoReplayPosition, waitedMs: number): void {
  clearTimeout(readyTimer);
  void invoke<{ covered: boolean; ready: boolean } | null>('video_replay_sync', {
    flightMs: p.flightMs,
    playing: p.playing,
    rate: p.rate,
  })
    .then((r) => {
      if (r == null) return;
      if (!r.covered) {
        inGap = true;
      } else if (inGap && !r.ready && waitedMs < READY_WAIT_MS) {
        readyTimer = setTimeout(() => report(last ?? p, waitedMs + READY_POLL_MS), READY_POLL_MS);
        return;
      } else if (inGap) {
        // "Ready" = handed to the decoder, which still has the run-up from the keyframe to work off.
        inGap = false;
        readyTimer = setTimeout(() => setVideoReplayGap(false), DECODE_MARGIN_MS);
        return;
      }
      setVideoReplayGap(inGap);
    })
    .catch(() => {});
}

function send(p: VideoReplayPosition): void {
  const key = `${p.playing}|${p.rate}`;
  const now = performance.now();
  const jumped = last != null && Math.abs(p.flightMs - last.flightMs) > 1000;
  if (p.playing && key === lastKey && !jumped && now - lastSent < REPORT_MS) return;
  lastKey = key;
  lastSent = now;
  report(p, 0);
}

/** Call with every change of the replay's flight, time, run state or speed. */
export function syncVideoReplay(p: VideoReplayPosition): void {
  if (!hasVideoRecording) return;
  if (p.flightId !== wanted) {
    wanted = p.flightId;
    const { flightId, dbPath } = p;
    queue = queue.then(close).then(() => (flightId != null ? open(flightId, dbPath) : undefined));
  }
  if (openFlight != null && openFlight === p.flightId) send(p);
  last = p;
}
