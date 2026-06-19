// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

/**
 * Per-waypoint detail lines — the single source of truth for "every parameter of this waypoint",
 * shared by the mission panel footer (selected WP) and the map hover tooltip (view mode). Building both
 * from one function keeps them consistent by construction (they used to drift: the footers listed full
 * params while the tooltips showed only name + altitude).
 *
 * One function per protocol since the parameter model differs (INAV WpAction + p1/p2/p3 vs ArduPilot
 * MAV_CMD + the declarative catalog). Each returns resolved `label · value` pairs; the caller renders
 * them (DOM rows in the panel, an HTML string in the Leaflet tooltip).
 */

import type { Waypoint } from '$lib/stores/mission';
import { WpAction, hasLocation } from '$lib/stores/mission';
import { isFlyByHome } from '$lib/helpers/missionGeometry';
import { convertAltitude, convertSpeed } from '$lib/utils/units';
import type { InterfaceSettings } from '$lib/stores/settings';
import type { ArduWaypoint } from '$lib/stores/missionArdupilot';
import { MAV_FRAME_GLOBAL, MAV_FRAME_GLOBAL_TERRAIN_ALT } from '$lib/stores/missionArdupilot';
import { cmdDef, cmdHasLocation, enumLabel } from '$lib/helpers/arduCommandCatalog';

export interface WpDetailLine { label: string; value: string; }

/** Minimal translate-function shape (svelte-i18n's `$t`); we only ever look up plain keys here. */
type TFn = (key: string) => string;

type Units = Pick<InterfaceSettings, 'altitudeUnit' | 'speedUnit'>;

// ── INAV ─────────────────────────────────────────────────────────────────────

function inavAltShort(wp: Waypoint, altitudeUnit: InterfaceSettings['altitudeUnit']): string {
  const m = wp.alt_mode ?? ((wp.p3 & 1) ? 1 : 0);
  const ref = m === 2 ? 'AGL' : m === 1 ? 'AMSL' : 'REL';
  const a = convertAltitude(wp.altitude / 100, altitudeUnit);
  return `${Math.round(a.value)}${a.unit} ${ref}`;
}

function inavSpeed(cms: number, speedUnit: InterfaceSettings['speedUnit']): string {
  const s = convertSpeed(cms / 100, speedUnit);
  return `${s.value.toFixed(1)} ${s.unit}`;
}

const coord = (e7: number) => (e7 / 1e7).toFixed(6);

/** All editable/displayed parameters of an INAV waypoint, mirroring the panel detail rows exactly. */
export function inavWpDetailLines(wp: Waypoint, t: TFn, units: Units): WpDetailLine[] {
  const lines: WpDetailLine[] = [];
  if (hasLocation(wp.action) && !isFlyByHome(wp)) {
    lines.push({ label: t('mission.lat'), value: coord(wp.lat) });
    lines.push({ label: t('mission.lon'), value: coord(wp.lon) });
  }
  lines.push({ label: t('mission.alt'), value: inavAltShort(wp, units.altitudeUnit) });

  if (wp.action === WpAction.Waypoint || wp.action === WpAction.Land) {
    lines.push({ label: t('mission.speed'), value: wp.p1 > 0 ? inavSpeed(wp.p1, units.speedUnit) : t('mission.speedDefault') });
  }
  if (wp.action === WpAction.PosholdTime) lines.push({ label: t('mission.hold'), value: `${wp.p1}s` });
  if (wp.action === WpAction.PosholdUnlim) lines.push({ label: t('mission.hold'), value: t('mission.holdUnlimited') });
  if (wp.action === WpAction.Rth) lines.push({ label: t('mission.land'), value: wp.p1 ? t('mission.landYes') : t('mission.landNoHover') });
  if (wp.action === WpAction.Jump) {
    lines.push({ label: t('mission.target'), value: `WP ${wp.p1}` });
    lines.push({ label: t('mission.repeat'), value: wp.p2 === -1 ? '∞' : String(wp.p2) });
  }
  if (wp.action === WpAction.SetHead) lines.push({ label: t('mission.heading'), value: wp.p1 === -1 ? t('mission.headingFree') : `${wp.p1}°` });

  lines.push({ label: t('mission.flag'), value: wp.flag === 0xa5 ? t('mission.flagLast') : wp.flag === 0x48 ? t('mission.flagFbh') : t('mission.flagNormal') });
  return lines;
}

// ── ArduPilot / PX4 ──────────────────────────────────────────────────────────

function arduFrameLabel(frame: number): string {
  if (frame === MAV_FRAME_GLOBAL) return 'AMSL';
  if (frame === MAV_FRAME_GLOBAL_TERRAIN_ALT) return 'TRN';
  return 'REL';
}

/** All parameters of an ArduPilot/PX4 waypoint: Lat/Lon/Alt for location commands plus every catalog
 *  param (param1..4, and 5/6/7 when a command repurposes them as data). Catalog labels are English
 *  (matching the rest of the ArduPilot UI); Lat/Lon/Alt use i18n. */
export function arduWpDetailLines(wp: ArduWaypoint, t: TFn): WpDetailLine[] {
  const lines: WpDetailLine[] = [];
  if (cmdHasLocation(wp.command)) {
    lines.push({ label: t('mission.lat'), value: coord(wp.lat) });
    lines.push({ label: t('mission.lon'), value: coord(wp.lon) });
    lines.push({ label: t('mission.alt'), value: `${wp.alt.toFixed(0)}m ${arduFrameLabel(wp.frame)}` });
  }
  const def = cmdDef(wp.command);
  if (def?.params) {
    const vals = [wp.param1, wp.param2, wp.param3, wp.param4, wp.lat, wp.lon, wp.alt];
    for (const pidx of [1, 2, 3, 4, 5, 6, 7] as const) {
      const spec = def.params[pidx];
      if (!spec) continue;
      const v = vals[pidx - 1];
      const value = spec.enumStrings && spec.enumValues ? enumLabel(spec, v) : `${v}${spec.units ? ' ' + spec.units : ''}`;
      lines.push({ label: spec.label, value });
    }
  }
  return lines;
}
