<script lang="ts">
  import type { ProfileData } from '$lib/helpers/terrainProfile';
  import { convertAltitude } from '$lib/utils/units';
  import type { InterfaceSettings } from '$lib/stores/settings';

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
    settings,
    live = false,
    follow = false,
    groundClearance = 50,
    warnThreshold = null,
    activeStartDist = -Infinity,
    activeEndDist = Infinity,
    placedDist = null,
    previewPath = null,
    viewStart = $bindable(null),
    viewEnd = $bindable(null),
    onhover,
    onpick,
  }: {
    data: ProfileData | null;
    datum?: 'msl' | 'agl';
    settings: InterfaceSettings;
    /** Live Track mode (FC connected) — min window 250 m */
    live?: boolean;
    /** Follow the newest data on the right (no pan, zoom-only) */
    follow?: boolean;
    groundClearance?: number;
    /** Clearance level below which the path is coloured unsafe (default = groundClearance) */
    warnThreshold?: number | null;
    /** Distance window where clearance alerts apply (excludes take-off/landing) */
    activeStartDist?: number;
    activeEndDist?: number;
    /** Distance of the pinned map marker along this profile (null = none/off-path) */
    placedDist?: number | null;
    /** Corrected altitude line (MSL) to preview over the current path */
    previewPath?: { dist: number; altMsl: number }[] | null;
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
      // include the correction preview (clearance of the corrected line)
      if (previewPath) {
        for (const p of previewPath) {
          const ter = terrainAtDist(p.dist);
          if (ter == null) continue;
          const c = p.altMsl - ter;
          if (c < min) min = c;
          if (c > max) max = c;
        }
      }
      const pad = Math.max(5, (max - min) * 0.1);
      return { min: min - pad, max: max + pad };
    }
    let min = data.minElev;
    let max = data.maxElev;
    // include the correction preview (raised line can exceed the current max)
    if (previewPath) {
      for (const p of previewPath) {
        if (p.altMsl < min) min = p.altMsl;
        if (p.altMsl > max) max = p.altMsl;
      }
    }
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
    const jumps = data ? data.jumpLegs : [];
    const warnLevel = warnThreshold ?? groundClearance;
    const runs: { kind: 'normal' | 'unsafe' | 'jump'; pts: { x: number; y: number }[] }[] = [];
    let prev: { x: number; y: number } | null = null;
    for (let i = 0; i < d.length; i++) {
      const defined = datum === 'agl' ? d[i].clearance != null : d[i].pathAlt != null;
      if (!defined) {
        prev = null;
        continue;
      }
      const v = datum === 'agl' ? (d[i].clearance as number) : (d[i].pathAlt as number);
      const dist = d[i].dist;
      const pt = { x: xS(dist), y: yS(v) };
      const c = d[i].clearance;
      const isJump = jumps.some((l) => dist >= l.start && dist <= l.end);
      const unsafe = c != null && c < warnLevel && dist >= activeStartDist && dist <= activeEndDist;
      const kind: 'normal' | 'unsafe' | 'jump' = isJump ? 'jump' : unsafe ? 'unsafe' : 'normal';
      let run = runs[runs.length - 1];
      if (!run || run.kind !== kind || prev == null) {
        run = { kind, pts: [] };
        if (prev) run.pts.push(prev);
        runs.push(run);
      }
      run.pts.push(pt);
      prev = pt;
    }
    return runs.map((r) => ({
      kind: r.kind,
      points: r.pts.map((p) => `${p.x.toFixed(1)},${p.y.toFixed(1)}`).join(' '),
    }));
  });

  const markerPts = $derived.by(() => {
    if (!data || data.source !== 'waypoint') return [] as { x: number; y: number; number: number }[];
    const out: { x: number; y: number; number: number }[] = [];
    for (const m of data.markers) {
      if (m.repeat && !m.resume) continue; // jump revisit — no dot (but the resume point is shown)
      let v: number | null;
      if (datum === 'agl') v = m.ground != null ? m.altMsl - m.ground : null;
      else v = m.altMsl;
      if (v == null) continue;
      out.push({ x: xS(m.dist), y: yS(v), number: m.number });
    }
    return out;
  });

  // Waypoint number labels: lifted into a band at the TOP of the plot (inside the area) with
  // a thin dashed connector down to the dot — like the 3D-map markers. Greedily staggered over
  // a few rows so dense patterns (survey grids) don't overlap; any that still can't fit without
  // overlapping are dropped (the dot stays). Avoids the old overlap when many WPs sit close in X.
  const WP_LABEL_ROWS = 3;
  const WP_LABEL_GAP = 20; // min px between two labels in the same row
  const WP_LABEL_TOP = PAD.t + 9;
  const WP_LABEL_ROW_H = 13;
  const wpLabels = $derived.by(() => {
    const pts = [...markerPts].sort((a, b) => a.x - b.x);
    const lastX = new Array(WP_LABEL_ROWS).fill(-Infinity);
    const out: { x: number; dotY: number; labelY: number; number: number }[] = [];
    for (const m of pts) {
      for (let row = 0; row < WP_LABEL_ROWS; row++) {
        if (m.x - lastX[row] >= WP_LABEL_GAP) {
          lastX[row] = m.x;
          out.push({ x: m.x, dotY: m.y, labelY: WP_LABEL_TOP + row * WP_LABEL_ROW_H, number: m.number });
          break;
        }
      }
    }
    return out;
  });

  const cutXs = $derived(data ? data.cuts.map((c) => xS(c)) : []);

  const jumpTargetPts = $derived.by(() => {
    if (!data || data.source !== 'waypoint') return [] as { x: number; y: number; number: number }[];
    const out: { x: number; y: number; number: number }[] = [];
    for (const m of data.markers) {
      if (!m.jumpTarget) continue;
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

  // Tick counts scale with the available plot size so the labels never overlap in compact
  // layouts / on small screens (each Y label needs roughly its font-height of vertical room).
  const xTickCount = $derived(Math.max(2, Math.min(6, Math.round(plotW / 120))));
  const yTickCount = $derived(Math.max(2, Math.min(6, Math.round(plotH / 36))));
  const xTicks = $derived(niceTicks(xDomain.a, xDomain.b, xTickCount));
  const yTicks = $derived(niceTicks(yDomain.min, yDomain.max, yTickCount));

  // X axis: distance in the user's distance unit (no auto-switch mid-pan)
  const xConv = $derived.by(() => {
    const range = xDomain.b - xDomain.a;
    if (settings.distanceUnit === 'imperial') {
      const ft = range * 3.280839895;
      return ft >= 5280
        ? { factor: 3.280839895 / 5280, unit: 'mi', digits: 1 }
        : { factor: 3.280839895, unit: 'ft', digits: 0 };
    }
    return range > 2000 ? { factor: 0.001, unit: 'km', digits: 1 } : { factor: 1, unit: 'm', digits: 0 };
  });
  function fmtX(v: number): string {
    return (v * xConv.factor).toFixed(xConv.digits);
  }

  // Y axis: altitude in the user's altitude unit
  const altUnit = $derived(convertAltitude(0, settings.altitudeUnit).unit);
  function fmtY(v: number): string {
    return Math.round(convertAltitude(v, settings.altitudeUnit).value).toString();
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
      if (live && follow) return; // no panning while following the live track
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
    const minRange = live ? 250 : 50; // smallest visible window (m)
    const factor = e.deltaY < 0 ? 0.85 : 1 / 0.85;
    let newRange = Math.max(range * factor, minRange);

    // Full zoom-out → auto-fit the whole (growing) range, regardless of follow
    // (only once the track is longer than the minimum window)
    if (newRange >= data.totalDist && data.totalDist > minRange) {
      viewStart = null;
      viewEnd = null;
      return;
    }
    newRange = Math.min(newRange, Math.max(data.totalDist, minRange));

    // While following, keep the right edge pinned (zoom adjusts the window only);
    // before the track reaches the window width, keep [0, window].
    if (live && follow) {
      const ve = Math.max(newRange, data.totalDist);
      viewEnd = ve;
      viewStart = Math.max(0, ve - newRange);
      return;
    }
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

  /** Terrain elevation interpolated at an arbitrary distance (for AGL preview). */
  function terrainAtDist(d: number): number | null {
    if (!data) return null;
    const t = data.terrain;
    const n = t.length;
    if (n === 0) return null;
    let i = lowerBound(t, d);
    if (i <= 0) i = 1;
    if (i >= n) i = n - 1;
    const a = t[i - 1];
    const b = t[i];
    if (a.elev != null && b.elev != null) {
      const span = b.dist - a.dist || 1;
      return a.elev + ((b.elev - a.elev) * (d - a.dist)) / span;
    }
    return (b.elev ?? a.elev) ?? null;
  }

  const previewRuns = $derived.by(() => {
    if (!previewPath || previewPath.length === 0 || !data) return [] as string[];
    const cuts = data.cuts;
    const runs: string[][] = [];
    let cur: string[] = [];
    let prevDist: number | null = null;
    for (const p of previewPath) {
      const pd = prevDist;
      // break the line at a jump cut between consecutive points
      if (pd != null && cuts.some((c) => c > pd && c < p.dist)) {
        if (cur.length) runs.push(cur);
        cur = [];
      }
      let y: number;
      if (datum === 'agl') {
        const ter = terrainAtDist(p.dist);
        if (ter == null) {
          if (cur.length) runs.push(cur);
          cur = [];
          prevDist = p.dist;
          continue;
        }
        y = yS(p.altMsl - ter);
      } else {
        y = yS(p.altMsl);
      }
      cur.push(`${xS(p.dist).toFixed(1)},${y.toFixed(1)}`);
      prevDist = p.dist;
    }
    if (cur.length) runs.push(cur);
    return runs.map((r) => r.join(' '));
  });

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
    <text class="axis-unit" x={PAD.l - 6} y={PAD.t - 2} text-anchor="end">{altUnit}</text>
    {#each yTicks as ty}
      <line class="grid" x1={PAD.l} y1={yS(ty)} x2={PAD.l + plotW} y2={yS(ty)} />
      <text class="axis-label" x={PAD.l - 6} y={yS(ty) + 4} text-anchor="end">{fmtY(ty)}</text>
    {/each}

    <!-- X grid + labels -->
    {#each xTicks as tx}
      {#if tx >= xDomain.a && tx <= xDomain.b}
        <line class="grid" x1={xS(tx)} y1={PAD.t} x2={xS(tx)} y2={baselineY} />
        <text class="axis-label" x={xS(tx)} y={baselineY + 18} text-anchor="middle">{fmtX(tx)}</text>
      {/if}
    {/each}
    <text class="axis-unit" x={PAD.l + plotW} y={baselineY + 18} text-anchor="end">
      {xConv.unit}
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

        <!-- Correction preview — behind the WP path so it never hides it -->
        {#each previewRuns as pts}
          <polyline class="preview-line" points={pts} />
        {/each}

        <!-- main path (non-jump), then jump legs drawn on top so they stay visible -->
        {#each pathRuns as run}
          {#if run.kind !== 'jump'}
            <polyline class="path-line" class:unsafe={run.kind === 'unsafe'} points={run.points} />
          {/if}
        {/each}
        {#each pathRuns as run}
          {#if run.kind === 'jump'}
            <polyline class="path-line jump" points={run.points} />
          {/if}
        {/each}

        <!-- Jump cut markers -->
        {#each cutXs as cx}
          <line class="cut-line" x1={cx} y1={PAD.t} x2={cx} y2={baselineY} />
        {/each}

        {#each markerPts as m}
          <circle class="wp-dot" cx={m.x} cy={m.y} r="4" />
        {/each}
        <!-- WP numbers in a staggered top band, with a dashed connector down to the dot -->
        {#each wpLabels as l}
          {#if l.dotY - l.labelY > 14}
            <line class="wp-connector" x1={l.x} y1={l.labelY + 3} x2={l.x} y2={l.dotY - 5} />
          {/if}
          <text class="wp-num" x={l.x} y={l.labelY} text-anchor="middle">{l.number}</text>
        {/each}

        <!-- Jump-target markers (end of each jump-back leg) -->
        {#each jumpTargetPts as j}
          <polygon
            class="jump-target"
            points="{j.x},{j.y - 5} {j.x + 5},{j.y} {j.x},{j.y + 5} {j.x - 5},{j.y}"
          />
          <text class="jump-target-num" x={j.x} y={j.y - 9} text-anchor="middle">↩{j.number}</text>
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
  .path-line.jump {
    stroke: #b56be0;
    stroke-width: 2.5;
    stroke-dasharray: 10 5;
    opacity: 1;
  }
  .jump-target {
    fill: #b56be0;
    stroke: #fff;
    stroke-width: 1;
  }
  .jump-target-num {
    fill: #d9b6ef;
    font-size: 12px;
    font-weight: 600;
  }
  .cut-line {
    stroke: #c98a2b;
    stroke-width: 1;
    stroke-dasharray: 2 4;
    opacity: 0.8;
  }
  .preview-line {
    fill: none;
    stroke: #2ecc71;
    stroke-width: 2;
    stroke-dasharray: 6 4;
    stroke-linejoin: round;
    stroke-linecap: round;
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
  .wp-connector {
    stroke: rgba(207, 232, 245, 0.35);
    stroke-width: 1;
    stroke-dasharray: 2 3;
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
