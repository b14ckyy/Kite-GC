// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Detached video window — the MAIN window's half (VIDEO_MULTISINK_WINDOW.md PR B).
//
// The picture leaves the app into its own transparent, always-on-top OS window: on a multi-monitor
// GCS the second screen becomes the video screen and is populated automatically at start (D11). The
// window itself is a second WebView running the `/video` route, with its own surface router — it
// publishes ONE hole under the label `video`, which the sink hub keys separately from the main
// window's (see controllers/nativeVideo.ts and video/surface.rs).
//
// This module owns the *decision*: `videoState.undocked` is the wish, and the reconciler below
// creates or destroys the window to match it. Nothing else calls the open/close commands.
//
//   undocked && enabled && (nativeSink || reconnecting)  →  the window exists
//   anything else                                        →  it does not, and `undocked` is left
//                                                            alone, so a stopped source detaches
//                                                            again on the next start
//
// A dropped link counts as "still wanted": the RTSP reconnect loop clears `nativeSink` on every
// attempt, and closing and re-creating a whole WebView window on each of them would make the picture
// flash off the second monitor for seconds. It keeps standing and shows the reconnect notice.
//
// Two JS contexts cannot share stores, so the little the viewer needs is pushed over Tauri events:
//   main  → video : `video-detached-state`     { status, error, aspect, nativeSink, reconnect }
//   video → main  : `video-detached-ready`     (mounted — send me the state)
//                   `video-detached-geometry`  the box to remember (physical px)
//                   `video-detached-dock`      the viewer's dock button: take the picture back. The
//                                              window is still ALIVE here — the reconciler closes it,
//                                              which is what stops its video before the widgets die.
//                   `video-detached-closed`    emitted by the BACKEND once the window is really gone
//                                              (Alt+F4, or our own close).

import { get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { emitTo, listen, type UnlistenFn } from '@tauri-apps/api/event';
import { availableMonitors, currentMonitor, type Monitor } from '@tauri-apps/api/window';
import {
  videoState,
  setUndocked,
  setDetachBox,
  setMapLocation,
  type DetachBox,
  type VideoState,
} from '$lib/stores/video';

/** Window label — must match `commands::video::DETACHED_LABEL`. */
const LABEL = 'video';

/** Default picture height (logical px) for a window that has no usable saved box (D13). */
const DEFAULT_H = 360;

/** What the viewer window renders from — everything else (telemetry, map, settings) stays here. */
export interface DetachedState {
  status: VideoState['status'];
  error: string | null;
  aspect: number;
  nativeSink: boolean;
  reconnecting: boolean;
  reconnectAttempt: number;
  /** A replayed flight has no recording for the current time. */
  replayGap: boolean;
}

let unlisteners: UnlistenFn[] = [];
let unsubscribe: (() => void) | null = null;
/** Our idea of whether the window exists — the backend's destroy event keeps it honest. */
let windowOpen = false;
/** An open/close command is in flight; the reconciler waits rather than firing a second one. */
let busy = false;
/** WE are closing it (source stopped, sink gone): the destroy event must not clear `undocked`. */
let closingByUs = false;
let lastPushed = '';

/** Start reconciling the detached window against `videoState`. Main window only; idempotent. */
export function startDetachedVideo(): void {
  if (unsubscribe) return;
  void listen(`video-detached-closed`, onClosed).then((u) => unlisteners.push(u));
  void listen('video-detached-dock', onDockRequest).then((u) => unlisteners.push(u));
  void listen('video-detached-ready', () => pushState(get(videoState), true)).then((u) => unlisteners.push(u));
  void listen<DetachBox>('video-detached-geometry', (e) => setDetachBox(e.payload)).then((u) => unlisteners.push(u));
  unsubscribe = videoState.subscribe(reconcile);
}

/** Stop reconciling (app teardown). The window itself dies with the app. */
export function stopDetachedVideo(): void {
  unsubscribe?.();
  unsubscribe = null;
  for (const u of unlisteners) u();
  unlisteners = [];
}

/** The unplug button: take the picture out of the app. The map cannot come along (D7), so a map
 *  parked in the floating frame goes back to full screen first. */
export function detachVideo(): void {
  if (get(videoState).mapLocation === 'floating') setMapLocation('main');
  setUndocked(true);
}

function reconcile(s: VideoState): void {
  const want = s.undocked && s.enabled && (s.nativeSink || s.reconnecting);
  if (busy) return;
  if (want && !windowOpen) void openWindow(s);
  else if (!want && windowOpen) void closeWindow();
  else if (windowOpen) pushState(s);
}

async function openWindow(s: VideoState): Promise<void> {
  busy = true;
  try {
    const box = await resolveBox(s);
    await invoke('video_detached_open', {
      x: box.x,
      y: box.y,
      w: box.w,
      h: box.h,
      fullscreen: box.fullscreen,
    });
    windowOpen = true;
    setDetachBox(box);
    pushState(get(videoState), true);
  } catch (e) {
    console.warn('[video] detach failed:', e);
    setUndocked(false); // no window, no pretending — the frame comes back into the app
  } finally {
    busy = false;
    // The wish may have changed while the window was being built (Stop pressed mid-flight): those
    // store updates found `busy` set and returned, so settle it here.
    reconcile(get(videoState));
  }
}

async function closeWindow(): Promise<void> {
  busy = true;
  closingByUs = true;
  try {
    await invoke('video_detached_close');
  } catch (e) {
    console.warn('[video] closing the detached window failed:', e);
  } finally {
    windowOpen = false;
    busy = false;
    // `onClosed` settles the rest once the destroy event lands — including a re-open if the wish
    // came back meanwhile. This backstop only covers an event that never arrives: the flag must not
    // poison the next USER close, which is the one that docks the picture back in.
    setTimeout(() => {
      if (!closingByUs) return;
      closingByUs = false;
      reconcile(get(videoState));
    }, 2000);
  }
}

/** The viewer's dock button. The window is still there, so `windowOpen` stays true and the
 *  reconciler does the closing — in the right order, which is the whole point (the window's own
 *  video has to stop before GTK takes its widgets down). */
function onDockRequest(): void {
  setUndocked(false);
  reconcile(get(videoState));
}

/** The window is gone. Ours → keep the wish (a restarted source detaches again); the user's → dock
 *  the picture back into the app (D6). */
function onClosed(): void {
  windowOpen = false;
  lastPushed = '';
  if (closingByUs) closingByUs = false;
  else setUndocked(false);
  reconcile(get(videoState));
}

/** Where to open it: the saved box while a monitor still carries it, otherwise the default size
 *  centred on the monitor the main window is on (D13). Physical px throughout — monitor geometry is
 *  reported that way, and the backend places the window with it. */
async function resolveBox(s: VideoState): Promise<DetachBox> {
  const monitors = await availableMonitors().catch(() => [] as Monitor[]);
  const saved = s.detachBox;
  if (saved && monitors.some((m) => holdsCentre(m, saved))) return saved;
  const home = (await currentMonitor().catch(() => null)) ?? monitors[0] ?? null;
  const scale = home?.scaleFactor ?? 1;
  const h = Math.round(DEFAULT_H * scale);
  const w = Math.round(h * (s.aspect || 16 / 9));
  const mx = home?.position.x ?? 0;
  const my = home?.position.y ?? 0;
  const mw = home?.size.width ?? Math.round(1280 * scale);
  const mh = home?.size.height ?? Math.round(800 * scale);
  return {
    x: Math.round(mx + (mw - w) / 2),
    y: Math.round(my + (mh - h) / 2),
    w,
    h,
    fullscreen: false,
  };
}

/** A box belongs to a monitor when its CENTRE is on it — a window straddling two screens is still
 *  reachable, one whose screen is gone is not. */
function holdsCentre(m: Monitor, b: DetachBox): boolean {
  const cx = b.x + b.w / 2;
  const cy = b.y + b.h / 2;
  return (
    cx >= m.position.x &&
    cx < m.position.x + m.size.width &&
    cy >= m.position.y &&
    cy < m.position.y + m.size.height
  );
}

/** Push the viewer's state, but only what changed (the store patches on every stats tick). */
function pushState(s: VideoState, force = false): void {
  const payload: DetachedState = {
    status: s.status,
    error: s.error,
    aspect: s.aspect,
    nativeSink: s.nativeSink,
    reconnecting: s.reconnecting,
    reconnectAttempt: s.reconnectAttempt,
    replayGap: s.replay && s.replayGap,
  };
  const key = JSON.stringify(payload);
  if (!force && key === lastPushed) return;
  lastPushed = key;
  void emitTo(LABEL, 'video-detached-state', payload).catch(() => {});
}
