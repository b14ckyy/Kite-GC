// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

import type { TelemetryRecord } from '$lib/stores/flightlog';

const TICK_MS = 100;
// Sub-1× slow-motion exists for the hi-res replay (HIRES_REPLAY plan): at 0.25× the instruments
// still get fluid updates as long as the source log carries enough rate.
const SPEEDS = [0.25, 0.5, 1, 2, 4, 10] as const;

/** Extra playback options (hi-res replay). */
export interface PlaybackOptions {
  /** Drive the clock with requestAnimationFrame (screen refresh rate) instead of the 100 ms
   *  interval — used while hi-res sampling is active so the values can update faster than 10 Hz. */
  raf?: boolean;
  /** Fires EVERY tick with the current virtual time (ms, track timebase) — index ticks only fire
   *  when the 10 Hz index actually moves, but the hi-res sampler needs the continuous clock. */
  onTime?: (virtualMs: number) => void;
}

/**
 * Manages the playback timer and provides pure seek/speed utilities.
 * The Svelte page owns the reactive state ($state); this class owns the interval.
 */
export class PlaybackController {
  private timer: ReturnType<typeof setInterval> | null = null;
  private rafId: number | null = null;

  /** Stop the interval/rAF timer. Does not reset index/speed. */
  stop(): void {
    if (this.timer != null) {
      clearInterval(this.timer);
      this.timer = null;
    }
    if (this.rafId != null) {
      cancelAnimationFrame(this.rafId);
      this.rafId = null;
    }
  }

  /**
   * Start playback.
   * Returns the (possibly reset) starting index.
   * `onTick` fires each interval with the new index.
   * `onFinish` fires when the track ends.
   */
  start(
    track: TelemetryRecord[],
    currentIndex: number,
    speed: number,
    onTick: (newIndex: number) => void,
    onFinish: () => void,
    options?: PlaybackOptions,
  ): number {
    if (track.length <= 1) return currentIndex;
    const startIdx = currentIndex >= track.length - 1 ? 0 : currentIndex;
    this.stop();
    let idx = startIdx;
    let virtualTime = track[startIdx].timestamp_ms;
    const endTs = track[track.length - 1].timestamp_ms;

    const step = (dtMs: number): void => {
      if (idx >= track.length - 1) {
        this.stop();
        onFinish();
        return;
      }
      virtualTime = Math.min(virtualTime + dtMs * speed, endTs);
      let newIdx = idx;
      while (newIdx < track.length - 1 && track[newIdx + 1].timestamp_ms <= virtualTime) newIdx++;
      if (newIdx !== idx) {
        idx = newIdx;
        onTick(idx);
      }
      options?.onTime?.(virtualTime);
    };

    if (options?.raf) {
      let last = performance.now();
      const frame = (now: number): void => {
        // Clamp a background-tab stall to one normal tick — replay shouldn't jump minutes ahead.
        const dt = Math.min(now - last, 1000);
        last = now;
        step(dt);
        // step() calls stop() at the end of the track, which clears rafId — don't re-arm then.
        if (this.rafId != null) this.rafId = requestAnimationFrame(frame);
      };
      this.rafId = requestAnimationFrame(frame);
    } else {
      this.timer = setInterval(() => step(TICK_MS), TICK_MS);
    }
    return startIdx;
  }

  /** Clean up on component destroy. */
  destroy(): void {
    this.stop();
  }

  /** Seek forward/backward by deltaMs. Returns the new index. */
  static seek(track: TelemetryRecord[], currentIndex: number, deltaMs: number): number {
    if (track.length === 0) return 0;
    const currentTs = track[currentIndex].timestamp_ms;
    const targetTs = currentTs + deltaMs;
    if (deltaMs < 0) {
      let i = currentIndex;
      while (i > 0 && track[i].timestamp_ms > targetTs) i--;
      return i;
    }
    let i = currentIndex;
    while (i < track.length - 1 && track[i + 1].timestamp_ms <= targetTs) i++;
    return Math.min(i, track.length - 1);
  }

  /** Cycle through speed presets. Returns the next speed value. */
  static cycleSpeed(currentSpeed: number): number {
    const idx = (SPEEDS as readonly number[]).indexOf(currentSpeed);
    return SPEEDS[(idx + 1) % SPEEDS.length];
  }
}
