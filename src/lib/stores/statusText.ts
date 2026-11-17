// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// FC system messages (MAVLink STATUSTEXT, `mavlink-statustext`) as top-of-screen toasts: the last few, each
// expiring 20 s after it arrived, with a severity-tiered audio cue. Severity = MAV_SEVERITY (0 = emergency … 7 = debug).

import { writable, get } from 'svelte/store';
import { isActive, vehicles } from '$lib/stores/vehicles';
import { fleetEnabled } from '$lib/stores/fleetMode';
import { orderedVehicles, vehicleColorFor } from '$lib/helpers/fleetStatus';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { settings } from './settings';

export type StatusTextLevel = 'error' | 'warning' | 'info' | 'debug';

/** Which vehicle a line came from — set only while 2+ vehicles are connected (single-vehicle sessions
 *  keep the plain banner). Drives the colour chip + name prefix in the toast. */
export interface StatusTextVehicle {
  vehicleId: string;
  name: string;
  color: string;
}

export interface StatusTextMsg {
  id: number;
  severity: number;      // MAV_SEVERITY 0..7
  level: StatusTextLevel; // collapsed for colour/sound
  text: string;
  expiresAt: number;     // epoch ms when this line fades out (per-message lifetime)
  repeats: number;       // how many times this identical text has arrived while on screen (1 = once)
  vehicle?: StatusTextVehicle;
  /** A GCS notice that stays until its owner clears it (`setPinnedStatus`); `pinned` is the owner's key. */
  pinned?: string;
}

const MAX_BUFFER = 12;          // lines kept (the banner shows a few and scrolls to the newest)
const CLEAR_AFTER_MS = 20_000;  // each message fades out 20 s after it individually arrived
const SOUND_MIN_GAP_MS = 1200;  // don't let an INFO flood machine-gun the speaker

/** MAV_SEVERITY → display/sound level. ≤3 ERROR/CRITICAL/ALERT/EMERGENCY, 4 WARNING, 5–6 NOTICE/INFO,
 *  7 DEBUG. DEBUG is the firmware's developer chatter (ArduPilot: "Sending unknown message (44)" at the
 *  rate a GCS asked for a message the build cannot send) — it stays off the banner unless asked for. */
export function statusLevel(severity: number): StatusTextLevel {
  if (severity <= 3) return 'error';
  if (severity === 4) return 'warning';
  if (severity >= 7) return 'debug';
  return 'info';
}

/** Honour the "System Messages" setting (off / error / warning / all / debug). */
function levelAllowed(level: StatusTextLevel): boolean {
  switch (get(settings).systemMessages) {
    case 'off': return false;
    case 'error': return level === 'error';
    case 'warning': return level === 'error' || level === 'warning';
    case 'all': return level !== 'debug';
    default: return true; // 'debug' — everything, DEBUG included
  }
}

export const statusTexts = writable<StatusTextMsg[]>([]);

/** Recent ArduPilot/PX4 "PreArm: …" reasons (newline-joined) for the arming tooltip, independent of the toast
 *  filter. The red "not ready" STATE comes from SYS_STATUS PREARM_CHECK, not this. Lines accumulate during a
 *  burst; the set clears once the FC stops repeating them. */
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
let sweepTimer: ReturnType<typeof setTimeout> | null = null;
let lastSoundAt = 0;
let unlisten: UnlistenFn | null = null;

/** Drop every message whose per-line lifetime has elapsed, then re-arm for the next-earliest expiry.
 *  One timer scheduled at the soonest `expiresAt` covers the whole (small) buffer. */
function sweepExpired(): void {
  const now = Date.now();
  statusTexts.update((list) => {
    const kept = list.filter((m) => m.expiresAt > now);
    return kept.length === list.length ? list : kept;
  });
  scheduleSweep();
}

function scheduleSweep(): void {
  if (sweepTimer) { clearTimeout(sweepTimer); sweepTimer = null; }
  const list = get(statusTexts).filter((m) => !m.pinned);
  if (!list.length) return;
  const soonest = Math.min(...list.map((m) => m.expiresAt));
  sweepTimer = setTimeout(sweepExpired, Math.max(0, soonest - Date.now()));
}

/** Show one line; false when it was not shown (empty, or filtered out by the "System Messages" setting). */
function push(severity: number, text: string, vehicle?: StatusTextVehicle): boolean {
  const clean = text.trim();
  if (!clean) return false;
  const level = statusLevel(severity);
  if (!levelAllowed(level)) return false; // filtered out by the "System Messages" setting

  const expiresAt = Date.now() + CLEAR_AFTER_MS;
  let repeated = false;
  statusTexts.update((list) => {
    // De-dup against the WHOLE buffer: an FC nagging with two alternating messages never repeats its last line.
    // A repeat refreshes the existing line's lifetime and counter in place (no reshuffle). Keyed by vehicle too,
    // so the same text from two craft is two lines.
    const idx = list.findIndex((m) => m.text === clean && (m.vehicle?.vehicleId ?? null) === (vehicle?.vehicleId ?? null));
    if (idx !== -1) {
      repeated = true;
      const refreshed = [...list];
      refreshed[idx] = { ...refreshed[idx], severity, level, expiresAt, repeats: refreshed[idx].repeats + 1, vehicle: vehicle ?? refreshed[idx].vehicle };
      return refreshed;
    }
    const pinned = list.filter((m) => m.pinned);
    const rest = [...list.filter((m) => !m.pinned), { id: nextId++, severity, level, text: clean, expiresAt, repeats: 1, vehicle }];
    return [...pinned, ...rest.slice(-(MAX_BUFFER - pinned.length))];
  });
  scheduleSweep();

  // Always cue errors/warnings; an INFO line already on screen never re-cues, so a nag loop stays
  // silent. DEBUG lines are never cued — they are on screen because someone is reading, not listening.
  if (level !== 'debug' && (!repeated || level !== 'info')) playTone(level);
  return true;
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

/** Inject a GCS-local message into the same toast banner (e.g. a geozone/geofence breach the GCS detects
 *  itself). Goes through the same severity filter + audio cue as FC STATUSTEXT. Returns whether it was
 *  shown (false: filtered out by the "System Messages" setting). */
export function pushLocalStatus(severity: number, text: string): boolean {
  return push(severity, text);
}

/** Show or update the GCS notice `key`: a red line above the messages that never expires and ignores the
 *  "System Messages" filter (it is not an FC message). No sound — see `playAlarm`. */
export function setPinnedStatus(key: string, text: string): void {
  statusTexts.update((list) => {
    const idx = list.findIndex((m) => m.pinned === key);
    if (idx === -1) return [{ id: nextId++, severity: 2, level: 'error', text, expiresAt: Infinity, repeats: 1, pinned: key }, ...list];
    if (list[idx].text === text) return list;
    const next = [...list];
    next[idx] = { ...next[idx], text };
    return next;
  });
}

export function clearPinnedStatus(key: string): void {
  statusTexts.update((list) => (list.some((m) => m.pinned === key) ? list.filter((m) => m.pinned !== key) : list));
}

/** The loudest cue: three falling notes, for a pinned notice. */
export function playAlarm(): void {
  lastSoundAt = Date.now();
  beep(880, 0, 160, 0.22);
  beep(660, 200, 160, 0.22);
  beep(440, 400, 320, 0.22);
}

/** Start listening for FC STATUSTEXT messages. Safe to call once on app init. */
export async function startStatusText(): Promise<void> {
  if (unlisten) return;
  unlisten = await listen<{ vehicleId?: string; severity: number; text: string }>('mavlink-statustext', (e) => {
    const active = isActive(e.payload);
    const known = get(vehicles);
    // Per-vehicle lines and tags are a fleet feature (behind the `--fleet` gate).
    const multi = get(fleetEnabled) && known.size > 1;
    // Single vehicle: the plain banner, active only (unchanged). Fleet: every vehicle's lines, each
    // tagged with its name + colour so a failing craft is identifiable at a glance.
    if (!multi && !active) return;
    if (active) trackPrearm(e.payload.text); // unfiltered — drives the arming indicator regardless of toast settings
    let vehicle: StatusTextVehicle | undefined;
    const vid = e.payload.vehicleId;
    if (multi && typeof vid === 'string') {
      const v = known.get(vid);
      if (v) vehicle = { vehicleId: vid, name: v.name, color: vehicleColorFor(orderedVehicles(known), vid) };
    }
    push(e.payload.severity, e.payload.text, vehicle);
  });
}

export function stopStatusText(): void {
  unlisten?.();
  unlisten = null;
  if (sweepTimer) { clearTimeout(sweepTimer); sweepTimer = null; }
  if (prearmTimer) { clearTimeout(prearmTimer); prearmTimer = null; }
  prearmLines = [];
  statusTexts.set([]);
  prearmReason.set(null);
}
