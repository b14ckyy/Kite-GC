// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Pure mission-geometry helpers shared by the 2D and 3D map mission rendering
// (display numbering, flight-path filtering, mission-end detection, modifiers).
// Keeping these in one place avoids divergent copies between the renderers.

import { WpAction, hasLocation, isModifier, WP_FLAG_FBH, type Waypoint } from '$lib/stores/mission';

/** Waypoint actions that INAV allows the Fly-by-Home flag on (since INAV 4.0). */
export const FBH_ACTIONS: WpAction[] = [WpAction.Waypoint, WpAction.PosholdTime, WpAction.Land];

/** Fly-by-Home waypoint: a WAYPOINT/POSHOLD_TIME/LAND that has no own coordinates and
 *  executes at the home/launch point at its altitude. Mirrors the backend
 *  `Waypoint::is_fly_by_home` (flag 0x48, or lat/lon both zero), restricted to the
 *  actions INAV supports FBH on. */
export function isFlyByHome(wp: Waypoint): boolean {
  return FBH_ACTIONS.includes(wp.action) && (wp.flag === WP_FLAG_FBH || (wp.lat === 0 && wp.lon === 0));
}

/** Map waypoint index → displayed WP number (modifiers don't get a number). */
export function buildDisplayNumbers(waypoints: Waypoint[]): Map<number, number> {
  const nums = new Map<number, number>();
  let dn = 1;
  for (let i = 0; i < waypoints.length; i++) {
    if (!isModifier(waypoints[i].action)) nums.set(i, dn++);
  }
  return nums;
}

/** Modifier waypoints (Jump/RTH/SetHead) attached after a geo-waypoint. */
export function getModifiersForWp(waypoints: Waypoint[], geoIdx: number): { wp: Waypoint; idx: number }[] {
  const mods: { wp: Waypoint; idx: number }[] = [];
  for (let j = geoIdx + 1; j < waypoints.length; j++) {
    if (isModifier(waypoints[j].action)) mods.push({ wp: waypoints[j], idx: j });
    else break;
  }
  return mods;
}

/** Whether a waypoint contributes to the drawn flight path (geo, excluding POI). */
export function isFlightPathWp(action: WpAction): boolean {
  return hasLocation(action) && action !== WpAction.SetPoi;
}

/** Index of the first Land/RTH (mission end); waypoints after it are "greyed". */
export function findMissionEndIndex(waypoints: Waypoint[]): number {
  for (let i = 0; i < waypoints.length; i++) {
    if (waypoints[i].action === WpAction.Land || waypoints[i].action === WpAction.Rth) return i;
  }
  return -1;
}

/** Nearest geo-waypoint before `fromIndex` (for Jump/RTH connector origins). */
export function findPreviousGeoWp(waypoints: Waypoint[], fromIndex: number): Waypoint | null {
  for (let i = fromIndex - 1; i >= 0; i--) {
    if (hasLocation(waypoints[i].action)) return waypoints[i];
  }
  return null;
}
