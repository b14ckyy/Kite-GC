<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // Terrain Radar widget (1×1) — a simple top-down, track-up terrain-awareness
  // display (EGPWS-style). A 120° forward fan is sampled as a polar grid and
  // coloured by clearance against a reference altitude:
  //   clearance = referenceAlt(dist) − terrain
  //     static     → referenceAlt = current MSL          (flat)
  //     predictive → referenceAlt = MSL + slope·dist      (sink-angle, FC vario)
  // Range scaling (300/900/1800/3600 m) + hysteresis are shared with the Live
  // AGL widget. The fan is fixed pointing up; terrain is sampled relative to the
  // heading, so it appears track-up. UAV marker (ring+dot) sits at the apex.
  import { untrack } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { TelemetryData } from '$lib/stores/telemetry';
  import type { InterfaceSettings } from '$lib/stores/settings';
  import { terrainAnalysis, patchTerrainAnalysis } from '$lib/stores/terrainAnalysis';
  import { isValidGpsCoordinate } from '$lib/helpers/telemetry';
  import { convertLength } from '$lib/utils/units';

  let {
    telem,
    interfaceSettings = { speedUnit: 'kmh', altitudeUnit: 'm', distanceUnit: 'metric', verticalSpeedUnit: 'ms', temperatureUnit: 'c' },
    size = 200,
  }: {
    telem: TelemetryData;
    interfaceSettings?: InterfaceSettings;
    size?: number;
  } = $props();

  interface FanData { ang_cells: number; rad_cells: number; range_m: number; elev: (number | null)[] }

  const SCALE_STEPS = [300, 900, 1800, 3600]; // forward fan distance (m), speed-driven
  const HALF_ANGLE = 60; // ± degrees → 120° fan
  const ANG_CELLS = 32;
  const RAD_CELLS = 16;
  const RADAR_SCALES = [60, 120, 250]; // total clearance colour scale (m)
  const TEX_FREQ = 0.14; // turbulence grain for the heatmap texture filter
  const SPEED_FILTER = 2; // m/s — below this use compass yaw, above use GPS course
  const VARIO_AVG = 5;
  const D2R = Math.PI / 180;
  const EARTH_R = 6371000;

  function haversineM(aLat: number, aLon: number, bLat: number, bLon: number): number {
    const dLat = (bLat - aLat) * D2R;
    const dLon = (bLon - aLon) * D2R;
    const h = Math.sin(dLat / 2) ** 2 + Math.cos(aLat * D2R) * Math.cos(bLat * D2R) * Math.sin(dLon / 2) ** 2;
    return 2 * EARTH_R * Math.asin(Math.min(1, Math.sqrt(h)));
  }
  const angleDiff = (a: number, b: number) => Math.abs(((a - b + 540) % 360) - 180);

  const STEP_DOWN_HYST = 0.7;
  function nextStep(speed: number, current: number): number {
    const need = speed * 120;
    const i = Math.max(0, SCALE_STEPS.indexOf(current));
    let target = SCALE_STEPS[SCALE_STEPS.length - 1];
    for (const s of SCALE_STEPS) {
      if (s >= need) {
        target = s;
        break;
      }
    }
    if (target > current) return target;
    if (target < current && need < SCALE_STEPS[i - 1] * STEP_DOWN_HYST) return target;
    return current;
  }

  // Heading EMA (sin/cos so it wraps cleanly) — keeps the track-up image from jittering
  let hSin = 0;
  let hCos = 1;
  let headingInit = false;
  function smoothHeading(speed: number, course: number, yaw: number): number {
    const raw = (speed >= SPEED_FILTER ? course : yaw) * D2R;
    if (!headingInit) {
      hSin = Math.sin(raw);
      hCos = Math.cos(raw);
      headingInit = true;
    } else {
      const k = 0.3;
      hSin += (Math.sin(raw) - hSin) * k;
      hCos += (Math.cos(raw) - hCos) * k;
    }
    return Math.atan2(hSin, hCos) / D2R;
  }

  // FC vario (smooth), lightly averaged — for the predictive slope
  const varioBuf: number[] = [];
  function avgVario(v: number): number {
    varioBuf.push(v);
    if (varioBuf.length > VARIO_AVG) varioBuf.shift();
    return varioBuf.reduce((a, b) => a + b, 0) / varioBuf.length;
  }

  // ── State ──────────────────────────────────────────────────────────
  let fan = $state<FanData | null>(null);
  let range = $state(SCALE_STEPS[0]);
  let heading = $state(0);
  let curAltMsl = $state(0);
  let slope = $state(0);
  let hasData = $state(false);

  let sampling = false;
  let lastFan: { lat: number; lon: number; heading: number; step: number; time: number } | null = null;

  const radarScale = $derived($terrainAnalysis.radarScale);
  const predictive = $derived($terrainAnalysis.radarPredictive);

  // Scale stays metric internally; in feet we show coarse round numbers (not exact
  // conversions): 60→200, 120→400, 250→800 ft.
  const FT_LABEL: Record<number, number> = { 60: 200, 120: 400, 250: 800 };
  const scaleLabel = $derived(
    interfaceSettings.altitudeUnit === 'ft'
      ? `${FT_LABEL[radarScale] ?? Math.round((radarScale * 3.28) / 50) * 50}ft`
      : `${radarScale}m`,
  );

  async function runUpdate() {
    if (sampling) return;
    const t = telem;
    if (!t.lastUpdate || !isValidGpsCoordinate(t.lat, t.lon)) {
      hasData = false;
      return;
    }
    sampling = true;
    try {
      const speed = t.groundSpeed || 0;
      heading = smoothHeading(speed, t.course || 0, t.yaw || 0);
      curAltMsl = t.altMsl;
      slope = speed > SPEED_FILTER ? avgVario(t.vario || 0) / speed : 0;
      range = nextStep(speed, range);

      // Re-sample only on meaningful change (movement > half a radial cell,
      // heading turn, scale change, or staleness) — don't hammer the backend.
      const cellM = range / RAD_CELLS;
      const needFan =
        !lastFan ||
        lastFan.step !== range ||
        haversineM(lastFan.lat, lastFan.lon, t.lat, t.lon) > cellM * 0.5 ||
        angleDiff(heading, lastFan.heading) > 2 ||
        Date.now() - lastFan.time > 1000;
      if (needFan) {
        fan = await invoke<FanData>('terrain_fan', {
          lat: t.lat,
          lon: t.lon,
          headingDeg: heading,
          halfAngleDeg: HALF_ANGLE,
          rangeM: range,
          angCells: ANG_CELLS,
          radCells: RAD_CELLS,
        });
        lastFan = { lat: t.lat, lon: t.lon, heading, step: range, time: Date.now() };
      }
      hasData = true;
    } catch (e) {
      console.error('[terrainRadar] update failed', e);
    } finally {
      sampling = false;
    }
  }

  $effect(() => {
    // Track only telemetry changes — these reads register the dependencies.
    void telem.lat;
    void telem.lon;
    void telem.yaw;
    void telem.course;
    void telem.groundSpeed;
    void telem.altMsl;
    void telem.lastUpdate;
    // runUpdate writes state it also reads (e.g. `range = nextStep(speed, range)`).
    // Run it untracked so those self-reads don't become effect dependencies —
    // otherwise the read+write loop trips Svelte's effect_update_depth_exceeded
    // guard and hard-freezes the main thread.
    untrack(() => void runUpdate());
  });

  // ── Geometry (track-up fan, apex = UAV at bottom centre) ───────────
  // The fan fills the square vertically; its wide flanks (±60°) overflow the
  // left/right edges and are clipped by the card — no dead space at the bottom.
  const fsVal = $derived(Math.max(8, Math.min(16, size * 0.06)));
  const cx = $derived(size / 2);
  const ringR = $derived(Math.max(5, size * 0.035));
  const apexY = $derived(size - ringR - 3); // UAV sits just above the bottom edge
  const R = $derived(apexY - Math.max(4, size * 0.03)); // reach the top edge
  const texDisp = $derived(Math.max(2, (R / RAD_CELLS) * 0.56)); // displacement (dissolves cell edges), noise −30%
  const blurStd = $derived(Math.max(0.6, (R / RAD_CELLS) * 0.18)); // very light blur over the texture

  /** Polar → screen. `thetaRel` rad from vertical (0 = up, + = right), `dist` in metres. */
  function px(thetaRel: number, dist: number): [number, number] {
    const r = (dist / range) * R;
    return [cx + r * Math.sin(thetaRel), apexY - r * Math.cos(thetaRel)];
  }

  // Coloured cells — geometry depends on the fan/size; colour is applied live
  // in the template (re-evaluates as altitude/slope change without re-sampling).
  const cells = $derived.by(() => {
    if (!fan) return [] as { path: string; dist: number; elev: number | null }[];
    const out: { path: string; dist: number; elev: number | null }[] = [];
    const half = HALF_ANGLE * D2R;
    for (let a = 0; a < fan.ang_cells; a++) {
      const tA = -half + (2 * half * a) / fan.ang_cells;
      const tB = -half + (2 * half * (a + 1)) / fan.ang_cells;
      for (let b = 0; b < fan.rad_cells; b++) {
        const r0 = (fan.range_m * b) / fan.rad_cells;
        const r1 = (fan.range_m * (b + 1)) / fan.rad_cells;
        const p0 = px(tA, r0);
        const p1 = px(tB, r0);
        const p2 = px(tB, r1);
        const p3 = px(tA, r1);
        out.push({
          path: `M${p0[0].toFixed(1)} ${p0[1].toFixed(1)}L${p1[0].toFixed(1)} ${p1[1].toFixed(1)}L${p2[0].toFixed(1)} ${p2[1].toFixed(1)}L${p3[0].toFixed(1)} ${p3[1].toFixed(1)}Z`,
          dist: (fan.range_m * (b + 0.5)) / fan.rad_cells,
          elev: fan.elev[a * fan.rad_cells + b],
        });
      }
    }
    return out;
  });

  // Continuous heatmap ramp red → orange → yellow → green (t ∈ 0…1, clamped).
  const RAMP: [number, number, number][] = [
    [231, 76, 60], // red    (terrain at/above reference)
    [230, 126, 34], // orange
    [241, 196, 15], // yellow
    [46, 204, 113], // green  (terrain `scale` below)
  ];
  function ramp(t: number): string {
    const x = Math.min(1, Math.max(0, t)) * (RAMP.length - 1);
    const i = Math.min(RAMP.length - 2, Math.floor(x));
    const f = x - i;
    const a = RAMP[i];
    const b = RAMP[i + 1];
    return `rgb(${Math.round(a[0] + (b[0] - a[0]) * f)},${Math.round(a[1] + (b[1] - a[1]) * f)},${Math.round(a[2] + (b[2] - a[2]) * f)})`;
  }

  /**
   * Continuous clearance colour over the total `radarScale` (null = unpainted = bg):
   * a smooth red→green ramp from 0…scale (clearance < 0 clamps to red), off above.
   */
  function colorFor(elev: number | null, dist: number): string | null {
    if (elev == null) return null;
    const ref = predictive ? curAltMsl + slope * dist : curAltMsl;
    const clear = ref - elev;
    if (clear >= radarScale) return null; // more than `scale` below → off
    return ramp(clear / radarScale); // < 0 clamps to red
  }

  // Range arcs (thirds) + their distance labels along the heading line
  const ARCS = 3;
  const arcs = $derived.by(() => {
    const half = HALF_ANGLE * D2R;
    const segs = ANG_CELLS;
    const out: { d: string; r: number; label: string }[] = [];
    for (let k = 1; k <= ARCS; k++) {
      const dist = (range * k) / ARCS;
      let d = '';
      for (let s = 0; s <= segs; s++) {
        const th = -half + (2 * half * s) / segs;
        const p = px(th, dist);
        d += `${s === 0 ? 'M' : 'L'}${p[0].toFixed(1)} ${p[1].toFixed(1)}`;
      }
      const c = convertLength(dist, interfaceSettings.distanceUnit);
      out.push({ d, r: (dist / range) * R, label: Math.round(c.value).toString() });
    }
    return out;
  });

  // Fan outline (two edges + outer arc reuses the last arc)
  const edges = $derived.by(() => {
    const half = HALF_ANGLE * D2R;
    const l = px(-half, range);
    const r = px(half, range);
    return `M${cx.toFixed(1)} ${apexY.toFixed(1)}L${l[0].toFixed(1)} ${l[1].toFixed(1)}M${cx.toFixed(1)} ${apexY.toFixed(1)}L${r[0].toFixed(1)} ${r[1].toFixed(1)}`;
  });
  const headingTop = $derived(px(0, range));

  // Fan sector outline (apex → outer arc → apex) — used to clip the blurred cells
  const fanSector = $derived.by(() => {
    const half = HALF_ANGLE * D2R;
    let d = `M${cx.toFixed(1)} ${apexY.toFixed(1)}`;
    for (let s = 0; s <= ANG_CELLS; s++) {
      const th = -half + (2 * half * s) / ANG_CELLS;
      const p = px(th, range);
      d += `L${p[0].toFixed(1)} ${p[1].toFixed(1)}`;
    }
    return d + 'Z';
  });

  // (track readout removed — the heading only drives sampling, not a label)

  function toggleMode() {
    patchTerrainAnalysis({ radarPredictive: !predictive });
  }
  function cycleScale() {
    const i = RADAR_SCALES.indexOf(radarScale);
    patchTerrainAnalysis({ radarScale: RADAR_SCALES[(i + 1) % RADAR_SCALES.length] });
  }
</script>

<div class="widget-card" style="width:{size}px; height:{size}px;">
  {#if hasData}
    <svg width={size} height={size} viewBox="0 0 {size} {size}">
      <defs>
        <!-- Texture filter: turbulence displaces the cell edges into an organic
             heatmap grain — dissolves the grid blocks without blurring detail. -->
        <filter id="radarTex" x="-20%" y="-20%" width="140%" height="140%">
          <feTurbulence type="fractalNoise" baseFrequency={TEX_FREQ} numOctaves="2" seed="7" result="noise" />
          <feDisplacementMap in="SourceGraphic" in2="noise" scale={texDisp} xChannelSelector="R" yChannelSelector="G" result="disp" />
          <feGaussianBlur in="disp" stdDeviation={blurStd} />
        </filter>
        <clipPath id="radarFanClip">
          <path d={fanSector} />
        </clipPath>
      </defs>

      <!-- coloured terrain cells, textured into a heatmap and clipped to the fan -->
      <g clip-path="url(#radarFanClip)" filter="url(#radarTex)" opacity="0.85">
        {#each cells as cell}
          {@const fill = colorFor(cell.elev, cell.dist)}
          {#if fill}<path d={cell.path} {fill} />{/if}
        {/each}
      </g>

      <!-- range arcs + fan edges -->
      {#each arcs as arc}
        <path class="arc" d={arc.d} />
      {/each}
      <path class="edge" d={edges} />

      <!-- heading line + distance labels -->
      <line class="hdg" x1={cx} y1={apexY} x2={headingTop[0]} y2={headingTop[1]} />
      {#each arcs as arc}
        <text class="dist-label" x={cx + 3} y={apexY - arc.r + fsVal * 0.4} style="font-size:{fsVal * 0.85}px;">{arc.label}</text>
      {/each}

      <!-- UAV marker at the apex -->
      <circle class="uav-ring" cx={cx} cy={apexY} r={ringR} />
      <circle class="uav-dot" cx={cx} cy={apexY} r={Math.max(2, size * 0.014)} />
    </svg>

    <button class="mode left" onclick={cycleScale} title="Clearance colour scale (total range)">
      {scaleLabel}
    </button>
    <button class="mode right" onclick={toggleMode} title="Colouring reference: current altitude vs sink-angle prediction">
      {predictive ? 'PRED' : 'REL'}
    </button>
  {:else}
    <div class="placeholder">Terrain Radar — no GPS</div>
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
  .arc {
    fill: none;
    stroke: rgba(255, 255, 255, 0.22);
    stroke-width: 1;
  }
  .edge {
    fill: none;
    stroke: rgba(255, 255, 255, 0.28);
    stroke-width: 1;
  }
  .hdg {
    stroke: rgba(255, 255, 255, 0.35);
    stroke-width: 1;
    stroke-dasharray: 4 3;
  }
  .dist-label {
    fill: #b8b8b8;
    font-variant-numeric: tabular-nums;
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
  .mode {
    position: absolute;
    top: 4px;
    padding: 1px 5px;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.05em;
    color: #37a8db;
    background: rgba(0, 0, 0, 0.4);
    border: 1px solid rgba(55, 168, 219, 0.4);
    border-radius: 4px;
    cursor: pointer;
  }
  .mode.left {
    left: 4px;
  }
  .mode.right {
    right: 4px;
  }
  .mode:hover {
    background: rgba(55, 168, 219, 0.18);
  }
  .placeholder {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #888;
    font-size: 12px;
    text-align: center;
    padding: 0 8px;
  }
</style>
