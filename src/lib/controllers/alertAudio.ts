// Conflict-alert audio (Plan C / C1). Plays a synthesised tone + a short spoken callout when the worst
// alert level escalates, re-announces a sustained warning, and re-arms once clear. Standard aviation
// phraseology ("Traffic" / "Collision") is kept in English regardless of UI locale.
//
// Gated by `settings.radar.alerts.audioEnabled` (+ the radar master switch). Tones use Web Audio; the
// callout uses the Web Speech API (graceful no-op if unavailable).

import { get } from 'svelte/store';
import { t } from 'svelte-i18n';
import { radarAlerts, type AlertLevel } from '$lib/controllers/radarAlerts';
import { settings } from '$lib/stores/settings';

const WARNING_REPEAT_MS = 8000; // re-announce a sustained collision warning this often

let audioCtx: AudioContext | null = null;
function ctx(): AudioContext | null {
  try {
    audioCtx ??= new AudioContext();
    if (audioCtx.state === 'suspended') void audioCtx.resume();
    return audioCtx;
  } catch {
    return null; // no Web Audio → tones are skipped (callout may still work)
  }
}

/** One beep at `freq` Hz starting `whenMs` from now. */
function beep(freq: number, startMs: number, durMs: number, gainVal: number) {
  const ac = ctx();
  if (!ac) return;
  const t0 = ac.currentTime + startMs / 1000;
  const osc = ac.createOscillator();
  const gain = ac.createGain();
  osc.type = 'square';
  osc.frequency.value = freq;
  // Quick attack + decay to avoid clicks.
  gain.gain.setValueAtTime(0, t0);
  gain.gain.linearRampToValueAtTime(gainVal, t0 + 0.01);
  gain.gain.setValueAtTime(gainVal, t0 + durMs / 1000 - 0.02);
  gain.gain.linearRampToValueAtTime(0, t0 + durMs / 1000);
  osc.connect(gain).connect(ac.destination);
  osc.start(t0);
  osc.stop(t0 + durMs / 1000 + 0.02);
}

function playTone(level: AlertLevel) {
  if (level === 'caution') {
    // Gentle two-note chime.
    beep(660, 0, 140, 0.18);
    beep(880, 0.18 * 1000, 160, 0.18);
  } else {
    // Urgent rising triple beep, louder.
    beep(1100, 0, 110, 0.30);
    beep(1100, 150, 110, 0.30);
    beep(1320, 300, 160, 0.32);
  }
}

// Spoken callout: localised to the UI language when a matching voice is installed, else English
// aviation phraseology (a wrong-language default voice would be worse than the standard term).
const LOCALE_TAG: Record<string, string> = { en: 'en-US', de: 'de-DE', fr: 'fr-FR' };
const VOICE_KEY: Record<AlertLevel, string> = {
  caution: 'radar.alert.voiceCaution',
  warning: 'radar.alert.voiceWarning',
};
const EN_FALLBACK: Record<AlertLevel, string> = { caution: 'Traffic', warning: 'Collision' };

function pickVoice(lang: string): SpeechSynthesisVoice | null {
  try {
    const voices = window.speechSynthesis.getVoices();
    if (!voices.length) return null;
    const base = lang.split('-')[0].toLowerCase();
    return (
      voices.find((v) => v.lang.replace('_', '-') === lang) ??
      voices.find((v) => v.lang.replace('_', '-').toLowerCase().startsWith(base)) ??
      null
    );
  } catch {
    return null;
  }
}

function speak(level: AlertLevel) {
  try {
    const synth = window.speechSynthesis;
    if (!synth) return;
    const loc = get(settings).locale;
    const tag = LOCALE_TAG[loc] ?? 'en-US';
    let voice = pickVoice(tag);
    let text: string;
    let lang: string;
    if (voice || loc === 'en') {
      // UI language: a matching voice exists (or it's English anyway).
      text = get(t)(VOICE_KEY[level]);
      lang = tag;
    } else {
      // No installed voice for this locale → English phraseology + an English voice.
      text = EN_FALLBACK[level];
      lang = 'en-US';
      voice = pickVoice('en-US');
    }
    synth.cancel(); // drop any queued callout so the latest wins
    const u = new SpeechSynthesisUtterance(text);
    u.lang = lang;
    if (voice) u.voice = voice;
    u.rate = 1.05;
    u.volume = 1;
    synth.speak(u);
  } catch {
    // no speech synthesis (e.g. WebKitGTK without voices) → tone-only
  }
}

function cue(level: AlertLevel) {
  const a = get(settings).radar.alerts;
  if (a.soundEnabled) playTone(level);
  if (a.voiceEnabled) {
    // Let the tone lead (if any), then the spoken word.
    const delay = a.soundEnabled ? (level === 'warning' ? 520 : 320) : 0;
    setTimeout(() => speak(level), delay);
  }
}

/** Last announced level per contact — detects a NEW contact entering a zone (Stage 1 chimes once on
 *  entry, never in a loop; a contact that leaves and returns chimes again). */
const prevLevels = new Map<string, AlertLevel>();
let repeatTimer: ReturnType<typeof setInterval> | null = null;
let unsub: (() => void) | null = null;

function clearRepeat() {
  if (repeatTimer) { clearInterval(repeatTimer); repeatTimer = null; }
}

export function startAlertAudio() {
  stopAlertAudio();
  // Kick off voice-list population (async on some engines) so the first callout can pick a voice.
  try { window.speechSynthesis?.getVoices(); } catch { /* no speech synthesis */ }
  unsub = radarAlerts.subscribe((alerts) => {
    const anyWarning = alerts.some((a) => a.level === 'warning');
    let newWarning = false;
    let newCaution = false;
    const seen = new Set<string>();
    for (const a of alerts) {
      seen.add(a.vehicleId);
      const prev = prevLevels.get(a.vehicleId) ?? null;
      if (a.level === 'warning') {
        if (prev !== 'warning') newWarning = true; // entered or escalated to a collision warning
      } else if (a.level === 'caution') {
        if (prev == null) newCaution = true; // newly entered the caution zone (not a de-escalation)
      }
      prevLevels.set(a.vehicleId, a.level);
    }
    // Forget contacts that left, so a later re-entry chimes again.
    for (const id of prevLevels.keys()) if (!seen.has(id)) prevLevels.delete(id);

    // A new collision warning always cues. Stage-1 cues only on entry AND only when no Stage-2 alert
    // is active (a collision warning overrides caution chatter).
    if (newWarning) cue('warning');
    else if (newCaution && !anyWarning) cue('caution');

    // Re-announce a sustained collision warning; Stage 1 never loops.
    if (anyWarning) {
      if (!repeatTimer) repeatTimer = setInterval(() => cue('warning'), WARNING_REPEAT_MS);
    } else {
      clearRepeat();
    }
  });
}

export function stopAlertAudio() {
  unsub?.(); unsub = null;
  clearRepeat();
  prevLevels.clear();
  try { window.speechSynthesis?.cancel(); } catch { /* ignore */ }
}
