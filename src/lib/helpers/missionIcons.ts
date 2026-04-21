import L from 'leaflet';
import { WpAction, type Waypoint } from '$lib/stores/mission';

/** WAYPOINT — upside-down teardrop with WP number (48×66) */
export function waypointIcon(num: number, selected: boolean): L.DivIcon {
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

/** POSHOLD — circle with orbit ring (88×88, tighter text) */
export function posholdIcon(num: number, seconds: number | null, selected: boolean): L.DivIcon {
  const fill = selected ? '#ff4444' : '#f39c12';
  const stroke = selected ? '#cc0000' : '#d68910';
  const label = seconds !== null ? `${seconds}s` : '∞';
  return L.divIcon({
    className: 'mission-wp-icon',
    html: `<svg viewBox="0 0 40 40" width="88" height="88">
      <circle cx="20" cy="20" r="17" fill="none" stroke="${stroke}" stroke-width="2" stroke-dasharray="4 2"/>
      <circle cx="20" cy="20" r="11" fill="${fill}" stroke="${stroke}" stroke-width="1.5"/>
      <text x="20" y="18" text-anchor="middle" fill="white" font-size="9" font-weight="bold"
            font-family="sans-serif">${num}</text>
      <text x="20" y="26" text-anchor="middle" fill="white" font-size="7"
            font-family="sans-serif">${label}</text>
    </svg>`,
    iconSize: [88, 88],
    iconAnchor: [44, 44],
  });
}

/** SET_POI — purple marker with eye icon (48×48) */
export function poiIcon(num: number, selected: boolean): L.DivIcon {
  const fill = selected ? '#ff4444' : '#8e44ad';
  const stroke = selected ? '#cc0000' : '#6c3483';
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

/** LAND — orange teardrop with down-arrow icon (48×66) */
export function landIcon(selected: boolean): L.DivIcon {
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

/** RTH — house icon (42×42) */
export function rthIcon(selected: boolean): L.DivIcon {
  const fill = selected ? '#ff4444' : '#e67e22';
  return L.divIcon({
    className: 'mission-wp-icon',
    html: `<svg viewBox="0 0 32 32" width="42" height="42">
      <path d="M16 4 L4 16 L8 16 L8 28 L24 28 L24 16 L28 16 Z" fill="${fill}" stroke="#7e5109" stroke-width="1.5"/>
      <text x="16" y="22" text-anchor="middle" fill="white" font-size="8" font-weight="bold"
            font-family="sans-serif">RTH</text>
    </svg>`,
    iconSize: [42, 42],
    iconAnchor: [21, 21],
  });
}

/** Generic fallback icon (48×48) */
export function genericIcon(num: number, label: string, selected: boolean): L.DivIcon {
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

/** Pick the right icon for a waypoint (displayNum used for numbered types) */
export function iconForWp(wp: Waypoint, displayNum: number, selected: boolean): L.DivIcon {
  switch (wp.action) {
    case WpAction.Waypoint:
      return waypointIcon(displayNum, selected);
    case WpAction.PosholdUnlim:
      return posholdIcon(displayNum, null, selected);
    case WpAction.PosholdTime:
      return posholdIcon(displayNum, wp.p1, selected);
    case WpAction.SetPoi:
      return poiIcon(displayNum, selected);
    case WpAction.Land:
      return landIcon(selected);
    case WpAction.Rth:
      return rthIcon(selected);
    case WpAction.Jump:
      return genericIcon(displayNum, 'JMP', selected);
    case WpAction.SetHead:
      return genericIcon(displayNum, 'HDG', selected);
    default:
      return genericIcon(displayNum, '?', selected);
  }
}
