import L from 'leaflet';
import { WpAction, type Waypoint } from '$lib/stores/mission';

/** Marker visual spec, shared by the 2D map (Leaflet divIcon) and the 3D map
 *  (Cesium billboard) so both render the exact same SVG. `anchorX/anchorY` are
 *  in image pixels from the top-left (the point that sits on the coordinate). */
export interface WpIconSpec {
  svg: string;
  width: number;
  height: number;
  anchorX: number;
  anchorY: number;
}

/** WAYPOINT — upside-down teardrop with WP number (48×66) */
function waypointSpec(num: number, selected: boolean): WpIconSpec {
  const fill = selected ? '#ff4444' : '#37a8db';
  const stroke = selected ? '#cc0000' : '#1a5276';
  return {
    svg: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 44" width="48" height="66">
      <path d="M16 44 C16 44 2 24 2 16 A14 14 0 1 1 30 16 C30 24 16 44 16 44Z"
            fill="${fill}" stroke="${stroke}" stroke-width="2"/>
      <text x="16" y="20" text-anchor="middle" fill="white" font-size="12" font-weight="bold"
            font-family="sans-serif">${num}</text>
    </svg>`,
    width: 48, height: 66, anchorX: 24, anchorY: 66,
  };
}

/** POSHOLD — circle with orbit ring (88×88, tighter text) */
function posholdSpec(num: number, seconds: number | null, selected: boolean): WpIconSpec {
  const fill = selected ? '#ff4444' : '#f39c12';
  const stroke = selected ? '#cc0000' : '#d68910';
  const label = seconds !== null ? `${seconds}s` : '∞';
  return {
    svg: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 40" width="88" height="88">
      <circle cx="20" cy="20" r="17" fill="none" stroke="${stroke}" stroke-width="2" stroke-dasharray="4 2"/>
      <circle cx="20" cy="20" r="11" fill="${fill}" stroke="${stroke}" stroke-width="1.5"/>
      <text x="20" y="18" text-anchor="middle" fill="white" font-size="9" font-weight="bold"
            font-family="sans-serif">${num}</text>
      <text x="20" y="26" text-anchor="middle" fill="white" font-size="7"
            font-family="sans-serif">${label}</text>
    </svg>`,
    width: 88, height: 88, anchorX: 44, anchorY: 44,
  };
}

/** SET_POI — purple marker with eye icon (48×48) */
function poiSpec(num: number, selected: boolean): WpIconSpec {
  const fill = selected ? '#ff4444' : '#8e44ad';
  const stroke = selected ? '#cc0000' : '#6c3483';
  return {
    svg: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" width="48" height="48">
      <circle cx="16" cy="16" r="13" fill="${fill}" stroke="${stroke}" stroke-width="2"/>
      <text x="16" y="13" text-anchor="middle" fill="white" font-size="10"
            font-family="sans-serif">👁</text>
      <text x="16" y="25" text-anchor="middle" fill="white" font-size="9" font-weight="bold"
            font-family="sans-serif">${num}</text>
    </svg>`,
    width: 48, height: 48, anchorX: 24, anchorY: 24,
  };
}

/** LAND — orange teardrop with down-arrow icon (48×66) */
function landSpec(selected: boolean): WpIconSpec {
  const fill = selected ? '#ff4444' : '#f39c12';
  const stroke = selected ? '#cc0000' : '#d68910';
  return {
    svg: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 44" width="48" height="66">
      <path d="M16 44 C16 44 2 24 2 16 A14 14 0 1 1 30 16 C30 24 16 44 16 44Z"
            fill="${fill}" stroke="${stroke}" stroke-width="2"/>
      <path d="M16 10 L16 25 M11 20 L16 25 L21 20" fill="none" stroke="white"
            stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"/>
    </svg>`,
    width: 48, height: 66, anchorX: 24, anchorY: 66,
  };
}

/** RTH — house icon (42×42) */
function rthSpec(selected: boolean): WpIconSpec {
  const fill = selected ? '#ff4444' : '#e67e22';
  return {
    svg: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" width="42" height="42">
      <path d="M16 4 L4 16 L8 16 L8 28 L24 28 L24 16 L28 16 Z" fill="${fill}" stroke="#7e5109" stroke-width="1.5"/>
      <text x="16" y="22" text-anchor="middle" fill="white" font-size="8" font-weight="bold"
            font-family="sans-serif">RTH</text>
    </svg>`,
    width: 42, height: 42, anchorX: 21, anchorY: 21,
  };
}

/** Generic fallback icon (48×48) */
function genericSpec(num: number, label: string, selected: boolean): WpIconSpec {
  const fill = selected ? '#ff4444' : '#7f8c8d';
  return {
    svg: `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" width="48" height="48">
      <circle cx="16" cy="16" r="13" fill="${fill}" stroke="#2c3e50" stroke-width="2"/>
      <text x="16" y="13" text-anchor="middle" fill="white" font-size="8"
            font-family="sans-serif">${label}</text>
      <text x="16" y="24" text-anchor="middle" fill="white" font-size="10" font-weight="bold"
            font-family="sans-serif">${num}</text>
    </svg>`,
    width: 48, height: 48, anchorX: 24, anchorY: 24,
  };
}

/** Pick the right icon SPEC for a waypoint (shared by 2D + 3D rendering). */
export function wpIconSpec(wp: Waypoint, displayNum: number, selected: boolean): WpIconSpec {
  switch (wp.action) {
    case WpAction.Waypoint:     return waypointSpec(displayNum, selected);
    case WpAction.PosholdUnlim: return posholdSpec(displayNum, null, selected);
    case WpAction.PosholdTime:  return posholdSpec(displayNum, wp.p1, selected);
    case WpAction.SetPoi:       return poiSpec(displayNum, selected);
    case WpAction.Land:         return landSpec(selected);
    case WpAction.Rth:          return rthSpec(selected);
    case WpAction.Jump:         return genericSpec(displayNum, 'JMP', selected);
    case WpAction.SetHead:      return genericSpec(displayNum, 'HDG', selected);
    default:                    return genericSpec(displayNum, '?', selected);
  }
}

/** 2D Leaflet divIcon for a waypoint (built from the shared spec). */
export function iconForWp(wp: Waypoint, displayNum: number, selected: boolean): L.DivIcon {
  const s = wpIconSpec(wp, displayNum, selected);
  return L.divIcon({
    className: 'mission-wp-icon',
    html: s.svg,
    iconSize: [s.width, s.height],
    iconAnchor: [s.anchorX, s.anchorY],
  });
}
