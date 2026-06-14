// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

import L from 'leaflet';
import { WpAction, type Waypoint } from '$lib/stores/mission';
import {
  type WpIconSpec,
  teardropNumberSpec, teardropArrowSpec, orbitSpec, houseSpec, eyeSpec, genericSpec,
  divIconFromSpec, wpColor,
} from './missionIconPrimitives';

// INAV waypoint → icon mapper. Shapes + colours come from the shared primitive layer
// (missionIconPrimitives.ts) so comparable waypoint types stay visually in sync with the ArduPilot
// mapper; only the WpAction→primitive mapping (and the INAV-specific FBH marker) live here.

export type { WpIconSpec };

/** FBH (Fly-by-Home) — small orange house (modifier-style colour), labelled "FBH" + number. Sits on
 *  the inbound leg toward home; centre-anchored (not on a coordinate). INAV-specific. */
export function fbhIconSpec(num: number, selected: boolean): WpIconSpec {
  const fill = selected ? '#ff4444' : '#e67e22';
  const stroke = selected ? '#cc0000' : '#a0521d';
  return {
    svg: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 36 42" width="44" height="51">
      <path d="M18 3 L4 16 L7 16 L7 34 L29 34 L29 16 L32 16 Z"
            fill="${fill}" stroke="${stroke}" stroke-width="1.5"/>
      <text x="18" y="26" text-anchor="middle" fill="white" font-size="8" font-weight="bold"
            font-family="sans-serif">FBH</text>
      <text x="18" y="33" text-anchor="middle" fill="white" font-size="7"
            font-family="sans-serif">${num}</text>
    </svg>`,
    width: 44, height: 51, anchorX: 22, anchorY: 25,
  };
}

/** Pick the right icon SPEC for a waypoint (shared by 2D + 3D rendering). */
export function wpIconSpec(wp: Waypoint, displayNum: number, selected: boolean): WpIconSpec {
  switch (wp.action) {
    case WpAction.Waypoint:     return teardropNumberSpec(displayNum, wpColor('waypoint', selected));
    case WpAction.PosholdUnlim: return orbitSpec(displayNum, '∞', wpColor('loiter', selected));
    case WpAction.PosholdTime:  return orbitSpec(displayNum, `${wp.p1}s`, wpColor('loiter', selected));
    case WpAction.SetPoi:       return eyeSpec(displayNum, wpColor('poi', selected));
    case WpAction.Land:         return teardropArrowSpec('down', wpColor('land', selected));
    case WpAction.Rth:          return houseSpec('RTH', wpColor('rth', selected));
    case WpAction.Jump:         return genericSpec(displayNum, 'JMP', wpColor('generic', selected));
    case WpAction.SetHead:      return genericSpec(displayNum, 'HDG', wpColor('generic', selected));
    default:                    return genericSpec(displayNum, '?', wpColor('generic', selected));
  }
}

/** 2D Leaflet divIcon for a waypoint (built from the shared spec). `active` adds the pulsing-glow
 *  class for the FC's current target waypoint. */
export function iconForWp(wp: Waypoint, displayNum: number, selected: boolean, active = false): L.DivIcon {
  return divIconFromSpec(wpIconSpec(wp, displayNum, selected), active);
}

/** 2D Leaflet divIcon for a Fly-by-Home waypoint's house marker. */
export function fbhDivIcon(displayNum: number, selected: boolean, active = false): L.DivIcon {
  const s = fbhIconSpec(displayNum, selected);
  return L.divIcon({
    className: `mission-fbh-icon${active ? ' mission-wp-active' : ''}`,
    html: s.svg,
    iconSize: [s.width, s.height],
    iconAnchor: [s.anchorX, s.anchorY],
  });
}
