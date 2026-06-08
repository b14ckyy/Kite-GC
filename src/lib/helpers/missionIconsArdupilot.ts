// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

import L from 'leaflet';
import {
  type ArduWaypoint,
  MAV_CMD_NAV_WAYPOINT, MAV_CMD_NAV_LOITER_UNLIM, MAV_CMD_NAV_LOITER_TURNS,
  MAV_CMD_NAV_LOITER_TIME, MAV_CMD_NAV_RETURN_TO_LAUNCH, MAV_CMD_NAV_LAND,
  MAV_CMD_NAV_TAKEOFF, MAV_CMD_DO_JUMP, MAV_CMD_DO_SET_ROI,
} from '$lib/stores/missionArdupilot';

// All icons use the same visual vocabulary as INAV (missionIcons.ts):
//   NAV_WAYPOINT  → teardrop with number  (48×66, same as INAV Waypoint)
//   NAV_LOITER_*  → orbit circle          (88×88, same shape as INAV POSHOLD, cyan)
//   NAV_RTL       → house icon            (42×42, same as INAV RTH)
//   NAV_LAND      → teardrop with ↓       (48×66, same as INAV Land)
//   NAV_TAKEOFF   → teardrop with ↑       (48×66, green)
//   DO_SET_ROI    → circle with eye       (48×48, same as INAV SetPOI, teal)
//   others        → circle with label     (48×48, same as INAV generic)

function waypointIcon(num: number, selected: boolean): L.DivIcon {
  const fill = selected ? '#ff4444' : '#37a8db';
  const stroke = selected ? '#cc0000' : '#1a5276';
  return L.divIcon({
    className: 'mission-wp-icon',
    html: `<svg viewBox="0 0 32 44" width="48" height="66">
      <path d="M16 44 C16 44 2 24 2 16 A14 14 0 1 1 30 16 C30 24 16 44 16 44Z"
            fill="${fill}" stroke="${stroke}" stroke-width="2"/>
      <text x="16" y="20" text-anchor="middle" fill="white" font-size="12" font-weight="bold"
            font-family="sans-serif">${num}</text>
    </svg>`,
    iconSize: [48, 66],
    iconAnchor: [24, 66],
  });
}

function loiterIcon(num: number, selected: boolean): L.DivIcon {
  const fill = selected ? '#ff4444' : '#00bcd4';
  const stroke = selected ? '#cc0000' : '#0097a7';
  return L.divIcon({
    className: 'mission-wp-icon',
    html: `<svg viewBox="0 0 40 40" width="88" height="88">
      <circle cx="20" cy="20" r="17" fill="none" stroke="${stroke}" stroke-width="2" stroke-dasharray="4 2"/>
      <circle cx="20" cy="20" r="11" fill="${fill}" stroke="${stroke}" stroke-width="1.5"/>
      <text x="20" y="18" text-anchor="middle" fill="white" font-size="9" font-weight="bold"
            font-family="sans-serif">${num}</text>
      <text x="20" y="26" text-anchor="middle" fill="white" font-size="7"
            font-family="sans-serif">LTR</text>
    </svg>`,
    iconSize: [88, 88],
    iconAnchor: [44, 44],
  });
}

function rtlIcon(selected: boolean): L.DivIcon {
  const fill = selected ? '#ff4444' : '#e67e22';
  return L.divIcon({
    className: 'mission-wp-icon',
    html: `<svg viewBox="0 0 32 32" width="42" height="42">
      <path d="M16 4 L4 16 L8 16 L8 28 L24 28 L24 16 L28 16 Z" fill="${fill}" stroke="#7e5109" stroke-width="1.5"/>
      <text x="16" y="22" text-anchor="middle" fill="white" font-size="8" font-weight="bold"
            font-family="sans-serif">RTL</text>
    </svg>`,
    iconSize: [42, 42],
    iconAnchor: [21, 21],
  });
}

function landIcon(selected: boolean): L.DivIcon {
  const fill = selected ? '#ff4444' : '#f39c12';
  const stroke = selected ? '#cc0000' : '#d68910';
  return L.divIcon({
    className: 'mission-wp-icon',
    html: `<svg viewBox="0 0 32 44" width="48" height="66">
      <path d="M16 44 C16 44 2 24 2 16 A14 14 0 1 1 30 16 C30 24 16 44 16 44Z"
            fill="${fill}" stroke="${stroke}" stroke-width="2"/>
      <path d="M16 10 L16 25 M11 20 L16 25 L21 20" fill="none" stroke="white"
            stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"/>
    </svg>`,
    iconSize: [48, 66],
    iconAnchor: [24, 66],
  });
}

function takeoffIcon(selected: boolean): L.DivIcon {
  const fill = selected ? '#ff4444' : '#27ae60';
  const stroke = selected ? '#cc0000' : '#1e8449';
  return L.divIcon({
    className: 'mission-wp-icon',
    html: `<svg viewBox="0 0 32 44" width="48" height="66">
      <path d="M16 44 C16 44 2 24 2 16 A14 14 0 1 1 30 16 C30 24 16 44 16 44Z"
            fill="${fill}" stroke="${stroke}" stroke-width="2"/>
      <path d="M16 26 L16 11 M11 16 L16 11 L21 16" fill="none" stroke="white"
            stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"/>
    </svg>`,
    iconSize: [48, 66],
    iconAnchor: [24, 66],
  });
}

function roiIcon(num: number, selected: boolean): L.DivIcon {
  const fill = selected ? '#ff4444' : '#1abc9c';
  const stroke = selected ? '#cc0000' : '#148f77';
  return L.divIcon({
    className: 'mission-wp-icon',
    html: `<svg viewBox="0 0 32 32" width="48" height="48">
      <circle cx="16" cy="16" r="13" fill="${fill}" stroke="${stroke}" stroke-width="2"/>
      <text x="16" y="13" text-anchor="middle" fill="white" font-size="10"
            font-family="sans-serif">👁</text>
      <text x="16" y="25" text-anchor="middle" fill="white" font-size="9" font-weight="bold"
            font-family="sans-serif">${num}</text>
    </svg>`,
    iconSize: [48, 48],
    iconAnchor: [24, 24],
  });
}

function genericIcon(num: number, label: string, selected: boolean): L.DivIcon {
  const fill = selected ? '#ff4444' : '#7f8c8d';
  return L.divIcon({
    className: 'mission-wp-icon',
    html: `<svg viewBox="0 0 32 32" width="48" height="48">
      <circle cx="16" cy="16" r="13" fill="${fill}" stroke="#2c3e50" stroke-width="2"/>
      <text x="16" y="13" text-anchor="middle" fill="white" font-size="8"
            font-family="sans-serif">${label}</text>
      <text x="16" y="24" text-anchor="middle" fill="white" font-size="10" font-weight="bold"
            font-family="sans-serif">${num}</text>
    </svg>`,
    iconSize: [48, 48],
    iconAnchor: [24, 24],
  });
}

export function iconForArduWp(wp: ArduWaypoint, displayNum: number, selected: boolean): L.DivIcon {
  switch (wp.command) {
    case MAV_CMD_NAV_WAYPOINT:         return waypointIcon(displayNum, selected);
    case MAV_CMD_NAV_LOITER_UNLIM:
    case MAV_CMD_NAV_LOITER_TURNS:
    case MAV_CMD_NAV_LOITER_TIME:      return loiterIcon(displayNum, selected);
    case MAV_CMD_NAV_RETURN_TO_LAUNCH: return rtlIcon(selected);
    case MAV_CMD_NAV_LAND:             return landIcon(selected);
    case MAV_CMD_NAV_TAKEOFF:          return takeoffIcon(selected);
    case MAV_CMD_DO_JUMP:              return genericIcon(displayNum, 'JMP', selected);
    case MAV_CMD_DO_SET_ROI:           return roiIcon(displayNum, selected);
    default:                           return genericIcon(displayNum, '?', selected);
  }
}
