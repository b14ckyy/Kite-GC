<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // Live AGL widget (2×1 wide) — a forward-looking terrain profile HUD.
  //  • left 1/3 = recently flown terrain + flight history (scrolls out left)
  //  • a small UAV icon sits at the "now" divider, at its current altitude
  //  • right 2/3 = ESTIMATED terrain ahead along the current heading, with a
  //    dashed flight line projected from the current vario (ground-intersect warn)
  //
  // The flown history is accumulated internally from the telemetry stream, so it
  // works both on a live connection AND during Blackbox/flight-log replay (the
  // shared liveTrack store is only filled while armed on a live link).
  //
  // Horizontal scale steps with UAV speed (300/900/1800/3600 m total, 1:2
  // history:forward), with hysteresis. Visual language follows the Terrain
  // Analysis panel (grid, ground gradient) inside a standard widget card.
  import { untrack } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { TelemetryData } from '$lib/stores/telemetry';
  import type { InterfaceSettings } from '$lib/stores/settings';
  import { LiveTrackProfiler, type ProfileData } from '$lib/helpers/terrainProfile';
  import { isValidGpsCoordinate } from '$lib/helpers/telemetry';
  import { convertAltitude, convertLength } from '$lib/utils/units';

  let {
    telem,
    interfaceSettings = { speedUnit: 'kmh', altitudeUnit: 'm', distanceUnit: 'metric', verticalSpeedUnit: 'ms', temperatureUnit: 'c' },
    width = 300,
    height = 150,
  }: {
    telem: TelemetryData;
    interfaceSettings?: InterfaceSettings;
    width?: number;
    height?: number;
  } = $props();

  interface RawSample { dist_m: number; lat: number; lon: number; elev_m: number | null }
  interface HistPoint { lat: number; lon: number; alt_m: number; timestamp_ms: number }

  const SCALE_STEPS = [300, 900, 1800, 3600]; // total render distance (m)
  const SPACING = 30; // forward terrain sampling resolution (m)
  const SPEED_FILTER = 2; // m/s — below this use compass yaw, above use GPS heading
  const HIST_MIN_DIST = 5; // m between accumulated history points (matches map trail)
  const HIST_RESET_DIST = 1000; // m — a jump farther than this between fixes = a new site (log switch / big seek) → reset
  const EARTH_R = 6371000;
  const D2R = Math.PI / 180;

  function bearing(aLat: number, aLon: number, bLat: number, bLon: number): number {
    const f1 = aLat * D2R;
    const f2 = bLat * D2R;
    const dl = (bLon - aLon) * D2R;
    const y = Math.sin(dl) * Math.cos(f2);
    const x = Math.cos(f1) * Math.sin(f2) - Math.sin(f1) * Math.cos(f2) * Math.cos(dl);
    return (Math.atan2(y, x) * 180) / Math.PI;
  }

  function haversineM(aLat: number, aLon: number, bLat: number, bLon: number): number {
    const dLat = (bLat - aLat) * D2R;
    const dLon = (bLon - aLon) * D2R;
    const h = Math.sin(dLat / 2) ** 2 + Math.cos(aLat * D2R) * Math.cos(bLat * D2R) * Math.sin(dLon / 2) ** 2;
    return 2 * EARTH_R * Math.asin(Math.min(1, Math.sqrt(h)));
  }
  const angleDiff = (a: number, b: number) => Math.abs(((a - b + 540) % 360) - 180);

  function destPoint(lat: number, lon: number, brngDeg: number, distM: number): [number, number] {
    const brng = brngDeg * D2R;
    const f1 = lat * D2R;
    const l1 = lon * D2R;
    const dr = distM / EARTH_R;
    const f2 = Math.asin(Math.sin(f1) * Math.cos(dr) + Math.cos(f1) * Math.sin(dr) * Math.cos(brng));
    const l2 = l1 + Math.atan2(Math.sin(brng) * Math.sin(dr) * Math.cos(f1), Math.cos(dr) - Math.sin(f1) * Math.sin(f2));
    return [(f2 * 180) / Math.PI, (l2 * 180) / Math.PI];
  }

  /**
   * Total render distance step from speed (≈ speed · 120 s), with hysteresis.
   * The dead-band sits at each *boundary*: once we've stepped up we only step
   * back down when `need` drops to 70 % of the step below the current one — so
   * cruising right on a boundary (e.g. ~54 km/h on the 1800↔3600 edge) doesn't
   * flap between two scales.
   */
  const STEP_DOWN_HYST = 0.7;
  function nextStep(speed: number, current: number): number {
    const need = speed * 120; // unclamped, so a slow craft can return to the lowest step
    const i = Math.max(0, SCALE_STEPS.indexOf(current));
    // smallest step that fits `need` (never below the lowest)
    let target = SCALE_STEPS[SCALE_STEPS.length - 1];
    for (const s of SCALE_STEPS) {
      if (s >= need) {
        target = s;
        break;
      }
    }
    if (target > current) return target; // outgrew the window → up immediately
    if (target < current && need < SCALE_STEPS[i - 1] * STEP_DOWN_HYST) return target; // down past the dead-band
    return current;
  }

  /** Heading: filtered GPS track when moving, compass yaw when slow/hovering. */
  function currentHeading(track: HistPoint[], speed: number, yaw: number): number {
    if (speed >= SPEED_FILTER && track.length >= 5) {
      const a = track[track.length - 5];
      const b = track[track.length - 1];
      return bearing(a.lat, a.lon, b.lat, b.lon);
    }
    return yaw;
  }

  // Climb rate: the FC's own vario (baro/nav-filtered, smooth — same source the
  // Vario widget shows), lightly averaged. Differencing GPS-MSL over the sparse
  // history points was coarse and made the forward angle snap in big steps.
  const VARIO_AVG = 5;
  const varioBuf: number[] = [];
  function avgVario(v: number): number {
    varioBuf.push(v);
    if (varioBuf.length > VARIO_AVG) varioBuf.shift();
    return varioBuf.reduce((a, b) => a + b, 0) / varioBuf.length;
  }

  // ── State ──────────────────────────────────────────────────────────
  const profiler = new LiveTrackProfiler();
  // Safety bound (the HUD shows ≤1200 m of history): let it accumulate to 5 km, then trim back
  // to the most recent 1500 m. The wide gap means the trim runs only once every few minutes, so
  // the per-tick fold cost stays flat and the widget never holds unbounded data on long replays.
  profiler.setWindow(1500, 5000);
  const histBuf: HistPoint[] = []; // flown history, accumulated from telem (plain, non-reactive)
  let lastTs = -Infinity; // last accumulated timestamp (detect scrub/new flight)

  let hist = $state<ProfileData | null>(null);
  let forward = $state<{ dist: number; elev: number | null }[]>([]);
  let step = $state(SCALE_STEPS[0]);
  let curAltMsl = $state(0);
  let terrainAtUav = $state<number | null>(null);
  let slope = $state(0); // m vertical per m horizontal (vario / groundspeed)
  let hasData = $state(false);
  // smoothed vertical range (expand fast, shrink slow)
  let sYMin = $state<number | null>(null);
  let sYMax = $state<number | null>(null);

  let sampling = false;
  let lastFwd: { lat: number; lon: number; heading: number; step: number; time: number } | null = null;

  /** Append the current fix to the internal history, resetting on scrub-back/new flight. */
  function accumulate(t: TelemetryData): void {
    const ts = t.lastUpdate || 0;
    const prev = histBuf[histBuf.length - 1];
    // Reset on a discontinuity: time going backwards (scrub-back / new flight) OR a large
    // position jump (loading a DIFFERENT log while the player stays open, or a big seek).
    // Without the jump check the profiler would bridge the two sites — the next terrain
    // sample would span thousands of km at 30 m spacing, flooding the backend and freezing
    // the app until the next log change.
    const jumped = !!prev && haversineM(prev.lat, prev.lon, t.lat, t.lon) > HIST_RESET_DIST;
    if (ts < lastTs || jumped) {
      histBuf.length = 0;
      varioBuf.length = 0;
      profiler.reset();
    }
    lastTs = ts;
    const last = histBuf[histBuf.length - 1];
    if (!last || haversineM(last.lat, last.lon, t.lat, t.lon) >= HIST_MIN_DIST) {
      histBuf.push({ lat: t.lat, lon: t.lon, alt_m: t.altMsl, timestamp_ms: ts });
    }
  }

  async function runUpdate() {
    if (sampling) return; // drop frame while the backend is busy (no queue, no aliasing)
    const t = telem;
    if (!t.lastUpdate || !isValidGpsCoordinate(t.lat, t.lon)) {
      hasData = false;
      return;
    }
    sampling = true;
    try {
      accumulate(t);
      const speed = t.groundSpeed || 0;
      const heading = currentHeading(histBuf, speed, t.yaw || 0);
      const vario = avgVario(t.vario || 0);

      step = nextStep(speed, step);
      const fwdDistM = (step * 2) / 3;
      const histDistM = step / 3;

      // Forward terrain along the heading (one sampled segment UAV → ahead).
      // Re-sample only on meaningful change to avoid hammering the backend on
      // yaw jitter while hovering.
      const needFwd =
        !lastFwd ||
        lastFwd.step !== step ||
        haversineM(lastFwd.lat, lastFwd.lon, t.lat, t.lon) > 5 ||
        angleDiff(heading, lastFwd.heading) > 2 ||
        Date.now() - lastFwd.time > 1000;
      if (needFwd) {
        const end = destPoint(t.lat, t.lon, heading, fwdDistM);
        const fwdRaw = await invoke<RawSample[]>('terrain_profile', {
          points: [[t.lat, t.lon], end],
          spacingM: SPACING,
        });
        forward = fwdRaw.map((s) => ({ dist: s.dist_m, elev: s.elev_m }));
        lastFwd = { lat: t.lat, lon: t.lon, heading, step, time: Date.now() };
      }

      // History terrain + flight path from the incremental profiler
      hist = await profiler.update(histBuf);

      curAltMsl = t.altMsl;
      terrainAtUav = forward.length ? forward[0].elev : null;
      slope = speed > SPEED_FILTER ? vario / speed : 0;

      // Vertical auto-fit over the visible window (expand fast, shrink slow)
      let tMin = Infinity;
      let tMax = -Infinity;
      const consider = (v: number | null | undefined) => {
        if (v == null) return;
        if (v < tMin) tMin = v;
        if (v > tMax) tMax = v;
      };
      consider(curAltMsl);
      // terrain ahead is a scaling reference, but the projected flight line is
      // NOT — a steep vario shouldn't blow out the vertical range.
      for (const s of forward) consider(s.elev);
      if (hist) {
        const total = hist.totalDist;
        for (const s of hist.terrain) if (s.dist >= total - histDistM) consider(s.elev);
        for (const p of hist.path) if (p.dist >= total - histDistM) consider(p.altMsl);
      }
      if (isFinite(tMin) && isFinite(tMax)) {
        const pad = Math.max(15, (tMax - tMin) * 0.12);
        tMin -= pad;
        tMax += pad;
        if (sYMin == null || sYMax == null) {
          sYMin = tMin;
          sYMax = tMax;
        } else {
          sYMin = tMin < sYMin ? tMin : sYMin + (tMin - sYMin) * 0.06;
          sYMax = tMax > sYMax ? tMax : sYMax + (tMax - sYMax) * 0.06;
        }
        hasData = true;
      }
    } catch (e) {
      console.error('[liveAgl] update failed', e);
    } finally {
      sampling = false;
    }
  }

  // Re-run on every telemetry frame (self-throttled by `sampling`)
  $effect(() => {
    // Track only telemetry changes — these reads register the dependencies.
    void telem.lat;
    void telem.lon;
    void telem.yaw;
    void telem.groundSpeed;
    void telem.altMsl;
    void telem.lastUpdate;
    // runUpdate writes state it also reads (e.g. `step = nextStep(speed, step)`).
    // Run it untracked so those self-reads don't become effect dependencies —
    // otherwise the read+write loop trips Svelte's effect_update_depth_exceeded
    // guard and hard-freezes the main thread.
    untrack(() => void runUpdate());
  });

  // ── Render geometry ────────────────────────────────────────────────
  // Readout font size scales with the widget (like the other instrument widgets).
  const fsVal = $derived(Math.max(11, Math.min(30, height * 0.13)));
  const fsLabel = $derived(fsVal * 0.72);
  // left pad for the relative-altitude axis, bottom pad for the distance axis,
  // top pad sized so the (altitude-tracking) UAV marker can't reach the AGL text
  const PAD = $derived({ l: 34, r: 8, t: Math.ceil(fsVal * 1.25) + 12, b: 18 });
  const plotW = $derived(Math.max(10, width - PAD.l - PAD.r));
  const plotH = $derived(Math.max(10, height - PAD.t - PAD.b));
  const histDist = $derived(step / 3);
  const fwdDist = $derived((step * 2) / 3);
  const baselineY = $derived(PAD.t + plotH);

  // x: relative distance from UAV (−histDist … +fwdDist); UAV (rel 0) sits at 1/3
  function xS(rel: number): number {
    return PAD.l + ((rel + histDist) / (histDist + fwdDist)) * plotW;
  }
  function yS(v: number): number {
    const lo = sYMin ?? 0;
    const hi = sYMax ?? 100;
    return PAD.t + ((hi - v) / (hi - lo || 1)) * plotH;
  }
  const markerX = $derived(xS(0));

  function niceTicks(min: number, max: number, count: number): number[] {
    const range = max - min || 1;
    const rawStep = range / count;
    const mag = Math.pow(10, Math.floor(Math.log10(rawStep)));
    const norm = rawStep / mag;
    let step: number;
    if (norm < 1.5) step = 1;
    else if (norm < 3) step = 2;
    else if (norm < 7) step = 5;
    else step = 10;
    step *= mag;
    const start = Math.ceil(min / step) * step;
    const ticks: number[] = [];
    for (let v = start; v <= max + step * 1e-6; v += step) ticks.push(v);
    return ticks;
  }

  // Left axis: altitude RELATIVE to the UAV (0 = current flight level), incl. negatives
  const altTicks = $derived.by(() => {
    if (sYMin == null || sYMax == null) return [] as { rel: number; y: number; label: string }[];
    const ticks = niceTicks(sYMin - curAltMsl, sYMax - curAltMsl, 4);
    return ticks.map((rel) => {
      const c = convertLength(rel, interfaceSettings.distanceUnit);
      return { rel, y: yS(curAltMsl + rel), label: Math.round(c.value).toString() };
    });
  });

  // Bottom axis: visible distance, 0 under the UAV, positive both directions
  const distTicks = $derived.by(() => {
    const ticks = niceTicks(-histDist, fwdDist, 5);
    return ticks.map((rel) => {
      const c = convertLength(Math.abs(rel), interfaceSettings.distanceUnit);
      return { rel, x: xS(rel), label: Math.round(c.value).toString() };
    });
  });

  /** Combined terrain (history rel<0 + forward rel≥0) as a filled area path. */
  const terrainPath = $derived.by(() => {
    const pts: { rel: number; elev: number }[] = [];
    if (hist) {
      const total = hist.totalDist;
      for (const s of hist.terrain) {
        if (s.elev != null && s.dist >= total - histDist) pts.push({ rel: s.dist - total, elev: s.elev });
      }
    }
    for (const s of forward) {
      if (s.elev != null) pts.push({ rel: s.dist, elev: s.elev });
    }
    pts.sort((a, b) => a.rel - b.rel);
    if (pts.length < 2) return '';
    let d = `M ${xS(pts[0].rel).toFixed(1)} ${baselineY.toFixed(1)}`;
    for (const p of pts) d += ` L ${xS(p.rel).toFixed(1)} ${yS(p.elev).toFixed(1)}`;
    d += ` L ${xS(pts[pts.length - 1].rel).toFixed(1)} ${baselineY.toFixed(1)} Z`;
    return d;
  });

  /** Flown history flight line (polyline points). */
  const historyLine = $derived.by(() => {
    if (!hist) return '';
    const total = hist.totalDist;
    const path = hist.path.filter((p) => p.dist >= total - histDist);
    if (path.length < 2) return '';
    return path.map((p) => `${xS(p.dist - total).toFixed(1)},${yS(p.altMsl).toFixed(1)}`).join(' ');
  });

  /** Dashed forward (estimated) flight line from current alt with the vario slope. */
  const forwardLine = $derived.by(() => {
    if (forward.length < 2) return '';
    return forward
      .map((s) => `${xS(s.dist).toFixed(1)},${yS(curAltMsl + slope * s.dist).toFixed(1)}`)
      .join(' ');
  });

  // UAV icon position (side-view silhouette translated here, scales with altitude)
  const uavX = $derived(markerX);
  const uavY = $derived(yS(curAltMsl));

  // Readouts
  const aglM = $derived(terrainAtUav != null ? curAltMsl - terrainAtUav : null);
  const aglLabel = $derived.by(() => {
    if (aglM == null) return '—';
    const c = convertAltitude(aglM, interfaceSettings.altitudeUnit);
    return `${Math.round(c.value)} ${c.unit}`;
  });
  /** Minimum clearance of the projected line vs terrain ahead. */
  const minAheadM = $derived.by(() => {
    let m: number | null = null;
    for (const s of forward) {
      if (s.elev == null) continue;
      const c = curAltMsl + slope * s.dist - s.elev;
      if (m == null || c < m) m = c;
    }
    return m;
  });
  const minAheadLabel = $derived.by(() => {
    if (minAheadM == null) return '—';
    const c = convertAltitude(minAheadM, interfaceSettings.altitudeUnit);
    return `${Math.round(c.value)} ${c.unit}`;
  });
  const aheadWarn = $derived(minAheadM != null && minAheadM < 0);
</script>

<div class="widget-card" style="width:{width}px; height:{height}px;">
  {#if hasData}
    <svg {width} {height} viewBox="0 0 {width} {height}">
      <defs>
        <linearGradient id="liveAglTerrain" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stop-color="#6e5b46" />
          <stop offset="100%" stop-color="#3c322789" />
        </linearGradient>
      </defs>

      <!-- altitude grid + relative-altitude axis (0 = UAV level) -->
      {#each altTicks as ty}
        <line class="grid" x1={PAD.l} y1={ty.y} x2={PAD.l + plotW} y2={ty.y} />
        <text class="axis-label" x={PAD.l - 5} y={ty.y + 3} text-anchor="end">{ty.label}</text>
      {/each}
      <!-- distance axis (0 under the UAV, positive both ways) -->
      {#each distTicks as tx}
        <line class="grid" x1={tx.x} y1={PAD.t} x2={tx.x} y2={baselineY} />
        <text class="axis-label" x={tx.x} y={baselineY + 13} text-anchor="middle">{tx.label}</text>
      {/each}

      <!-- terrain (history + estimated ahead) -->
      <path d={terrainPath} class="terrain" />

      <!-- flown history flight line -->
      {#if historyLine}<polyline points={historyLine} class="hist-line" />{/if}
      <!-- forward projected (dashed, vario slope) -->
      {#if forwardLine}<polyline points={forwardLine} class="fwd-line" />{/if}

      <!-- "now" divider -->
      <line x1={markerX} y1={PAD.t} x2={markerX} y2={baselineY} class="now" />

      <!-- UAV marker at current altitude — neutral (airframe-agnostic), scales with altitude -->
      <g transform="translate({uavX},{uavY})">
        <circle class="uav-ring" r="10" />
        <circle class="uav-dot" r="4" />
      </g>

      <rect class="plot-border" x={PAD.l} y={PAD.t} width={plotW} height={plotH} />
    </svg>

    <div class="readout agl" style="left:{markerX}px;">
      <span class="r-label" style="font-size:{fsLabel}px;">AGL</span>
      <span class="r-val" style="font-size:{fsVal}px;">{aglLabel}</span>
    </div>
    <div class="readout fwd" class:warn={aheadWarn}>
      <span class="r-label" style="font-size:{fsLabel}px;">▸</span>
      <span class="r-val" style="font-size:{fsVal}px;">{minAheadLabel}</span>
    </div>
  {:else}
    <div class="placeholder">Live AGL — no GPS</div>
  {/if}
</div>

<style>
  .widget-card {
    position: relative;
    box-sizing: border-box;
    background: rgba(30, 30, 30, 0.75);
    backdrop-filter: blur(10px);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 8px;
    overflow: hidden;
  }
  svg {
    display: block;
  }
  .grid {
    stroke: rgba(255, 255, 255, 0.07);
    stroke-width: 1;
  }
  .plot-border {
    fill: none;
    stroke: rgba(255, 255, 255, 0.15);
    stroke-width: 1;
  }
  .terrain {
    fill: url(#liveAglTerrain);
    stroke: #8a7250;
    stroke-width: 1;
  }
  .hist-line {
    fill: none;
    stroke: #37a8db;
    stroke-width: 2;
    stroke-linejoin: round;
    stroke-linecap: round;
  }
  .fwd-line {
    fill: none;
    stroke: #37a8db;
    stroke-width: 2;
    stroke-dasharray: 6 4;
    stroke-linejoin: round;
    stroke-linecap: round;
    opacity: 0.7;
  }
  .now {
    stroke: rgba(255, 255, 255, 0.25);
    stroke-width: 1;
    stroke-dasharray: 3 3;
  }
  .uav-ring {
    fill: rgba(55, 168, 219, 0.25);
    stroke: #ffffff;
    stroke-width: 2;
  }
  .uav-dot {
    fill: #ffffff;
    stroke: #1a1a1a;
    stroke-width: 1;
  }
  .axis-label {
    fill: #949494;
    font-size: 13px;
  }
  /* readouts — overlay, widget typography */
  .readout {
    position: absolute;
    top: 2px;
    display: flex;
    align-items: baseline;
    gap: 0.3em;
    pointer-events: none;
    text-shadow: 0 1px 2px rgba(0, 0, 0, 0.7);
    white-space: nowrap;
  }
  .readout.agl {
    transform: translateX(-50%); /* centered over the UAV marker (left set inline) */
  }
  .readout.fwd {
    right: 8px;
  }
  .r-label {
    font-weight: 600;
    color: #37a8db;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .r-val {
    font-weight: 700;
    color: #e0e0e0;
    font-variant-numeric: tabular-nums;
  }
  .readout.warn .r-label,
  .readout.warn .r-val {
    color: #e74c3c;
  }
  .placeholder {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #888;
    font-size: 12px;
  }
</style>
