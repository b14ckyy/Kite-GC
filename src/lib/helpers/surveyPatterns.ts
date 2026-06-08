// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

import type { RectanglePatternParams, CirclePatternParams, PolygonPatternParams } from '$lib/stores/surveyPattern.svelte';

/** Degrees, { lat, lng } pair */
export interface LngLat {
  lat: number;
  lng: number;
}

export type AltMode = 'relative' | 'amsl' | 'ground';

export interface RectangleCorners {
  corners: LngLat[];
  center: LngLat;
}

// ──────────────────────────────────────────────────────
//  LOCAL COORDINATE HELPERS
// ──────────────────────────────────────────────────────

function toRad(deg: number) { return (deg * Math.PI) / 180; }

export function getOrientationVectors(orientationDeg: number) {
  const rad = toRad(orientationDeg);
  return {
    along: { x: Math.sin(rad), y: Math.cos(rad) },  // forward direction
    perp:  { x: Math.cos(rad), y: -Math.sin(rad) }, // right direction
  };
}

/** Convert local ENU offsets (east, north) to lat/lng */
function localToLatLng(off: { x: number; y: number }, center: LngLat): LngLat {
  const lat = center.lat + (off.y / 111320);
  const lng = center.lng + (off.x / (111320 * Math.cos((center.lat * Math.PI) / 180)));
  return { lat, lng };
}

/** Convert lat/lng delta to local meters relative to center */
export function latLngToLocalMeters(
  point: LngLat, center: LngLat, orientationDeg: number
): { x: number; y: number } {
  const { along, perp } = getOrientationVectors(orientationDeg);
  const dLat = (point.lat - center.lat) * 111320;
  const dLng = (point.lng - center.lng) * 111320 * Math.cos((center.lat * Math.PI) / 180);
  return {
    x: dLng * perp.x + dLat * perp.y,
    y: dLng * along.x + dLat * along.y,
  };
}

// ──────────────────────────────────────────────────────
//  RECTANGLE CORNERS
// ──────────────────────────────────────────────────────

export function computeRectangleCorners(
  center: LngLat, length: number, width: number, orientationDeg: number
): RectangleCorners {
  const rad = toRad(orientationDeg);
  const halfLen = length / 2;
  const halfWid = width / 2;
  const cosH = Math.cos(rad), sinH = Math.sin(rad);

  const offsets = [
    { x: -halfWid, y: -halfLen },  // 0: rear-left
    { x:  halfWid, y: -halfLen },  // 1: rear-right
    { x:  halfWid, y:  halfLen },  // 2: front-right
    { x: -halfWid, y:  halfLen },  // 3: front-left
  ];

  const corners = offsets.map(o => {
    // Use same rotation convention as getOrientationVectors:
    // perp = (cos, -sin), along = (sin, cos)
    // east = x*perp.x + y*along.x = x*cos + y*sin
    // north = x*perp.y + y*along.y = -x*sin + y*cos
    const east  = o.x * cosH + o.y * sinH;
    const north = -o.x * sinH + o.y * cosH;
    return localToLatLng({ x: east, y: north }, center);
  });

  return { corners, center };
}

// ──────────────────────────────────────────────────────
//  SURVEY WAYPOINT TYPE
// ──────────────────────────────────────────────────────

export interface SurveyWaypoint {
  lat: number;
  lng: number;
  alt: number;
  speed?: number;
  altMode?: AltMode;
  userActionFlags?: number;
}

/**
 * A survey path segment for preview display.
 * surveyTrack = inside the shape (main survey line)
 * turnTrack   = connection between survey tracks (outside shape)
 */
export interface SurveyPathSegment {
  kind: 'survey' | 'turn';
  points: SurveyWaypoint[];
}

/**
 * Generate a rectangle zigzag survey pattern.
 *
 * Tracks are always generated parallel to the shape's length axis first,
 * then rotated to match the target orientation.
 *
 * When trackOrientationEnabled: tracks rotated to trackOrientation,
 * then clipped against shape boundary. Spacing is absolute.
 *
 * When disabled: tracks follow shapeOrientation (parallel to shape length).
 *
 * turnDistance extends the exit side of each survey track.
 * reverse swaps start/end without changing flight direction.
 */
export function generateRectangleZigzag(
  params: RectanglePatternParams
): SurveyPathSegment[] {
  const {
    center, length, width, shapeOrientation, baseAltitude, baseSpeed,
    targetLineSpacing, turnDistance, reverse,
    trackOrientationEnabled, trackOrientation,
    altMode = 'relative',
    userActionLineStartFlags = 0,
    userActionLineEndFlags = 0,
  } = params;

  if (length <= 0 || width <= 0 || targetLineSpacing <= 0) return [];

  // When track orientation = shape orientation (default), use the simple fast path
  if (!trackOrientationEnabled) {
    return generateClassicZigzag(center, length, width, shapeOrientation,
      baseAltitude, baseSpeed, targetLineSpacing, turnDistance, reverse,
      altMode, userActionLineStartFlags, userActionLineEndFlags);
  }

  // ── Track Orientation enabled: rotate tracks within shape ──

  // Shape corners and edges in world coords
  const shapeCorners = computeRectangleCorners(center, length, width, shapeOrientation).corners;
  const shapeEdges: Array<[LngLat, LngLat]> = [];
  for (let i = 0; i < 4; i++) {
    shapeEdges.push([shapeCorners[i], shapeCorners[(i + 1) % 4]]);
  }

  // Project shape corners onto axis perpendicular to track orientation
  // to determine how to distribute tracks across the shape
  const trackRad = toRad(trackOrientation);
  const perpDir: [number, number] = [Math.cos(trackRad), -Math.sin(trackRad)];

  const projs = shapeCorners.map(c => {
    const dLng = (c.lng - center.lng) * 111320 * Math.cos((center.lat * Math.PI) / 180);
    const dLat = (c.lat - center.lat) * 111320;
    return dLng * perpDir[0] + dLat * perpDir[1];
  });
  const perpMin = Math.min(...projs);
  const perpMax = Math.max(...projs);
  const perpSpan = perpMax - perpMin;

  const trackNum = Math.max(1, Math.ceil(perpSpan / targetLineSpacing));
  const trackSpacing = targetLineSpacing;
  const totalSpan = (trackNum - 1) * trackSpacing;
  const startP = -totalSpan / 2;

  // Track direction vector (world coords, meters)
  const trackDir: [number, number] = [Math.sin(trackRad), Math.cos(trackRad)];
  const farDist = Math.max(length, width) * 2;

  // Line-line intersection helper
  function intersect(p1: LngLat, p2: LngLat, p3: LngLat, p4: LngLat): LngLat | null {
    const x1 = p1.lng, y1 = p1.lat, x2 = p2.lng, y2 = p2.lat;
    const x3 = p3.lng, y3 = p3.lat, x4 = p4.lng, y4 = p4.lat;
    const denom = (x1 - x2) * (y3 - y4) - (y1 - y2) * (x3 - x4);
    if (Math.abs(denom) < 1e-12) return null;
    const t = ((x1 - x3) * (y3 - y4) - (y1 - y3) * (x3 - x4)) / denom;
    if (t < 0 || t > 1) return null;
    const u = -((x1 - x2) * (y1 - y3) - (y1 - y2) * (x1 - x3)) / denom;
    if (u < 0 || u > 1) return null;
    return { lng: x1 + t * (x2 - x1), lat: y1 + t * (y2 - y1) };
  }

  // Clip a line to the shape, returns entry and exit points
  function clip(a: LngLat, b: LngLat): { start: LngLat; end: LngLat } | null {
    const pts: LngLat[] = [];
    for (const [e1, e2] of shapeEdges) {
      const pt = intersect(a, b, e1, e2);
      if (pt) pts.push(pt);
    }
    if (pts.length < 2) return null;
    const dx = b.lng - a.lng, dy = b.lat - a.lat, d2 = dx * dx + dy * dy || 1;
    pts.sort((p, q) => {
      const tp = ((p.lng - a.lng) * dx + (p.lat - a.lat) * dy) / d2;
      const tq = ((q.lng - a.lng) * dx + (q.lat - a.lat) * dy) / d2;
      return tp - tq;
    });
    return { start: pts[0], end: pts[pts.length - 1] };
  }

  const segments: SurveyPathSegment[] = [];
  const scaleLng = 111320 * Math.cos((center.lat * Math.PI) / 180);
  const makeWp = (p: LngLat, flags: number): SurveyWaypoint => ({ ...p, alt: baseAltitude, speed: baseSpeed, altMode, userActionFlags: flags });

  // Pre-compensate flags for reversal so they land on the correct waypoints after reverse()
  const effLineStartFlags = reverse ? userActionLineEndFlags : userActionLineStartFlags;
  const effLineEndFlags   = reverse ? userActionLineStartFlags : userActionLineEndFlags;

  for (let i = 0; i < trackNum; i++) {
    const offset = (trackNum === 1) ? 0 : startP + i * trackSpacing;
    const forward = (i % 2 === 0);

    // Base point on perp axis (meters from center)
    const bLng = offset * perpDir[0];
    const bLat = offset * perpDir[1];

    // Two points far along track direction
    const p1: LngLat = {
      lng: center.lng + (bLng - trackDir[0] * farDist) / scaleLng,
      lat: center.lat + (bLat - trackDir[1] * farDist) / 111320,
    };
    const p2: LngLat = {
      lng: center.lng + (bLng + trackDir[0] * farDist) / scaleLng,
      lat: center.lat + (bLat + trackDir[1] * farDist) / 111320,
    };

    const clipped = forward ? clip(p1, p2) : clip(p2, p1);
    if (!clipped) continue;

    // Extend exit by turnDistance
    const ex = clipped.end.lng - clipped.start.lng;
    const ey = clipped.end.lat - clipped.start.lat;
    const eLen = Math.sqrt(ex * ex + ey * ey) || 1;
    const extEnd: LngLat = {
      lng: clipped.end.lng + (ex / eLen) * (turnDistance / scaleLng),
      lat: clipped.end.lat + (ey / eLen) * (turnDistance / 111320),
    };

    segments.push({
      kind: 'survey',
      points: [makeWp(clipped.start, effLineStartFlags), makeWp(extEnd, effLineEndFlags)],
    });

    // Turn track
    if (i < trackNum - 1) {
      const nextOffset = startP + (i + 1) * trackSpacing;
      const nf = ((i + 1) % 2 === 0);
      const nbLng = nextOffset * perpDir[0];
      const nbLat = nextOffset * perpDir[1];

      const np1: LngLat = {
        lng: center.lng + (nbLng - trackDir[0] * farDist) / scaleLng,
        lat: center.lat + (nbLat - trackDir[1] * farDist) / 111320,
      };
      const np2: LngLat = {
        lng: center.lng + (nbLng + trackDir[0] * farDist) / scaleLng,
        lat: center.lat + (nbLat + trackDir[1] * farDist) / 111320,
      };

      const nextClip = nf ? clip(np1, np2) : clip(np2, np1);
      if (!nextClip) continue;

      segments.push({
        kind: 'turn',
        points: [makeWp(extEnd, 0), makeWp(nextClip.start, 0)],
      });
    }
  }

  if (reverse) {
    segments.reverse();
    for (const seg of segments) seg.points.reverse();
  }

  return segments;
}

/**
 * Classic zigzag: tracks parallel to shape length, first/last track on shape boundary.
 * This is the fast path used when trackOrientation is disabled.
 */
function generateClassicZigzag(
  center: LngLat, length: number, width: number, orientation: number,
  baseAltitude: number, baseSpeed: number,
  lineSpacing: number, turnDist: number, rev: boolean,
  altMode: AltMode = 'relative',
  lineStartFlags: number = 0,
  lineEndFlags: number = 0
): SurveyPathSegment[] {
  const { along, perp } = getOrientationVectors(orientation);
  const halfLen = length / 2;
  const halfWid = width / 2;
  const numTracks = Math.max(1, Math.ceil(width / lineSpacing));
  const actualSpacing = width / Math.max(1, numTracks - 1);

  const makeWp = (x: number, y: number, flags: number): SurveyWaypoint => ({
    ...localToLatLng({ x, y }, center),
    alt: baseAltitude, speed: baseSpeed, altMode, userActionFlags: flags,
  });

  // When reversed, what will be the first point in flight order was originally the last —
  // swap start/end flags so they end up on the correct waypoints after reversal.
  const effStartFlags = rev ? lineEndFlags  : lineStartFlags;
  const effEndFlags   = rev ? lineStartFlags : lineEndFlags;

  const segments: SurveyPathSegment[] = [];

  for (let i = 0; i < numTracks; i++) {
    const crossOffset = (numTracks === 1) ? 0 : -halfWid + i * actualSpacing;
    const baseX = crossOffset * perp.x;
    const baseY = crossOffset * perp.y;
    const forward = (i % 2 === 0);

    const innerStart = {
      x: baseX + (forward ? -halfLen : +halfLen) * along.x,
      y: baseY + (forward ? -halfLen : +halfLen) * along.y,
    };
    const innerEnd = {
      x: baseX + (forward ? +halfLen : -halfLen) * along.x,
      y: baseY + (forward ? +halfLen : -halfLen) * along.y,
    };

    const extEnd = {
      x: innerEnd.x + (forward ? turnDist : -turnDist) * along.x,
      y: innerEnd.y + (forward ? turnDist : -turnDist) * along.y,
    };

    segments.push({
      kind: 'survey',
      points: [makeWp(innerStart.x, innerStart.y, effStartFlags), makeWp(extEnd.x, extEnd.y, effEndFlags)],
    });

    if (i < numTracks - 1) {
      const nc = -halfWid + (i + 1) * actualSpacing;
      const nf = ((i + 1) % 2 === 0);
      const nbx = nc * perp.x, nby = nc * perp.y;
      const nextStart = {
        x: nbx + (nf ? -halfLen : +halfLen) * along.x,
        y: nby + (nf ? -halfLen : +halfLen) * along.y,
      };

      segments.push({
        kind: 'turn',
        points: [makeWp(extEnd.x, extEnd.y, 0), makeWp(nextStart.x, nextStart.y, 0)],
      });
    }
  }

  if (rev) {
    segments.reverse();
    for (const seg of segments) seg.points.reverse();
  }

  return segments;
}

// ──────────────────────────────────────────────────────
//  RECTANGLE LAWNMOWER (CONTOUR-OFFSET)
// ──────────────────────────────────────────────────────

/**
 * Generate a rectangle lawnmower (contour-offset) pattern.
 *
 * Flies concentric rectangles, each shrunk by lineSpacing from the previous.
 *
 *   CW (clockwise): A[front-right] → A[front-left] → A[rear-left] → A[rear-right]{short}
 *                    → B[front-right] → B[front-left] → B[rear-left] → B[rear-right]{short}
 *                    → C[front-right] → ...
 *
 *   CCW (counter-clockwise): A[front-left] → A[front-right] → A[rear-right] → A[rear-left]{short}
 *                             → B[front-left] → B[front-right] → B[rear-right] → B[rear-left]{short}
 *                             → C[front-left] → ...
 *
 * The 4th corner of each layer is shortened (moved toward the 3rd corner by lineSpacing),
 * so the turn transitions cleanly to the next inner rectangle (slight diagonal across
 * the short side). This saves one waypoint per layer with minimal area loss.
 *
 * When shape can no longer fit another full loop, a final single track
 * is placed along whichever axis still exceeds lineSpacing.
 *
 * Returns a single survey segment containing all waypoints in flight order.
 *
 * reverse: flips the final waypoint order (outside→inside vs inside→outside).
 * clockwise: determines CW vs CCW traversal direction.
 */
export function generateRectangleLawnmower(
  params: RectanglePatternParams
): SurveyPathSegment[] {
    const {
    center, length, width, shapeOrientation, baseAltitude, baseSpeed,
    targetLineSpacing, reverse, clockwise = true,
    altMode = 'relative',
    userActionStartFlags = 0,
    userActionTrackFlags = 0,
    userActionEndFlags = 0,
  } = params;

  if (length <= 0 || width <= 0 || targetLineSpacing <= 0) return [];

  const toWorld = (x: number, y: number): LngLat => {
    const rad = toRad(shapeOrientation);
    const east  = x * Math.cos(rad) + y * Math.sin(rad);
    const north = -x * Math.sin(rad) + y * Math.cos(rad);
    return localToLatLng({ x: east, y: north }, center);
  };

    const makeWp = (p: LngLat, flags: number): SurveyWaypoint => ({
    ...p, alt: baseAltitude, speed: baseSpeed, altMode, userActionFlags: flags,
  });

  //   0=rear-left, 1=rear-right, 2=front-right, 3=front-left
  // Flight order for each layer:
  //   CW:  2(fr) → 3(fl) → 0(rl) → 1(rr)  — 4th corner shortened
  //   CCW: 3(fl) → 2(fr) → 1(rr) → 0(rl)  — 4th corner shortened
    const orderCW  = [2, 3, 0, 1]; // fr → fl → rl → rr
  const orderCCW = [3, 2, 1, 0]; // fl → fr → rr → rl
  let baseOrder = clockwise ? orderCW : orderCCW;

  // Rotate so startCorner (1-4) is the first visited corner
  const sc = Math.max(0, Math.min(3, (params.startCorner ?? 1) - 1));
  const order = [...baseOrder.slice(sc), ...baseOrder.slice(0, sc)];

  let curLen = length;
  let curWid = width;
  let layerIdx = 0;

  // Build flat list of { pt, flags } in flight order
  const allPts: Array<{ pt: LngLat; flags: number }> = [];

  while (curLen > targetLineSpacing && curWid > targetLineSpacing) {
    const halfLen = curLen / 2;
    const halfWid = curWid / 2;

    // 4 corners in local frame
    const c = [
      { x: -halfWid, y: -halfLen },  // 0: rear-left
      { x:  halfWid, y: -halfLen },  // 1: rear-right
      { x:  halfWid, y:  halfLen },  // 2: front-right
      { x: -halfWid, y:  halfLen },  // 3: front-left
    ];

        // All 4 corners of this layer in flight order
        for (let j = 0; j < 4; j++) {
          const pt = toWorld(c[order[j]].x, c[order[j]].y);
          // Flags assigned later in bulk (start/track/end)
          allPts.push({ pt, flags: 0 });
        }

    // Shrink for next layer
    curLen -= 2 * targetLineSpacing;
    curWid -= 2 * targetLineSpacing;
    layerIdx++;
  }

  // ── Final track ──
  if (curLen > targetLineSpacing || curWid > targetLineSpacing) {
    const halfLen = curLen / 2;
    const halfWid = curWid / 2;

    let sPt, ePt;
    if (curLen > curWid) {
      sPt = { x: 0, y: -halfLen };
      ePt = { x: 0, y:  halfLen };
    } else {
      sPt = { x: -halfWid, y: 0 };
      ePt = { x:  halfWid, y: 0 };
    }
    allPts.push({ pt: toWorld(sPt.x, sPt.y), flags: 0 });
    allPts.push({ pt: toWorld(ePt.x, ePt.y), flags: 0 });
  }

      if (allPts.length < 2) return [];

  if (reverse) allPts.reverse();

  // Apply user action flags in final flight order (after reverse)
  const totalWp = allPts.length;
  allPts[0].flags = userActionStartFlags;
  allPts[totalWp - 1].flags = userActionEndFlags;
  for (let i = 1; i < totalWp - 1; i++) allPts[i].flags = userActionTrackFlags;

  const segments: SurveyPathSegment[] = [];
  segments.push({
    kind: 'survey',
    points: allPts.map(p => makeWp(p.pt, p.flags)),
  });

  return segments;
}

// ──────────────────────────────────────────────────────
//  INTERACTIVE EDITING HELPERS
// ──────────────────────────────────────────────────────

export function updateRectangleFromDraggedCorner(
  current: RectanglePatternParams,
  cornerIndex: 0 | 1 | 2 | 3,
  newCorner: LngLat
): Partial<RectanglePatternParams> {
  const { center, shapeOrientation, length, width } = current;
  const orig = computeRectangleCorners(center, length, width, shapeOrientation).corners;
  const anchorIdx = (cornerIndex + 2) % 4;
  const anchor = orig[anchorIdx];
  const newCenter: LngLat = {
    lat: (anchor.lat + newCorner.lat) / 2,
    lng: (anchor.lng + newCorner.lng) / 2,
  };
  const localAnchor = latLngToLocalMeters(anchor, newCenter, shapeOrientation);
  const localDrag = latLngToLocalMeters(newCorner, newCenter, shapeOrientation);
  const newHalfWid = Math.max(10, Math.abs(localDrag.x - localAnchor.x) / 2);
  const newHalfLen = Math.max(10, Math.abs(localDrag.y - localAnchor.y) / 2);
  return { center: newCenter, length: newHalfLen * 2, width: newHalfWid * 2 };
}

export function updateRectangleFromDraggedCenter(
  current: RectanglePatternParams,
  newCenter: LngLat
): Partial<RectanglePatternParams> {
  return { center: { ...newCenter } };
}

// ──────────────────────────────────────────────────────
//  CIRCLE STEPPED PATTERN
// ──────────────────────────────────────────────────────

/**
 * Generate a stepped-circle (concentric rings) survey pattern.
 *
 * Rings are spaced by `targetLineSpacing` from the outermost ring (radius) inward.
 * Each ring uses up to `ringPoints` waypoints distributed evenly around the circle.
 * For small inner rings the point count is auto-reduced so no arc segment is shorter
 * than `targetLineSpacing`.
 *
 * `trackOrientation` sets the compass bearing (0 = North, CW) of the first waypoint
 * on every ring. `clockwise` controls the orbit direction. `reverse` flips
 * outside→inside to inside→outside.
 *
 * User action flags: `userActionStartFlags` on the very first WP,
 * `userActionEndFlags` on the very last WP, `userActionTrackFlags` on all others.
 */
export function generateCircleStepped(
  params: CirclePatternParams
): SurveyPathSegment[] {
  const {
    center, radius, baseAltitude, baseSpeed,
    targetLineSpacing, reverse = false, clockwise = true,
    altMode = 'relative',
    trackOrientation = 0,
    ringPoints = 10,
    userActionStartFlags = 0,
    userActionTrackFlags = 0,
    userActionEndFlags = 0,
  } = params;

  if (radius <= 0 || targetLineSpacing <= 0) return [];

  // Coordinate scale: 1 metre → lat/lng increment
  const scaleLat = 111320;
  const scaleLng = 111320 * Math.cos((center.lat * Math.PI) / 180);

  // Place a point on a ring at a given mathematical angle (0 = East, CCW positive)
  const toWorld = (r: number, angleRad: number): LngLat => ({
    lat: center.lat + (r * Math.sin(angleRad)) / scaleLat,
    lng: center.lng + (r * Math.cos(angleRad)) / scaleLng,
  });

  const makeWp = (p: LngLat, flags: number): SurveyWaypoint => ({
    ...p, alt: baseAltitude, speed: baseSpeed, altMode, userActionFlags: flags,
  });

  // Convert compass bearing (0 = North, CW) → math angle (0 = East, CCW)
  const startAngle = (Math.PI / 2) - (trackOrientation * Math.PI) / 180;
  // CW flight = decreasing math angle; CCW = increasing
  const direction = clockwise ? -1 : 1;

  const numRings = Math.max(1, Math.ceil(radius / targetLineSpacing));
  const allPts: Array<{ pt: LngLat; flags: number }> = [];

  for (let ring = 0; ring < numRings; ring++) {
    const ringRadius = radius - ring * targetLineSpacing;
    if (ringRadius <= 0) break;

    const circumference = 2 * Math.PI * ringRadius;
    let numPts = ringPoints;
    // Reduce until arc distance between consecutive points >= targetLineSpacing
    while (numPts > 3 && circumference / numPts < targetLineSpacing) {
      numPts--;
    }

    // If the minimum of 3 points still produces arcs shorter than the spacing,
    // the circle is too small to continue — add a single center point and stop.
    if (circumference / numPts < targetLineSpacing) {
      allPts.push({ pt: center, flags: 0 });
      break;
    }

    const angleStep = (2 * Math.PI) / numPts;
    for (let i = 0; i < numPts; i++) {
      const angle = startAngle + direction * i * angleStep;
      allPts.push({ pt: toWorld(ringRadius, angle), flags: 0 });
    }
  }

  if (allPts.length < 2) return [];

  if (reverse) allPts.reverse();

  // Assign user action flags in final flight order (after reverse)
  const total = allPts.length;
  allPts[0].flags = userActionStartFlags;
  allPts[total - 1].flags = userActionEndFlags;
  for (let i = 1; i < total - 1; i++) allPts[i].flags = userActionTrackFlags;

  return [{ kind: 'survey', points: allPts.map(p => makeWp(p.pt, p.flags)) }];
}

// ──────────────────────────────────────────────────────
//  SPIRAL PATTERN (Archimedean)
// ──────────────────────────────────────────────────────

/**
 * Generate an Archimedean spiral survey pattern.
 *
 * The spiral winds inward from `radius` to the center. Radius decreases
 * linearly with the total angle turned: r(θ) = radius - (θ/2π) × targetLineSpacing.
 *
 * **Outer phase** (arc ≥ targetLineSpacing): fixed angular step = 360°/ringPoints.
 * Waypoints are evenly distributed on 360°, getting geometrically denser inward.
 *
 * **Inner phase** (arc < targetLineSpacing): angular step is widened to
 * targetLineSpacing/r so every consecutive arc remains ≥ targetLineSpacing.
 *
 * **Stop condition**: the interior angle at the second-to-last waypoint
 * (angle at P_{n-2} between vectors to P_{n-3} and P_{n-1}) drops below 45°,
 * meaning the required turn exceeds 135° — impractical for fixed-wing UAVs.
 *
 * `trackOrientation` sets the compass bearing (0 = North, CW) of the first waypoint.
 * `clockwise` controls inward winding direction. `reverse` flips to inside→out.
 */
export function generateSpiral(
  params: CirclePatternParams
): SurveyPathSegment[] {
  const {
    center, radius, baseAltitude, baseSpeed,
    targetLineSpacing, reverse = false, clockwise = true,
    altMode = 'relative',
    trackOrientation = 0,
    ringPoints = 10,
    userActionStartFlags = 0,
    userActionTrackFlags = 0,
    userActionEndFlags = 0,
  } = params;

  if (radius <= 0 || targetLineSpacing <= 0) return [];

  const scaleLat = 111320;
  const scaleLng = 111320 * Math.cos((center.lat * Math.PI) / 180);

  const toWorld = (r: number, angleRad: number): LngLat => ({
    lat: center.lat + (r * Math.sin(angleRad)) / scaleLat,
    lng: center.lng + (r * Math.cos(angleRad)) / scaleLng,
  });

  const makeWp = (p: LngLat, flags: number): SurveyWaypoint => ({
    ...p, alt: baseAltitude, speed: baseSpeed, altMode, userActionFlags: flags,
  });

  // Convert compass bearing (0 = North, CW) → math angle (0 = East, CCW)
  const startAngle = (Math.PI / 2) - (trackOrientation * Math.PI) / 180;
  const direction = clockwise ? -1 : 1;  // CW = decreasing math angle

  // Base angular step for the outer phase (e.g. 36° for ringPoints=10)
  const baseAngleStep = (2 * Math.PI) / ringPoints;

  const allPts: Array<{ pt: LngLat; flags: number }> = [];

  let θ = startAngle;    // current math angle
  let totalTurned = 0;   // cumulative angle (always positive, used for radius calc)

  // Stop when the UAV turn at the middle waypoint would exceed 60°
  // (interior angle < 120° → turn = 180° − 120° = 60°)
  const STOP_INTERIOR_DEG = 120;
  const MAX_PTS = 10000;

  for (let iter = 0; iter < MAX_PTS; iter++) {
    const r = radius - (totalTurned / (2 * Math.PI)) * targetLineSpacing;
    if (r <= 0) break;

    const newPt = toWorld(r, θ);

    // Minimum distance guard: if the arc to this point is shorter than lineSpacing,
    // discard it and stop — the spiral has wound tight enough.
    if (allPts.length > 0) {
      const prev = allPts[allPts.length - 1].pt;
      const dLat = (newPt.lat - prev.lat) * scaleLat;
      const dLng = (newPt.lng - prev.lng) * scaleLng;
      if (Math.sqrt(dLat * dLat + dLng * dLng) < targetLineSpacing) break;
    }

    allPts.push({ pt: newPt, flags: 0 });

    // Stop condition: interior angle at the middle of the last 3 points drops below
    // STOP_INTERIOR_DEG — the UAV turn angle (180° − interior) would exceed 60°.
    if (allPts.length >= 3) {
      const n = allPts.length;
      const pa = allPts[n - 3].pt;
      const pb = allPts[n - 2].pt;
      const pc = allPts[n - 1].pt;

      const v1lat = pa.lat - pb.lat, v1lng = pa.lng - pb.lng;
      const v2lat = pc.lat - pb.lat, v2lng = pc.lng - pb.lng;
      const dot = v1lat * v2lat + v1lng * v2lng;
      const l1 = Math.sqrt(v1lat ** 2 + v1lng ** 2);
      const l2 = Math.sqrt(v2lat ** 2 + v2lng ** 2);
      if (l1 > 0 && l2 > 0) {
        const interior = Math.acos(Math.max(-1, Math.min(1, dot / (l1 * l2)))) * 180 / Math.PI;
        if (interior < STOP_INTERIOR_DEG) break;
      }
    }

    // Compute next angular step
    const arcAtBase = r * baseAngleStep;
    const dθ = arcAtBase >= targetLineSpacing
      ? baseAngleStep               // outer phase: fixed step
      : targetLineSpacing / r;      // inner phase: widen step to maintain arc = lineSpacing

    θ += direction * dθ;
    totalTurned += dθ;
  }

  if (allPts.length < 1) return [];

  // Always terminate at the exact center point
  allPts.push({ pt: center, flags: 0 });

  if (reverse) allPts.reverse();

  // Assign user action flags in final flight order (after reverse)
  const total = allPts.length;
  allPts[0].flags = userActionStartFlags;
  allPts[total - 1].flags = userActionEndFlags;
  for (let i = 1; i < total - 1; i++) allPts[i].flags = userActionTrackFlags;

  return [{ kind: 'survey', points: allPts.map(p => makeWp(p.pt, p.flags)) }];
}

// ──────────────────────────────────────────────────────

// ──────────────────────────────────────────────────────
//  POLYGON HELPERS
// ──────────────────────────────────────────────────────

/** Average of all vertex positions — used as centroid marker and coordinate origin. */
export function polygonCentroid(points: LngLat[]): LngLat {
  const n = points.length;
  return {
    lat: points.reduce((s, p) => s + p.lat, 0) / n,
    lng: points.reduce((s, p) => s + p.lng, 0) / n,
  };
}

/** True if any two non-adjacent polygon edges intersect (simple O(n²) check). */
export function isPolygonSelfIntersecting(points: LngLat[]): boolean {
  const n = points.length;
  if (n < 4) return false; // triangle can't self-intersect

  // Segment intersection test (excluding shared endpoints)
  function cross2d(ax: number, ay: number, bx: number, by: number): number {
    return ax * by - ay * bx;
  }
  function segsIntersect(p1: LngLat, p2: LngLat, p3: LngLat, p4: LngLat): boolean {
    const d1x = p2.lng - p1.lng, d1y = p2.lat - p1.lat;
    const d2x = p4.lng - p3.lng, d2y = p4.lat - p3.lat;
    const denom = cross2d(d1x, d1y, d2x, d2y);
    if (Math.abs(denom) < 1e-12) return false; // parallel
    const dx = p3.lng - p1.lng, dy = p3.lat - p1.lat;
    const t = cross2d(dx, dy, d2x, d2y) / denom;
    const u = cross2d(dx, dy, d1x, d1y) / denom;
    return t > 0 && t < 1 && u > 0 && u < 1; // strict interior — no endpoint coincidences
  }

  for (let i = 0; i < n; i++) {
    for (let j = i + 2; j < n; j++) {
      if (i === 0 && j === n - 1) continue; // wrap-around adjacent edges
      if (segsIntersect(points[i], points[(i + 1) % n], points[j], points[(j + 1) % n])) {
        return true;
      }
    }
  }
  return false;
}

// ──────────────────────────────────────────────────────
//  POLYGON ZIGZAG PATTERN
// ──────────────────────────────────────────────────────

/**
 * Generate a zigzag survey pattern for an arbitrary (possibly concave) polygon.
 *
 * Two concave-handling modes (params.stayInsideArea):
 *
 * false (cross gaps) — classic serpentine: flies all segments of each scan
 *                line in order, crossing any gaps within the polygon (good for
 *                area photography where UAV actions trigger at each crossing).
 *
 * true (stay inside / connected fill) — DFS-based connected sweep (like
 *                3D-printer infill): traverses spatially adjacent segments
 *                across scan lines, staying within connected sub-regions. For
 *                a U-shape this produces: left arm → bottom → right arm with
 *                only one cross-gap transition.
 *
 * `trackOrientation` rotates the SCAN LINES, not the polygon shape.
 * Intersection counting uses a half-open interval (t ∈ (0, 1]).
 */
export function generatePolygonZigzag(
  params: PolygonPatternParams
): SurveyPathSegment[] {
  const {
    points, baseAltitude, baseSpeed,
    targetLineSpacing, turnDistance = 0, reverse = false,
    trackOrientation = 0,
    altMode = 'relative',
    stayInsideArea = false,
    userActionLineStartFlags = 0,
    userActionLineEndFlags = 0,
  } = params;

  if (points.length < 3 || targetLineSpacing <= 0) return [];

  const centroid = polygonCentroid(points);
  const { along, perp } = getOrientationVectors(trackOrientation);

  // Local frame: x = perp to track (right), y = along track
  const local = points.map(p => latLngToLocalMeters(p, centroid, trackOrientation));

  const xs = local.map(p => p.x);
  const xMin = Math.min(...xs);
  const xMax = Math.max(...xs);
  const perpSpan = xMax - xMin;
  if (perpSpan <= 0) return [];

  const numTracks = Math.max(1, Math.ceil(perpSpan / targetLineSpacing));
  const totalSpan = (numTracks - 1) * targetLineSpacing;
  const xStart = (xMin + xMax) / 2 - totalSpan / 2;

  const n = local.length;
  const edges = local.map((p, i) => ({
    x0: p.x, y0: p.y,
    x1: local[(i + 1) % n].x, y1: local[(i + 1) % n].y,
  }));

  // Convert track-frame (lx=perp, ly=along) back to lat/lng via ENU rotation.
  // Bug fix: localToLatLng treats x=East, y=North — but lx/ly are in track frame,
  // not ENU. We must unrotate first.
  const makeWp = (lx: number, ly: number, flags: number): SurveyWaypoint => {
    const east  = lx * perp.x + ly * along.x;
    const north = lx * perp.y + ly * along.y;
    return {
      ...localToLatLng({ x: east, y: north }, centroid),
      alt: baseAltitude, speed: baseSpeed, altMode, userActionFlags: flags,
    };
  };

  const effStartFlags = reverse ? userActionLineEndFlags : userActionLineStartFlags;
  const effEndFlags   = reverse ? userActionLineStartFlags : userActionLineEndFlags;

  function scanIntersections(xScan: number): number[] {
    const ys: number[] = [];
    for (const { x0, y0, x1, y1 } of edges) {
      const dx = x1 - x0;
      if (Math.abs(dx) < 1e-10) continue;
      const t = (xScan - x0) / dx;
      if (t <= 0 || t > 1) continue; // half-open interval (0, 1]
      ys.push(y0 + t * (y1 - y0));
    }
    return ys.sort((a, b) => a - b);
  }

  const allIntersections = Array.from({ length: numTracks }, (_, k) =>
    scanIntersections(xStart + k * targetLineSpacing)
  );

  const segments: SurveyPathSegment[] = [];

  if (!stayInsideArea) {
    // ── Classic serpentine — cross any intra-line gaps ──────────────────────
    for (let k = 0; k < numTracks; k++) {
      const ys = allIntersections[k];
      if (ys.length < 2) continue;

      const xScan = xStart + k * targetLineSpacing;
      const forward = k % 2 === 0;

      const pairs: Array<[number, number]> = [];
      for (let i = 0; i + 1 < ys.length; i += 2) pairs.push([ys[i], ys[i + 1]]);
      const ordered = forward ? pairs : [...pairs].reverse().map(([a, b]) => [b, a] as [number, number]);

      for (let s = 0; s < ordered.length; s++) {
        const [yA, yB] = ordered[s];
        const dir = yB > yA ? 1 : -1;
        // Turn distance only applies before a real turn (the move to the next
        // scan line). Intermediate segments continue collinearly across a gap on
        // the same scan line — no turn there, so no extension (and the end-flag
        // trigger stays at the true boundary crossing instead of being delayed).
        const isLastOnLine = s === ordered.length - 1;
        const yExt = isLastOnLine ? yB + dir * turnDistance : yB;

        segments.push({
          kind: 'survey',
          points: [makeWp(xScan, yA, effStartFlags), makeWp(xScan, yExt, effEndFlags)],
        });

        if (s + 1 < ordered.length) {
          segments.push({
            kind: 'turn',
            points: [makeWp(xScan, yExt, 0), makeWp(xScan, ordered[s + 1][0], 0)],
          });
        }
      }

      if (k + 1 < numTracks) {
        const nextYs = allIntersections[k + 1];
        if (nextYs.length >= 2) {
          const nextForward = (k + 1) % 2 === 0;
          const nextFirst = nextForward ? nextYs[0] : nextYs[nextYs.length - 1];
          const lastPt = segments[segments.length - 1].points[segments[segments.length - 1].points.length - 1];
          segments.push({ kind: 'turn', points: [lastPt, makeWp(xStart + (k + 1) * targetLineSpacing, nextFirst, 0)] });
        }
      }
    }

  } else {
    // ── Connected fill — DFS with LIFO, stays within connected sub-regions ──
    // Each segment is a node; adjacent segments on consecutive scan lines that
    // overlap in Y are connected. DFS naturally traverses: left arm → bottom
    // → right arm for U-shapes, with only one cross-gap jump per disconnection.

    type Seg = { k: number; a: number; b: number; visited: boolean };
    const allSegs: Seg[] = [];
    for (let k = 0; k < numTracks; k++) {
      const ys = allIntersections[k];
      for (let i = 0; i + 1 < ys.length; i += 2) {
        allSegs.push({ k, a: ys[i], b: ys[i + 1], visited: false });
      }
    }
    if (allSegs.length === 0) return segments;

    function yOverlap(s1: Seg, s2: Seg): boolean {
      return Math.max(s1.a, s1.b) > Math.min(s2.a, s2.b) &&
             Math.max(s2.a, s2.b) > Math.min(s1.a, s1.b);
    }

    let prevExit: SurveyWaypoint | null = null;

    function visit(seg: Seg) {
      if (seg.visited) return;
      seg.visited = true;

      const xScan = xStart + seg.k * targetLineSpacing;
      const forward = seg.k % 2 === 0;
      const [yA, yB] = forward ? [seg.a, seg.b] : [seg.b, seg.a];
      const dir = yB > yA ? 1 : -1;
      const yExt = yB + dir * turnDistance;

      const entryWp = makeWp(xScan, yA, effStartFlags);
      const exitWp  = makeWp(xScan, yExt, effEndFlags);

      if (prevExit) {
        segments.push({ kind: 'turn', points: [prevExit, entryWp] });
      }
      segments.push({ kind: 'survey', points: [entryWp, exitWp] });
      prevExit = exitWp;

      // Push k−1 adjacents FIRST (LIFO ⇒ processed after k+1), then k+1 last
      const stack: Seg[] = [];
      for (const s of allSegs) {
        if (!s.visited && s.k === seg.k - 1 && yOverlap(seg, s)) stack.push(s);
      }
      for (const s of allSegs) {
        if (!s.visited && s.k === seg.k + 1 && yOverlap(seg, s)) stack.push(s);
      }
      // Process stack (LIFO: last pushed = next processed)
      while (stack.length > 0) {
        const next = stack.pop()!;
        if (!next.visited) {
          // Recursion-safe: add k−1 and k+1 adjacents of `next` via the outer loop
          visit(next);
        }
      }
    }

    // Process all segments, starting new DFS trees when disconnected components exist
    for (const seg of [...allSegs].sort((a, b) => a.k !== b.k ? a.k - b.k : Math.min(a.a, a.b) - Math.min(b.a, b.b))) {
      if (!seg.visited) visit(seg);
    }
  }

  if (reverse) {
    segments.reverse();
    for (const seg of segments) seg.points.reverse();
  }

  return segments;
}

// ──────────────────────────────────────────────────────
//  POLYGON LAWNMOWER (convex decomposition + contour offset)
// ──────────────────────────────────────────────────────

interface Pt { x: number; y: number; }

function signedAreaXY(poly: Pt[]): number {
  let a = 0;
  for (let i = 0; i < poly.length; i++) {
    const j = (i + 1) % poly.length;
    a += poly[i].x * poly[j].y - poly[j].x * poly[i].y;
  }
  return a / 2;
}

/** Strict (interior) segment intersection — shared endpoints don't count. */
function segProperIntersectXY(p1: Pt, p2: Pt, p3: Pt, p4: Pt): boolean {
  const d1x = p2.x - p1.x, d1y = p2.y - p1.y;
  const d2x = p4.x - p3.x, d2y = p4.y - p3.y;
  const denom = d1x * d2y - d1y * d2x;
  if (Math.abs(denom) < 1e-12) return false;
  const dx = p3.x - p1.x, dy = p3.y - p1.y;
  const t = (dx * d2y - dy * d2x) / denom;
  const u = (dx * d1y - dy * d1x) / denom;
  return t > 1e-9 && t < 1 - 1e-9 && u > 1e-9 && u < 1 - 1e-9;
}

function pointInPolyXY(pt: Pt, poly: Pt[]): boolean {
  let inside = false;
  for (let i = 0, j = poly.length - 1; i < poly.length; j = i++) {
    const xi = poly[i].x, yi = poly[i].y, xj = poly[j].x, yj = poly[j].y;
    const intersect = (yi > pt.y) !== (yj > pt.y) &&
      pt.x < ((xj - xi) * (pt.y - yi)) / (yj - yi) + xi;
    if (intersect) inside = !inside;
  }
  return inside;
}

/** CCW polygon — interior is left of each directed edge; reflex if right turn. */
function isReflexAtXY(poly: Pt[], i: number): boolean {
  const n = poly.length;
  const prev = poly[(i - 1 + n) % n], cur = poly[i], next = poly[(i + 1) % n];
  const cross = (cur.x - prev.x) * (next.y - cur.y) - (cur.y - prev.y) * (next.x - cur.x);
  return cross < 0;
}

function isConvexPolyXY(poly: Pt[]): boolean {
  for (let i = 0; i < poly.length; i++) if (isReflexAtXY(poly, i)) return false;
  return true;
}

/** Is the diagonal (i,j) a valid internal diagonal of the simple polygon? */
function validDiagonalXY(poly: Pt[], i: number, j: number): boolean {
  const n = poly.length;
  if (i === j || j === (i + 1) % n || j === (i - 1 + n) % n) return false;
  const a = poly[i], b = poly[j];
  // No proper intersection with any edge
  for (let k = 0; k < n; k++) {
    const k2 = (k + 1) % n;
    if (k === i || k === j || k2 === i || k2 === j) continue;
    if (segProperIntersectXY(a, b, poly[k], poly[k2])) return false;
  }
  // Midpoint must lie inside the polygon (rejects external diagonals across concavities)
  const mid: Pt = { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
  return pointInPolyXY(mid, poly);
}

/** Recursively split a simple polygon into convex pieces at reflex vertices. */
function decomposeConvexXY(input: Pt[]): Pt[][] {
  if (input.length < 3) return [];
  // Normalise to CCW
  let poly = signedAreaXY(input) < 0 ? [...input].reverse() : input;
  if (isConvexPolyXY(poly)) return [poly];

  const n = poly.length;
  for (let i = 0; i < n; i++) {
    if (!isReflexAtXY(poly, i)) continue;
    // Find the best diagonal from this reflex vertex — prefer connecting to another reflex
    let best = -1, bestReflex = false;
    for (let k = 0; k < n; k++) {
      if (!validDiagonalXY(poly, i, k)) continue;
      const rf = isReflexAtXY(poly, k);
      if (best < 0 || (rf && !bestReflex)) { best = k; bestReflex = rf; }
    }
    if (best >= 0) {
      const lo = Math.min(i, best), hi = Math.max(i, best);
      const partA = poly.slice(lo, hi + 1);
      const partB = [...poly.slice(hi), ...poly.slice(0, lo + 1)];
      return [...decomposeConvexXY(partA), ...decomposeConvexXY(partB)];
    }
  }
  return [poly]; // fallback — no valid diagonal found (shouldn't happen for simple polygons)
}

function centroidXY(poly: Pt[]): Pt {
  let x = 0, y = 0;
  for (const p of poly) { x += p.x; y += p.y; }
  return { x: x / poly.length, y: y / poly.length };
}

function ptEqXY(p: Pt, q: Pt): boolean {
  return Math.abs(p.x - q.x) < 1e-6 && Math.abs(p.y - q.y) < 1e-6;
}

function ensureCCW(poly: Pt[]): Pt[] {
  return signedAreaXY(poly) < 0 ? [...poly].reverse() : poly;
}

/** If A and B share an edge and their union is convex, return the merged polygon. */
function tryMergeXY(A: Pt[], B: Pt[]): Pt[] | null {
  const nA = A.length, nB = B.length;
  for (let i = 0; i < nA; i++) {
    const a1 = A[i], a2 = A[(i + 1) % nA];
    for (let j = 0; j < nB; j++) {
      const b1 = B[j], b2 = B[(j + 1) % nB];
      if (ptEqXY(a1, b2) && ptEqXY(a2, b1)) {
        // Shared edge a1→a2 == b2→b1. Stitch A (from a2 around to a1) + B (b2 around to b1).
        const Arot: Pt[] = [];
        for (let k = 0; k < nA; k++) Arot.push(A[(i + 1 + k) % nA]); // a2 … a1
        const Brot: Pt[] = [];
        for (let k = 0; k < nB; k++) Brot.push(B[(j + 1 + k) % nB]); // b2 … b1
        const merged = ensureCCW([...Arot, ...Brot.slice(1, nB - 1)]);
        return isConvexPolyXY(merged) ? merged : null;
      }
    }
  }
  return null;
}

/** Hertel-Mehlhorn-style merge: combine adjacent convex pieces whose union stays convex. */
function mergeConvexPiecesXY(pieces: Pt[][]): Pt[][] {
  let list = pieces.slice();
  let didMerge = true;
  while (didMerge) {
    didMerge = false;
    outer:
    for (let i = 0; i < list.length; i++) {
      for (let j = i + 1; j < list.length; j++) {
        const m = tryMergeXY(list[i], list[j]);
        if (m) {
          list = [m, ...list.filter((_, k) => k !== i && k !== j)];
          didMerge = true;
          break outer;
        }
      }
    }
  }
  return list;
}

/** Drop vertices that would create edges shorter than minLen (incl. the closing edge). */
function removeShortEdgesXY(ring: Pt[], minLen: number): Pt[] {
  if (ring.length < 3) return ring;
  const out: Pt[] = [ring[0]];
  for (let i = 1; i < ring.length; i++) {
    const last = out[out.length - 1];
    if (Math.hypot(ring[i].x - last.x, ring[i].y - last.y) >= minLen) out.push(ring[i]);
  }
  while (out.length >= 3 && Math.hypot(out[out.length - 1].x - out[0].x, out[out.length - 1].y - out[0].y) < minLen) {
    out.pop();
  }
  return out;
}

/** Clip a convex polygon to the half-plane { P : (P - O)·N >= 0 } (Sutherland-Hodgman). */
function clipHalfPlaneXY(poly: Pt[], ox: number, oy: number, nx: number, ny: number): Pt[] {
  const res: Pt[] = [];
  const m = poly.length;
  for (let i = 0; i < m; i++) {
    const P = poly[i], Q = poly[(i + 1) % m];
    const dP = (P.x - ox) * nx + (P.y - oy) * ny;
    const dQ = (Q.x - ox) * nx + (Q.y - oy) * ny;
    if (dP >= 0) res.push(P);
    if ((dP >= 0) !== (dQ >= 0)) {
      const t = dP / (dP - dQ);
      res.push({ x: P.x + t * (Q.x - P.x), y: P.y + t * (Q.y - P.y) });
    }
  }
  return res;
}

/**
 * Offset a convex CCW polygon inward by d via half-plane intersection.
 * Robust by construction: clipping a convex polygon against half-planes can
 * never self-intersect, and collapsed edges naturally disappear. Returns null
 * when the polygon collapses (no area left).
 */
function offsetConvexInwardXY(poly: Pt[], d: number): Pt[] | null {
  let out = poly;
  const n = poly.length;
  for (let i = 0; i < n; i++) {
    const a = poly[i], b = poly[(i + 1) % n];
    let dx = b.x - a.x, dy = b.y - a.y;
    const len = Math.hypot(dx, dy);
    if (len < 1e-9) continue;
    dx /= len; dy /= len;
    const nx = -dy, ny = dx;            // inward normal (left side for CCW)
    const ox = a.x + nx * d, oy = a.y + ny * d; // edge line shifted inward by d
    out = clipHalfPlaneXY(out, ox, oy, nx, ny);
    if (out.length < 3) return null;
  }
  if (signedAreaXY(out) <= 1e-6) return null;
  return out;
}

/**
 * Approximate the medial spine of a convex polygon: offset it inward until it
 * nearly collapses, then take the two farthest points of the residual sliver.
 * Returns the spine endpoints, or null if the polygon collapses to ~a point.
 */
function spineOfConvexXY(ring: Pt[]): [Pt, Pt] | null {
  const c = centroidXY(ring);
  let maxR = 0;
  for (const p of ring) maxR = Math.max(maxR, Math.hypot(p.x - c.x, p.y - c.y));
  let lo = 0, hi = maxR;
  let best: Pt[] | null = null;
  for (let it = 0; it < 24; it++) {
    const mid = (lo + hi) / 2;
    const off = offsetConvexInwardXY(ring, mid);
    if (off) { lo = mid; best = off; } else hi = mid;
  }
  if (!best || best.length < 2) return null;
  let a = best[0], b = best[0], d2 = -1;
  for (let i = 0; i < best.length; i++) {
    for (let j = i + 1; j < best.length; j++) {
      const dd = (best[i].x - best[j].x) ** 2 + (best[i].y - best[j].y) ** 2;
      if (dd > d2) { d2 = dd; a = best[i]; b = best[j]; }
    }
  }
  return [a, b];
}

/**
 * Generate a polygon lawnmower (contour-offset) pattern.
 *
 * The (possibly concave) polygon is first split into convex pieces. Each convex
 * piece is covered by concentric inward-offset rings (convex pieces only ever
 * shrink — they never pinch off into islands). Rings are flown outer → inner,
 * each as a closed loop, connected to the next inner ring with a short connector.
 * Pieces are flown sequentially with travel legs between them.
 *
 * Stop criterion per piece: when an inner ring's shortest edge would fall below
 * targetLineSpacing (same idea as the rectangle lawnmower).
 *
 * `reverse` flips the whole flight order.
 */
export function generatePolygonLawnmower(
  params: PolygonPatternParams
): SurveyPathSegment[] {
  const {
    points, baseAltitude, baseSpeed,
    targetLineSpacing, reverse = false,
    altMode = 'relative',
    userActionStartFlags = 0,
    userActionTrackFlags = 0,
    userActionEndFlags = 0,
  } = params;

  if (points.length < 3 || targetLineSpacing <= 0) return [];

  const centroid = polygonCentroid(points);
  const sLat = 111320, sLng = 111320 * Math.cos((centroid.lat * Math.PI) / 180);
  const toXY = (p: LngLat): Pt => ({ x: (p.lng - centroid.lng) * sLng, y: (p.lat - centroid.lat) * sLat });
  const toLL = (q: Pt): LngLat => ({ lat: centroid.lat + q.y / sLat, lng: centroid.lng + q.x / sLng });

  // Convex decomposition, then merge adjacent pieces back into larger convex
  // polygons (fewer triangles — two triangles forming a convex quad re-merge).
  const pieces = mergeConvexPiecesXY(decomposeConvexXY(points.map(toXY)));

  // Collect rings per zone (one zone = one convex piece). Offset until the piece
  // collapses, simplify away sub-spacing edges, and capture a final spine track.
  interface Zone { rings: LngLat[][]; spine: [LngLat, LngLat] | null; }
  const zones: Zone[] = [];
  for (const piece of pieces) {
    let cur: Pt[] = ensureCCW(piece);
    const rawRings: Pt[][] = [];
    let guard = 0;
    while (cur.length >= 3 && guard++ < 500) {
      rawRings.push(cur);
      const next = offsetConvexInwardXY(cur, targetLineSpacing);
      if (!next) break;
      cur = next;
    }
    if (rawRings.length === 0) continue;

    const rings: LngLat[][] = [];
    for (const ring of rawRings) {
      const cleaned = removeShortEdgesXY(ring, targetLineSpacing);
      if (cleaned.length >= 3) rings.push(cleaned.map(toLL));
    }
    // Final centre track: a single line along the medial spine of the innermost
    // ring — only if it still spans > lineSpacing. Round remainders collapse to
    // ~a point → no extra waypoint.
    let spine: [LngLat, LngLat] | null = null;
    const sp = spineOfConvexXY(rawRings[rawRings.length - 1]);
    if (sp) {
      const len = Math.hypot(sp[0].x - sp[1].x, sp[0].y - sp[1].y);
      if (len >= targetLineSpacing) spine = [toLL(sp[0]), toLL(sp[1])];
    }
    if (rings.length === 0 && !spine) continue;
    zones.push({ rings, spine });
  }
  if (zones.length === 0) return [];

  const makeWp = (p: LngLat, flags: number): SurveyWaypoint => ({
    ...p, alt: baseAltitude, speed: baseSpeed, altMode, userActionFlags: flags,
  });
  const nearestIdx = (ring: LngLat[], ref: { lat: number; lng: number }): number => {
    let best = Infinity, idx = 0;
    for (let k = 0; k < ring.length; k++) {
      const dl = ring[k].lat - ref.lat, dg = ring[k].lng - ref.lng;
      const dd = dl * dl + dg * dg;
      if (dd < best) { best = dd; idx = k; }
    }
    return idx;
  };

  const segments: SurveyPathSegment[] = [];
  let prevEnd: SurveyWaypoint | null = null;

  for (const zone of zones) {
    // Each zone is ONE continuous survey path. Rings are flown open; the entry
    // into each inner ring is offset by one vertex past the nearest one, so the
    // ring-to-ring step is a diagonal (no perpendicular hop, no re-flown point).
    const zonePts: SurveyWaypoint[] = [];
    for (const ring of zone.rings) {
      let startIdx = 0;
      if (zonePts.length > 0) {
        startIdx = (nearestIdx(ring, zonePts[zonePts.length - 1]) + 1) % ring.length;
      } else if (prevEnd) {
        startIdx = nearestIdx(ring, prevEnd); // first ring: short entry from previous zone
      }
      const ordered = [...ring.slice(startIdx), ...ring.slice(0, startIdx)];
      for (const p of ordered) zonePts.push(makeWp(p, userActionTrackFlags));
    }
    if (zone.spine) {
      let sp = zone.spine;
      if (zonePts.length > 0) {
        const last = zonePts[zonePts.length - 1];
        const d0 = (sp[0].lat - last.lat) ** 2 + (sp[0].lng - last.lng) ** 2;
        const d1 = (sp[1].lat - last.lat) ** 2 + (sp[1].lng - last.lng) ** 2;
        if (d1 < d0) sp = [sp[1], sp[0]];
      }
      zonePts.push(makeWp(sp[0], userActionTrackFlags));
      zonePts.push(makeWp(sp[1], userActionTrackFlags));
    }
    if (zonePts.length === 0) continue;

    // Transfer leg between zones (only here — never inside a zone)
    if (prevEnd) {
      segments.push({ kind: 'turn', points: [{ ...prevEnd, userActionFlags: 0 }, { ...zonePts[0] }] });
    }
    segments.push({ kind: 'survey', points: zonePts });
    prevEnd = zonePts[zonePts.length - 1];
  }

  if (reverse) {
    segments.reverse();
    for (const s of segments) s.points.reverse();
  }

  // Global start/end user-action flags on the very first / last survey waypoint
  const surveySegs = segments.filter(s => s.kind === 'survey');
  if (surveySegs.length > 0) {
    surveySegs[0].points[0].userActionFlags = userActionStartFlags;
    const last = surveySegs[surveySegs.length - 1];
    last.points[last.points.length - 1].userActionFlags = userActionEndFlags;
  }

  return segments;
}

// ──────────────────────────────────────────────────────

/** Simple bounding box helper */
export function getRectangleBounds(corners: LngLat[]) {
  let minLat = Infinity, maxLat = -Infinity;
  let minLng = Infinity, maxLng = -Infinity;
  for (const p of corners) {
    if (p.lat < minLat) minLat = p.lat;
    if (p.lat > maxLat) maxLat = p.lat;
    if (p.lng < minLng) minLng = p.lng;
    if (p.lng > maxLng) maxLng = p.lng;
  }
  return { minLat, maxLat, minLng, maxLng };
}
