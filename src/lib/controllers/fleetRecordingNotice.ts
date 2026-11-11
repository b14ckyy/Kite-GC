// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Fleet mode without group recording (the default until the group end screen exists, backend
// `fleet_mode`): one vehicle at a time is recorded. Said once per app run when a second vehicle appears
// — a toast in the system-message banner, not a dialog (it must never displace an open End-Flight
// question). "Once" counts only a notice that was actually shown: one the "System Messages" filter
// dropped is tried again on the next vehicle change.

import { get } from 'svelte/store';
import { t } from 'svelte-i18n';
import { vehicles } from '$lib/stores/vehicles';
import { fleetEnabled, fleetGroupRecording } from '$lib/stores/fleetMode';
import { pushLocalStatus } from '$lib/stores/statusText';

/** STATUSTEXT warning severity — passes the default "Warning" toast filter. */
const SEVERITY_WARNING = 4;
let shown = false;
let started = false;

/** Watch for the second vehicle of a fleet session that records only the first one. Idempotent. */
export function startFleetRecordingNotice(): void {
  if (started) return;
  started = true;
  vehicles.subscribe((known) => {
    if (shown || known.size < 2 || !get(fleetEnabled) || get(fleetGroupRecording)) return;
    shown = pushLocalStatus(SEVERITY_WARNING, get(t)('fleet.recordingNotice'));
  });
}
