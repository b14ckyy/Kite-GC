// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Global RC-control session manager.
//
// Once engaged, GCS RC control MUST keep running no matter where the user goes in the app — the RC panel
// is only a **config + initiator** surface, NOT the owner of the live stream. So the pieces that used to
// live on the panel's lifecycle move here, module-level, and stay alive while the panel is closed:
//   • HID input runs while the panel is open (config/preview) OR while engaged (control);
//   • an engaged session is ended ONLY by an explicit disengage (the panel button) or by a genuine loss
//     of control authority — link disconnect, the input device disappearing, or an INAV safety lock —
//     NEVER by leaving the panel or navigating elsewhere.
//
// Protocol-agnostic: this applies to INAV (MSP), ArduPilot and PX4 alike — the injection pump in
// rcStream.ts branches per platform, this layer doesn't care. Import once (in +page.svelte) so it is
// active for the whole app session, independent of whether the RC panel has ever been opened.

import { writable, derived, get } from 'svelte/store';
import { rcEngaged, disengage } from './rcEngage';
import { startHid, stopHid, hidActive, hidDevices } from './hid';
import { connection } from './connection';
import { rcFcConfig } from './rcFcConfig';
import { currentChannels } from './rcProfiles';
import { rcLayout } from './rcLayout';
import { evaluateRcSafety } from '../helpers/rcSafety';
import './rcStream'; // the engage-driven injection pump (self-initialising)

/** True while the RC control panel is mounted — keeps HID running for config/preview even before engage. */
export const rcPanelOpen = writable(false);

// ── HID lifecycle: input runs while the panel is open OR while engaged ────────────────────────────────
// (So control keeps reading the stick after you leave the panel; without this the pump would re-send the
// last frozen values.) `startHid`/`stopHid` are idempotent + async, so we reconcile in a small guarded
// loop that re-checks the desired state after each transition (handles rapid open/close/engage toggles).
const hidWanted = derived([rcPanelOpen, rcEngaged], ([$open, $eng]) => $open || $eng.on);
let hidBusy = false;
async function reconcileHid(): Promise<void> {
  if (hidBusy) return;
  hidBusy = true;
  try {
    while (get(hidWanted) !== get(hidActive)) {
      if (get(hidWanted)) await startHid();
      else await stopHid();
    }
  } finally {
    hidBusy = false;
  }
}
hidWanted.subscribe(() => void reconcileHid());

// ── Involuntary disengage: ONLY a genuine loss of control authority ends a background session ─────────
// NOT app navigation. Holding a frozen-stick stream after a real loss is unsafe, so we stop and let the
// FC's RC failsafe / RC_OVERRIDE_TIME grace window take over (no explicit release frame — same as before).
const authorityLost = derived(
  [rcEngaged, connection, hidDevices, rcFcConfig, currentChannels, rcLayout],
  ([$eng, $conn, $devs, $fc, $chans, $layout]) => {
    if (!$eng.on) return false;
    if ($conn.status !== 'connected') return true; // link/FC disconnect
    if ($devs.length === 0) return true; // input device unplugged / driver drop
    if ($fc) {
      // INAV: a critical mode-channel configuration lock (never set for ArduPilot/PX4 — no rcFcConfig).
      const safety = evaluateRcSafety($fc.mode_ranges, Object.keys($chans).map(Number), $layout.rawMax);
      if (safety.locked) return true;
    }
    return false;
  },
);
authorityLost.subscribe((lost) => {
  if (lost) disengage();
});
