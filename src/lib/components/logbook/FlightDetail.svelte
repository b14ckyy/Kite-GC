<script lang="ts">
  import { t } from 'svelte-i18n';
  import { convertAltitude, convertDistance, convertSpeed, convertTemperature, formatConverted } from '$lib/utils/units';
  import { formatDurationSec, missionDbForFlight, flightLoggedWpCount, flightLinkMission, flightUnlinkMission, missionDbSave, missionDbGeocode, flightSetBatterySerial, batteryDbFindBySerial } from '$lib/stores/flightlog';
  import type { Flight, LibraryMission, BatteryPack } from '$lib/stores/flightlogTypes';
  import type { InterfaceSettings } from '$lib/stores/settings';
  import { settings } from '$lib/stores/settings';
  import { mission, missionFlags, loadedMissionId, markMissionSynced } from '$lib/stores/mission';
  import { batteryManagerOpen, batteryManagerSelectedId } from '$lib/stores/batteryManager';
  import { requestOpenMissionId } from '$lib/stores/missionManager';
  import { replayWpTotal } from '$lib/stores/navStatus';
  import { buildMissionInput } from '$lib/helpers/missionLibrary';
  import { get } from 'svelte/store';
  import { locale } from 'svelte-i18n';
  import WeatherEditor from './WeatherEditor.svelte';
  import Button from '$lib/components/panel/Button.svelte';

  let {
    flight,
    trackCount,
    minimized = false,
    interfaceSettings,
    notes = $bindable(),
    weatherEditing = $bindable(),
    weatherTempC = $bindable(),
    weatherWindMs = $bindable(),
    weatherWindDir = $bindable(),
    weatherDesc = $bindable(),
    onSaveNotes,
    onSaveWeather,
    onSaveCraftName,
    onSavePlatformType,
    onSavePilot,
    onDeleteFlight,
    onExportTrack,
  }: {
    flight: Flight;
    trackCount: number;
    minimized?: boolean;
    interfaceSettings: InterfaceSettings;
    notes: string;
    weatherEditing: boolean;
    weatherTempC: string;
    weatherWindMs: string;
    weatherWindDir: string;
    weatherDesc: string;
    onSaveNotes: () => void;
    onSaveWeather: () => void;
    onSaveCraftName: (name: string) => void;
    onSavePlatformType: (platformType: number) => void;
    onSavePilot: (pilotName: string, pilotId: string) => void;
    onDeleteFlight: () => void;
    onExportTrack: () => void;
  } = $props();

  let craftNameEditing = $state(false);
  let craftNameDraft = $state('');

  let pilotEditing = $state(false);
  let pilotNameDraft = $state('');
  let pilotIdDraft = $state('');

  // ── Linked mission (Phase 1b) ──────────────────────────────────────
  let linkedMission = $state<LibraryMission | null>(null);
  let loggedWpCount = $state<number | null>(null);
  let linkBusy = $state(false);

  // ── Linked battery (soft link by serial) ───────────────────────────
  let batterySerial = $state('');           // the flight's serial (local copy, survives link/unlink)
  let batteryPack = $state<BatteryPack | null>(null); // resolved pack, or null = "not in library"
  let batteryEditing = $state(false);
  let batterySerialDraft = $state('');
  let batteryBusy = $state(false);

  // The map mission can be linked only if it is a "real" mission (has a provenance flag);
  // a pure unsaved scratch mission can't. DB → link directly; FILE/FC → save-to-DB-then-link.
  let canLink = $derived(
    $mission.waypoints.length > 0 && ($missionFlags.db || $missionFlags.file || $missionFlags.fc)
  );

  /** WP count for display: the linked mission's, else the Blackbox-header fallback. */
  let missionWpCount = $derived(linkedMission?.wp_count ?? loggedWpCount);

  async function refreshLink(flightId: number) {
    const dbPath = get(settings).flightLogDbPath;
    try {
      linkedMission = await missionDbForFlight(flightId, dbPath);
      loggedWpCount = await flightLoggedWpCount(flightId, dbPath);
    } catch {
      linkedMission = null;
      loggedWpCount = null;
    }
  }

  // Refresh on flight change (the async writes happen after the await, so they are not
  // dependencies of this effect → no read/write loop).
  $effect(() => {
    void refreshLink(flight.id);
  });

  function defaultMissionName(): string {
    const d = new Date();
    const p = (n: number) => String(n).padStart(2, '0');
    return `New Mission - ${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
  }

  /** Link the currently map-loaded mission to this flight. DB mission → link directly;
   *  FILE/FC mission not yet in the library → save it first, then link. */
  async function linkMission() {
    if (linkBusy || !canLink) return;
    linkBusy = true;
    const dbPath = get(settings).flightLogDbPath;
    const lang = get(locale) ?? 'en';
    try {
      const flags = get(missionFlags);
      let missionId = get(loadedMissionId);
      if (!(flags.db && missionId != null)) {
        const input = await buildMissionInput(get(mission).waypoints, { name: defaultMissionName() });
        missionId = await missionDbSave(input, dbPath);
        loadedMissionId.set(missionId);
        markMissionSynced('db');
        void missionDbGeocode(missionId, lang, dbPath).catch(() => {});
      }
      await flightLinkMission(flight.id, missionId, dbPath);
      await refreshLink(flight.id);
      replayWpTotal.set(linkedMission?.wp_count ?? loggedWpCount ?? null);
    } catch (e) {
      console.warn('[flight-detail] link mission failed', e);
    } finally {
      linkBusy = false;
    }
  }

  async function unlinkMission() {
    if (linkBusy) return;
    linkBusy = true;
    const dbPath = get(settings).flightLogDbPath;
    try {
      await flightUnlinkMission(flight.id, dbPath);
      await refreshLink(flight.id);
      replayWpTotal.set(loggedWpCount ?? null);
    } catch (e) {
      console.warn('[flight-detail] unlink mission failed', e);
    } finally {
      linkBusy = false;
    }
  }

  // Resolve the flight's battery serial to a library pack (null = not in library).
  async function refreshBattery(serial: string) {
    if (!serial) { batteryPack = null; return; }
    const dbPath = get(settings).flightLogDbPath;
    try {
      batteryPack = await batteryDbFindBySerial(serial, dbPath);
    } catch {
      batteryPack = null;
    }
  }

  // On flight change: take the serial from the flight and resolve the pack.
  $effect(() => {
    batterySerial = flight.battery_serial ?? '';
    batteryEditing = false;
    void refreshBattery(flight.battery_serial ?? '');
  });

  function startBatteryEdit() {
    batterySerialDraft = batterySerial;
    batteryEditing = true;
  }

  async function linkBattery() {
    if (batteryBusy) return;
    const serial = batterySerialDraft.trim();
    batteryBusy = true;
    const dbPath = get(settings).flightLogDbPath;
    try {
      await flightSetBatterySerial(flight.id, serial, dbPath);
      batterySerial = serial;
      batteryEditing = false;
      await refreshBattery(serial);
    } catch (e) {
      console.warn('[flight-detail] link battery failed', e);
    } finally {
      batteryBusy = false;
    }
  }

  // Jump to this pack in the Battery Manager (reverse of the battery's linked-flights jump) —
  // handy when a voltage issue in replay should lead straight to the pack's history.
  function openBattery() {
    if (!batteryPack) return;
    batteryManagerSelectedId.set(batteryPack.id);
    batteryManagerOpen.set(true);
  }

  // Jump to the linked mission in the Mission Manager (mission tab).
  function openMission() {
    if (!linkedMission) return;
    requestOpenMissionId.set(linkedMission.id);
  }

  async function unlinkBattery() {
    if (batteryBusy) return;
    batteryBusy = true;
    const dbPath = get(settings).flightLogDbPath;
    try {
      await flightSetBatterySerial(flight.id, '', dbPath);
      batterySerial = '';
      batteryPack = null;
    } catch (e) {
      console.warn('[flight-detail] unlink battery failed', e);
    } finally {
      batteryBusy = false;
    }
  }

  function startCraftNameEdit() {
    craftNameDraft = flight.craft_name ?? '';
    craftNameEditing = true;
  }

  function saveCraftName() {
    craftNameEditing = false;
    onSaveCraftName(craftNameDraft);
  }

  function cancelCraftNameEdit() {
    craftNameEditing = false;
  }

  // ── Platform type (INAV mixer enum; manually editable, drives the map replay symbol) ──
  let platformEditing = $state(false);
  const PLATFORM_KEYS: Record<number, string> = {
    0: 'platform.multirotor', 1: 'platform.airplane', 2: 'platform.helicopter',
    3: 'platform.tricopter', 4: 'platform.rover', 5: 'platform.boat', 6: 'platform.other',
  };
  const PLATFORM_OPTIONS = [0, 1, 2, 3, 4, 5, 6];
  function platformLabel(type: number): string {
    return PLATFORM_KEYS[type] ? $t(PLATFORM_KEYS[type]) : $t('platform.unknown', { values: { type } });
  }
  function changePlatformType(e: Event) {
    const value = Number((e.currentTarget as HTMLSelectElement).value);
    platformEditing = false;
    if (value !== flight.platform_type) onSavePlatformType(value);
  }
  // Reset the editor when switching flights.
  $effect(() => { void flight.id; platformEditing = false; });

  function startPilotEdit() {
    pilotNameDraft = flight.pilot_name ?? '';
    pilotIdDraft = flight.pilot_id ?? '';
    pilotEditing = true;
  }

  function savePilot() {
    pilotEditing = false;
    onSavePilot(pilotNameDraft, pilotIdDraft);
  }

  function cancelPilotEdit() {
    pilotEditing = false;
  }

  function focusOnMount(node: HTMLElement) {
    node.focus();
  }

  function formatWeatherTemp(tempC: number | null | undefined): string {
    if (tempC == null) return '';
    const converted = convertTemperature(tempC, interfaceSettings.temperatureUnit);
    return `${converted.value.toFixed(1)} ${converted.unit}`;
  }

  function formatAltitudeMeters(valueM: number | null | undefined): string {
    if (valueM == null) return '—';
    return formatConverted(convertAltitude(valueM, interfaceSettings.altitudeUnit), 1);
  }

  function formatSpeedMs(valueMs: number | null | undefined): string {
    if (valueMs == null) return '—';
    return formatConverted(convertSpeed(valueMs, interfaceSettings.speedUnit), 1);
  }

  function formatDistanceMeters(valueM: number | null | undefined): string {
    if (valueM == null) return '—';
    const converted = convertDistance(valueM, interfaceSettings.distanceUnit);
    const digits = converted.unit === 'm' || converted.unit === 'ft' ? 0 : 1;
    return formatConverted(converted, digits);
  }

  function formatWindSpeedMs(valueMs: number | null | undefined): string {
    if (valueMs == null) return '';
    return formatConverted(convertSpeed(valueMs, interfaceSettings.speedUnit), 1);
  }

  function windDegToLabel(deg: number): string {
    const dirs = ['N', 'NE', 'E', 'SE', 'S', 'SW', 'W', 'NW'];
    return dirs[Math.round(deg / 45) % 8];
  }

  function formatFlightSource(source: string): string {
    if (source === 'blackbox') return $t('logbook.sourceBlackbox');
    if (source === 'both') return $t('logbook.sourceBoth');
    return $t('logbook.sourceLive');
  }

  function formatDateTime(value: string): string {
    const d = new Date(value);
    return d.toLocaleString();
  }

  let tempUnitLabel = $derived(interfaceSettings.temperatureUnit === 'f' ? '°F' : '°C');
  let windUnitLabel = $derived(convertSpeed(1, interfaceSettings.speedUnit).unit);

  function autoResizeNotes(el: HTMLTextAreaElement, allowShrink = false) {
    const current = el.offsetHeight;
    el.style.height = 'auto';
    const minH = allowShrink ? 44 : Math.max(44, current);
    el.style.height = Math.max(minH, Math.min(el.scrollHeight, 140)) + 'px';
  }

  function notesAutoSize(el: HTMLTextAreaElement) {
    autoResizeNotes(el, true);
    return { update() { autoResizeNotes(el, true); } };
  }
</script>

<div class="logbook-detail" class:logbook-detail-minimized={minimized}>
  <div class="fc-info-grid">
    <span class="fc-label">{$t('logbook.craft')}</span>
    <span class="fc-value craft-value-row">
      {#if craftNameEditing}
        <input
          class="craft-name-input"
          type="text"
          bind:value={craftNameDraft}
          onkeydown={(e: KeyboardEvent) => { if (e.key === 'Enter') saveCraftName(); if (e.key === 'Escape') cancelCraftNameEdit(); }}
          onblur={saveCraftName}
          use:focusOnMount
        />
      {:else}
        <span>{flight.craft_name || $t('logbook.unnamedCraft')}</span>
        <button class="weather-edit-btn" onclick={startCraftNameEdit} title={$t('logbook.editCraftName')}>✎</button>
      {/if}
    </span>
    <span class="fc-label">{$t('logbook.type')}</span>
    <span class="fc-value craft-value-row">
      {#if platformEditing}
        <select class="platform-select" onchange={changePlatformType} use:focusOnMount>
          {#each PLATFORM_OPTIONS as pt}
            <option value={pt} selected={pt === flight.platform_type}>{platformLabel(pt)}</option>
          {/each}
        </select>
      {:else}
        <span>{platformLabel(flight.platform_type)}</span>
        {#if !minimized}<button class="weather-edit-btn" onclick={() => (platformEditing = true)} title={$t('logbook.editType')}>✎</button>{/if}
      {/if}
    </span>
    <span class="fc-label">{$t('logbook.pilot')}</span>
    <span class="fc-value craft-value-row">
      {#if pilotEditing}
        <input
          class="craft-name-input"
          type="text"
          placeholder={$t('logbook.pilotNamePlaceholder')}
          bind:value={pilotNameDraft}
          onkeydown={(e: KeyboardEvent) => { if (e.key === 'Enter') savePilot(); if (e.key === 'Escape') cancelPilotEdit(); }}
          use:focusOnMount
        />
      {:else}
        <span>{flight.pilot_name || $t('logbook.pilotNone')}</span>
        {#if !minimized}<button class="weather-edit-btn" onclick={startPilotEdit} title={$t('logbook.editPilot')}>✎</button>{/if}
      {/if}
    </span>
    <span class="fc-label">{$t('logbook.pilotId')}</span>
    <span class="fc-value craft-value-row">
      {#if pilotEditing}
        <input
          class="craft-name-input"
          type="text"
          placeholder={$t('logbook.pilotIdPlaceholder')}
          bind:value={pilotIdDraft}
          onkeydown={(e: KeyboardEvent) => { if (e.key === 'Enter') savePilot(); if (e.key === 'Escape') cancelPilotEdit(); }}
        />
        <button class="weather-edit-btn" onclick={savePilot} title={$t('logbook.savePilot')}>✓</button>
      {:else}
        <span>{flight.pilot_id || '—'}</span>
      {/if}
    </span>
    <span class="fc-label">{$t('logbook.firmware')}</span>
    <span class="fc-value">{flight.fc_version || `${flight.fc_variant || '—'}`}</span>
    <span class="fc-label">{$t('logbook.source')}</span>
    <span class="fc-value">
      {formatFlightSource(flight.source)}
      {#if !minimized}<span class="flight-id-tag">#{flight.id}</span>{/if}
      {#if flight.linked_flight_id} 🔗 #{flight.linked_flight_id}{/if}
    </span>
    <span class="fc-label">{$t('logbook.mission')}</span>
    <span class="fc-value craft-value-row">
      {#if linkedMission}
        <Button variant="compact" onclick={openMission} title={$t('logbook.openMission')}>{linkedMission.name || $t('logbook.unnamedMission')}</Button>
      {:else}
        <span>{$t('logbook.missionNone')}</span>
      {/if}
      {#if !minimized}
        {#if linkedMission}
          <Button variant="compact" icon="close" onclick={unlinkMission} disabled={linkBusy} title={$t('logbook.unlinkMission')} />
        {:else}
          <Button variant="compact" icon="link" onclick={linkMission} disabled={linkBusy || !canLink} title={canLink ? $t('logbook.linkMissionTip') : $t('logbook.linkMissionDisabledTip')}>{$t('logbook.linkMission')}</Button>
        {/if}
      {/if}
    </span>
    {#if missionWpCount != null}
      <span class="fc-label">{$t('logbook.missionWps')}</span>
      <span class="fc-value">{missionWpCount}</span>
    {/if}
    <span class="fc-label">{$t('logbook.battery')}</span>
    <span class="fc-value craft-value-row">
      {#if batteryEditing}
        <input
          class="craft-name-input"
          type="text"
          placeholder={$t('logbook.batterySerialPlaceholder')}
          bind:value={batterySerialDraft}
          onkeydown={(e: KeyboardEvent) => { if (e.key === 'Enter') linkBattery(); if (e.key === 'Escape') batteryEditing = false; }}
          use:focusOnMount
        />
        <Button variant="compact" icon="check" onclick={linkBattery} disabled={batteryBusy} title={$t('logbook.batteryLinkSave')} />
      {:else if batterySerial}
        {#if batteryPack}
          <Button variant="compact" onclick={openBattery} title={$t('logbook.openBattery')}>{batteryPack.label || batteryPack.serial}</Button>
        {:else}
          <span>{batterySerial}</span>
          <span class="battery-missing">{$t('logbook.batteryNotInLibrary')}</span>
        {/if}
        {#if !minimized}<Button variant="compact" icon="close" onclick={unlinkBattery} disabled={batteryBusy} title={$t('logbook.batteryUnlink')} />{/if}
      {:else}
        <span>{$t('logbook.missionNone')}</span>
        {#if !minimized}<Button variant="compact" icon="battery" onclick={startBatteryEdit} title={$t('logbook.linkBatteryTip')}>{$t('logbook.linkBattery')}</Button>{/if}
      {/if}
    </span>
    <span class="fc-label">{$t('logbook.started')}</span>
    <span class="fc-value">{formatDateTime(flight.start_time)}</span>
    <span class="fc-label">{$t('logbook.duration')}</span>
    <span class="fc-value">{formatDurationSec(flight.duration_sec)}</span>
    <span class="fc-label">{$t('logbook.location')}</span>
    <span class="fc-value">{flight.location_name || $t('logbook.unknownLocation')}</span>
    <span class="fc-label">{$t('logbook.maxAlt')}</span>
    <span class="fc-value">{formatAltitudeMeters(flight.max_alt_m)}</span>
    <span class="fc-label">{$t('logbook.maxSpeed')}</span>
    <span class="fc-value">{formatSpeedMs(flight.max_speed_ms)}</span>
    <span class="fc-label">{$t('logbook.totalDistance')}</span>
    <span class="fc-value">{formatDistanceMeters(flight.total_distance_m)}</span>
    <span class="fc-label">{$t('logbook.maxDistance')}</span>
    <span class="fc-value">{formatDistanceMeters(flight.max_distance_m)}</span>
    <span class="fc-label">{$t('logbook.batteryUsed')}</span>
    <span class="fc-value">{flight.battery_used_mah ?? '—'} mAh</span>
    <span class="fc-label">{$t('logbook.trackPoints')}</span>
    <span class="fc-value">{trackCount}</span>
    <span class="fc-label">{$t('logbook.weather')}</span>
    <span class="fc-value" class:weather-value-row={!minimized}>
      <span>
        {#if flight.weather_temp_c != null || flight.weather_desc}
          {formatWeatherTemp(flight.weather_temp_c)}
          {flight.weather_wind_ms != null ? ', ' + formatWindSpeedMs(flight.weather_wind_ms) : ''}
          {flight.weather_wind_deg != null ? ' ' + windDegToLabel(flight.weather_wind_deg) : ''}
          {flight.weather_desc ? ', ' + flight.weather_desc : ''}
        {:else}
          {$t('logbook.weatherUnavailable')}
        {/if}
      </span>
      {#if !minimized}
        <button class="weather-edit-btn" onclick={() => { weatherEditing = !weatherEditing; }} title={$t('logbook.editWeather')}>✎</button>
      {/if}
    </span>
  </div>

  {#if !minimized && weatherEditing}
    <WeatherEditor
      bind:weatherTempC
      bind:weatherWindMs
      bind:weatherWindDir
      bind:weatherDesc
      {tempUnitLabel}
      {windUnitLabel}
      onSave={onSaveWeather}
    />
  {/if}

  <div class="setting-row setting-row-stack">
    <span class="setting-label">{$t('logbook.notes')}</span>
    <textarea
      class="setting-input notes-input notes-input-auto"
      rows="2"
      readonly={minimized}
      bind:value={notes}
      oninput={minimized ? undefined : (e: Event) => autoResizeNotes(e.target as HTMLTextAreaElement)}
      use:notesAutoSize
    ></textarea>
  </div>

  {#if !minimized}
    <div class="setting-row">
      <Button variant="standard" icon="save" onclick={onSaveNotes}>{$t('logbook.saveNotes')}</Button>
      <Button variant="data" icon="export" onclick={onExportTrack}>{$t('logbook.exportTrack')}</Button>
      <Button variant="danger" icon="delete" onclick={onDeleteFlight}>{$t('logbook.deleteFlight')}</Button>
    </div>
  {/if}
</div>

<style>
  .logbook-detail {
    border: 1px solid #555;
    border-radius: 4px;
    background: rgba(0, 0, 0, 0.12);
    padding: 10px;
    overflow: auto;
    max-height: 560px;
  }

  .logbook-detail-minimized {
    border: none;
    background: none;
    padding: 0;
  }

  .fc-info-grid {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 6px 10px;
    font-size: 12px;
  }

  .fc-label {
    color: #949494;
  }

  .fc-value {
    color: #e0e0e0;
    font-weight: 600;
  }

  .weather-value-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .craft-value-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .craft-name-input {
    background: rgba(255, 255, 255, 0.06);
    border: 1px solid rgba(55, 168, 219, 0.4);
    border-radius: 4px;
    color: #ccc;
    font-size: 12px;
    padding: 1px 6px;
    outline: none;
    width: 100%;
    min-width: 0;
  }

  .craft-name-input:focus {
    border-color: #37a8db;
  }

  /* Matches the app's dark form-control convention; color-scheme:dark keeps the native
     option popup dark too (it's rendered by the WebView outside the DOM). */
  .platform-select {
    height: 24px;
    padding: 0 6px;
    background: #434343;
    border: 1px solid #555;
    border-radius: 4px;
    color: #e0e0e0;
    font-size: 12px;
    font-family: inherit;
    outline: none;
    max-width: 100%;
    color-scheme: dark;
  }
  .platform-select:hover {
    border-color: rgba(55, 168, 219, 0.6);
  }
  .platform-select:focus {
    border-color: #37a8db;
  }

  .weather-edit-btn {
    background: none;
    border: none;
    color: #777;
    cursor: pointer;
    font-size: 13px;
    padding: 0 2px;
    line-height: 1;
    flex-shrink: 0;
  }

  .weather-edit-btn:hover {
    color: #37a8db;
  }

  .flight-id-tag {
    font-size: 10px;
    color: #777;
    margin-left: 2px;
  }

  .battery-missing {
    font-size: 10px;
    color: #e0b050;
    font-weight: 400;
  }

  .setting-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 6px 0;
  }

  .setting-row-stack {
    flex-direction: column;
    align-items: stretch;
    gap: 6px;
  }

  .setting-label {
    font-size: 12px;
    color: #e0e0e0;
  }

  .setting-input {
    padding: 3px 6px;
    background: #434343;
    border: 1px solid #555;
    border-radius: 3px;
    color: #e0e0e0;
    font-size: 11px;
  }

  .notes-input {
    width: 100%;
    box-sizing: border-box;
    resize: vertical;
    min-height: 44px;
  }

  .notes-input-auto {
    overflow-y: auto;
    max-height: 140px;
  }

  .notes-input-auto[readonly] {
    resize: none;
    cursor: pointer;
    opacity: 0.85;
  }

</style>
