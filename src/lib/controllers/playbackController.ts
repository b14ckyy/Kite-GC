import type { TelemetryRecord } from '$lib/stores/flightlog';

const TICK_MS = 100;
const SPEEDS = [1, 2, 4, 10] as const;

/**
 * Manages the playback timer and provides pure seek/speed utilities.
 * The Svelte page owns the reactive state ($state); this class owns the interval.
 */
export class PlaybackController {
  private timer: ReturnType<typeof setInterval> | null = null;

  /** Stop the interval timer. Does not reset index/speed. */
  stop(): void {
    if (this.timer != null) {
      clearInterval(this.timer);
      this.timer = null;
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
  ): number {
    if (track.length <= 1) return currentIndex;
    const startIdx = currentIndex >= track.length - 1 ? 0 : currentIndex;
    this.stop();
    let idx = startIdx;
    let virtualTime = track[startIdx].timestamp_ms;
    this.timer = setInterval(() => {
      if (idx >= track.length - 1) {
        this.stop();
        onFinish();
        return;
      }
      virtualTime += TICK_MS * speed;
      let newIdx = idx;
      while (newIdx < track.length - 1 && track[newIdx + 1].timestamp_ms <= virtualTime) newIdx++;
      if (newIdx !== idx) {
        idx = newIdx;
        onTick(idx);
      }
    }, TICK_MS);
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
