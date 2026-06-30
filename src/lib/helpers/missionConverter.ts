// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

import { WpAction, ALT_MODE_REL, ALT_MODE_AMSL, ALT_MODE_AGL, type Waypoint } from '$lib/stores/mission';
import {
  type ArduWaypoint, type MavFrame,
  MAV_CMD_NAV_WAYPOINT, MAV_CMD_NAV_LOITER_UNLIM, MAV_CMD_NAV_LOITER_TIME,
  MAV_CMD_NAV_RETURN_TO_LAUNCH, MAV_CMD_NAV_LAND, MAV_CMD_DO_SET_ROI,
  MAV_CMD_DO_JUMP, MAV_CMD_NAV_LOITER_TURNS, MAV_CMD_NAV_TAKEOFF,
  MAV_FRAME_GLOBAL, MAV_FRAME_GLOBAL_RELATIVE_ALT, MAV_FRAME_GLOBAL_TERRAIN_ALT,
} from '$lib/stores/missionArdupilot';

// ── Helpers ───────────────────────────────────────────────────────────

function inavAltToArdu(altCm: number): number { return altCm / 100; }
function arduAltToInav(altM: number): number   { return Math.round(altM * 100); }

/** Map an ArduPilot altitude frame to INAV's alt_mode so converted waypoints keep their reference
 *  (GLOBAL = AMSL, TERRAIN_ALT = AGL, RELATIVE_ALT = launch/home-relative). */
function frameToAltMode(frame: MavFrame): number {
  if (frame === MAV_FRAME_GLOBAL) return ALT_MODE_AMSL;
  if (frame === MAV_FRAME_GLOBAL_TERRAIN_ALT) return ALT_MODE_AGL;
  return ALT_MODE_REL;
}

// ── INAV → ArduPilot ──────────────────────────────────────────────────

export function inavToArdu(wps: Waypoint[]): ArduWaypoint[] {
  return wps
    .filter(wp => wp.action !== WpAction.SetHead) // no ArduPilot equivalent
    .map(wp => {
      const base = {
        frame: MAV_FRAME_GLOBAL_RELATIVE_ALT as MavFrame,
        param1: 0, param2: 0, param3: 0, param4: 0,
        lat: wp.lat,
        lon: wp.lon,
        alt: inavAltToArdu(wp.altitude),
        autocontinue: true,
      };
      switch (wp.action) {
        case WpAction.Waypoint:
          return { ...base, command: MAV_CMD_NAV_WAYPOINT };
        case WpAction.PosholdUnlim:
          return { ...base, command: MAV_CMD_NAV_LOITER_UNLIM };
        case WpAction.PosholdTime:
          return { ...base, command: MAV_CMD_NAV_LOITER_TIME, param1: wp.p1 };
        case WpAction.Land:
          return { ...base, command: MAV_CMD_NAV_LAND };
        case WpAction.Rth:
          return { ...base, command: MAV_CMD_NAV_RETURN_TO_LAUNCH, lat: 0, lon: 0, alt: 0 };
        case WpAction.SetPoi:
          return { ...base, command: MAV_CMD_DO_SET_ROI };
        case WpAction.Jump:
          return { ...base, command: MAV_CMD_DO_JUMP, param1: wp.p1, param2: wp.p2, lat: 0, lon: 0, alt: 0 };
        default:
          return { ...base, command: MAV_CMD_NAV_WAYPOINT };
      }
    });
}

// ── ArduPilot → INAV ──────────────────────────────────────────────────

export function arduToInav(wps: ArduWaypoint[]): Waypoint[] {
  return arduToInavIndexed(wps).waypoints;
}

/** Like {@link arduToInav}, but also returns `srcIdx`: for each emitted INAV waypoint, the index of its
 *  source command in the input `wps`. Needed to write an edit (e.g. terrain correction) back to the
 *  original ArduPilot/PX4 mission, since unsupported commands are dropped (so positions don't line up). */
export function arduToInavIndexed(wps: ArduWaypoint[]): { waypoints: Waypoint[]; srcIdx: number[] } {
  const out: Waypoint[] = [];
  const srcIdx: number[] = [];
  wps.forEach((wp, i) => {
    const before = out.length;
    const base: Omit<Waypoint, 'action'> = {
      number: out.length + 1, // renumber after dropping unsupported commands
      lat: wp.lat,
      lon: wp.lon,
      altitude: arduAltToInav(wp.alt),
      alt_mode: frameToAltMode(wp.frame),
      p1: 0, p2: 0, p3: 0,
      flag: 0,
    };
    switch (wp.command) {
      case MAV_CMD_NAV_WAYPOINT:
        out.push({ ...base, action: WpAction.Waypoint });
        break;
      case MAV_CMD_NAV_LOITER_UNLIM:
      case MAV_CMD_NAV_LOITER_TURNS:
        // INAV has no turn-count loiter — map both to an unlimited hold at the point (turns lost).
        out.push({ ...base, action: WpAction.PosholdUnlim });
        break;
      case MAV_CMD_NAV_LOITER_TIME:
        out.push({ ...base, action: WpAction.PosholdTime, p1: Math.round(wp.param1) });
        break;
      case MAV_CMD_NAV_LAND:
        out.push({ ...base, action: WpAction.Land });
        break;
      case MAV_CMD_NAV_RETURN_TO_LAUNCH:
        out.push({ ...base, action: WpAction.Rth });
        break;
      case MAV_CMD_DO_SET_ROI:
        out.push({ ...base, action: WpAction.SetPoi });
        break;
      case MAV_CMD_DO_JUMP:
        out.push({ ...base, action: WpAction.Jump, p1: Math.round(wp.param1), p2: Math.round(wp.param2) });
        break;
      case MAV_CMD_NAV_TAKEOFF:
        // INAV auto-launches; nearest equivalent is a waypoint at the takeoff point.
        out.push({ ...base, action: WpAction.Waypoint });
        break;
      default:
        // Unsupported on INAV (Delay, Change Speed, splines, camera, …) — drop it rather than
        // emit a phantom waypoint (these often carry no/zeroed coordinates).
        break;
    }
    if (out.length > before) srcIdx.push(i); // each command emits at most one waypoint
  });
  return { waypoints: out, srcIdx };
}
