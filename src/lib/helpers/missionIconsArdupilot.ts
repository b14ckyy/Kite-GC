// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

import L from 'leaflet';
import { type ArduWaypoint } from '$lib/stores/missionArdupilot';
import { cmdDef, cmdShort, CMD } from '$lib/helpers/arduCommandCatalog';
import {
  type WpIconSpec,
  teardropNumberSpec, teardropArrowSpec, orbitSpec, houseSpec, eyeSpec, genericSpec,
  divIconFromSpec, wpColor,
} from './missionIconPrimitives';

// ArduPilot command → icon mapper. Driven by the command catalog's flags (isTakeoff / isLand /
// isLoiter / specifiesCoordinate / standaloneCoordinate + category) rather than a hand-maintained
// per-command switch, so it covers the whole catalog (and any command added to it) automatically.
// Shapes + colours come from the shared primitive layer (missionIconPrimitives.ts) — the same source
// the INAV mapper uses — so comparable types (waypoint, loiter, land, takeoff, ROI) stay in visual sync.

/** Hold-descriptor label for the loiter ring, mirroring INAV's PosHold marker. */
function loiterLabel(wp: ArduWaypoint): string {
  switch (wp.command) {
    case CMD.NAV_LOITER_TIME:   return `${Math.round(wp.param1)}s`;
    case CMD.NAV_LOITER_TURNS:  return `×${Math.round(wp.param1)}`;
    case CMD.NAV_LOITER_TO_ALT: return '⇅'; // loiters while climbing/descending to the target altitude
    default:                    return '∞'; // LOITER_UNLIM
  }
}

/** Small corner badge for positioned-waypoint variants that share the teardrop shape. */
function waypointBadge(cmd: number): string | undefined {
  if (cmd === CMD.NAV_SPLINE_WAYPOINT) return 'S';
  if (cmd === CMD.NAV_PAYLOAD_PLACE) return 'P';
  return undefined;
}

/** Pick the icon SPEC for an ArduPilot waypoint (shared by 2D + future 3D rendering). The layer only
 *  calls this for items that get a marker (commands with a coordinate — flight-path or standalone). */
export function arduWpIconSpec(wp: ArduWaypoint, displayNum: number, selected: boolean): WpIconSpec {
  const def = cmdDef(wp.command);
  // Catalog-unknown command (legacy / round-trip-only) → labelled generic marker, not a bare "?".
  if (!def) return genericSpec(displayNum, cmdShort(wp.command), wpColor('generic', selected));

  const isVtol = def.category === 'VTOL'; // VTOL takeoff/land share the arrow shape + a "V" badge

  if (def.isTakeoff) return teardropArrowSpec('up', wpColor('takeoff', selected), isVtol ? 'V' : undefined);
  if (def.isLand) return teardropArrowSpec('down', wpColor('land', selected), isVtol ? 'V' : undefined);
  if (def.isLoiter) return orbitSpec(displayNum, loiterLabel(wp), wpColor('loiter', selected));

  // Flight-path waypoints (plain / spline / payload-place) — numbered teardrop, badged per variant.
  if (def.specifiesCoordinate) {
    return teardropNumberSpec(displayNum, wpColor('waypoint', selected), waypointBadge(wp.command));
  }

  // Standalone coordinates (not flight-path nodes): ROI = eye, Set Home = house, else labelled generic.
  if (wp.command === CMD.DO_SET_ROI_LOCATION) return eyeSpec(displayNum, wpColor('poi', selected));
  if (wp.command === CMD.DO_SET_HOME) return houseSpec('HOME', wpColor('rth', selected));
  return genericSpec(displayNum, cmdShort(wp.command), wpColor('generic', selected));
}

/** 2D Leaflet divIcon for an ArduPilot waypoint (built from the shared spec). `active` adds the
 *  pulsing-glow class for the FC's current target waypoint (same as INAV). */
export function iconForArduWp(wp: ArduWaypoint, displayNum: number, selected: boolean, active = false): L.DivIcon {
  return divIconFromSpec(arduWpIconSpec(wp, displayNum, selected), active);
}
