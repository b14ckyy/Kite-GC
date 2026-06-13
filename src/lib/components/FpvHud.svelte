<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // Projected-style FPV HUD, in two layers:
  //  • the artificial horizon (horizon line + pitch ladder) is CONFORMAL — its scaling is derived
  //    from the camera FOV so the ladder lines up with the real horizon plane in the 3D scene. It
  //    spans the full viewport because the conformal mapping needs the full vertical FOV ↔ full height.
  //  • the instrument cluster (boresight, bank scale, heading tape, speed, altitude) is a compact,
  //    schematic overlay capped at 50% of the viewport.
  // Driven purely by props (attitude + display values already converted to the user's units).

  let {
    heading = 0,   // deg, 0..360 (0 = N)
    pitch = 0,     // deg, + = nose up
    roll = 0,      // deg, + = right wing down
    speed = 0,     // already in display unit
    speedUnit = 'm/s',
    altitude = 0,  // already in display unit
    altitudeUnit = 'm',
    fov = 60,      // horizontal field of view (deg) — drives the conformal pitch scaling
  }: {
    heading?: number;
    pitch?: number;
    roll?: number;
    speed?: number;
    speedUnit?: string;
    altitude?: number;
    altitudeUnit?: string;
    fov?: number;
  } = $props();

  // viewBox geometry (16:9), centred.
  const W = 1200, H = 675;
  const CX = W / 2, CY = H / 2;
  const RAD = Math.PI / 180;
  const PXHDG = 6;     // heading tape pixels per degree (compressed, standard HUD tape)
  const HDG_HALF = 44; // heading tape half-window in degrees
  const BANK_R = 150;  // bank scale radius

  const PITCH_ANGLES = [-30, -20, -10, 10, 20, 30];
  const BANK_TICKS = [-60, -45, -30, -20, -10, 10, 20, 30, 45, 60];

  // ── Conformal AHI scaling ───────────────────────────────────────────
  // Vertical FOV from the horizontal FOV at the viewBox (≈ canvas) aspect. The AHI layer fills the
  // viewport, so the full vertical FOV maps onto the full height. The focal length (in viewBox units)
  // then places a line-of-sight at angle θ at y = CY − f·tan(θ) — a true perspective horizon that
  // tracks the real one as the FOV (zoom) changes. (Assumes the map panel is ≥16:9 wide, so `meet`
  // makes the AHI layer height-fill; if it were taller than 16:9 the mapping would be slightly off.)
  const vFovRad = $derived(2 * Math.atan(Math.tan((fov * RAD) / 2) * (H / W)));
  const fAhi = $derived((H / 2) / Math.tan(vFovRad / 2));
  const rungY = (a: number) => CY - fAhi * Math.tan((a + pitch) * RAD);
  const horizonY = $derived(rungY(0));
  const ahiRungs = $derived.by(() =>
    PITCH_ANGLES
      .filter((a) => Math.abs(a + pitch) < 85) // keep tan well-behaved; off-screen rungs are clipped anyway
      .map((a) => ({ a, y: rungY(a), dive: a < 0 })),
  );

  const cardinal = (h: number): string => {
    const n = ((h % 360) + 360) % 360;
    if (n === 0) return 'N';
    if (n === 90) return 'E';
    if (n === 180) return 'S';
    if (n === 270) return 'W';
    return String(n);
  };

  // Heading tape ticks within the visible window.
  const headingTicks = $derived.by(() => {
    const out: { x: number; major: boolean; label: string | null }[] = [];
    for (let h = 0; h < 360; h += 5) {
      let dx = ((h - heading + 540) % 360) - 180; // signed offset, -180..180
      if (Math.abs(dx) > HDG_HALF) continue;
      const major = h % 30 === 0;
      out.push({ x: CX + dx * PXHDG, major, label: major ? cardinal(h) : null });
    }
    return out;
  });

  // Bank scale tick positions on the arc (0 = straight up).
  const bankTicks = $derived(
    BANK_TICKS.map((t) => {
      const a = (t * Math.PI) / 180;
      const inner = t % 30 === 0 ? 16 : 10;
      return {
        t,
        x1: CX + BANK_R * Math.sin(a), y1: CY - BANK_R * Math.cos(a),
        x2: CX + (BANK_R - inner) * Math.sin(a), y2: CY - (BANK_R - inner) * Math.cos(a),
      };
    }),
  );

  const fmt = (n: number) => (Math.abs(n) >= 100 ? Math.round(n).toString() : n.toFixed(0));
</script>

<!-- ── Conformal artificial horizon (full viewport) ── -->
<svg class="fpv-hud fpv-ahi" viewBox="0 0 {W} {H}" preserveAspectRatio="xMidYMid meet" aria-hidden="true">
  <g transform="rotate({-roll} {CX} {CY})">
    <!-- horizon line (0°) -->
    <line class="hud-line horizon" x1={CX - 300} y1={horizonY} x2={CX - 70} y2={horizonY} />
    <line class="hud-line horizon" x1={CX + 70} y1={horizonY} x2={CX + 300} y2={horizonY} />
    <!-- pitch ladder -->
    {#each ahiRungs as r}
      <g class="hud-line" class:dive={r.dive}>
        <line x1={CX - 150} y1={r.y} x2={CX - 60} y2={r.y} class:dashed={r.dive} />
        <line x1={CX + 60} y1={r.y} x2={CX + 150} y2={r.y} class:dashed={r.dive} />
        <!-- inward end ticks (point toward the horizon) -->
        <line x1={CX - 60} y1={r.y} x2={CX - 60} y2={r.y + (r.dive ? -8 : 8)} />
        <line x1={CX + 60} y1={r.y} x2={CX + 60} y2={r.y + (r.dive ? -8 : 8)} />
        <text class="hud-text small" x={CX - 158} y={r.y + 5} text-anchor="end">{Math.abs(r.a)}</text>
        <text class="hud-text small" x={CX + 158} y={r.y + 5} text-anchor="start">{Math.abs(r.a)}</text>
      </g>
    {/each}
  </g>
</svg>

<!-- ── Instrument cluster (compact, ≤50% of the viewport) ── -->
<svg class="fpv-hud fpv-instruments" viewBox="0 0 {W} {H}" preserveAspectRatio="xMidYMid meet" aria-hidden="true">
  <!-- Fixed boresight (aircraft reference) -->
  <g class="hud-bore">
    <line x1={CX - 95} y1={CY} x2={CX - 35} y2={CY} />
    <line x1={CX - 35} y1={CY} x2={CX - 18} y2={CY + 14} />
    <line x1={CX + 95} y1={CY} x2={CX + 35} y2={CY} />
    <line x1={CX + 35} y1={CY} x2={CX + 18} y2={CY + 14} />
    <circle cx={CX} cy={CY} r="3" />
  </g>

  <!-- Bank scale (rotates with the world) + fixed pointer -->
  <g transform="rotate({-roll} {CX} {CY})">
    {#each bankTicks as b}
      <line class="hud-line" x1={b.x1} y1={b.y1} x2={b.x2} y2={b.y2} />
    {/each}
    <!-- Roll pointer (sky pointer) — rides with the horizon, points UP at the fixed index -->
    <polygon class="hud-fill bright" points="{CX},{CY - BANK_R + 2} {CX - 9},{CY - BANK_R + 18} {CX + 9},{CY - BANK_R + 18}" />
  </g>
  <!-- Fixed roll index (zero-bank reference) — points DOWN at the sky pointer, so the two converge
       when wings are level and read as distinct marks (not two identical pointers). -->
  <polygon class="hud-fill" points="{CX},{CY - BANK_R} {CX - 8},{CY - BANK_R - 16} {CX + 8},{CY - BANK_R - 16}" />

  <!-- Heading tape (top) -->
  <g>
    <line class="hud-line dim" x1={CX - HDG_HALF * PXHDG} y1="64" x2={CX + HDG_HALF * PXHDG} y2="64" />
    {#each headingTicks as t}
      <line class="hud-line" x1={t.x} y1="64" x2={t.x} y2={t.major ? 50 : 57} />
      {#if t.label}<text class="hud-text small" x={t.x} y="42" text-anchor="middle">{t.label}</text>{/if}
    {/each}
    <!-- centre index + readout -->
    <polygon class="hud-fill bright" points="{CX},70 {CX - 8},58 {CX + 8},58" />
    <rect class="hud-box" x={CX - 34} y="74" width="68" height="26" />
    <text class="hud-text" x={CX} y="93" text-anchor="middle">{Math.round(((heading % 360) + 360) % 360)}</text>
  </g>

  <!-- Speed (left) -->
  <g>
    <text class="hud-text small dim" x={CX - 430} y={CY - 34} text-anchor="middle">SPD {speedUnit}</text>
    <rect class="hud-box" x={CX - 490} y={CY - 18} width="120" height="36" />
    <text class="hud-text big" x={CX - 430} y={CY + 10} text-anchor="middle">{fmt(speed)}</text>
  </g>

  <!-- Altitude (right) -->
  <g>
    <text class="hud-text small dim" x={CX + 430} y={CY - 34} text-anchor="middle">ALT {altitudeUnit}</text>
    <rect class="hud-box" x={CX + 370} y={CY - 18} width="120" height="36" />
    <text class="hud-text big" x={CX + 430} y={CY + 10} text-anchor="middle">{fmt(altitude)}</text>
  </g>
</svg>

<style>
  .fpv-hud {
    position: absolute;
    pointer-events: none;
    z-index: 20;
    /* projected-glow look + a dark shadow so lines/readouts stay legible against a bright sky */
    filter: drop-shadow(0 0 2px rgba(60, 255, 140, 0.55))
            drop-shadow(0 1px 1.5px rgba(0, 0, 0, 0.9));
  }
  /* Conformal AHI fills the viewport (full vertical FOV ↔ full height). */
  .fpv-ahi {
    inset: 0;
    width: 100%;
    height: 100%;
  }
  /* The conformal AHI uses much finer lines/digits than the instrument cluster (≈1/3 weight):
     it spans the full viewport, so full-weight strokes would dominate the view. */
  .fpv-ahi .hud-line { stroke-width: 0.7; }
  .fpv-ahi .hud-line.horizon { stroke-width: 1; }
  /* More specific than `.hud-text.small` so it actually overrides the shared size/weight. */
  .fpv-ahi .hud-text.small { font-size: 9px; font-weight: 300; }

  /* Instrument cluster: centred, never larger than half the viewport. */
  .fpv-instruments {
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: 100%;
    height: 100%;
    max-width: 50%;
    max-height: 50%;
  }

  /* HUD green */
  .hud-line { stroke: #3cff8e; stroke-width: 2; fill: none; }
  .hud-line.horizon { stroke-width: 3; }
  .hud-line.dim { stroke: rgba(60, 255, 140, 0.4); }
  .hud-line.dive line, .hud-line .dashed { stroke-dasharray: 8 6; }
  .hud-bore line, .hud-bore circle { stroke: #eaffe9; stroke-width: 3; fill: none; }
  .hud-fill { fill: #3cff8e; stroke: none; }
  .hud-fill.bright { fill: #eaffe9; }
  .hud-box { fill: rgba(8, 24, 14, 0.35); stroke: #3cff8e; stroke-width: 2; }

  .hud-text {
    fill: #3cff8e;
    font-family: 'Consolas', 'JetBrains Mono', monospace;
    font-size: 22px;
    font-weight: 700;
  }
  .hud-text.small { font-size: 18px; font-weight: 600; }
  .hud-text.big { font-size: 26px; }
  .hud-text.dim { fill: rgba(60, 255, 140, 0.6); }
</style>
