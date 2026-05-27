import type { RectanglePatternParams } from '$lib/stores/surveyPattern.svelte';

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
      points: [makeWp(clipped.start, userActionLineStartFlags), makeWp(extEnd, userActionLineEndFlags)],
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

    // Survey track: start gets lineStartFlags, end gets lineEndFlags
    segments.push({
      kind: 'survey',
      points: [makeWp(innerStart.x, innerStart.y, lineStartFlags), makeWp(extEnd.x, extEnd.y, lineEndFlags)],
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
