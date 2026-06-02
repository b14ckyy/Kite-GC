// Navigation status store — the FC's current target waypoint, unified across live
// telemetry (MSP_NAV_STATUS) and replay (blackbox / ArduPilot `active_wp_number`).
// `+page` writes it from the unified `telem`; the mission layer reads it to highlight
// the active waypoint. 0 = none / not navigating a mission.

import { writable } from 'svelte/store';

/** De-duplicating store: it is fed from the high-rate telemetry effect (~5 Hz), so it
 *  only notifies subscribers when the value actually changes — otherwise the mission
 *  layer would re-render the whole mission on every telemetry frame. */
function createActiveWp() {
  const inner = writable<number>(0);
  let cur = 0;
  return {
    subscribe: inner.subscribe,
    set(v: number) {
      if (v !== cur) {
        cur = v;
        inner.set(v);
      }
    },
  };
}

export const activeWpNumber = createActiveWp();
