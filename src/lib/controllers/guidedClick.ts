// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Guided "Fly Here" map click — the rules the 2D map (Map.svelte) and the 3D map (Map3D.svelte) share:
// when a click may open the target form, which form variant it shows, and the send with its re-check.
// The maps own only the gesture, the pick and the popup. See VEHICLE_CONTROL.md.

import { get } from 'svelte/store';
import { t } from 'svelte-i18n';
import { autopilotSystem } from '$lib/stores/autopilotContext';
import { arduVehicleClass, arduEditMode } from '$lib/stores/missionArdupilot';
import { editMode } from '$lib/stores/mission';
import { activeSurveyPattern } from '$lib/stores/surveyPattern.svelte';
import { guidedActive, activeMode, repositionTo, lastFeedback, type GuidedParams } from '$lib/controllers/vehicleControl';

/** Fly-Here click is armed when the Guided toggle is on OR the FC is already in its guided mode —
 *  a mid-flight connect to a vehicle sitting in GUIDED must be commandable without first toggling
 *  the panel button (which would re-send the mode change). */
export function guidedClickArmed(): boolean {
  // No map target for ArduSub: "Fly Here" sends an altitude above home, which for a Sub is the surface.
  if (get(autopilotSystem) === 'ardupilot' && get(arduVehicleClass) === 'sub') return false;
  return get(guidedActive) || get(activeMode)?.guided === true;
}

/** The mission editors and the survey generator own map clicks while active — before, arming
 *  Guided was always an explicit button press, now it can be implicit (FC already in guided),
 *  so the Fly-Here click must step aside explicitly. */
export function mapClickOwnedByEditor(): boolean {
  return get(editMode) || get(arduEditMode) || activeSurveyPattern.isActive;
}

/** Form variant: Copter/VTOL (alt + heading) vs Plane (alt + loiter radius). */
export function guidedTargetMultirotor(): boolean {
  const cls = get(arduVehicleClass);
  return get(autopilotSystem) === 'px4' ? true : (cls === 'copter' || cls === 'quadplane');
}

/** "Fly Here" pressed. Re-checked at send time: the popup can outlive what armed it — the FC left its
 *  guided mode, or the vehicle is now a Sub (reconnect). Say so instead of dropping the click silently. */
export function sendGuidedTarget(lat: number, lon: number, p: GuidedParams): void {
  if (guidedClickArmed()) void repositionTo(lat, lon, p);
  else lastFeedback.set({ action: 'reposition', ok: false, message: get(t)('control.guidedGone'), ts: Date.now() });
}
