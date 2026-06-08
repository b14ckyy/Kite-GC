<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts" module>
  export interface EndFlightStats {
    durationSec: number | null;
    maxAltM: number | null;
    maxSpeedMs: number | null;
    maxDistM: number | null;
    batteryUsedMah: number | null;
    locationName?: string | null;
  }
  export interface EndFlightResult { batterySerial: string; notes: string; linkMission: boolean; }
  export interface EndFlightOptions {
    stats: EndFlightStats;
    /** Recorded to the DB → show the editable battery/notes/mission section. */
    recorded: boolean;
    /** The loaded mission is NOT FC-synced → offer to link it (confirmation). FC-synced
     *  missions are linked automatically by the caller and need no prompt. */
    missionConfirm?: boolean;
  }
</script>

<script lang="ts">
  import { t } from 'svelte-i18n';
  import type { InterfaceSettings } from '$lib/stores/settings';
  import { convertAltitude, convertSpeed, convertDistance, formatConverted } from '$lib/utils/units';
  import { formatDurationSec } from '$lib/stores/flightlog';

  let { interfaceSettings }: { interfaceSettings: InterfaceSettings } = $props();

  let open = $state(false);
  let stats = $state<EndFlightStats | null>(null);
  let recorded = $state(false);
  let missionConfirm = $state(false);
  let serial = $state('');
  let notes = $state('');
  let linkMission = $state(false);
  let resolver: ((v: EndFlightResult | null) => void) | null = null;

  export function show(opts: EndFlightOptions): Promise<EndFlightResult | null> {
    stats = opts.stats;
    recorded = opts.recorded;
    missionConfirm = opts.missionConfirm ?? false;
    serial = '';
    notes = '';
    linkMission = false; // unverified mission → opt-in
    open = true;
    return new Promise((resolve) => { resolver = resolve; });
  }

  /** Force-close without a result (e.g. on re-arm). */
  export function close() { settle(null); }

  function settle(v: EndFlightResult | null) {
    if (!open) return;
    open = false;
    if (resolver) { resolver(v); resolver = null; }
  }
  function save() { settle({ batterySerial: serial.trim(), notes: notes.trim(), linkMission: missionConfirm && linkMission }); }
  function handleKeydown(e: KeyboardEvent) { if (e.key === 'Escape') settle(null); }

  let ui = $derived(interfaceSettings);
  function fmtAlt(m: number | null | undefined): string { return m == null ? '—' : formatConverted(convertAltitude(m, ui.altitudeUnit), 1); }
  function fmtSpeed(m: number | null | undefined): string { return m == null ? '—' : formatConverted(convertSpeed(m, ui.speedUnit), 1); }
  function fmtDist(m: number | null | undefined): string { return m == null ? '—' : formatConverted(convertDistance(m, ui.distanceUnit), m < 1000 ? 0 : 1); }
</script>

{#if open && stats}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="dialog-backdrop" onclick={() => settle(null)} onkeydown={handleKeydown}>
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="dialog-box" onclick={(e) => e.stopPropagation()}>
      <div class="dialog-title">{$t('endFlight.title')}</div>

      <div class="fc-info-grid">
        <span class="fc-label">{$t('logbook.duration')}</span><span class="fc-value">{formatDurationSec(stats.durationSec)}</span>
        <span class="fc-label">{$t('logbook.maxAlt')}</span><span class="fc-value">{fmtAlt(stats.maxAltM)}</span>
        <span class="fc-label">{$t('logbook.maxSpeed')}</span><span class="fc-value">{fmtSpeed(stats.maxSpeedMs)}</span>
        <span class="fc-label">{$t('logbook.maxDistance')}</span><span class="fc-value">{fmtDist(stats.maxDistM)}</span>
        <span class="fc-label">{$t('logbook.batteryUsed')}</span><span class="fc-value">{stats.batteryUsedMah ?? '—'} mAh</span>
        {#if stats.locationName}
          <span class="fc-label">{$t('logbook.location')}</span><span class="fc-value">{stats.locationName}</span>
        {/if}
      </div>

      {#if recorded}
        <div class="ef-divider"></div>
        <label class="fld">
          <span class="fld-label">{$t('endFlight.battery')}</span>
          <!-- svelte-ignore a11y_autofocus -->
          <input class="fld-input" type="text" placeholder={$t('endFlight.batteryHint')} bind:value={serial} autofocus />
        </label>
        <label class="fld">
          <span class="fld-label">{$t('logbook.notes')}</span>
          <textarea class="fld-input fld-area" rows="2" bind:value={notes}></textarea>
        </label>
        {#if missionConfirm}
          <label class="ef-check">
            <input type="checkbox" bind:checked={linkMission} />
            <span>{$t('endFlight.linkMission')}</span>
          </label>
        {/if}
        <div class="dialog-buttons">
          <button class="dialog-btn dialog-btn-cancel" onclick={() => settle(null)}>{$t('endFlight.skip')}</button>
          <button class="dialog-btn dialog-btn-primary" onclick={save}>{$t('endFlight.save')}</button>
        </div>
      {:else}
        <div class="ef-note">{$t('endFlight.notRecordedHint')}</div>
        <div class="dialog-buttons">
          <button class="dialog-btn dialog-btn-primary" onclick={() => settle(null)}>{$t('endFlight.close')}</button>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .dialog-backdrop { position: fixed; inset: 0; z-index: 9999; background: rgba(0, 0, 0, 0.55); display: flex; align-items: center; justify-content: center; }
  .dialog-box { background: #2e2e2e; border: 1px solid rgba(55, 168, 219, 0.45); border-radius: 8px; box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5); padding: 20px 24px 16px; min-width: 340px; max-width: 460px; }
  .dialog-title { font-size: 14px; font-weight: 700; color: #e0e0e0; margin-bottom: 14px; }

  .fc-info-grid { display: grid; grid-template-columns: auto minmax(0, 1fr); gap: 6px 14px; font-size: 13px; }
  .fc-label { color: #949494; }
  .fc-value { color: #e0e0e0; font-weight: 600; }

  .ef-divider { height: 1px; background: #3a3a3a; margin: 14px 0 12px; }
  .ef-note { font-size: 12px; color: #949494; margin: 12px 0 4px; }

  .fld { display: block; margin-bottom: 12px; }
  .fld-label { display: block; font-size: 11px; font-weight: 600; color: #949494; text-transform: uppercase; letter-spacing: 0.04em; margin-bottom: 4px; }
  .fld-input { box-sizing: border-box; width: 100%; padding: 6px 8px; font-size: 13px; color: #e0e0e0; background: #1f1f1f; border: 1px solid #444; border-radius: 4px; font-family: 'Segoe UI', Tahoma, sans-serif; }
  .fld-input:focus { outline: none; border-color: #37a8db; }
  .fld-area { resize: vertical; }

  .ef-check { display: flex; align-items: center; gap: 8px; font-size: 12px; color: #e0e0e0; margin-bottom: 12px; cursor: pointer; }
  .ef-check input { accent-color: #37a8db; }

  .dialog-buttons { display: flex; justify-content: flex-end; gap: 8px; margin-top: 4px; }
  .dialog-btn { padding: 6px 14px; font-size: 12px; font-weight: 600; border-radius: 4px; border: 1px solid #555; background: #434343; color: #e0e0e0; cursor: pointer; transition: background 0.15s; }
  .dialog-btn:hover { background: #505050; }
  .dialog-btn-cancel { color: #999; }
  .dialog-btn-primary { background: #1a6b94; border-color: #2590c8; color: #fff; }
  .dialog-btn-primary:hover { background: #237fae; }
</style>
