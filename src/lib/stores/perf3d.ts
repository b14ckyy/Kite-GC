// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Debug bridge between Map3D and the Debug Panel's Performance tab: lets the panel live-tune the
// running CesiumJS scene (fog / LOD / sky / MSAA …) and read its frame rate, to localise the
// Linux/WebKitGTK 3D performance bottleneck empirically. Map3D attaches/detaches this gated on the
// RUNTIME debug flag (dev builds, or a release started with `--debug`), so it ships in release but
// stays dormant otherwise. The Viewer object itself is not reactive (read on demand) — only the fps
// readout is a reactive store.

import { writable } from 'svelte/store';
import type { Viewer } from 'cesium'; // type-only — erased at build, no runtime/bundle cost

let viewer: Viewer | null = null;
let removeFpsListener: (() => void) | null = null;

/** Smoothed frames-per-second of the 3D view (sampled twice a second). 0 when 3D is inactive. */
export const perf3dFps = writable<number>(0);

/** Dev: when true, Map3D forces continuous rendering (disables requestRenderMode) so the fps
 *  overlay keeps ticking while the map is idle. Map3D owns requestRenderMode (it toggles it for
 *  alert/WP pulses), so this flag is folded into its own logic rather than set from the panel. */
export const perf3dForceContinuous = writable<boolean>(false);

/** True while a live 3D viewer is published. Reactive so the Performance tab can auto-(re)load when
 *  the 3D view mounts AFTER the tab was opened — otherwise a one-shot load on tab-open would stay
 *  stuck on "3D not loaded" even once 3D is active. */
export const perf3dAttached = writable<boolean>(false);

/** Map3D publishes its Cesium viewer here (debug only) and starts an fps sampler. */
export function attachPerf3d(cesiumViewer: Viewer): void {
  detachPerf3d();
  viewer = cesiumViewer;
  perf3dAttached.set(true);
  let frames = 0;
  let last = performance.now();
  const onPost = () => {
    frames++;
    const now = performance.now();
    if (now - last >= 500) {
      perf3dFps.set(Math.round(frames / ((now - last) / 1000)));
      frames = 0;
      last = now;
    }
  };
  // postRender only fires when Cesium actually renders (requestRenderMode), so the readout reflects
  // the real interaction frame rate; it simply holds its last value while the view sits idle.
  viewer.scene.postRender.addEventListener(onPost);
  removeFpsListener = () => viewer?.scene.postRender.removeEventListener(onPost);
}

/** Map3D calls this on teardown. */
export function detachPerf3d(): void {
  removeFpsListener?.();
  removeFpsListener = null;
  viewer = null;
  perf3dFps.set(0);
  perf3dAttached.set(false);
}

/** The Performance tab reads the live viewer/scene to mutate it. Null when 3D isn't active. */
export function getPerf3dViewer(): Viewer | null {
  return viewer;
}
