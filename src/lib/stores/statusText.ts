// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// FC system messages (MAVLink STATUSTEXT) shown as top-of-screen toasts. The backend emits
// `mavlink-statustext` ({ severity, text }); we keep the most recent few, auto-clear them 60 s after
// the last one arrived, and play a severity-tiered audio cue. Severity is MAV_SEVERITY (0 = emergency
// … 7 = debug).

import { writable, get } from 'svelte/store';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { settings } from './settings';

export type StatusTextLevel = 'error' | 'warning' | 'info';

export interface StatusTextMsg {
  id: number;
  severity: number;      // MAV_SEVERITY 0..7
  level: StatusTextLevel; // collapsed for colour/sound
  text: string;
}

const MAX_BUFFER = 12;          // lines kept (the banner shows a few and scrolls to the newest)
const CLEAR_AFTER_MS = 60_000;  // fade everything out 60 s after the last message
const SOUND_MIN_GAP_MS = 1200;  // don't let an INFO flood machine-gun the speaker

/** MAV_SEVERITY → display/sound level. ≤3 ERROR/CRITICAL/ALERT/EMERGENCY, 4 WARNING, ≥5 NOTICE/INFO/DEBUG. */
export function statusLevel(severity: number): StatusTextLevel {
  if (severity <= 3) return 'error';
  if (severity === 4) return 'warning';
  return 'info';
}

/** Honour the "System Messages" setting (off / error / warning / all). */
function levelAllowed(level: StatusTextLevel): boolean {
  switch (get(settings).systemMessages) {
    case 'off': return false;
    case 'error': return level === 'error';
    case 'warning': return level !== 'info';
    default: return true; // 'all'
  }
}

export const statusTexts = writable<StatusTextMsg[]>([]);

/** Recent ArduPilot/PX4 "PreArm: …" failure reasons (newline-joined), for the arming indicator tooltip.
 *  Tracked independently of the toast `systemMessages` filter (so the detail is available even with
 *  toasts off). The red "not ready" STATE itself comes from the SYS_STATUS PREARM_CHECK bit, not this —
 *  this only supplies the human-readable reasons. Distinct lines accumulate during a check burst and the
 *  whole set clears once the FC stops repeating them (block cleared / armed). */
export const prearmReason = writable<string | null>(null);
// ArduPilot phrases arming blockers as "PreArm: …"; PX4 uses "Preflight Fail: …" / "Arming denied: …".
// (PX4 matching is best-effort / untested — the SYS_STATUS PREARM_CHECK bit is the other fallback.)
const PREARM_PREFIX = /^(pre[\s-]?arm|preflight fail|arming denied)[:\s]+/i;
const PREARM_CLEAR_MS = 40_000; // wide enough to bridge a repeating prearm burst without green flicker
                                // between nags; exact ArduPilot cadence is uncertain (see arming notes)
let prearmTimer: ReturnType<typeof setTimeout> | null = null;
let prearmLines: string[] = [];

function trackPrearm(text: string): void {
  const clean = text.trim();
  if (!PREARM_PREFIX.test(clean)) return;
  const reason = clean.replace(PREARM_PREFIX, '').trim() || clean;
  if (!prearmLines.includes(reason)) prearmLines = [...prearmLines, reason].slice(-8);
  prearmReason.set(prearmLines.join('\n'));
  if (prearmTimer) clearTimeout(prearmTimer);
  prearmTimer = setTimeout(() => { prearmLines = []; prearmReason.set(null); }, PREARM_CLEAR_MS);
}

let nextId = 1;
let clearTimer: ReturnType<typeof setTimeout> | null = null;
let lastSoundAt = 0;
let lastText = '';
let unlisten: UnlistenFn | null = null;

function push(severity: number, text: string): void {
  const clean = text.trim();
  if (!clean) return;
  const level = statusLevel(severity);
  if (!levelAllowed(level)) return; // filtered out by the "System Messages" setting

  statusTexts.update((list) => {
    // Light de-dup: a repeated identical line just refreshes the timer, no duplicate row.
    if (list.length && list[list.length - 1].text === clean) return list;
    return [...list, { id: nextId++, severity, level, text: clean }].slice(-MAX_BUFFER);
  });

  if (clearTimer) clearTimeout(clearTimer);
  clearTimer = setTimeout(() => statusTexts.set([]), CLEAR_AFTER_MS);

  if (clean !== lastText || level !== 'info') playTone(level); // always cue errors; rate-limit info
  lastText = clean;
}

// ── Audio cue (Web Audio) — gentle for info, discreetly alarming for warnings/errors ──

let audioCtx: AudioContext | null = null;
function ctx(): AudioContext | null {
  try {
    audioCtx ??= new AudioContext();
    if (audioCtx.state === 'suspended') void audioCtx.resume();
    return audioCtx;
  } catch {
    return null;
  }
}

function beep(freq: number, startMs: number, durMs: number, gainVal: number): void {
  const ac = ctx();
  if (!ac) return;
  const t0 = ac.currentTime + startMs / 1000;
  const osc = ac.createOscillator();
  const gain = ac.createGain();
  osc.type = 'sine';
  osc.frequency.value = freq;
  gain.gain.setValueAtTime(0, t0);
  gain.gain.linearRampToValueAtTime(gainVal, t0 + 0.012);
  gain.gain.setValueAtTime(gainVal, t0 + durMs / 1000 - 0.03);
  gain.gain.linearRampToValueAtTime(0, t0 + durMs / 1000);
  osc.connect(gain).connect(ac.destination);
  osc.start(t0);
  osc.stop(t0 + durMs / 1000 + 0.03);
}

function playTone(level: StatusTextLevel): void {
  const now = Date.now();
  if (now - lastSoundAt < SOUND_MIN_GAP_MS) return; // throttle bursts
  lastSoundAt = now;
  if (level === 'info') {
    beep(620, 0, 120, 0.06); // soft single note
  } else if (level === 'warning') {
    beep(720, 0, 120, 0.12);
    beep(560, 150, 150, 0.12); // gentle two-note fall
  } else {
    beep(440, 0, 150, 0.16);
    beep(440, 200, 200, 0.16); // discreet double low tone
  }
}

/** Start listening for FC STATUSTEXT messages. Safe to call once on app init. */
export async function startStatusText(): Promise<void> {
  if (unlisten) return;
  unlisten = await listen<{ severity: number; text: string }>('mavlink-statustext', (e) => {
    trackPrearm(e.payload.text); // unfiltered — drives the arming indicator regardless of toast settings
    push(e.payload.severity, e.payload.text);
  });
}

export function stopStatusText(): void {
  unlisten?.();
  unlisten = null;
  if (clearTimer) { clearTimeout(clearTimer); clearTimer = null; }
  if (prearmTimer) { clearTimeout(prearmTimer); prearmTimer = null; }
  prearmLines = [];
  statusTexts.set([]);
  prearmReason.set(null);
  lastText = '';
}
