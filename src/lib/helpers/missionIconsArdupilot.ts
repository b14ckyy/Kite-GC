// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

import L from 'leaflet';
import {
  type ArduWaypoint,
  MAV_CMD_NAV_WAYPOINT, MAV_CMD_NAV_LOITER_UNLIM, MAV_CMD_NAV_LOITER_TURNS,
  MAV_CMD_NAV_LOITER_TIME, MAV_CMD_NAV_RETURN_TO_LAUNCH, MAV_CMD_NAV_LAND,
  MAV_CMD_NAV_TAKEOFF, MAV_CMD_DO_JUMP, MAV_CMD_DO_SET_ROI,
} from '$lib/stores/missionArdupilot';
import {
  type WpIconSpec,
  teardropNumberSpec, teardropArrowSpec, orbitSpec, houseSpec, eyeSpec, genericSpec,
  divIconFromSpec, wpColor,
} from './missionIconPrimitives';

// ArduPilot command → icon mapper. Shapes + colours come from the shared primitive layer
// (missionIconPrimitives.ts) — the same source the INAV mapper uses — so comparable types (waypoint,
// loiter, land, RTL/RTH, ROI/POI) stay visually in sync; only the MavCmd→primitive mapping and the
// ArduPilot-specific types (Takeoff) live here.

/** Hold-descriptor label for the loiter ring, mirroring INAV's PosHold marker. */
function loiterLabel(wp: ArduWaypoint): string {
  if (wp.command === MAV_CMD_NAV_LOITER_TIME) return `${Math.round(wp.param1)}s`;
  if (wp.command === MAV_CMD_NAV_LOITER_TURNS) return `×${Math.round(wp.param1)}`;
  return '∞'; // LOITER_UNLIM
}

/** Pick the icon SPEC for an ArduPilot waypoint (shared by 2D + future 3D rendering). */
export function arduWpIconSpec(wp: ArduWaypoint, displayNum: number, selected: boolean): WpIconSpec {
  switch (wp.command) {
    case MAV_CMD_NAV_WAYPOINT:         return teardropNumberSpec(displayNum, wpColor('waypoint', selected));
    case MAV_CMD_NAV_LOITER_UNLIM:
    case MAV_CMD_NAV_LOITER_TURNS:
    case MAV_CMD_NAV_LOITER_TIME:      return orbitSpec(displayNum, loiterLabel(wp), wpColor('loiter', selected));
    case MAV_CMD_NAV_RETURN_TO_LAUNCH: return houseSpec('RTL', wpColor('rth', selected));
    case MAV_CMD_NAV_LAND:             return teardropArrowSpec('down', wpColor('land', selected));
    case MAV_CMD_NAV_TAKEOFF:          return teardropArrowSpec('up', wpColor('takeoff', selected));
    case MAV_CMD_DO_SET_ROI:           return eyeSpec(displayNum, wpColor('poi', selected));
    case MAV_CMD_DO_JUMP:              return genericSpec(displayNum, 'JMP', wpColor('generic', selected));
    default:                           return genericSpec(displayNum, '?', wpColor('generic', selected));
  }
}

/** 2D Leaflet divIcon for an ArduPilot waypoint (built from the shared spec). */
export function iconForArduWp(wp: ArduWaypoint, displayNum: number, selected: boolean): L.DivIcon {
  return divIconFromSpec(arduWpIconSpec(wp, displayNum, selected));
}
