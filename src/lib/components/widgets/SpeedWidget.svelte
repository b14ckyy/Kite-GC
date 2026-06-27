<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- Speed widget — ground/airspeed readout, flanked by a throttle bar (left) and a derived
     acceleration bar (right). -->
<script lang="ts">
  import type { TelemetryData } from "$lib/stores/telemetry";
  import type { InterfaceSettings } from "$lib/stores/settings";
  import { convertSpeed } from "$lib/utils/units";
  import { t } from 'svelte-i18n';

  let {
    telem,
    size = 9,
    interfaceSettings = { speedUnit: 'kmh', altitudeUnit: 'm', distanceUnit: 'metric', verticalSpeedUnit: 'ms', temperatureUnit: 'c' },
  }: {
    telem: TelemetryData;
    size?: number;
    interfaceSettings?: InterfaceSettings;
  } = $props();

  // Airspeed is the primary readout when available (it's what matters for flight); ground speed then
  // drops to the secondary line. Without airspeed, ground speed stays primary.
  let hasAir = $derived(telem.lastUpdate && telem.airspeed > 0);
  let gsConv = $derived(convertSpeed(telem.groundSpeed, interfaceSettings.speedUnit));
  let asConv = $derived(convertSpeed(telem.airspeed, interfaceSettings.speedUnit));

  let label = $derived(hasAir ? $t('widgetLabels.aspd') : $t('widgetLabels.spd'));
  let primaryConv = $derived(hasAir ? asConv : gsConv);
  let speed = $derived(telem.lastUpdate ? primaryConv.value.toFixed(0) : '—');
  let secondary = $derived(
    hasAir
      ? $t('widgetLabels.gs', { values: { value: `${gsConv.value.toFixed(0)} ${gsConv.unit}` } })
      : null
  );

  // Throttle bar fill, 0–100% (FC output via MSP2_INAV_MISC2 / VFR_HUD).
  let thrPct = $derived(Math.max(0, Math.min(100, telem.lastUpdate ? telem.throttle : 0)));

  // ── Derived longitudinal acceleration (m/s²) ──────────────────────────────
  // No FC field for it — we estimate it from the speed we display (airspeed when present, else ground
  // speed). The raw speed is quantized (it only changes by ~1 km/h every few frames), so an
  // instantaneous derivative spikes on the one frame the value steps and reads ~0 on every frame in
  // between — the bar flicks up then drops to zero. Instead we fit a least-squares line to the speed
  // samples in a sliding time window (ACC_WINDOW_MS) and take its slope: the estimate stays steady as
  // long as the speed is trending across the window (it doesn't collapse to 0 between quantization
  // steps) and only fades to 0 once the speed is genuinely flat over the whole window. Robust to slow /
  // irregular update rates. A small deadband reads sub-threshold drift as "level". Scale ±4 m/s².
  // `samples` is a plain (non-reactive) var; the $effect only writes `accel` ($state) — never reads the
  // state it writes (that read+write-same-state pattern freezes the main thread — see other widgets).
  const ACC_SCALE = 4;          // m/s² at full deflection (small range — long-range craft change gently)
  const ACC_WINDOW_MS = 1500;   // sliding regression window — wider = steadier/laggier, narrower = livelier
  const ACC_DEADBAND = 0.05;    // m/s² — suppress sub-threshold drift around zero
  let accel = $state(0);
  let samples: { t: number; v: number }[] = [];

  /** Least-squares slope (m/s²) of the windowed (t, v) samples; 0 with <2 samples. */
  function windowSlope(s: { t: number; v: number }[]): number {
    const n = s.length;
    if (n < 2) return 0;
    const t0 = s[0].t;
    let sx = 0, sy = 0, sxx = 0, sxy = 0;
    for (const p of s) {
      const x = (p.t - t0) / 1000; // seconds
      sx += x; sy += p.v; sxx += x * x; sxy += x * p.v;
    }
    const denom = n * sxx - sx * sx;
    if (Math.abs(denom) < 1e-9) return 0;
    return (n * sxy - sx * sy) / denom;
  }

  $effect(() => {
    const now = telem.lastUpdate;
    const v = telem.airspeed > 0 ? telem.airspeed : telem.groundSpeed; // raw m/s (unit-independent)
    if (!now) { samples = []; accel = 0; return; }
    const last = samples[samples.length - 1];
    if (last && now - last.t > 5000) samples = []; // stale gap (reconnect/pause) → restart the window
    if (!last || now > last.t) samples.push({ t: now, v });
    const cutoff = now - ACC_WINDOW_MS;
    while (samples.length > 2 && samples[0].t < cutoff) samples.shift();
    const a = windowSlope(samples);
    accel = Math.abs(a) < ACC_DEADBAND ? 0 : a;
  });

  let accFrac = $derived(Math.max(-1, Math.min(1, accel / ACC_SCALE))); // −1…+1 of the bar
</script>

<div class="widget-card" style="--ws: {size}px">
  <!-- Throttle (left): fills bottom→top 0–100%. -->
  <div class="vbar">
    <span class="vbar-label">{$t('widgetLabels.thr')}</span>
    <div class="vbar-track">
      <div class="vbar-fill thr" style="height: {thrPct}%"></div>
      <!-- 25 / 50 / 75 % reference ticks (above the fill so they stay visible) -->
      <div class="thr-tick" style="bottom: 25%"></div>
      <div class="thr-tick" style="bottom: 50%"></div>
      <div class="thr-tick" style="bottom: 75%"></div>
    </div>
  </div>

  <div class="center">
    <span class="w-label">{label}</span>
    <span class="w-value">{speed}</span>
    <span class="w-unit">{primaryConv.unit}</span>
    {#if secondary}
      <span class="w-secondary">{secondary}</span>
    {/if}
  </div>

  <!-- Acceleration (right): bipolar from the centre — up = speeding up, down = slowing down. -->
  <div class="vbar">
    <span class="vbar-label">{$t('widgetLabels.acc')}</span>
    <div class="vbar-track acc-track">
      <div
        class="vbar-fill acc {accFrac >= 0 ? 'pos' : 'neg'}"
        style={accFrac >= 0
          ? `bottom: 50%; height: ${accFrac * 50}%`
          : `top: 50%; height: ${-accFrac * 50}%`}
      ></div>
    </div>
  </div>
</div>

<style>
  .widget-card {
    width: var(--ws);
    height: var(--ws);
    display: flex;
    flex-direction: row;
    align-items: stretch;
    justify-content: space-between;
    background: rgba(30, 30, 30, 0.75);
    backdrop-filter: blur(10px);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: calc(var(--ws) * 0.08);
    gap: calc(var(--ws) * 0.015);
    box-sizing: border-box;
    padding: calc(var(--ws) * 0.05) calc(var(--ws) * 0.035) calc(var(--ws) * 0.05);
    overflow: hidden;
  }

  .center {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: flex-start;
    gap: calc(var(--ws) * 0.02);
    white-space: nowrap;
  }

  .w-label {
    font-size: calc(var(--ws) * 0.13);
    font-weight: 600;
    color: #37a8db;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .w-value {
    font-size: calc(var(--ws) * 0.3);
    font-weight: 700;
    color: #e0e0e0;
    font-variant-numeric: tabular-nums;
    line-height: 1.1;
    margin-top: calc(var(--ws) * 0.02);
  }
  .w-unit {
    font-size: calc(var(--ws) * 0.11);
    color: #888;
  }
  .w-secondary {
    font-size: calc(var(--ws) * 0.11);
    color: #aaa;
  }

  /* ── Side bars (throttle / acceleration) ── */
  .vbar {
    flex: 0 0 auto;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: calc(var(--ws) * 0.02);
  }
  .vbar-label {
    font-size: calc(var(--ws) * 0.08);
    font-weight: 600;
    color: #888;
    letter-spacing: 0.02em;
  }
  .vbar-track {
    position: relative;
    flex: 1 1 auto;
    width: calc(var(--ws) * 0.05);
    background: rgba(255, 255, 255, 0.08);
    border-radius: calc(var(--ws) * 0.03);
  }
  .vbar-fill {
    position: absolute;
    left: 0;
    right: 0;
    border-radius: calc(var(--ws) * 0.025);
  }
  .vbar-fill.thr {
    bottom: 0;
    background: #37a8db;
    transition: height 0.2s ease;
  }
  /* 25/50/75 % reference ticks on the throttle bar — thicker and overhanging both sides for clarity. */
  .thr-tick {
    position: absolute;
    left: calc(var(--ws) * -0.018);
    right: calc(var(--ws) * -0.018);
    height: calc(var(--ws) * 0.014);
    transform: translateY(50%); /* centre the line on the % mark */
    background: rgba(255, 255, 255, 0.6);
    border-radius: 1px;
  }
  /* Centre baseline for the bipolar acceleration bar. */
  .acc-track::before {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    top: 50%;
    height: 1px;
    background: rgba(255, 255, 255, 0.25);
  }
  .vbar-fill.acc {
    transition: height 0.15s ease, bottom 0.15s ease, top 0.15s ease;
  }
  .vbar-fill.acc.pos { background: #59aa29; } /* accelerating */
  .vbar-fill.acc.neg { background: #f5a623; } /* decelerating */
</style>
