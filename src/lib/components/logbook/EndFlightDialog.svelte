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
    totalDistM: number | null;
    batteryUsedMah: number | null;
    locationName?: string | null;
  }
  export interface EndFlightResult { batterySerial: string; notes: string; linkMission: boolean; discard?: boolean; }
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
  import { get } from 'svelte/store';
  import type { InterfaceSettings } from '$lib/stores/settings';
  import { settings } from '$lib/stores/settings';
  import { convertAltitude, convertSpeed, convertDistance, formatConverted } from '$lib/utils/units';
  import { formatDurationSec, batteryDbList } from '$lib/stores/flightlog';
  import { normalizeSerial, normalizeSerialInput, serialTokens, normalizeSerialList } from '$lib/stores/batteryManager';
  import type { BatteryPack } from '$lib/stores/flightlogTypes';

  let { interfaceSettings }: { interfaceSettings: InterfaceSettings } = $props();

  let open = $state(false);
  let stats = $state<EndFlightStats | null>(null);
  let recorded = $state(false);
  let missionConfirm = $state(false);
  let serial = $state('');
  let notes = $state('');
  let linkMission = $state(false);
  let confirmingDiscard = $state(false);
  let confirmingArchived = $state(false);
  let resolver: ((v: EndFlightResult | null) => void) | null = null;

  // Battery serial combobox: filter text + a dropdown of existing packs. The typed value is what's
  // saved; if it matches no pack the caller offers to create one (see +page onRecordingEnded).
  let batteries = $state<BatteryPack[]>([]);
  let listOpen = $state(false);

  // `serial` is a comma-separated list (one flight may link several packs). It's kept input-normalized
  // (upper alnum + the separators) live. The picker operates on the *active* (last) token being typed;
  // the tokens before it are already committed.
  const isArchived = (b: BatteryPack) => b.status === 'retired' || b.status === 'damaged';
  const activeToken = $derived(normalizeSerial(serial.split(',').pop() ?? '')); // segment being typed
  const committedTokens = $derived(serialTokens(serial.split(',').slice(0, -1).join(','))); // already added
  const selectedTokens = $derived(serialTokens(serial)); // all tokens (for archived check)
  const filteredBatteries = $derived.by(() => {
    const q = activeToken.toLowerCase();
    // Hide retired/damaged packs and ones already added to this flight. (You can still link an archived
    // pack by typing its exact serial, with a confirmation, but it isn't offered as a normal choice.)
    const list = batteries.filter((b) => !isArchived(b) && !committedTokens.includes(normalizeSerial(b.serial)));
    const matches = q
      ? list.filter((b) => b.serial.toLowerCase().includes(q) || (b.label ?? '').toLowerCase().includes(q))
      : list;
    return matches.slice(0, 8);
  });
  const serialIsNew = $derived(
    activeToken.length > 0 && !batteries.some((b) => normalizeSerial(b.serial) === activeToken),
  );
  // A retired/damaged pack among the selected tokens — link is allowed but must be confirmed
  // (could be an accidental serial reuse).
  const archivedMatch = $derived(
    selectedTokens.length
      ? (batteries.find((b) => selectedTokens.includes(normalizeSerial(b.serial)) && isArchived(b)) ?? null)
      : null,
  );

  // Pick a pack from the dropdown → complete the active token and leave a trailing ", " so the next
  // pack can be typed/picked straight away.
  function pickSerial(pickedSerial: string) {
    const prior = serialTokens(serial.split(',').slice(0, -1).join(','));
    prior.push(normalizeSerial(pickedSerial));
    serial = prior.join(', ') + ', ';
    listOpen = true;
  }
  // Re-arm the confirmation whenever the serial changes.
  $effect(() => {
    void serial;
    confirmingArchived = false;
  });

  function batteryMeta(b: BatteryPack): string {
    return [b.label, b.capacity_mah ? `${b.capacity_mah} mAh` : null, b.cell_count ? `${b.cell_count}S` : null]
      .filter(Boolean)
      .join(' · ');
  }

  async function loadBatteries() {
    try {
      batteries = await batteryDbList(get(settings).flightLogDbPath);
    } catch {
      batteries = [];
    }
  }

  export function show(opts: EndFlightOptions): Promise<EndFlightResult | null> {
    stats = opts.stats;
    recorded = opts.recorded;
    missionConfirm = opts.missionConfirm ?? false;
    serial = '';
    notes = '';
    linkMission = false; // unverified mission → opt-in
    confirmingDiscard = false;
    confirmingArchived = false;
    listOpen = false;
    batteries = [];
    if (opts.recorded) void loadBatteries();
    open = true;
    return new Promise((resolve) => { resolver = resolve; });
  }

  /** Force-close without a result (e.g. on re-arm — the flight is already saved). */
  export function close() { settle(null); }

  function settle(v: EndFlightResult | null) {
    if (!open) return;
    open = false;
    confirmingDiscard = false;
    confirmingArchived = false;
    if (resolver) { resolver(v); resolver = null; }
  }
  function save() {
    // Linking to a retired/damaged pack needs an explicit confirm (possible accidental serial reuse).
    if (archivedMatch && !confirmingArchived) { confirmingArchived = true; return; }
    settle({ batterySerial: normalizeSerialList(serial), notes: notes.trim(), linkMission: missionConfirm && linkMission });
  }
  function confirmDiscard() { settle({ batterySerial: '', notes: '', linkMission: false, discard: true }); }
  // Modal: deliberately no backdrop-click / Escape dismissal — closing is an explicit Save or
  // Discard (a stray click next to the popup must not lose the just-recorded flight).

  let ui = $derived(interfaceSettings);
  function fmtAlt(m: number | null | undefined): string { return m == null ? '—' : formatConverted(convertAltitude(m, ui.altitudeUnit), 1); }
  function fmtSpeed(m: number | null | undefined): string { return m == null ? '—' : formatConverted(convertSpeed(m, ui.speedUnit), 1); }
  function fmtDist(m: number | null | undefined): string { return m == null ? '—' : formatConverted(convertDistance(m, ui.distanceUnit), m < 1000 ? 0 : 1); }
</script>

{#if open && stats}
  <!-- Modal: the backdrop intentionally has no click/Escape handler so it can't be dismissed by
       a stray click — the flight is only kept (Save) or dropped (Discard) by an explicit button. -->
  <div class="dialog-backdrop">
    <div class="dialog-box">
      <div class="dialog-title">{$t('endFlight.title')}</div>

      <div class="fc-info-grid">
        <span class="fc-label">{$t('logbook.duration')}</span><span class="fc-value">{formatDurationSec(stats.durationSec)}</span>
        <span class="fc-label">{$t('logbook.totalDistance')}</span><span class="fc-value">{fmtDist(stats.totalDistM)}</span>
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
          <div class="battery-combo">
            <!-- svelte-ignore a11y_autofocus -->
            <input
              class="fld-input"
              type="text"
              placeholder={$t('endFlight.batteryHint')}
              value={serial}
              autofocus
              autocapitalize="characters"
              autocomplete="off"
              spellcheck="false"
              onfocus={() => (listOpen = true)}
              oninput={(e) => { serial = normalizeSerialInput(e.currentTarget.value); listOpen = true; }}
              onblur={() => setTimeout(() => (listOpen = false), 120)}
            />
            {#if listOpen && filteredBatteries.length > 0}
              <ul class="battery-dropdown">
                {#each filteredBatteries as b (b.id)}
                  <li>
                    <button type="button" class="battery-opt" onmousedown={(e) => { e.preventDefault(); pickSerial(b.serial); }}>
                      <span class="bo-serial">{b.serial}</span>
                      {#if batteryMeta(b)}<span class="bo-meta">{batteryMeta(b)}</span>{/if}
                    </button>
                  </li>
                {/each}
              </ul>
            {/if}
          </div>
          {#if serialIsNew}<span class="battery-new-hint">{$t('endFlight.batteryNewHint')}</span>{/if}
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
        {#if confirmingArchived && archivedMatch}
          <div class="ef-discard-warn">
            {$t('endFlight.batteryArchivedWarn', { values: { status: $t(`batteryMgr.statusVal.${archivedMatch.status}`) } })}
          </div>
          <div class="dialog-buttons">
            <button class="dialog-btn" onclick={() => (confirmingArchived = false)}>{$t('endFlight.cancel')}</button>
            <button class="dialog-btn dialog-btn-danger" onclick={save}>{$t('endFlight.batteryArchivedYes')}</button>
          </div>
        {:else if confirmingDiscard}
          <div class="ef-discard-warn">{$t('endFlight.discardConfirm')}</div>
          <div class="dialog-buttons">
            <button class="dialog-btn" onclick={() => (confirmingDiscard = false)}>{$t('endFlight.cancel')}</button>
            <button class="dialog-btn dialog-btn-danger" onclick={confirmDiscard}>{$t('endFlight.discardConfirmYes')}</button>
          </div>
        {:else}
          <div class="dialog-buttons">
            <button class="dialog-btn dialog-btn-danger" onclick={() => (confirmingDiscard = true)}>{$t('endFlight.discard')}</button>
            <span class="ef-spacer"></span>
            <button class="dialog-btn dialog-btn-primary" onclick={save}>{$t('endFlight.save')}</button>
          </div>
        {/if}
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

  /* Battery serial combobox: filter input + dropdown of existing packs. */
  .battery-combo { position: relative; }
  .battery-dropdown {
    position: absolute; top: 100%; left: 0; right: 0; z-index: 10;
    margin: 2px 0 0; padding: 4px; list-style: none;
    max-height: 180px; overflow-y: auto;
    background: #262626; border: 1px solid #444; border-radius: 4px;
    box-shadow: 0 6px 18px rgba(0, 0, 0, 0.5);
  }
  .battery-opt {
    display: flex; align-items: baseline; gap: 8px; width: 100%;
    padding: 5px 8px; border: none; border-radius: 3px; background: transparent;
    color: #e0e0e0; font-size: 12px; text-align: left; cursor: pointer;
  }
  .battery-opt:hover { background: rgba(55, 168, 219, 0.18); }
  .bo-serial { font-weight: 600; }
  .bo-meta { color: #949494; font-size: 11px; }
  .battery-new-hint { display: block; margin-top: 4px; font-size: 11px; color: #37a8db; }

  .ef-check { display: flex; align-items: center; gap: 8px; font-size: 12px; color: #e0e0e0; margin-bottom: 12px; cursor: pointer; }
  .ef-check input { accent-color: #37a8db; }

  .ef-discard-warn { font-size: 12px; color: #f0b0b0; background: rgba(212, 0, 0, 0.12); border: 1px solid rgba(212, 0, 0, 0.4); border-radius: 4px; padding: 8px 10px; margin: 10px 0 12px; }

  .dialog-buttons { display: flex; justify-content: flex-end; align-items: center; gap: 8px; margin-top: 4px; }
  .ef-spacer { flex: 1 1 auto; }
  .dialog-btn { padding: 6px 14px; font-size: 12px; font-weight: 600; border-radius: 4px; border: 1px solid #555; background: #434343; color: #e0e0e0; cursor: pointer; transition: background 0.15s; }
  .dialog-btn:hover { background: #505050; }
  .dialog-btn-primary { background: #1a6b94; border-color: #2590c8; color: #fff; }
  .dialog-btn-primary:hover { background: #237fae; }
  .dialog-btn-danger { background: #5a1414; border-color: #d40000; color: #f3c5c5; }
  .dialog-btn-danger:hover { background: #7a1a1a; }
</style>
