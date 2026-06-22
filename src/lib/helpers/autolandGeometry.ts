// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Fixed-wing autoland approach geometry (shared by the 2D + 3D map overlays). Reproduces the path the
// INAV configurator draws (tabs/mission_control.js: paintApproach / addFwApproach): for each active
// landing heading, a downwind+base pair (blue) into the final-approach line (orange). See
// docs/active/AUTOLAND_SAFEHOME.md.
//
// INAV heading encoding: positive = bidirectional (the opposite direction is also drawn, with the turn
// side flipped), negative = exclusive (that direction only), 0 = off. approachDirection: 0 = left, 1 = right.

import { destinationPoint, bearing } from '$lib/utils/geo';

export type LatLon = [number, number];

const LEFT = 0;
const RIGHT = 1;
const wrap360 = (d: number) => ((d % 360) + 360) % 360;

export interface ApproachInput {
  heading1: number;
  heading2: number;
  /** Global nav_fw_land_approach_length in metres. */
  approachLengthM: number;
  /** Global nav_fw_loiter_radius in metres. */
  loiterRadiusM: number;
  /** 0 = left turns, 1 = right turns. */
  approachDirection: number;
}

/** One drawn leg. `final` = the orange final-approach line; otherwise a blue downwind/base leg. */
export interface ApproachLeg {
  points: LatLon[];
  final: boolean;
}

/** Small arrowhead (two barbs) at the end of a leg, pointing along it. Inherits the leg's `final`. */
function arrowhead(from: LatLon, to: LatLon, final: boolean, sizeM: number): ApproachLeg {
  const brg = bearing(from[0], from[1], to[0], to[1]);
  const back = wrap360(brg + 180);
  const a1 = destinationPoint(to[0], to[1], wrap360(back + 24), sizeM);
  const a2 = destinationPoint(to[0], to[1], wrap360(back - 24), sizeM);
  return { points: [[a1.lat, a1.lon], to, [a2.lat, a2.lon]], final };
}

/** One approach (mirrors the configurator's paintApproach): land → pos2 → pos1 (blue), pos1 → land
 *  (orange final). pos1 = approachLength out along `bearing`; pos2 = pos1 offset perpendicular by
 *  max(loiter·4, approachLength/2) on the turn-direction side. */
function paintApproach(
  lat: number, lon: number, brg: number, approachDirection: number,
  approachLengthM: number, loiterRadiusM: number,
): ApproachLeg[] {
  const pos1 = destinationPoint(lat, lon, brg, approachLengthM);
  const perp = approachDirection === LEFT ? wrap360(brg + 90) : wrap360(brg - 90);
  const off = Math.max(loiterRadiusM * 4, approachLengthM / 2);
  const pos2 = destinationPoint(pos1.lat, pos1.lon, perp, off);
  const p1: LatLon = [pos1.lat, pos1.lon];
  const p2: LatLon = [pos2.lat, pos2.lon];
  const land: LatLon = [lat, lon];
  const barb = Math.max(8, approachLengthM * 0.05);
  return [
    { points: [land, p2], final: false },
    arrowhead(land, p2, false, barb),
    { points: [p2, p1], final: false },
    arrowhead(p2, p1, false, barb),
    { points: [p1, land], final: true },
    arrowhead(p1, land, true, barb),
  ];
}

/** Full planned approach path(s) for a safehome's fwapproach config. */
export function buildApproachGeometry(lat: number, lon: number, input: ApproachInput): ApproachLeg[] {
  const { heading1: h1, heading2: h2, approachLengthM, loiterRadiusM, approachDirection } = input;
  const len = Math.max(10, approachLengthM || 0);
  const flip = (d: number) => (d === LEFT ? RIGHT : LEFT);
  const out: ApproachLeg[] = [];
  // heading1
  if (h1 !== 0) out.push(...paintApproach(lat, lon, wrap360(Math.abs(h1) + 180), approachDirection, len, loiterRadiusM));
  if (h1 > 0) out.push(...paintApproach(lat, lon, wrap360(h1), flip(approachDirection), len, loiterRadiusM));
  // heading2
  if (h2 !== 0) out.push(...paintApproach(lat, lon, wrap360(Math.abs(h2) + 180), approachDirection, len, loiterRadiusM));
  if (h2 > 0) out.push(...paintApproach(lat, lon, wrap360(h2), flip(approachDirection), len, loiterRadiusM));
  return out;
}
