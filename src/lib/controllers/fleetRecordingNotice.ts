// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Fleet mode without group recording (backend `fleet_mode`) records one vehicle at a time — said once per run
// when a second vehicle appears, as a toast (never a dialog that could displace an End-Flight question). A
// notice the "System Messages" filter dropped doesn't count; it is tried again on the next vehicle change.

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
