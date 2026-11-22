<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // The mini view's content, shared by the desktop mini window and Android PiP. Fills the viewport in any
  // aspect; renders a snapshot only, so it must not import stores. Cell text sizes from the cell (cq units).
  import { t } from 'svelte-i18n';
  import { modeLabel, modeColor } from '$lib/helpers/flightModeRegistry';
  import { batteryColor } from '$lib/helpers/battery';
  import { convertAltitude, convertDistance, convertSpeed, convertVerticalSpeed, speedDigits } from '$lib/utils/units';
  import type { MiniLink, MiniSnapshot } from '$lib/helpers/miniSnapshot';

  let { snap, stale = false }: { snap: MiniSnapshot | null; stale?: boolean } = $props();

  interface Readout {
    value: string;
    unit: string;
  }
  const NONE: Readout = { value: '—', unit: '' };

  const link = $derived<MiniLink>(!snap ? 'waiting' : stale && snap.link === 'live' ? 'stale' : snap.link);
  const live = $derived(link === 'live' || link === 'stale' ? snap : null);

  const modeText = $derived(live?.mode ? modeLabel(live.mode) : '—');
  const modeBg = $derived(live?.mode ? modeColor(live.mode) : null);

  const battery = $derived.by<Readout>(() => {
    if (live?.batteryPct != null) return { value: String(live.batteryPct), unit: '%' };
    if (live?.voltage != null) return { value: live.voltage.toFixed(1), unit: 'V' };
    return NONE;
  });
  const batterySub = $derived(live?.batteryPct != null && live.voltage != null ? `${live.voltage.toFixed(1)} V` : '');
  const batteryTint = $derived(live?.batteryPct != null ? batteryColor(live.batteryPct) : undefined);

  // Home widget's rule: about three significant digits, so the width stays steady.
  const home = $derived.by<Readout>(() => {
    if (live?.homeDistM == null) return NONE;
    const c = convertDistance(live.homeDistM, live.units.distanceUnit);
    const digits = c.unit === 'm' || c.unit === 'ft' ? 0 : c.value < 10 ? 2 : c.value < 100 ? 1 : 0;
    return { value: c.value.toFixed(digits), unit: c.unit };
  });

  // ALT widget's rule: one decimal, metric above 999 m as km.
  const alt = $derived.by<Readout>(() => {
    if (live?.altM == null) return NONE;
    const c = convertAltitude(live.altM, live.units.altitudeUnit);
    if (c.unit === 'm' && Math.abs(c.value) > 999) return { value: (c.value / 1000).toFixed(2), unit: 'km' };
    return { value: c.value.toFixed(1), unit: c.unit };
  });
  const vario = $derived.by(() => {
    if (live?.varioMs == null) return null;
    const c = convertVerticalSpeed(live.varioMs, live.units.verticalSpeedUnit);
    const arrow = live.varioMs > 0.1 ? '▲' : live.varioMs < -0.1 ? '▼' : '•';
    const tint = live.varioMs > 0.1 ? '#27ae60' : live.varioMs < -0.1 ? '#e74c3c' : undefined;
    return { text: `${arrow} ${c.value >= 0 ? '+' : ''}${c.value.toFixed(1)} ${c.unit}`, tint };
  });

  const speed = $derived.by<Readout>(() => {
    if (live?.speedMs == null) return NONE;
    const c = convertSpeed(live.speedMs, live.units.speedUnit);
    return { value: c.value.toFixed(speedDigits(live.units.speedUnit)), unit: c.unit };
  });

  // GPS widget's fix label and colour.
  const fixLabel = $derived.by(() => {
    if (!live) return '';
    if (live.fixRaw === 0) return $t('gps.noGps');
    if (live.fixRaw === 5) return $t('gps.rtkFloat');
    if (live.fixRaw === 6) return $t('gps.rtkFixed');
    if (live.fixType === 0) return $t('gps.noFix');
    const types: Record<number, string> = { 1: $t('gps.fix2d'), 2: $t('gps.fix3d'), 3: $t('gps.fix3dDgps') };
    return types[live.fixType] || `FIX ${live.fixType}`;
  });
  const fixTint = $derived(!live ? undefined : live.fixType < 1 ? '#e74c3c' : live.fixType === 1 ? '#f39c12' : '#27ae60');

  // RC Link widget's thresholds.
  const rcTint = $derived(
    live?.rcPct == null ? undefined : live.rcPct >= 70 ? '#59aa29' : live.rcPct >= 40 ? '#e8c820' : '#d40000',
  );

  const dotTitle = $derived(
    link === 'live' ? $t('statusBox.live')
    : link === 'stale' ? $t('statusBox.noData')
    : link === 'waiting' ? $t('statusBox.waiting')
    : $t('widgetLabels.offline'),
  );
</script>

{#snippet cell(label: string, r: Readout, tint: string | undefined)}
  <span class="lbl">{label}</span>
  <span class="val" style:color={tint}>{r.value}{#if r.unit}<span class="unit">{r.unit}</span>{/if}</span>
{/snippet}

<div class="mv" class:stale={link === 'stale'}>
  <div class="mv-top">
    <span class="mv-side">
      {#if live?.arming}
        <span class="mv-arm {live.arming}">{live.arming === 'armed' ? $t('arming.armed') : $t('arming.disarmed')}</span>
      {/if}
    </span>
    <span
      class="mv-mode"
      class:empty={!modeBg}
      style:background={modeBg}
      style:color={!modeBg ? undefined : modeBg === '#c0c0c0' ? '#1a1a1a' : '#fff'}
    >{modeText}</span>
    <span class="mv-side end"><span class="mv-dot {link}" title={dotTitle}></span></span>
  </div>
  <div class="mv-grid">
    <div class="c">
      {@render cell($t('widgetLabels.bat'), battery, batteryTint)}
      <span class="sub">{batterySub}</span>
    </div>
    <div class="c">
      {@render cell($t('widgetLabels.alt'), alt, undefined)}
      <span class="sub" style:color={vario?.tint}>{vario?.text ?? ''}</span>
    </div>
    <div class="c">
      {@render cell(live?.speedIsAir ? $t('widgetLabels.aspd') : $t('widgetLabels.spd'), speed, undefined)}
      <span class="sub"></span>
    </div>
    <div class="c">
      {@render cell($t('widgetLabels.home'), home, undefined)}
      <span class="sub">
        {#if live?.homeRelDeg != null}
          <svg viewBox="0 0 60 60" class="arrow" aria-hidden="true">
            <g transform="rotate({live.homeRelDeg}, 30, 30)"><polygon points="30,4 14,52 30,42 46,52" /></g>
          </svg>
        {/if}
      </span>
    </div>
    <div class="c">
      {@render cell($t('miniView.sat'), live?.sats != null ? { value: String(live.sats), unit: '' } : NONE, fixTint)}
      <span class="sub" style:color={fixTint}>{fixLabel}</span>
    </div>
    <div class="c">
      {@render cell($t('rcLink.title'), live?.rcPct != null ? { value: String(Math.round(live.rcPct)), unit: '%' } : NONE, rcTint)}
      <span class="sub">{live?.rcPct != null ? (live.rcIsLq ? $t('rcLink.lq') : $t('rcLink.rssi')) : ''}</span>
    </div>
  </div>
</div>

<style>
  .mv {
    position: absolute;
    inset: 0;
    box-sizing: border-box;
    display: grid;
    grid-template-rows: auto minmax(0, 1fr);
    gap: 3vmin;
    padding: 4vmin;
    background: #2e2e2e;
    border: 1px solid #272727;
    color: #e0e0e0;
    font-family: 'Segoe UI', Tahoma, sans-serif;
    overflow: hidden;
    user-select: none;
    -webkit-user-select: none;
  }
  /* Equal side columns keep the mode centred whatever stands left and right of it. */
  .mv-top {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr);
    align-items: center;
    gap: 3vmin;
    min-width: 0;
  }
  .mv-side {
    display: flex;
    min-width: 0;
  }
  .mv-side.end {
    justify-content: flex-end;
  }
  .mv-mode {
    min-width: 0;
    font-size: 9vmin;
    font-weight: 700;
    line-height: 1.15;
    padding: 0.5vmin 3vmin;
    border-radius: 2vmin;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .mv-mode.empty {
    color: #949494;
    border: 1px solid #3d3f3e;
  }
  /* ArmingIndicator's palette. */
  .mv-arm {
    font-size: 6vmin;
    font-weight: 700;
    letter-spacing: 0.04em;
    padding: 0.5vmin 2vmin;
    border-radius: 1.5vmin;
    border: 1px solid transparent;
    white-space: nowrap;
  }
  .mv-arm.armed {
    color: #f0b443;
    background: rgba(232, 163, 23, 0.14);
    border-color: rgba(232, 163, 23, 0.45);
  }
  .mv-arm.ready {
    color: #7ec850;
    background: rgba(89, 170, 41, 0.12);
    border-color: rgba(89, 170, 41, 0.35);
  }
  .mv-arm.notReady {
    color: #ff6b6b;
    background: rgba(212, 0, 0, 0.14);
    border-color: rgba(212, 0, 0, 0.45);
  }
  .mv-dot {
    flex: none;
    width: 5vmin;
    height: 5vmin;
    border-radius: 50%;
    background: #949494;
  }
  .mv-dot.live {
    background: #59aa29;
    box-shadow: 0 0 2vmin rgba(89, 170, 41, 0.7);
  }
  .mv-dot.waiting,
  .mv-dot.stale {
    background: #e8a317;
  }
  .mv-dot.off {
    background: #d40000;
  }
  .mv-grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    grid-template-rows: repeat(2, minmax(0, 1fr));
    gap: 2vmin;
    min-height: 0;
  }
  @media (max-aspect-ratio: 1/1) {
    .mv-grid {
      grid-template-columns: repeat(2, minmax(0, 1fr));
      grid-template-rows: repeat(3, minmax(0, 1fr));
    }
  }
  .c {
    container-type: size;
    display: flex;
    flex-direction: column;
    justify-content: center;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    padding: 0 2.5vmin;
    background: rgba(255, 255, 255, 0.04);
    border-radius: 2vmin;
  }
  .lbl {
    font-size: min(18cqh, 12cqw);
    font-weight: 600;
    color: #37a8db;
    letter-spacing: 0.05em;
    white-space: nowrap;
  }
  .val {
    font-size: min(46cqh, 25cqw);
    font-weight: 700;
    line-height: 1.05;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .unit {
    margin-left: 0.15em;
    font-size: 0.5em;
    font-weight: 600;
    color: #949494;
  }
  .sub {
    line-height: 1.2;
    min-height: 1.2em;
    font-size: min(16cqh, 10cqw);
    font-weight: 600;
    color: #949494;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .arrow {
    height: 1.2em;
    width: 1.2em;
    vertical-align: middle;
    fill: #37a8db;
  }
  .mv.stale .val,
  .mv.stale .sub,
  .mv.stale .mv-mode {
    opacity: 0.45;
  }
</style>
