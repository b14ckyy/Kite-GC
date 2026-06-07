// Radar conflict-alert controller (Plan C — docs/active/RADAR_ALERTS.md)
//
// Pure frontend. Protected point = the connected UAV (valid fix). Evaluates two stages against every
// tracked foreign vehicle each radar snapshot:
//   Stage 1 (caution): contact inside the horizontal warn radius + vertical band AND closing.
//   Stage 2 (warning): predicted CPA (closest point of approach) miss-distance under threshold within a
//                      look-ahead window, using the course + vertical speed of contact AND UAV.
// The UAV's horizontal course is only trusted for the CPA when it has been steady (±gateSpread over the
// gate window) — otherwise the UAV is treated as a non-translating point (vario still used).
//
// C0: no user-facing UI. Outputs: the `radarAlerts` store (for the later banner/map) + the
// `radarAlertDebug` store (the dev Debug Monitor "Alerts" tab) + throttled console logging.

import { writable, derived, get, type Readable } from 'svelte/store';
import { radarVehicles, type TrackedVehicle, type VehicleSystem } from '$lib/stores/radarTracking';
import { telemetry } from '$lib/stores/telemetry';
import { settings } from '$lib/stores/settings';
import { isValidGpsCoordinate } from '$lib/helpers/telemetry';

// ── Tunable parameters (single source of truth) ─────────────────────
// All SI: metres, seconds, m/s, degrees. Not user-editable yet (see RADAR_ALERTS §4/§5); the evaluator
// reads only from this object merged with optional overrides, so per-user tuning can be added later by
// feeding overrides into `resolveConfig()` without touching the logic.
export interface AlertConfig {
  /** Stage 1 horizontal warn radius (m). */
  rWarn: number;
  /** Stage 1 vertical band (m) — also the GLOBAL vertical relevance cutoff (both stages). */
  hWarn: number;
  /** Stage 1 closing-rate deadband (m/s): only fire if closing faster than this. */
  closingMin: number;
  /** Stage 1 clears once the contact recedes faster than this (m/s) — i.e. has flown past / is leaving. */
  recedeDeadband: number;
  /** Stage 2 CPA horizontal miss radius (m). */
  rCpa: number;
  /** Stage 2 CPA vertical miss height (m). */
  hCpa: number;
  /** Stage 2 look-ahead window (s). */
  lookAhead: number;
  /** Stage 2 arming range (m): CPA only computed for contacts already within this horizontal range. */
  armRange: number;
  /** UAV course-stability window (ms). */
  gateWindowMs: number;
  /** UAV course-stability max heading spread over the window (deg). */
  gateSpreadDeg: number;
  /** Minimum UAV ground speed (m/s) for the course to mean anything (else treat UAV as a point). */
  uavMinSpeed: number;
  /** A raw stage must persist this long (ms) before the alert latches (anti-chatter, esp. noisy CPA). */
  confirmMs: number;
  /** Exit hysteresis: alert clears only once separation exceeds the threshold by this factor… */
  exitFactor: number;
  /** …and stays out for this long (ms). */
  exitHoldMs: number;
}

export const ALERT_CONFIG: AlertConfig = {
  rWarn: 5000,
  hWarn: 2000,
  closingMin: 10,
  recedeDeadband: 1,
  rCpa: 1000,
  hCpa: 250,
  lookAhead: 45,
  armRange: 10000,
  gateWindowMs: 10_000,
  gateSpreadDeg: 20,
  uavMinSpeed: 5,
  confirmMs: 3000,
  exitFactor: 1.3,
  exitHoldMs: 3000,
};

function resolveConfig(overrides?: Partial<AlertConfig>): AlertConfig {
  return overrides ? { ...ALERT_CONFIG, ...overrides } : ALERT_CONFIG;
}

// ── Public output types/stores ──────────────────────────────────────
export type AlertLevel = 'caution' | 'warning';

/** One active alert (level != null). Consumed by the banner + map highlight (C1). */
export interface ContactAlert {
  vehicleId: string;
  system: VehicleSystem;
  callsign: string | null;
  category: string | null;
  level: AlertLevel;
  /** Current horizontal / vertical separation (m). dV is absolute; relAltM is signed (+above UAV). */
  dH: number;
  dV: number;
  relAltM: number;
  /** Contact ground speed (m/s), or null. */
  groundSpeedMs: number | null;
  /** Bearing from the UAV to the contact (deg, 0–360). */
  bearingDeg: number;
  /** CPA time (s) / miss distances (m) — present when Stage 2 was evaluated. */
  tCpa: number | null;
  missH: number | null;
  missV: number | null;
  /** Suggested evade heading (deg) — perpendicular to the contact's track, away from the CPA. Warnings only. */
  evadeBearingDeg: number | null;
}

/** Live alerts (worst first). */
export const radarAlerts = writable<ContactAlert[]>([]);
/** Worst current level, or null. */
export const radarAlertWorst: Readable<AlertLevel | null> = derived(radarAlerts, ($a) =>
  $a.some((a) => a.level === 'warning') ? 'warning' : $a.some((a) => a.level === 'caution') ? 'caution' : null,
);

/** vehicleId → current alert level, for map highlight lookup (2D + 3D). */
export const radarAlertLevels: Readable<Map<string, AlertLevel>> = derived(radarAlerts, ($a) => {
  const m = new Map<string, AlertLevel>();
  for (const a of $a) m.set(a.vehicleId, a.level);
  return m;
});

// ── Debug snapshot (dev Debug Monitor "Alerts" tab) ─────────────────
export interface AlertDebugRow {
  id: string;
  callsign: string | null;
  dH: number;
  dV: number;
  /** Range-rate (m/s); negative = closing. Null until a second sighting. */
  rangeRate: number | null;
  tCpa: number | null;
  missH: number | null;
  missV: number | null;
  stage1Raw: boolean;
  stage2Raw: boolean;
  /** Latched (post-hysteresis) level. */
  level: AlertLevel | null;
}

export interface AlertDebugSnapshot {
  /** Controller running with a usable UAV reference. */
  active: boolean;
  uavValid: boolean;
  /** UAV course is currently trusted for the CPA (steady + fast enough). */
  courseStable: boolean;
  uavCourseDeg: number;
  uavSpeedMs: number;
  uavVarioMs: number;
  rows: AlertDebugRow[];
  worst: AlertLevel | null;
  evaluated: number;
}

export const radarAlertDebug = writable<AlertDebugSnapshot>({
  active: false, uavValid: false, courseStable: false,
  uavCourseDeg: 0, uavSpeedMs: 0, uavVarioMs: 0,
  rows: [], worst: null, evaluated: 0,
});

// ── Internal geometry helpers ───────────────────────────────────────
const DEG2RAD = Math.PI / 180;
const R_EARTH = 6371000;

interface Vec3 { e: number; n: number; u: number; }

/** ENU velocity from ground speed + heading (deg, 0=N, 90=E) + vertical speed. */
function velENU(speedMs: number, hdgDeg: number, vsMs: number): Vec3 {
  const h = hdgDeg * DEG2RAD;
  return { e: speedMs * Math.sin(h), n: speedMs * Math.cos(h), u: vsMs };
}

// ── Per-contact + UAV evaluation state ──────────────────────────────
interface AlertTrack {
  prevDH: number | null;
  prevT: number;
  /** `lastSeenMs` of the contact at the last range-rate sample (detects a genuinely new position). */
  lastSeen: number;
  /** Persisted range-rate (m/s) — held between bursty position updates. */
  rangeRate: number | null;
  warnLatched: boolean;
  warnPendingSince: number;
  warnClearSince: number;
  cautLatched: boolean;
  cautPendingSince: number;
  cautClearSince: number;
  lastEval: number;
}

const tracks = new Map<string, AlertTrack>();

function freshTrack(): AlertTrack {
  return {
    prevDH: null, prevT: 0, lastSeen: 0, rangeRate: null,
    warnLatched: false, warnPendingSince: 0, warnClearSince: 0,
    cautLatched: false, cautPendingSince: 0, cautClearSince: 0,
    lastEval: 0,
  };
}

/** UAV heading ring buffer for the course-stability gate. */
const headingBuf: { t: number; hdg: number }[] = [];
let lastHeadingSample = 0;

function pushHeadingSample(now: number, hdg: number) {
  // Throttle to keep the buffer small but dense enough for the gate window.
  if (now - lastHeadingSample < 200) return;
  lastHeadingSample = now;
  headingBuf.push({ t: now, hdg });
  const cutoff = now - ALERT_CONFIG.gateWindowMs;
  while (headingBuf.length && headingBuf[0].t < cutoff) headingBuf.shift();
}

/** True if the UAV heading stayed within `gateSpreadDeg` over the gate window (needs enough coverage). */
function courseStable(now: number, cfg: AlertConfig): boolean {
  if (headingBuf.length < 3) return false; // too little history → conservative (treat as point)
  const span = headingBuf[headingBuf.length - 1].t - headingBuf[0].t;
  if (span < cfg.gateWindowMs * 0.6) return false; // not enough coverage yet
  // Mean direction (circular), then max deviation spread — handles 0/360 wrap.
  let sx = 0, sy = 0;
  for (const s of headingBuf) { sx += Math.cos(s.hdg * DEG2RAD); sy += Math.sin(s.hdg * DEG2RAD); }
  const mean = Math.atan2(sy, sx) / DEG2RAD;
  let lo = 0, hi = 0;
  for (const s of headingBuf) {
    let d = s.hdg - mean;
    d = ((d + 540) % 360) - 180; // wrap to [-180,180]
    if (d < lo) lo = d;
    if (d > hi) hi = d;
  }
  return (hi - lo) <= cfg.gateSpreadDeg;
}

// ── CPA ─────────────────────────────────────────────────────────────
interface CpaResult { tCpa: number; missH: number; missV: number; offE: number; offN: number; }

/** Closest point of approach within [0, lookAhead]. Null if diverging or no relative motion.
 *  `offE`/`offN` = the contact's horizontal offset from the UAV at the CPA (for the evade direction). */
function computeCpa(r: Vec3, vRel: Vec3, lookAhead: number): CpaResult | null {
  const vv = vRel.e * vRel.e + vRel.n * vRel.n + vRel.u * vRel.u;
  if (vv < 1e-6) return null; // no relative motion
  let t = -(r.e * vRel.e + r.n * vRel.n + r.u * vRel.u) / vv;
  if (t <= 0) return null; // already at/after closest approach → diverging
  if (t > lookAhead) t = lookAhead; // clamp to the window (still report the windowed miss)
  const ce = r.e + vRel.e * t;
  const cn = r.n + vRel.n * t;
  const cu = r.u + vRel.u * t;
  return { tCpa: t, missH: Math.hypot(ce, cn), missV: Math.abs(cu), offE: ce, offN: cn };
}

/** Evade heading (deg): perpendicular to the contact's track, on the side that moves the UAV AWAY from
 *  where the contact will be at the CPA (`offE`/`offN` = that offset from the UAV). */
function evadeHeading(courseDeg: number, offE: number, offN: number): number {
  const ch = courseDeg * DEG2RAD;
  let pe = Math.cos(ch);   // unit perpendicular to the course (course + 90°), ENU (e, n)
  let pn = -Math.sin(ch);
  if (pe * offE + pn * offN > 0) { pe = -pe; pn = -pn; } // flip to point away from the CPA offset
  return (Math.atan2(pe, pn) / DEG2RAD + 360) % 360;
}

// ── Evaluation ──────────────────────────────────────────────────────
let lastLog = 0;

function evaluate() {
  const cfg = ALERT_CONFIG;
  const s = get(settings);
  const t = get(telemetry);
  const now = Date.now();

  const radarOn = s.radar.enabled;
  const stage1On = s.radar.alerts.stage1Enabled;
  const stage2On = s.radar.alerts.stage2Enabled;

  const uavValid = isValidGpsCoordinate(t.lat, t.lon) && t.fixType >= 2;

  // Maintain the heading buffer whenever we have a moving UAV (independent of radar state).
  if (uavValid) pushHeadingSample(now, t.course);

  if (!radarOn || !uavValid || (!stage1On && !stage2On)) {
    if (tracks.size) tracks.clear();
    if (get(radarAlerts).length) radarAlerts.set([]);
    radarAlertDebug.set({
      active: false, uavValid, courseStable: false,
      uavCourseDeg: t.course, uavSpeedMs: t.groundSpeed, uavVarioMs: t.vario,
      rows: [], worst: null, evaluated: 0,
    });
    return;
  }

  const stable = courseStable(now, cfg) && t.groundSpeed >= cfg.uavMinSpeed;
  const vUav: Vec3 = stable
    ? velENU(t.groundSpeed, t.course, t.vario)
    : { e: 0, n: 0, u: t.vario };

  const snap = get(radarVehicles);
  const contacts: TrackedVehicle[] = [...snap.adsb, ...snap.formationFlight, ...snap.radio];

  const cosLat = Math.cos(t.lat * DEG2RAD);
  const alerts: ContactAlert[] = [];
  const rows: AlertDebugRow[] = [];
  const seen = new Set<string>();

  for (const c of contacts) {
    if (!c.validPos || c.altM == null) continue; // need a 3D fix to judge a conflict
    seen.add(c.id);

    // Relative position (ENU, m) of the contact from the UAV.
    const r: Vec3 = {
      e: (c.lon - t.lon) * DEG2RAD * R_EARTH * cosLat,
      n: (c.lat - t.lat) * DEG2RAD * R_EARTH,
      u: c.altM - t.altMsl,
    };
    const dH = Math.hypot(r.e, r.n);
    const dV = Math.abs(r.u);
    const bearingDeg = (Math.atan2(r.e, r.n) / DEG2RAD + 360) % 360;
    const inBand = dV <= cfg.hWarn; // global vertical relevance cutoff

    const track = tracks.get(c.id) ?? freshTrack();

    // Range-rate ḋ (negative = closing). Recomputed ONLY when the contact reports a new position —
    // ADS-B updates arrive in bursts, and between them dH is unchanged (which would read as "not
    // closing" and drop S1 every other frame). The last value is held across the gap.
    if (c.lastSeenMs !== track.lastSeen) {
      if (track.prevDH != null) {
        const dt = (now - track.prevT) / 1000;
        if (dt > 0.05) track.rangeRate = (dH - track.prevDH) / dt;
      }
      track.prevDH = dH;
      track.prevT = now;
      track.lastSeen = c.lastSeenMs;
    }
    const rangeRate = track.rangeRate;

    // Stage 1 (caution) raw: inside radius + band AND closing past the deadband.
    const stage1Raw = stage1On && inBand && dH <= cfg.rWarn
      && rangeRate != null && rangeRate < -cfg.closingMin;

    // Stage 2 (warning) raw: CPA miss inside thresholds within the look-ahead window.
    let tCpa: number | null = null, missH: number | null = null, missV: number | null = null;
    let cpaOffE: number | null = null, cpaOffN: number | null = null;
    let stage2Raw = false;
    if (stage2On && inBand && dH <= cfg.armRange && c.headingDeg != null && c.groundSpeedMs != null) {
      const vC = velENU(c.groundSpeedMs, c.headingDeg, c.verticalSpeedMs ?? 0);
      const vRel: Vec3 = { e: vC.e - vUav.e, n: vC.n - vUav.n, u: vC.u - vUav.u };
      const cpa = computeCpa(r, vRel, cfg.lookAhead);
      if (cpa) {
        tCpa = cpa.tCpa; missH = cpa.missH; missV = cpa.missV;
        cpaOffE = cpa.offE; cpaOffN = cpa.offN;
        stage2Raw = cpa.missH <= cfg.rCpa && cpa.missV <= cfg.hCpa;
      }
    }

    // ── Hysteresis: two independent latches (warning, caution). ──
    // Warning latch.
    if (!track.warnLatched) {
      if (stage2Raw) {
        if (track.warnPendingSince === 0) track.warnPendingSince = now;
        if (now - track.warnPendingSince >= cfg.confirmMs) { track.warnLatched = true; track.warnClearSince = 0; }
      } else {
        track.warnPendingSince = 0;
      }
    } else {
      // Stay latched while the widened CPA condition still holds.
      const stillWarn = missH != null && missV != null
        && missH <= cfg.rCpa * cfg.exitFactor && missV <= cfg.hCpa * cfg.exitFactor;
      if (stillWarn) {
        track.warnClearSince = 0;
      } else {
        if (track.warnClearSince === 0) track.warnClearSince = now;
        if (now - track.warnClearSince >= cfg.exitHoldMs) { track.warnLatched = false; track.warnPendingSince = 0; }
      }
    }

    // Caution latch: enter on a fast approach inside the radius; STAY only while still closing (a contact
    // that has flown past and is now receding clears, even if it's still inside the radius).
    if (!track.cautLatched) {
      if (stage1Raw) {
        if (track.cautPendingSince === 0) track.cautPendingSince = now;
        if (now - track.cautPendingSince >= cfg.confirmMs) { track.cautLatched = true; track.cautClearSince = 0; }
      } else {
        track.cautPendingSince = 0;
      }
    } else {
      // Still approaching (negative range-rate, small deadband) AND inside the widened radius/band.
      const stillCaut = inBand && dH <= cfg.rWarn * cfg.exitFactor
        && (rangeRate == null || rangeRate < cfg.recedeDeadband);
      if (stillCaut) {
        track.cautClearSince = 0;
      } else {
        if (track.cautClearSince === 0) track.cautClearSince = now;
        if (now - track.cautClearSince >= cfg.exitHoldMs) { track.cautLatched = false; track.cautPendingSince = 0; }
      }
    }

    const level: AlertLevel | null = track.warnLatched ? 'warning' : track.cautLatched ? 'caution' : null;

    track.lastEval = now;
    tracks.set(c.id, track);

    if (level) {
      const evadeBearingDeg = (level === 'warning' && c.headingDeg != null && cpaOffE != null && cpaOffN != null)
        ? evadeHeading(c.headingDeg, cpaOffE, cpaOffN)
        : null;
      alerts.push({
        vehicleId: c.id, system: c.system, callsign: c.callsign, category: c.category, level,
        dH, dV, relAltM: r.u, groundSpeedMs: c.groundSpeedMs, bearingDeg,
        tCpa, missH, missV, evadeBearingDeg,
      });
    }
    // Debug rows: anything within the arming range, plus anything currently alerting.
    if (dH <= cfg.armRange || level) {
      rows.push({ id: c.id, callsign: c.callsign, dH, dV, rangeRate, tCpa, missH, missV, stage1Raw, stage2Raw, level });
    }
  }

  // Prune tracks for contacts no longer present.
  for (const id of tracks.keys()) if (!seen.has(id)) tracks.delete(id);

  // Sort: warnings first, then by horizontal distance.
  const rank = (l: AlertLevel | null) => (l === 'warning' ? 2 : l === 'caution' ? 1 : 0);
  alerts.sort((a, b) => rank(b.level) - rank(a.level) || a.dH - b.dH);
  rows.sort((a, b) => rank(b.level) - rank(a.level) || a.dH - b.dH);

  radarAlerts.set(alerts);
  const worst: AlertLevel | null = alerts.some((a) => a.level === 'warning') ? 'warning'
    : alerts.some((a) => a.level === 'caution') ? 'caution' : null;
  radarAlertDebug.set({
    active: true, uavValid, courseStable: stable,
    uavCourseDeg: t.course, uavSpeedMs: t.groundSpeed, uavVarioMs: t.vario,
    rows, worst, evaluated: contacts.length,
  });

  // Throttled dev console summary.
  if (import.meta.env.DEV && worst && now - lastLog > 5000) {
    lastLog = now;
    const top = alerts[0];
    console.log(
      `[radarAlerts] worst=${worst} active=${alerts.length} top=${top.callsign ?? top.vehicleId} ` +
      `dH=${top.dH.toFixed(0)}m dV=${top.dV.toFixed(0)}m` +
      (top.tCpa != null ? ` tCpa=${top.tCpa.toFixed(1)}s missH=${top.missH?.toFixed(0)}m` : ''),
    );
  }
}

// ── Lifecycle ───────────────────────────────────────────────────────
let unsubRadar: (() => void) | null = null;
let unsubTelemetry: (() => void) | null = null;

/** Start evaluating conflict alerts. Driven by each radar snapshot; the heading buffer is fed by
 *  telemetry. Idempotent. */
export function startRadarAlerts() {
  stopRadarAlerts();
  // Radar snapshots are the natural evaluation tick.
  unsubRadar = radarVehicles.subscribe(() => evaluate());
  // Sample the UAV heading densely for the stability gate (radar polls can be slow).
  unsubTelemetry = telemetry.subscribe((t) => {
    if (isValidGpsCoordinate(t.lat, t.lon) && t.fixType >= 2) pushHeadingSample(Date.now(), t.course);
  });
}

export function stopRadarAlerts() {
  unsubRadar?.(); unsubRadar = null;
  unsubTelemetry?.(); unsubTelemetry = null;
  tracks.clear();
  headingBuf.length = 0;
  radarAlerts.set([]);
}
