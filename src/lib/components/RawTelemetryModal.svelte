<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- Raw telemetry popup (Dev-Docs active/WIDGET_OVERHAUL.md §6) — the replacement for the old Raw
     Telemetry widget. Opened from the toolbar button next to Relay while connected; lists EVERY value
     the telemetry pipeline holds, grouped, with its name and the store's canonical RAW unit (no
     user-unit conversion — that is what the widgets are for). Bitfields as decimal + hex. Purely
     informational: no actions. Live-reactive on `telem`. -->
<script lang="ts">
  import { t } from 'svelte-i18n';
  import type { TelemetryData } from '$lib/stores/telemetry';

  let { telem, onclose }: { telem: TelemetryData; onclose: () => void } = $props();

  type Row = { key: string; value: string; unit: string };
  type Group = { key: string; rows: Row[] };

  const NONE = '—';
  const num = (v: number | null | undefined, digits = 2): string =>
    v == null || Number.isNaN(v) ? NONE : v.toFixed(digits);
  const int = (v: number | null | undefined): string => (v == null ? NONE : String(Math.trunc(v)));
  const bits = (v: number): string => `${v} (0x${(v >>> 0).toString(16).toUpperCase()})`;
  const bool = (v: boolean): string => (v ? '1' : '0');

  const groups = $derived.by((): Group[] => {
    const g: Group[] = [
      { key: 'gps', rows: [
        { key: 'lat', value: num(telem.lat, 7), unit: '°' },
        { key: 'lon', value: num(telem.lon, 7), unit: '°' },
        { key: 'altMsl', value: num(telem.altMsl), unit: 'm' },
        { key: 'groundSpeed', value: num(telem.groundSpeed), unit: 'm/s' },
        { key: 'course', value: num(telem.course, 1), unit: '°' },
        { key: 'numSat', value: int(telem.numSat), unit: '' },
        { key: 'fixType', value: int(telem.fixType), unit: '' },
        { key: 'gpsHdop', value: num(telem.gpsHdop), unit: '' },
      ] },
      { key: 'attitude', rows: [
        { key: 'roll', value: num(telem.roll, 1), unit: '°' },
        { key: 'pitch', value: num(telem.pitch, 1), unit: '°' },
        { key: 'yaw', value: num(telem.yaw, 1), unit: '°' },
      ] },
      { key: 'altitude', rows: [
        { key: 'altitude', value: num(telem.altitude), unit: 'm' },
        { key: 'vario', value: num(telem.vario), unit: 'm/s' },
        { key: 'airspeed', value: num(telem.airspeed), unit: 'm/s' },
      ] },
      { key: 'wind', rows: [
        { key: 'windDirFrom', value: num(telem.windDirFrom, 1), unit: '°' },
        { key: 'windSpeedMs', value: num(telem.windSpeedMs), unit: 'm/s' },
      ] },
      { key: 'battery', rows: [
        { key: 'voltage', value: num(telem.voltage), unit: 'V' },
        { key: 'current', value: num(telem.current), unit: 'A' },
        { key: 'power', value: num(telem.power), unit: 'W' },
        { key: 'mAhDrawn', value: int(telem.mAhDrawn), unit: 'mAh' },
        { key: 'batteryPercentage', value: int(telem.batteryPercentage), unit: '%' },
        { key: 'cellCount', value: int(telem.cellCount), unit: '' },
        { key: 'throttle', value: int(telem.throttle), unit: '%' },
        { key: 'rssi', value: int(telem.rssi), unit: '/1023' },
      ] },
    ];
    for (const b of telem.batteries) {
      g.push({ key: `battery-${b.id}`, rows: [
        { key: 'id', value: int(b.id), unit: '' },
        { key: 'voltage', value: num(b.voltage), unit: 'V' },
        { key: 'current', value: num(b.current), unit: 'A' },
        { key: 'mAhDrawn', value: int(b.mahDrawn), unit: 'mAh' },
        { key: 'batteryPercentage', value: int(b.percentage), unit: '%' },
        { key: 'cellCount', value: int(b.cellCount), unit: '' },
        { key: 'temperature', value: b.temperature == null ? NONE : num(b.temperature, 1), unit: '°C' },
      ] });
    }
    g.push(
      { key: 'link', rows: [
        { key: 'rssiPercent', value: telem.link.rssiPercent == null ? NONE : num(telem.link.rssiPercent, 0), unit: '%' },
        { key: 'rssiDbm', value: int(telem.link.rssiDbm), unit: 'dBm' },
        { key: 'lq', value: int(telem.link.lq), unit: '%' },
        { key: 'snrDb', value: int(telem.link.snrDb), unit: 'dB' },
      ] },
      { key: 'status', rows: [
        { key: 'armingFlags', value: bits(telem.armingFlags), unit: '' },
        { key: 'flightModeFlags', value: bits(telem.flightModeFlags), unit: '' },
        { key: 'sensorStatus', value: bits(telem.sensorStatus), unit: '' },
        { key: 'statusSeen', value: bool(telem.statusSeen), unit: '' },
        { key: 'mspRcOverride', value: bool(telem.mspRcOverride), unit: '' },
        { key: 'prearmHealthy', value: int(telem.prearmHealthy), unit: '' },
        { key: 'cpuLoad', value: int(telem.cpuLoad), unit: '%' },
        { key: 'navState', value: int(telem.navState), unit: '' },
        { key: 'activeWpNumber', value: int(telem.activeWpNumber), unit: '' },
      ] },
      { key: 'sensors', rows: [
        { key: 'sensorGyro', value: int(telem.sensorGyro), unit: '' },
        { key: 'sensorAcc', value: int(telem.sensorAcc), unit: '' },
        { key: 'sensorMag', value: int(telem.sensorMag), unit: '' },
        { key: 'sensorBaro', value: int(telem.sensorBaro), unit: '' },
        { key: 'sensorGps', value: int(telem.sensorGps), unit: '' },
        { key: 'sensorRangefinder', value: int(telem.sensorRangefinder), unit: '' },
        { key: 'sensorPitot', value: int(telem.sensorPitot), unit: '' },
        { key: 'sensorOpflow', value: int(telem.sensorOpflow), unit: '' },
        { key: 'sensorRcReceiver', value: int(telem.sensorRcReceiver), unit: '' },
      ] },
      { key: 'mode', rows: [
        { key: 'flightMode', value: telem.flightMode.primary || NONE, unit: '' },
        { key: 'modifiers', value: telem.flightMode.modifiers.join(', ') || NONE, unit: '' },
        { key: 'ekfStatus', value: int(telem.ekfStatus), unit: '' },
        { key: 'ekfType', value: int(telem.ekfType), unit: '' },
        { key: 'fcVariant', value: telem.fcVariant || NONE, unit: '' },
        { key: 'lastUpdate', value: int(telem.lastUpdate), unit: 'ms' },
      ] },
    );
    return g;
  });

  /** Group title: the per-instance battery groups carry their id, everything else a plain key. */
  function groupTitle(key: string): string {
    if (key.startsWith('battery-')) {
      return $t('rawTelemetry.groups.batteryInstance', { values: { id: key.slice('battery-'.length) } });
    }
    return $t(`rawTelemetry.groups.${key}`);
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') onclose();
  }
</script>

<svelte:window onkeydown={onKeydown} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="dialog-backdrop" onclick={onclose}>
  <div class="dialog-box" role="dialog" aria-modal="true" tabindex="-1" onclick={(e) => e.stopPropagation()}>
    <div class="head">
      <div>
        <div class="dialog-title">{$t('rawTelemetry.title')}</div>
        <div class="hint">{$t('rawTelemetry.hint')}</div>
      </div>
      <button class="close" type="button" title={$t('rawTelemetry.close')} aria-label={$t('rawTelemetry.close')} onclick={onclose}>✕</button>
    </div>
    <div class="body">
      {#each groups as group (group.key)}
        <section class="group">
          <h4>{groupTitle(group.key)}</h4>
          {#each group.rows as row (row.key)}
            <div class="row">
              <span class="name">{$t(`rawTelemetry.fields.${row.key}`)}</span>
              <span class="value">{row.value}</span>
              <span class="unit">{row.unit}</span>
            </div>
          {/each}
        </section>
      {/each}
    </div>
  </div>
</div>

<style>
  .dialog-backdrop {
    position: fixed;
    inset: 0;
    z-index: 9999;
    background: rgba(0, 0, 0, 0.55);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .dialog-box {
    display: flex;
    flex-direction: column;
    width: min(1100px, 95vw);
    max-height: 88vh;
    background: #2e2e2e;
    border: 1px solid rgba(55, 168, 219, 0.45);
    border-radius: 8px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
    color: #e0e0e0;
    overflow: hidden;
  }

  .head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
    padding: 12px 14px 8px;
    border-bottom: 1px solid #272727;
  }

  .dialog-title {
    font-size: 15px;
    font-weight: 600;
    color: #37a8db;
  }

  .hint {
    margin-top: 2px;
    font-size: 11px;
    color: #949494;
  }

  .close {
    flex-shrink: 0;
    width: 26px;
    height: 26px;
    border: 1px solid #555;
    border-radius: 4px;
    background: #434343;
    color: #cfcfcf;
    cursor: pointer;
    font-size: 12px;
  }
  .close:hover {
    background: rgba(212, 0, 0, 0.35);
    color: #fff;
  }

  .body {
    overflow-y: auto;
    padding: 10px 14px 14px;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(250px, 1fr));
    gap: 10px;
  }

  .group {
    background: rgba(0, 0, 0, 0.25);
    border: 1px solid #272727;
    border-radius: 6px;
    padding: 6px 10px 8px;
  }

  .group h4 {
    margin: 0 0 4px;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: #949494;
  }

  .row {
    display: grid;
    grid-template-columns: 1fr auto minmax(2.6em, auto);
    gap: 8px;
    align-items: baseline;
    font-size: 12px;
    line-height: 1.6;
  }

  .name {
    color: #c0c0c0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .value {
    font-family: 'JetBrains Mono', 'Fira Code', Consolas, monospace;
    font-variant-numeric: tabular-nums;
    text-align: right;
    color: #f0f0f0;
  }

  .unit {
    font-size: 11px;
    color: #949494;
  }
</style>
