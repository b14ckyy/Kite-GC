// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

import { WpAction, type Waypoint } from '$lib/stores/mission';
import {
  type ArduWaypoint, type MavFrame,
  MAV_CMD_NAV_WAYPOINT, MAV_CMD_NAV_LOITER_UNLIM, MAV_CMD_NAV_LOITER_TIME,
  MAV_CMD_NAV_RETURN_TO_LAUNCH, MAV_CMD_NAV_LAND, MAV_CMD_DO_SET_ROI,
  MAV_CMD_DO_JUMP, MAV_CMD_NAV_LOITER_TURNS, MAV_CMD_NAV_TAKEOFF,
  MAV_FRAME_GLOBAL_RELATIVE_ALT,
} from '$lib/stores/missionArdupilot';

// ── Helpers ───────────────────────────────────────────────────────────

function inavAltToArdu(altCm: number): number { return altCm / 100; }
function arduAltToInav(altM: number): number   { return Math.round(altM * 100); }

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
  return wps.map((wp, i) => {
    const base: Omit<Waypoint, 'action'> = {
      number: i + 1,
      lat: wp.lat,
      lon: wp.lon,
      altitude: arduAltToInav(wp.alt),
      p1: 0, p2: 0, p3: 0,
      flag: 0,
    };
    switch (wp.command) {
      case MAV_CMD_NAV_WAYPOINT:
        return { ...base, action: WpAction.Waypoint };
      case MAV_CMD_NAV_LOITER_UNLIM:
        return { ...base, action: WpAction.PosholdUnlim };
      case MAV_CMD_NAV_LOITER_TIME:
        return { ...base, action: WpAction.PosholdTime, p1: Math.round(wp.param1) };
      case MAV_CMD_NAV_LAND:
        return { ...base, action: WpAction.Land };
      case MAV_CMD_NAV_RETURN_TO_LAUNCH:
        return { ...base, action: WpAction.Rth };
      case MAV_CMD_DO_SET_ROI:
        return { ...base, action: WpAction.SetPoi };
      case MAV_CMD_DO_JUMP:
        return { ...base, action: WpAction.Jump, p1: Math.round(wp.param1), p2: Math.round(wp.param2) };
      case MAV_CMD_NAV_LOITER_TURNS:
        return { ...base, action: WpAction.PosholdTime, p1: Math.round(wp.param1 * 10) };
      case MAV_CMD_NAV_TAKEOFF:
        return { ...base, action: WpAction.Waypoint };
      default:
        return { ...base, action: WpAction.Waypoint };
    }
  });
}
