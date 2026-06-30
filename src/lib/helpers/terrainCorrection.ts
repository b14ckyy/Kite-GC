// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Terrain Correction engine for the Terrain Analysis panel (Waypoint mode).
//
// Pure functions over the already-sampled ProfileData. Two modes:
//   • Terrain Follow — set WPs to a target AGL, then lift legs to clear.
//   • Clearance Check — raise only, never lower; clear WPs then legs.
// A fixed-wing climb/descent-angle limit may be layered on top (raises the
// lower endpoint of any too-steep leg). All passes only ever *raise* altitude
// (except Terrain Follow's initial set-to-target), so the convergence loop is
// monotonic and bounded. Corrected waypoints are returned in AGL terms.
//
// Waypoint insertion is *not* automatic — the user adds WPs manually on the
// chart and re-runs the correction.

import { WpAction } from '$lib/stores/mission';
import type { ProfileData, TerrainSample } from './terrainProfile';

export type CorrectionMode = 'follow' | 'check';

export interface CorrectionParams {
  mode: CorrectionMode;
  groundClearance: number;
  /** Range by WP display number; 0 = auto (first / last). */
  rangeStart: number;
  rangeEnd: number;
  fixedWing: boolean;
  /** degrees, 0 = off */
  climbAngle: number;
  descentAngle: number;
}

export interface CorrectionChange {
  /** index into mission.waypoints */
  index: number;
  number: number;
  newAltMsl: number;
  /** AGL value to store (newAltMsl − ground), metres */
  aglValue: number;
}

export interface CorrectionResult {
  changes: CorrectionChange[];
  /** corrected altitude line (MSL) across the whole route, for the preview */
  previewPath: { dist: number; altMsl: number }[];
  changedCount: number;
  minClearanceAfter: number | null;
  /** the climb/descent limit forced WPs above the target clearance */
  climbForcedAboveClearance: boolean;
  /** a leg between two fixed anchors stayed below clearance (can't fix) */
  unresolvableLeg: boolean;
}

const MAX_ITER = 100;
const EPS = 0.01;

// One cell per unique mission WP. Jump revisits share a cell, so a WP that
// appears twice (its first-pass leg + the jump-back leg) keeps one altitude.
interface Cell {
  index: number;
  number: number;
  ground: number;
  alt: number;
  origAlt: number;
  correctable: boolean;
}

// A point along the expanded route (order incl. jump revisits).
interface Pt {
  dist: number;
  cell: Cell;
}

function isCorrectableAction(action: WpAction): boolean {
  return (
    action === WpAction.Waypoint ||
    action === WpAction.PosholdUnlim ||
    action === WpAction.PosholdTime
  );
}

function lowerBound(arr: TerrainSample[], x: number): number {
  let lo = 0;
  let hi = arr.length;
  while (lo < hi) {
    const m = (lo + hi) >> 1;
    if (arr[m].dist < x) lo = m + 1;
    else hi = m;
  }
  return lo;
}

/** Max clearance deficit along a leg: max(terrain + clearance − straight line). */
function legDeficit(
  terrain: TerrainSample[],
  dA: number,
  dB: number,
  altA: number,
  altB: number,
  clearance: number,
): number {
  const span = dB - dA || 1;
  let maxDef = 0;
  for (let i = lowerBound(terrain, dA); i < terrain.length && terrain[i].dist <= dB; i++) {
    const s = terrain[i];
    if (s.elev == null) continue;
    const line = altA + ((altB - altA) * (s.dist - dA)) / span;
    const def = s.elev + clearance - line;
    if (def > maxDef) maxDef = def;
  }
  return maxDef;
}

/** Raise a leg's endpoints so the straight line clears terrain + clearance. */
function raiseLeg(
  terrain: TerrainSample[],
  pa: Pt,
  pb: Pt,
  clearance: number,
): { changed: boolean; unresolvable: boolean } {
  const a = pa.cell;
  const b = pb.cell;
  const def = legDeficit(terrain, pa.dist, pb.dist, a.alt, b.alt, clearance);
  if (def <= EPS) return { changed: false, unresolvable: false };

  if (a.correctable && b.correctable) {
    a.alt += def;
    b.alt += def;
    return { changed: true, unresolvable: false };
  }

  const span = pb.dist - pa.dist || 1;
  // Only ONE endpoint can move. The per-sample requirement `(T+c)·span / leverage` diverges for terrain
  // near the FIXED endpoint (its leverage on the movable one → 0), which once raised a WP to ~12 km / a
  // 230 m spike. Cap the movable endpoint at `max-terrain-on-leg + clearance` — raising it higher can't
  // help (the line near the fixed endpoint is governed by that fixed altitude). If a sample still
  // demanded more, the leg can't be cleared by moving one end → flag it unresolvable (the user raises
  // the fixed WP, widens the range, or inserts a WP).
  if (a.correctable && !b.correctable) {
    // raise A so the line from A→B(fixed) clears: altA' ≥ ((T+c)(dB−dA) − altB(s−dA)) / (dB−s)
    let req = a.alt;
    let maxTerr = -Infinity;
    for (let i = lowerBound(terrain, pa.dist); i < terrain.length && terrain[i].dist < pb.dist; i++) {
      const s = terrain[i];
      if (s.elev == null) continue;
      if (s.elev > maxTerr) maxTerr = s.elev;
      const denom = pb.dist - s.dist;
      if (s.dist <= pa.dist + EPS || denom <= EPS) continue;
      const need = ((s.elev + clearance) * span - b.alt * (s.dist - pa.dist)) / denom;
      if (need > req) req = need;
    }
    const cap = Math.max(a.alt, maxTerr + clearance);
    let blocked = false;
    if (req > cap + EPS) { req = cap; blocked = true; }
    if (req > a.alt + EPS) {
      a.alt = req;
      return { changed: true, unresolvable: blocked };
    }
    return { changed: false, unresolvable: blocked };
  }

  if (b.correctable && !a.correctable) {
    // symmetric: altB' ≥ ((T+c)(dB−dA) − altA(dB−s)) / (s−dA)
    let req = b.alt;
    let maxTerr = -Infinity;
    for (let i = lowerBound(terrain, pa.dist); i < terrain.length && terrain[i].dist < pb.dist; i++) {
      const s = terrain[i];
      if (s.elev == null) continue;
      if (s.elev > maxTerr) maxTerr = s.elev;
      const denom = s.dist - pa.dist;
      if (s.dist >= pb.dist - EPS || denom <= EPS) continue;
      const need = ((s.elev + clearance) * span - a.alt * (pb.dist - s.dist)) / denom;
      if (need > req) req = need;
    }
    const cap = Math.max(b.alt, maxTerr + clearance);
    let blocked = false;
    if (req > cap + EPS) { req = cap; blocked = true; }
    if (req > b.alt + EPS) {
      b.alt = req;
      return { changed: true, unresolvable: blocked };
    }
    return { changed: false, unresolvable: blocked };
  }

  // both anchors → cannot fix
  return { changed: false, unresolvable: true };
}

/** Enforce the climb/descent-angle limit by raising the lower endpoint. */
function anglePass(pa: Pt, pb: Pt, climbAngle: number, descentAngle: number): boolean {
  const d = pb.dist - pa.dist;
  if (d <= 0) return false;
  const a = pa.cell;
  const b = pb.cell;
  const dAlt = b.alt - a.alt;
  let changed = false;
  if (dAlt > 0 && climbAngle > 0) {
    const maxRise = Math.tan((climbAngle * Math.PI) / 180) * d;
    if (dAlt > maxRise + EPS && a.correctable) {
      a.alt = b.alt - maxRise;
      changed = true;
    }
  } else if (dAlt < 0 && descentAngle > 0) {
    const maxDrop = Math.tan((descentAngle * Math.PI) / 180) * d;
    if (-dAlt > maxDrop + EPS && b.correctable) {
      b.alt = a.alt - maxDrop;
      changed = true;
    }
  }
  return changed;
}

export function computeCorrection(data: ProfileData, params: CorrectionParams): CorrectionResult {
  const terrain = data.terrain;
  const markers = data.markers;
  const { mode, groundClearance: clr, fixedWing } = params;
  const climbAngle = fixedWing ? params.climbAngle : 0;
  const descentAngle = fixedWing ? params.descentAngle : 0;

  // Resolve the WP-number range (0 = auto)
  let lo = params.rangeStart;
  let hi = params.rangeEnd;
  if (markers.length > 0) {
    if (lo <= 0) lo = Math.min(...markers.map((m) => m.number));
    if (hi <= 0) hi = Math.max(...markers.map((m) => m.number));
  }

  // One cell per unique WP (jump revisits share it); `pts` is the route order.
  const cells = new Map<number, Cell>();
  const pts: Pt[] = [];
  for (const m of markers) {
    let cell = cells.get(m.index);
    if (!cell) {
      const inRange = m.number >= lo && m.number <= hi;
      const correctable = isCorrectableAction(m.action) && inRange && m.ground != null;
      cell = {
        index: m.index,
        number: m.number,
        ground: m.ground ?? m.altMsl,
        alt: m.altMsl,
        origAlt: m.altMsl,
        correctable,
      };
      cells.set(m.index, cell);
    }
    pts.push({ dist: m.dist, cell });
  }

  // A leg is "real" unless a jump cut falls between its endpoints
  const cuts = data.cuts;
  const isCut = (a: number, b: number) => cuts.some((c) => c > a && c < b);

  // Terrain Follow: set every correctable WP to the target clearance AGL
  if (mode === 'follow') {
    for (const c of cells.values()) if (c.correctable) c.alt = c.ground + clr;
  }

  // Convergence loop (monotonic raises)
  let unresolvableLeg = false;
  for (let iter = 0; iter < MAX_ITER; iter++) {
    let changed = false;

    // WP clearance (Clearance Check raises below-clearance WPs; Follow already at target)
    for (const c of cells.values()) {
      if (c.correctable && c.alt < c.ground + clr - EPS) {
        c.alt = c.ground + clr;
        changed = true;
      }
    }

    // Leg clearance (skip cut legs)
    for (let i = 0; i < pts.length - 1; i++) {
      if (isCut(pts[i].dist, pts[i + 1].dist)) continue;
      const r = raiseLeg(terrain, pts[i], pts[i + 1], clr);
      if (r.changed) changed = true;
      if (r.unresolvable) unresolvableLeg = true;
    }

    // Climb/descent angle limit
    if (climbAngle > 0 || descentAngle > 0) {
      for (let i = 0; i < pts.length - 1; i++) {
        if (isCut(pts[i].dist, pts[i + 1].dist)) continue;
        if (anglePass(pts[i], pts[i + 1], climbAngle, descentAngle)) changed = true;
      }
    }

    if (!changed) break;
  }

  // Detect whether the angle limit pushed any WP above the target clearance
  let climbForcedAboveClearance = false;
  for (const c of cells.values()) {
    if (c.correctable && c.alt > c.ground + clr + EPS) {
      climbForcedAboveClearance = true;
      break;
    }
  }

  // Collect changes (unique WPs)
  const changes: CorrectionChange[] = [];
  for (const c of cells.values()) {
    if (c.correctable && Math.abs(c.alt - c.origAlt) > EPS) {
      changes.push({ index: c.index, number: c.number, newAltMsl: c.alt, aglValue: c.alt - c.ground });
    }
  }

  // Preview path (incl. jump revisits) + min clearance after (over real legs)
  const previewPath = pts.map((p) => ({ dist: p.dist, altMsl: p.cell.alt }));
  let minClearanceAfter: number | null = null;
  for (let i = 0; i < pts.length - 1; i++) {
    const a = pts[i];
    const b = pts[i + 1];
    if (isCut(a.dist, b.dist)) continue;
    if (!a.cell.correctable && !b.cell.correctable) continue;
    const span = b.dist - a.dist || 1;
    for (let k = lowerBound(terrain, a.dist); k < terrain.length && terrain[k].dist <= b.dist; k++) {
      const s = terrain[k];
      if (s.elev == null) continue;
      const line = a.cell.alt + ((b.cell.alt - a.cell.alt) * (s.dist - a.dist)) / span;
      const c = line - s.elev;
      if (minClearanceAfter == null || c < minClearanceAfter) minClearanceAfter = c;
    }
  }

  return {
    changes,
    previewPath,
    changedCount: changes.length,
    minClearanceAfter,
    climbForcedAboveClearance,
    unresolvableLeg,
  };
}
