// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// RF link / radio-shadow analysis over a terrain profile.
//
// Per chart sample it computes an excess gain/loss in dB (signed, relative to free-space LOS) from
// terrain + frequency: geometric LOS occlusion, Fresnel / knife-edge diffraction (ITU-R P.526), and
// two-ray ground reflection. Terrain is sampled RADIALLY from the launch point in 1° azimuth bins
// (each bin's home→far radial sampled once, reused for every chart sample sharing that bearing) — so
// the backend cost is bounded by the number of distinct bearings, not the sample count.
//
// Phase 1: relative obstacle loss only (no link budget / RF power). Honest limits: near-field and
// antenna pattern/attitude are not modelled; two-ray null *depth* is approximate (lobe positions are
// the trustworthy part). See docs/active/RF_LINK_ANALYSIS.md.

import { invoke } from '@tauri-apps/api/core';
import { bearing, destinationPoint } from '$lib/utils/geo';
import type { ProfileData } from './terrainProfile';
import { PROFILE_SPACING_M } from './terrainProfile';

export type RfBand = '5800' | '2400' | '900' | '433';

const C = 299_792_458;
const BAND_HZ: Record<RfBand, number> = { '5800': 5.8e9, '2400': 2.4e9, '900': 0.9e9, '433': 0.433e9 };

/** Free-space wavelength (m) for a band. */
export function bandWavelengthM(band: RfBand): number {
  return C / BAND_HZ[band];
}

const RE = 6_371_000;          // earth radius (m)
const K_FACTOR = 4 / 3;        // standard-atmosphere effective earth (radio horizon)
const AE = RE * K_FACTOR;
const GCS_ANTENNA_M = 2;       // assumed GCS antenna height above ground (link-budget phase will expose)

/** Colour-scale floor (dB). Total loss at/below this renders fully red. */
export const RF_RED_DB = -18;
/** Excess loss assigned to a hard geometric block (≪ RF_RED_DB so it clamps to red). */
const BLOCKED_DB = -120;

export interface RfOptions {
  band: RfBand;
  /** Pure geometric line-of-sight occlusion (naïve). Ignored when `fresnel` is on. */
  los: boolean;
  /** Fresnel-zone / knife-edge diffraction loss (supersedes `los`). */
  fresnel: boolean;
  /** Two-ray ground-reflection lobing (signed: nulls and up to +6 dB peaks). */
  tworay: boolean;
  /** Clutter/vegetation height (m) added to bare terrain for the obstacle analysis (forest, small
   *  buildings). Has an outsized effect on obstacles near an endpoint (the knife-edge `1/d₁` term). */
  clutterM: number;
}

export interface RfField {
  /** Excess gain/loss (dB, signed; null = no data), aligned to `profile.terrain`. */
  db: (number | null)[];
  /** Min clearance (m) of the home→sample sightline above terrain (null = no data), aligned. */
  losClearance: (number | null)[];
}

export interface RfHome {
  lat: number;
  lon: number;
  /** Ground elevation at home (MSL). */
  ground: number;
}

interface RawSample {
  dist_m: number;
  lat: number;
  lon: number;
  elev_m: number | null;
}

/** A home→bearing radial terrain profile (elevation MSL at each step, voids forward-filled). */
interface Radial {
  step: number;        // spacing (m)
  elev: number[];      // elev[i] at distance i*step from home
}

/** ITU-R P.526 single knife-edge diffraction loss (dB, ≥ 0) for Fresnel parameter v. */
function knifeEdgeLossDb(v: number): number {
  if (v <= -0.78) return 0;
  return 6.9 + 20 * Math.log10(Math.sqrt((v - 0.1) ** 2 + 1) + v - 0.1);
}

/** Earth-curvature bulge (m) at distance d1 from home along a chord of length D (d2 = D − d1). */
function earthBulge(d1: number, d2: number): number {
  return (d1 * d2) / (2 * AE);
}

/**
 * Evaluate the obstacle terms (LOS block + diffraction) for one sample at distance `D` from home,
 * with the UAV at MSL altitude `uavAlt`, against a home→bearing radial. Returns the excess loss (dB,
 * ≤ 0) and the minimum sightline clearance (m) over the path.
 */
function evalObstacle(
  radial: Radial,
  D: number,
  homeAlt: number,
  uavAlt: number,
  lambda: number,
  clutterM: number,
): { diffractionDb: number; minClear: number; blocked: boolean } {
  const n = Math.max(1, Math.floor(D / radial.step));
  let blocked = false;
  let worstV = -Infinity;
  let minClear = Infinity;

  for (let i = 1; i < n; i++) {
    const d1 = i * radial.step;
    const d2 = D - d1;
    if (d2 <= 0) break;
    const ray = homeAlt + ((uavAlt - homeAlt) * d1) / D;          // straight chord
    const terr = (radial.elev[i] ?? radial.elev[radial.elev.length - 1] ?? 0) + clutterM + earthBulge(d1, d2);
    const clear = ray - terr;                                     // + = ray above terrain
    if (clear < minClear) minClear = clear;
    if (clear < 0) blocked = true;
    // knife-edge parameter (h = obstruction above the ray = −clearance)
    const v = -clear * Math.sqrt((2 / lambda) * (1 / d1 + 1 / d2));
    if (v > worstV) worstV = v;
  }
  if (!isFinite(minClear)) minClear = uavAlt - homeAlt;

  // Continuous knife-edge diffraction loss (negative dB) + whether the chord is geometrically blocked.
  return { diffractionDb: -knifeEdgeLossDb(worstV), minClear, blocked };
}

/** Two-ray ground-reflection excess gain/loss (dB, signed) — flat-earth far-field, Γ ≈ −1 (grazing). */
function twoRayDb(D: number, homeAlt: number, uavAlt: number, homeGround: number, uavGround: number, lambda: number): number {
  const hr = Math.max(0.5, homeAlt - homeGround + GCS_ANTENNA_M); // GCS antenna height above ground
  const ht = Math.max(0.5, uavAlt - uavGround);                   // UAV height above ground
  if (D <= 0) return 0;
  const pathDiff = (2 * hr * ht) / D;                             // direct vs ground-reflected
  const phi = (2 * Math.PI * pathDiff) / lambda;
  // |1 + Γ·e^{jφ}| with Γ = −1  →  |1 − e^{jφ}| = 2·|sin(φ/2)|
  const factor = 2 * Math.abs(Math.sin(phi / 2));
  if (factor <= 1e-3) return RF_RED_DB;                           // deep null → clamp to scale floor
  return Math.max(RF_RED_DB, 20 * Math.log10(factor));            // up to +6 dB; nulls toward −∞
}

/** Build a home→bearing radial terrain profile out to `maxDist` (one backend call). */
async function sampleRadial(home: RfHome, bearingDeg: number, maxDist: number): Promise<Radial> {
  const far = destinationPoint(home.lat, home.lon, bearingDeg, maxDist);
  const raw = await invoke<RawSample[]>('terrain_profile', {
    points: [[home.lat, home.lon], [far.lat, far.lon]],
    spacingM: PROFILE_SPACING_M,
  });
  // forward-fill voids so the radial is continuous; index i ↔ distance i*spacing
  const elev: number[] = [];
  let last = home.ground;
  for (const s of raw) {
    last = s.elev_m ?? last;
    elev.push(last);
  }
  if (elev.length === 0) elev.push(home.ground);
  return { step: PROFILE_SPACING_M, elev };
}

/** Map an excess dB value to a (pale, dark-background) green→yellow→red colour. */
export function rfColor(db: number | null): string {
  if (db == null) return 'transparent';
  // ≥ 0 dB (free space or constructive) = green; 0 → −24 dB ramps to red.
  const t = Math.max(0, Math.min(1, -db / -RF_RED_DB)); // 0 (green) … 1 (red)
  // green (90,170,70) → yellow (200,170,50) → red (200,60,50)
  let r: number, g: number, b: number;
  if (t < 0.5) {
    const u = t / 0.5;
    r = 90 + (200 - 90) * u;
    g = 170;
    b = 70 + (50 - 70) * u;
  } else {
    const u = (t - 0.5) / 0.5;
    r = 200;
    g = 170 + (60 - 170) * u;
    b = 50;
  }
  return `rgb(${Math.round(r)},${Math.round(g)},${Math.round(b)})`;
}

/**
 * Compute the RF excess-loss field + LOS clearance for a profile, sampling terrain radially from
 * `home` in 1° azimuth bins. Aligned to `profile.terrain`. Samples with no path/terrain → null.
 */
export async function computeRfField(
  profile: ProfileData,
  home: RfHome,
  opts: RfOptions,
): Promise<RfField> {
  const lambda = bandWavelengthM(opts.band);
  const t = profile.terrain;
  const n = t.length;
  const db: (number | null)[] = new Array(n).fill(null);
  const losClearance: (number | null)[] = new Array(n).fill(null);

  // Group sample indices by 1° azimuth bin from home; track the max distance per bin.
  const bins = new Map<number, { idx: number; dist: number; uavAlt: number }[]>();
  const maxDist = new Map<number, number>();
  for (let i = 0; i < n; i++) {
    const s = t[i];
    const uavAlt = profile.pathAtTerrain[i];
    if (s.cut || s.elev == null || uavAlt == null) continue;
    const D = haversineLocal(home.lat, home.lon, s.lat, s.lon);
    if (D < 1) continue; // at/inside home → trivially fine, leave null (near-field not modelled)
    const bin = Math.round(bearing(home.lat, home.lon, s.lat, s.lon)) % 360;
    const arr = bins.get(bin) ?? [];
    arr.push({ idx: i, dist: D, uavAlt });
    bins.set(bin, arr);
    maxDist.set(bin, Math.max(maxDist.get(bin) ?? 0, D));
  }

  // The radial terrain is needed whenever any method is on — even two-ray-only, since two-ray is a
  // LOS model and must be suppressed where the direct path is geometrically blocked.
  const wantRadial = opts.fresnel || opts.los || opts.tworay;
  for (const [bin, entries] of bins) {
    const radial = wantRadial ? await sampleRadial(home, bin, (maxDist.get(bin) as number) + PROFILE_SPACING_M) : null;
    for (const e of entries) {
      let total = 0;
      let clear: number | null = null;
      let blocked = false;
      if (radial) {
        // Raise the home/GCS endpoint by the clutter offset too: the operator launches from a clear
        // spot (antenna at/above local vegetation), not from the forest floor — otherwise the
        // clutter-raised terrain right beside the GCS would block every sample.
        const o = evalObstacle(radial, e.dist, home.ground + opts.clutterM, e.uavAlt, lambda, opts.clutterM);
        clear = o.minClear;
        blocked = o.blocked;
        if (opts.fresnel) {
          total += o.diffractionDb;              // realistic, continuous obstacle loss (covers blockage)
        } else if (opts.los && blocked) {
          total = BLOCKED_DB;                     // naïve binary block
        }
      }
      // Two-ray needs a direct ray to interfere with — only valid in clear line-of-sight. A
      // geometrically shadowed point has no path at all (the ground can't reflect *up to* a blocked
      // point), so it stays blocked → red, even in two-ray-only mode. Where Fresnel is on, its finite
      // diffraction loss already governs (don't override it with a hard block).
      if (opts.tworay) {
        if (blocked) {
          if (!opts.fresnel) total = BLOCKED_DB;
        } else {
          const uavGround = (t[e.idx].elev as number);
          total += twoRayDb(e.dist, home.ground, e.uavAlt, home.ground, uavGround, lambda);
        }
      }
      db[e.idx] = Math.max(BLOCKED_DB, total);
      losClearance[e.idx] = clear;
    }
  }
  return { db, losClearance };
}

const D2R = Math.PI / 180;
function haversineLocal(aLat: number, aLon: number, bLat: number, bLon: number): number {
  const dLat = (bLat - aLat) * D2R;
  const dLon = (bLon - aLon) * D2R;
  const h = Math.sin(dLat / 2) ** 2 + Math.cos(aLat * D2R) * Math.cos(bLat * D2R) * Math.sin(dLon / 2) ** 2;
  return 2 * RE * Math.asin(Math.min(1, Math.sqrt(h)));
}
