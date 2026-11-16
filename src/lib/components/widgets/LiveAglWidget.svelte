<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // Forward-looking terrain profile HUD (2×1). History is accumulated here from telemetry, so it works live and in
  // replay.
  import { untrack } from 'svelte';
  import { get } from 'svelte/store';
  import { invoke } from '@tauri-apps/api/core';
  import { altReference, groundAnchor, resolveTrueMsl, type TelemetryData } from '$lib/stores/telemetry';
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
  const HIST_SHARE = 0.15; // of the render distance, behind the UAV
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

  /** Total render distance step from speed (≈ speed · 120 s). Hysteresis at each boundary: step down only when
   *  `need` drops to 70 % of the step below, so cruising on a boundary doesn't flap. */
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

  // Glide-path slope from the FC's own vario / ground speed (GPS-MSL differences were too coarse), smoothed with
  // a time-based PT1 (fc = 0.2 Hz, τ ≈ 0.8 s); dt from telemetry timestamps, so sample-rate independent. A
  // discontinuity resets it (see `accumulate`).
  const GLIDE_LPF_HZ = 0.2;
  const GLIDE_TAU_S = 1 / (2 * Math.PI * GLIDE_LPF_HZ);
  let slopeFilt: number | null = null;
  let slopeTs = 0;
  function filterSlope(raw: number, ts: number): number {
    if (slopeFilt == null) {
      slopeFilt = raw;
      slopeTs = ts;
      return raw;
    }
    if (ts <= slopeTs) return slopeFilt; // same telemetry frame — nothing new to integrate
    const dt = Math.min(5, (ts - slopeTs) / 1000);
    slopeTs = ts;
    slopeFilt += (raw - slopeFilt) * (1 - Math.exp(-dt / GLIDE_TAU_S));
    return slopeFilt;
  }

  // ── State ──────────────────────────────────────────────────────────
  const profiler = new LiveTrackProfiler();
  // Safety bound (the HUD shows far less history): let it accumulate to 5 km, then trim back
  // to the most recent 1500 m. The wide gap means the trim runs only once every few minutes, so
  // the per-tick fold cost stays flat and the widget never holds unbounded data on long replays.
  profiler.setWindow(1500, 5000);
  const histBuf: HistPoint[] = []; // flown history, accumulated from telem (plain, non-reactive)
  let lastTs = -Infinity; // last accumulated timestamp (detect scrub/new flight)

  let hist = $state<ProfileData | null>(null);
  let forward = $state<{ dist: number; elev: number | null }[]>([]);
  let step = $state(SCALE_STEPS[0]);
  let curAltMsl = $state(0);
  let mslValid = $state(true); // false → relative-only protocol with no ground anchor → AGL is N/A
  let terrainAtUav = $state<number | null>(null);
  let slope = $state(0); // m vertical per m horizontal (vario / groundspeed)
  let hasData = $state(false);
  // smoothed vertical range (expand fast, shrink slow)
  let sYMin = $state<number | null>(null);
  let sYMax = $state<number | null>(null);

  let sampling = false;
  const FIT_SHRINK_TAU_S = 2;
  let fitTs = 0;
  let lastFwd: { lat: number; lon: number; heading: number; step: number; time: number } | null = null;

  /** Append the current fix to the internal history, resetting on scrub-back/new flight. */
  function accumulate(t: TelemetryData): void {
    const ts = t.lastUpdate || 0;
    const prev = histBuf[histBuf.length - 1];
    // Reset on a discontinuity: time going backwards OR a large position jump (another log, a big seek). Without
    // the jump check the profiler bridges the two sites — one terrain sample spanning thousands of km at 30 m
    // spacing floods the backend and freezes the app.
    const jumped = !!prev && haversineM(prev.lat, prev.lon, t.lat, t.lon) > HIST_RESET_DIST;
    if (ts < lastTs || jumped) {
      histBuf.length = 0;
      slopeFilt = null;
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
      const vario = t.vario || 0;

      step = nextStep(speed, step);
      const fwdDistM = step * (1 - HIST_SHARE);
      const histDistM = step * HIST_SHARE;

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

      // Resolve true MSL: relative-only protocols (LTM/CRSF) are anchored to the ground MSL captured at
      // arm; without an anchor the true altitude is unknown (→ AGL N/A). Fall back to the relative value
      // only to keep the plot positioned.
      const tm = resolveTrueMsl(t.altMsl, get(altReference), get(groundAnchor));
      mslValid = tm != null;
      curAltMsl = tm ?? t.altMsl;
      terrainAtUav = forward.length ? forward[0].elev : null;
      slope = filterSlope(speed > SPEED_FILTER ? vario / speed : 0, t.lastUpdate);

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
          // Shrinks over time, not per update, so the pace does not depend on the redraw rate.
          const k = 1 - Math.exp(-Math.min(5, Math.max(0, (t.lastUpdate - fitTs) / 1000)) / FIT_SHRINK_TAU_S);
          sYMin = tMin < sYMin ? tMin : sYMin + (tMin - sYMin) * k;
          sYMax = tMax > sYMax ? tMax : sYMax + (tMax - sYMax) * k;
        }
        fitTs = t.lastUpdate;
        hasData = true;
      }
    } catch (e) {
      console.error('[liveAgl] update failed', e);
    } finally {
      sampling = false;
      // A change that arrived while the backend was busy was dropped by the effect: catch up.
      const n = telem;
      if (drawn && (n.lat !== drawn.lat || n.lon !== drawn.lon || n.altMsl !== drawn.alt)) redraw();
    }
  }

  // A redraw repaints the whole widget dock, so only a new 3D position or a turn asks for one, not every
  // telemetry message; once a second anyway, for what changes without them (terrain still loading, the MSL anchor).
  const YAW_REDRAW_DEG = 5;
  const IDLE_REDRAW_MS = 1000;
  let drawn: { lat: number; lon: number; alt: number; yaw: number } | null = null;
  let drawnAt = 0;
  function redraw(): void {
    drawn = { lat: telem.lat, lon: telem.lon, alt: telem.altMsl, yaw: telem.yaw };
    drawnAt = performance.now();
    void runUpdate();
  }
  $effect(() => {
    const { lat, lon, altMsl, yaw } = telem;
    void telem.lastUpdate;
    // runUpdate writes state it also reads (e.g. `step = nextStep(speed, step)`): untracked, or the read+write loop
    // trips effect_update_depth_exceeded and hard-freezes the main thread.
    untrack(() => {
      if (sampling) return;
      const moved = !drawn || lat !== drawn.lat || lon !== drawn.lon || altMsl !== drawn.alt;
      const turned = !!drawn && angleDiff(yaw, drawn.yaw) > YAW_REDRAW_DEG;
      if (moved || turned || performance.now() - drawnAt > IDLE_REDRAW_MS) redraw();
    });
  });

  // ── Render geometry ── readout font scales with the widget, bounded by WIDTH too: in the square states the
  // two readouts share half the room of the wide tile. The AGL readout is left-aligned, so the two split the
  // width cleanly.
  const fsVal = $derived(Math.max(9, Math.min(30, height * 0.13, width * 0.08)));
  const fsLabel = $derived(fsVal * 0.72);
  const fsAxis = $derived(Math.round(Math.max(9, Math.min(13, fsVal * 0.8))));
  // Pads follow the axis font: left = room for a 3-digit relative-altitude label, bottom = one
  // distance-label line. Top pad sized so the (altitude-tracking) UAV marker can't reach the AGL text.
  const PAD = $derived({
    l: Math.round(fsAxis * 2.2) + 5,
    r: 6,
    t: Math.ceil(fsVal * 1.25) + 12,
    b: fsAxis + 6,
  });
  const plotW = $derived(Math.max(10, width - PAD.l - PAD.r));
  const plotH = $derived(Math.max(10, height - PAD.t - PAD.b));
  const histDist = $derived(step * HIST_SHARE);
  const fwdDist = $derived(step * (1 - HIST_SHARE));
  const baselineY = $derived(PAD.t + plotH);

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
  const aglM = $derived(mslValid && terrainAtUav != null ? curAltMsl - terrainAtUav : null);
  const aglLabel = $derived.by(() => {
    if (aglM == null) return '—';
    const c = convertAltitude(aglM, interfaceSettings.altitudeUnit);
    return `${Math.round(c.value)} ${c.unit}`;
  });
  /** Minimum clearance of the projected line vs terrain ahead. */
  const minAheadM = $derived.by(() => {
    if (!mslValid) return null; // no MSL reference → clearance is meaningless
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

<div class="widget-card" style="width:{width}px; height:{height}px; --axis-fs:{fsAxis}px;">
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
        <text class="axis-label" x={tx.x} y={baselineY + fsAxis} text-anchor="middle">{tx.label}</text>
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

    <div class="readout agl" style="left:{PAD.l}px;">
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
    backdrop-filter: var(--glass-blur, blur(6px));
    -webkit-backdrop-filter: var(--glass-blur, blur(6px));
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
    font-size: var(--axis-fs, 13px);
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
  /* .readout.agl is left-aligned with the plot (left set inline) — a centred-over-marker readout
     collided with the forward one in the square size states. */
  .readout.fwd {
    right: 6px;
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
