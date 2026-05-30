<script lang="ts">
  import type { ProfileData } from '$lib/helpers/terrainProfile';

  export interface HoverInfo {
    dist: number;
    terrainElev: number | null;
    pathAlt: number | null;
    clearance: number | null;
    lat: number;
    lon: number;
  }

  let {
    data,
    datum = 'msl',
    groundClearance = 50,
    activeStartDist = -Infinity,
    activeEndDist = Infinity,
    placedDist = null,
    viewStart = $bindable(null),
    viewEnd = $bindable(null),
    onhover,
    onpick,
  }: {
    data: ProfileData | null;
    datum?: 'msl' | 'agl';
    groundClearance?: number;
    /** Distance window where clearance alerts apply (excludes take-off/landing) */
    activeStartDist?: number;
    activeEndDist?: number;
    /** Distance of the pinned map marker along this profile (null = none/off-path) */
    placedDist?: number | null;
    viewStart?: number | null;
    viewEnd?: number | null;
    onhover?: (info: HoverInfo | null) => void;
    /** Click (not drag) on the chart — pin/unpin the map marker at this point */
    onpick?: (p: { lat: number; lon: number }) => void;
  } = $props();

  const PAD = { l: 54, r: 16, t: 14, b: 32 };

  let svgEl: SVGSVGElement;
  let cw = $state(900);
  let ch = $state(320);

  const plotW = $derived(Math.max(10, cw - PAD.l - PAD.r));
  const plotH = $derived(Math.max(10, ch - PAD.t - PAD.b));
  const baselineY = $derived(PAD.t + plotH);

  const xDomain = $derived.by(() => {
    const total = data?.totalDist ?? 1;
    let a = viewStart ?? 0;
    let b = viewEnd ?? total;
    if (b <= a) b = a + 1;
    return { a, b };
  });

  const yDomain = $derived.by(() => {
    if (!data) return { min: 0, max: 100 };
    if (datum === 'agl') {
      let min = 0;
      let max = groundClearance;
      for (const c of data.clearance) {
        if (c != null) {
          if (c < min) min = c;
          if (c > max) max = c;
        }
      }
      const pad = Math.max(5, (max - min) * 0.1);
      return { min: min - pad, max: max + pad };
    }
    const min = data.minElev;
    const max = data.maxElev;
    const pad = Math.max(10, (max - min) * 0.1);
    return { min: min - pad, max: max + pad };
  });

  function xS(d: number): number {
    return PAD.l + ((d - xDomain.a) / (xDomain.b - xDomain.a)) * plotW;
  }
  function yS(v: number): number {
    return PAD.t + ((yDomain.max - v) / (yDomain.max - yDomain.min)) * plotH;
  }
  function distFromX(px: number): number {
    return xDomain.a + ((px - PAD.l) / plotW) * (xDomain.b - xDomain.a);
  }

  // ── Visible-range decimation ───────────────────────────────────────
  // Render only the visible slice, reduced to ~plotW resolution. Per bucket we
  // keep the worst-clearance (or peak-terrain) sample so peaks and unsafe spots
  // survive — full-resolution data is still used for the readouts/min-clearance.

  interface DSample {
    dist: number;
    elev: number | null;
    pathAlt: number | null;
    clearance: number | null;
    lat: number;
    lon: number;
  }

  function lowerBound(arr: { dist: number }[], x: number): number {
    let lo = 0;
    let hi = arr.length;
    while (lo < hi) {
      const m = (lo + hi) >> 1;
      if (arr[m].dist < x) lo = m + 1;
      else hi = m;
    }
    return lo;
  }

  const display = $derived.by<DSample[]>(() => {
    if (!data || data.terrain.length === 0) return [];
    const t = data.terrain;
    const span = xDomain.b - xDomain.a;
    const margin = span * 0.02;
    let i0 = Math.max(0, lowerBound(t, xDomain.a - margin) - 1);
    let i1 = Math.min(t.length - 1, lowerBound(t, xDomain.b + margin));
    const n = i1 - i0 + 1;
    if (n <= 0) return [];

    const targetBuckets = Math.max(2, Math.floor(plotW * 1.5));
    if (n <= targetBuckets) {
      const out: DSample[] = [];
      for (let i = i0; i <= i1; i++) {
        out.push({
          dist: t[i].dist,
          elev: t[i].elev,
          pathAlt: data.pathAtTerrain[i],
          clearance: data.clearance[i],
          lat: t[i].lat,
          lon: t[i].lon,
        });
      }
      return out;
    }

    const out: DSample[] = [];
    const bucket = n / targetBuckets;
    for (let bk = 0; bk < targetBuckets; bk++) {
      const s = i0 + Math.floor(bk * bucket);
      const e = Math.min(i1, i0 + Math.floor((bk + 1) * bucket) - 1);
      let worst = Infinity;
      let repi = s;
      let maxElev = -Infinity;
      let maxElevI = s;
      for (let i = s; i <= e; i++) {
        const c = data.clearance[i];
        if (c != null && c < worst) {
          worst = c;
          repi = i;
        }
        const el = t[i].elev;
        if (el != null && el > maxElev) {
          maxElev = el;
          maxElevI = i;
        }
      }
      const idx = worst === Infinity ? maxElevI : repi;
      out.push({
        dist: t[idx].dist,
        elev: t[idx].elev,
        pathAlt: data.pathAtTerrain[idx],
        clearance: data.clearance[idx],
        lat: t[idx].lat,
        lon: t[idx].lon,
      });
    }
    return out;
  });

  // ── Series (built from the decimated visible slice) ────────────────

  const terrainAreaPath = $derived.by(() => {
    if (datum === 'agl') return '';
    const d = display;
    let path = '';
    let runStart = -1;
    const flush = (end: number) => {
      if (runStart < 0) return;
      path += ` M ${xS(d[runStart].dist).toFixed(1)} ${baselineY.toFixed(1)}`;
      for (let k = runStart; k <= end; k++) path += ` L ${xS(d[k].dist).toFixed(1)} ${yS(d[k].elev as number).toFixed(1)}`;
      path += ` L ${xS(d[end].dist).toFixed(1)} ${baselineY.toFixed(1)} Z`;
      runStart = -1;
    };
    for (let i = 0; i < d.length; i++) {
      if (d[i].elev == null) flush(i - 1);
      else if (runStart < 0) runStart = i;
    }
    flush(d.length - 1);
    return path;
  });

  const floorPath = $derived.by(() => {
    if (datum === 'agl') {
      const y = yS(groundClearance);
      return `M ${PAD.l} ${y.toFixed(1)} L ${(PAD.l + plotW).toFixed(1)} ${y.toFixed(1)}`;
    }
    const d = display;
    let path = '';
    let open = false;
    for (let i = 0; i < d.length; i++) {
      if (d[i].elev == null) {
        open = false;
        continue;
      }
      const x = xS(d[i].dist);
      const y = yS((d[i].elev as number) + groundClearance);
      path += `${open ? ' L' : ' M'} ${x.toFixed(1)} ${y.toFixed(1)}`;
      open = true;
    }
    return path;
  });

  const pathRuns = $derived.by(() => {
    const d = display;
    const runs: { unsafe: boolean; pts: { x: number; y: number }[] }[] = [];
    let prev: { x: number; y: number } | null = null;
    for (let i = 0; i < d.length; i++) {
      const defined = datum === 'agl' ? d[i].clearance != null : d[i].pathAlt != null;
      if (!defined) {
        prev = null;
        continue;
      }
      const v = datum === 'agl' ? (d[i].clearance as number) : (d[i].pathAlt as number);
      const pt = { x: xS(d[i].dist), y: yS(v) };
      const c = d[i].clearance;
      const unsafe =
        c != null && c < groundClearance && d[i].dist >= activeStartDist && d[i].dist <= activeEndDist;
      let run = runs[runs.length - 1];
      if (!run || run.unsafe !== unsafe || prev == null) {
        run = { unsafe, pts: [] };
        if (prev) run.pts.push(prev);
        runs.push(run);
      }
      run.pts.push(pt);
      prev = pt;
    }
    return runs.map((r) => ({
      unsafe: r.unsafe,
      points: r.pts.map((p) => `${p.x.toFixed(1)},${p.y.toFixed(1)}`).join(' '),
    }));
  });

  const markerPts = $derived.by(() => {
    if (!data || data.source !== 'waypoint') return [] as { x: number; y: number; number: number }[];
    const out: { x: number; y: number; number: number }[] = [];
    for (const m of data.markers) {
      let v: number | null;
      if (datum === 'agl') v = m.ground != null ? m.altMsl - m.ground : null;
      else v = m.altMsl;
      if (v == null) continue;
      out.push({ x: xS(m.dist), y: yS(v), number: m.number });
    }
    return out;
  });

  // ── Axis ticks ─────────────────────────────────────────────────────

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

  const xTicks = $derived(niceTicks(xDomain.a, xDomain.b, 6));
  const yTicks = $derived(niceTicks(yDomain.min, yDomain.max, 5));
  const xUnitKm = $derived(xDomain.b - xDomain.a > 2000);
  function fmtX(v: number): string {
    return xUnitKm ? (v / 1000).toFixed(1) : Math.round(v).toString();
  }

  // ── Interaction: hover, zoom, pan ──────────────────────────────────

  let hoverIdx = $state<number | null>(null);
  let dragging = false;
  let dragMoved = false;
  let dragClientX = 0;
  let dragA = 0;
  let dragB = 0;

  function nearestDisplayIdx(d: number): number {
    const arr = display;
    if (arr.length === 0) return 0;
    let best = 0;
    let bd = Infinity;
    for (let i = 0; i < arr.length; i++) {
      const delta = Math.abs(arr[i].dist - d);
      if (delta < bd) {
        bd = delta;
        best = i;
      }
    }
    return best;
  }

  function emitHover() {
    if (hoverIdx == null || hoverIdx >= display.length) {
      onhover?.(null);
      return;
    }
    const s = display[hoverIdx];
    onhover?.({ dist: s.dist, terrainElev: s.elev, pathAlt: s.pathAlt, clearance: s.clearance, lat: s.lat, lon: s.lon });
  }

  function onPointerMove(e: PointerEvent) {
    if (!data) return;
    const rect = svgEl.getBoundingClientRect();
    const px = e.clientX - rect.left;

    if (dragging) {
      if (Math.abs(e.clientX - dragClientX) > 4) dragMoved = true;
      const range = dragB - dragA;
      const dDist = (-(e.clientX - dragClientX) / plotW) * range;
      let na = dragA + dDist;
      let nb = dragB + dDist;
      if (na < 0) {
        na = 0;
        nb = range;
      }
      if (nb > data.totalDist) {
        nb = data.totalDist;
        na = nb - range;
      }
      viewStart = na;
      viewEnd = nb;
      return;
    }

    if (px < PAD.l || px > PAD.l + plotW) {
      hoverIdx = null;
      emitHover();
      return;
    }
    hoverIdx = nearestDisplayIdx(distFromX(px));
    emitHover();
  }

  function onPointerLeave() {
    if (dragging) return;
    hoverIdx = null;
    emitHover();
  }

  function onPointerDown(e: PointerEvent) {
    if (!data) return;
    dragging = true;
    dragMoved = false;
    dragClientX = e.clientX;
    dragA = xDomain.a;
    dragB = xDomain.b;
    svgEl.setPointerCapture(e.pointerId);
  }

  function onPointerUp(e: PointerEvent) {
    dragging = false;
    try {
      svgEl.releasePointerCapture(e.pointerId);
    } catch {
      /* ignore */
    }
    // A click (no drag) on the plot pins/unpins the map marker
    if (!dragMoved && data) {
      const rect = svgEl.getBoundingClientRect();
      const px = e.clientX - rect.left;
      if (px >= PAD.l && px <= PAD.l + plotW) {
        const s = display[nearestDisplayIdx(distFromX(px))];
        if (s) onpick?.({ lat: s.lat, lon: s.lon });
      }
    }
  }

  function onWheel(e: WheelEvent) {
    if (!data) return;
    e.preventDefault();
    const rect = svgEl.getBoundingClientRect();
    const px = e.clientX - rect.left;
    const cursorDist = distFromX(Math.min(Math.max(px, PAD.l), PAD.l + plotW));
    const range = xDomain.b - xDomain.a;
    const minRange = Math.max(50, data.totalDist / 500);
    const factor = e.deltaY < 0 ? 0.85 : 1 / 0.85;
    const newRange = Math.min(data.totalDist, Math.max(range * factor, minRange));
    const frac = (cursorDist - xDomain.a) / range;
    let na = cursorDist - frac * newRange;
    let nb = na + newRange;
    if (na < 0) {
      na = 0;
      nb = newRange;
    }
    if (nb > data.totalDist) {
      nb = data.totalDist;
      na = Math.max(0, nb - newRange);
    }
    viewStart = na;
    viewEnd = nb;
  }

  function onDblClick() {
    viewStart = null;
    viewEnd = null;
  }

  const placedX = $derived(placedDist != null && data ? xS(placedDist) : null);

  const hoverX = $derived(
    hoverIdx != null && hoverIdx < display.length ? xS(display[hoverIdx].dist) : null,
  );
  const hoverTerrainY = $derived(
    hoverIdx != null && hoverIdx < display.length && display[hoverIdx].elev != null
      ? yS(datum === 'agl' ? 0 : (display[hoverIdx].elev as number))
      : null,
  );
  const hoverPathY = $derived.by(() => {
    if (hoverIdx == null || hoverIdx >= display.length) return null;
    const v = datum === 'agl' ? display[hoverIdx].clearance : display[hoverIdx].pathAlt;
    return v != null ? yS(v) : null;
  });
</script>

<div class="chart" bind:clientWidth={cw} bind:clientHeight={ch}>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <svg
    bind:this={svgEl}
    width={cw}
    height={ch}
    onpointermove={onPointerMove}
    onpointerleave={onPointerLeave}
    onpointerdown={onPointerDown}
    onpointerup={onPointerUp}
    onwheel={onWheel}
    ondblclick={onDblClick}
    role="img"
    aria-label="Terrain profile"
  >
    <defs>
      <linearGradient id="terrainGrad" x1="0" y1="0" x2="0" y2="1">
        <stop offset="0%" stop-color="#6e5b46" />
        <stop offset="100%" stop-color="#3c322789" />
      </linearGradient>
      <clipPath id="plotClip">
        <rect x={PAD.l} y={PAD.t} width={plotW} height={plotH} />
      </clipPath>
    </defs>

    <!-- Y grid + labels -->
    {#each yTicks as ty}
      <line class="grid" x1={PAD.l} y1={yS(ty)} x2={PAD.l + plotW} y2={yS(ty)} />
      <text class="axis-label" x={PAD.l - 6} y={yS(ty) + 4} text-anchor="end">{Math.round(ty)}</text>
    {/each}

    <!-- X grid + labels -->
    {#each xTicks as tx}
      {#if tx >= xDomain.a && tx <= xDomain.b}
        <line class="grid" x1={xS(tx)} y1={PAD.t} x2={xS(tx)} y2={baselineY} />
        <text class="axis-label" x={xS(tx)} y={baselineY + 18} text-anchor="middle">{fmtX(tx)}</text>
      {/if}
    {/each}
    <text class="axis-unit" x={PAD.l + plotW} y={baselineY + 18} text-anchor="end">
      {xUnitKm ? 'km' : 'm'}
    </text>

    <g clip-path="url(#plotClip)">
      {#if data}
        {#if datum === 'agl'}
          <rect class="terrain-fill" x={PAD.l} y={yS(0)} width={plotW} height={Math.max(0, baselineY - yS(0))} />
          <line class="ground-line" x1={PAD.l} y1={yS(0)} x2={PAD.l + plotW} y2={yS(0)} />
        {:else}
          <path class="terrain-fill" d={terrainAreaPath} />
        {/if}

        <path class="floor-line" d={floorPath} />

        {#each pathRuns as run}
          <polyline class="path-line" class:unsafe={run.unsafe} points={run.points} />
        {/each}

        {#each markerPts as m}
          <circle class="wp-dot" cx={m.x} cy={m.y} r="4" />
          <text class="wp-num" x={m.x} y={m.y - 9} text-anchor="middle">{m.number}</text>
        {/each}

        {#if placedX != null}
          <line class="placed-line" x1={placedX} y1={PAD.t} x2={placedX} y2={baselineY} />
          <polygon
            class="placed-flag"
            points="{placedX - 5},{PAD.t} {placedX + 5},{PAD.t} {placedX},{PAD.t + 8}"
          />
        {/if}

        {#if hoverX != null}
          <line class="crosshair" x1={hoverX} y1={PAD.t} x2={hoverX} y2={baselineY} />
          {#if hoverTerrainY != null}
            <circle class="hover-dot terrain" cx={hoverX} cy={hoverTerrainY} r="3" />
          {/if}
          {#if hoverPathY != null}
            <circle class="hover-dot path" cx={hoverX} cy={hoverPathY} r="3" />
          {/if}
        {/if}
      {/if}
    </g>

    <rect class="plot-border" x={PAD.l} y={PAD.t} width={plotW} height={plotH} />
  </svg>
</div>

<style>
  .chart {
    width: 100%;
    height: 100%;
    min-height: 0;
  }
  svg {
    display: block;
    cursor: crosshair;
    touch-action: none;
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
  .axis-label {
    fill: #949494;
    font-size: 13px;
  }
  .axis-unit {
    fill: #6e6e6e;
    font-size: 13px;
  }
  .terrain-fill {
    fill: url(#terrainGrad);
    stroke: #8a7250;
    stroke-width: 1;
  }
  .ground-line {
    stroke: #8a7250;
    stroke-width: 1.5;
  }
  .floor-line {
    fill: none;
    stroke: rgba(231, 76, 60, 0.55);
    stroke-width: 1;
    stroke-dasharray: 4 4;
  }
  .path-line {
    fill: none;
    stroke: #37a8db;
    stroke-width: 2;
    stroke-linejoin: round;
    stroke-linecap: round;
  }
  .path-line.unsafe {
    stroke: #e74c3c;
  }
  .wp-dot {
    fill: #37a8db;
    stroke: #fff;
    stroke-width: 1.5;
  }
  .wp-num {
    fill: #cfe8f5;
    font-size: 13px;
    font-weight: 600;
  }
  .placed-line {
    stroke: #37a8db;
    stroke-width: 1.5;
  }
  .placed-flag {
    fill: #37a8db;
  }
  .crosshair {
    stroke: rgba(255, 255, 255, 0.4);
    stroke-width: 1;
    stroke-dasharray: 3 3;
  }
  .hover-dot.terrain {
    fill: #d8b88a;
  }
  .hover-dot.path {
    fill: #fff;
  }
</style>
