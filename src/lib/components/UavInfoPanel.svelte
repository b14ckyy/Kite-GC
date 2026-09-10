<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // UAV Info on the panel framework (docs/active/PANEL_FRAMEWORK.md): the `info` variant —
  // content-sized, unframed. Besides the read-only FC facts it hosts two small live actions:
  // the platform-type override (session-only, see connectionController.setLivePlatformType) and
  // "Save to vehicle library" for links with an FC identity (MSP / MAVLink).
  import { t } from 'svelte-i18n';
  import { get } from 'svelte/store';
  import type { FcInfo } from '$lib/stores/connection';
  import { detectedPlatformType } from '$lib/stores/connection';
  import { setLivePlatformType } from '$lib/controllers/connectionController';
  import { settings } from '$lib/stores/settings';
  import { telemetry } from '$lib/stores/telemetry';
  import { vehicleDbCreate, vehicleDbFindByCraftName, vehicleDbFindByFcUid } from '$lib/stores/flightlog';
  import { vehicleLibraryChanged } from '$lib/stores/vehicleManager';
  import type { VehicleInput } from '$lib/stores/flightlogTypes';
  import PanelShell from './panel/PanelShell.svelte';
  import FcUidChip from './FcUidChip.svelte';

  let {
    connStatus,
    fcInfo,
    onOpenVehicle = (_id: number) => {},
  }: {
    connStatus: string;
    fcInfo: FcInfo | null;
    /** "View in library": open the Vehicle Manager on this vehicle (the page owns the tab switch). */
    onOpenVehicle?: (id: number) => void;
  } = $props();

  const PLATFORM_KEYS: Record<number, string> = {
    0: 'platform.multirotor', 1: 'platform.airplane', 2: 'platform.helicopter',
    3: 'platform.tricopter', 4: 'platform.rover', 5: 'platform.boat', 6: 'platform.other',
    7: 'platform.vtol', 255: 'platform.generic',
  };
  const PLATFORM_OPTIONS = [0, 1, 2, 3, 4, 5, 6, 7, 255];
  function getPlatformLabel(type: number): string {
    return PLATFORM_KEYS[type] ? $t(PLATFORM_KEYS[type]) : $t('platform.unknown', { values: { type } });
  }

  // ── Platform-type override ──────────────────────────────────────────
  // The select writes through the controller (backend FC info + recorder, store); the new value
  // flows back in via `fcInfo`, so the select is never bound.
  let typeBusy = $state(false);
  const overridden = $derived(fcInfo != null && fcInfo.platform_type !== $detectedPlatformType);

  async function changePlatformType(e: Event) {
    const value = Number((e.currentTarget as HTMLSelectElement).value);
    if (!fcInfo || value === fcInfo.platform_type) return;
    typeBusy = true;
    try {
      await setLivePlatformType(value);
    } catch (err) {
      console.warn('[uav-info] platform-type override failed', err);
    } finally {
      typeBusy = false;
    }
  }

  // ── Save to vehicle library ─────────────────────────────────────────
  // Only links with an FC identity: MSP and MAVLink fill the variant; passive telemetry leaves it
  // empty (nothing to describe a vehicle with).
  const canSave = $derived(connStatus === 'connected' && fcInfo != null && fcInfo.fc_variant.length > 0);
  // Identity for the "already in the library" check: the FC hardware id (works for crafts without a
  // name — ArduPilot has none), else the craft name; neither → every click creates a vehicle.
  const libKey = $derived.by((): { uid: string } | { craft: string } | null => {
    if (connStatus !== 'connected' || !fcInfo) return null;
    if (fcInfo.fc_uid) return { uid: fcInfo.fc_uid };
    const craft = fcInfo.craft_name.trim();
    return craft ? { craft } : null;
  });
  let libState = $state<'idle' | 'exists' | 'saving' | 'saved' | 'error'>('idle');
  let libError = $state('');
  let existingId = $state<number | null>(null);

  $effect(() => {
    const key = libKey;
    libState = 'idle';
    libError = '';
    existingId = null;
    if (!key) return;
    let alive = true;
    const dbPath = get(settings).flightLogDbPath;
    const lookup = 'uid' in key ? vehicleDbFindByFcUid(key.uid, dbPath) : vehicleDbFindByCraftName(key.craft, dbPath);
    lookup
      .then((v) => { if (alive && v) { existingId = v.id; libState = 'exists'; } })
      .catch(() => {});
    return () => { alive = false; };
  });

  function viewInLibrary() {
    if (existingId != null) onOpenVehicle(existingId);
  }

  // INAV mixer platform → vehicle library type.
  const VEHICLE_TYPE_FOR: Record<number, string> = {
    0: 'multirotor', 1: 'fixed_wing', 2: 'helicopter', 3: 'multirotor', 4: 'rover', 5: 'boat',
    6: 'other', 7: 'vtol', 255: 'other',
  };
  // FC variant → the library's firmware choices (VehicleManager FIRMWARES).
  function firmwareFor(variant: string): string | null {
    if (variant === 'INAV') return 'INAV';
    if (variant === 'BTFL') return 'Betaflight';
    if (variant.startsWith('Ardu')) return 'ArduPilot';
    if (variant.startsWith('PX4')) return 'PX4';
    return variant ? 'Other' : null;
  }

  async function saveToLibrary() {
    if (!fcInfo) return;
    libState = 'saving';
    const craft = fcInfo.craft_name.trim();
    // MAVLink has no craft name and no board target (board_id = "MAVLink") → variant as the name.
    const mavlink = fcInfo.board_id === 'MAVLink';
    // Sensors as the FC reports them right now (MSP_SENSOR_STATUS / SYS_STATUS present bits): any
    // non-zero state — OK, unavailable, unhealthy — means the sensor is configured on this craft. RTK
    // is only visible through the fix type (RTK float / fixed), so it needs an RTK fix at click time.
    const tel = get(telemetry);
    const input: VehicleInput = {
      name: craft || fcInfo.fc_variant,
      craft_name: craft || null,
      vehicle_type: VEHICLE_TYPE_FOR[fcInfo.platform_type] ?? 'other',
      status: 'active',
      image: null,
      notes: null,
      model: null,
      wingspan_mm: null, length_mm: null, weight_auw_g: null, weight_dry_g: null,
      motors: null, props: null, esc: null,
      recommended_cells: null, recommended_capacity_mah: null,
      rx: null, vtx: null, camera: null, gimbal_camera: null, datalink: null,
      sensor_airspeed: tel.sensorPitot !== 0,
      sensor_rangefinder: tel.sensorRangefinder !== 0,
      sensor_optical_flow: tel.sensorOpflow !== 0,
      sensor_gps: tel.sensorGps !== 0,
      sensor_rtk: tel.fixType >= 5,
      sensor_compass: tel.sensorMag !== 0,
      fc_model: mavlink ? null : fcInfo.board_id || null,
      fc_manufacturer: null,
      fc_firmware: firmwareFor(fcInfo.fc_variant),
      fc_firmware_version: fcInfo.fc_version || null,
      blackbox_available: fcInfo.blackbox ?? false,
      fc_uid: fcInfo.fc_uid,
    };
    try {
      existingId = await vehicleDbCreate(input, get(settings).flightLogDbPath);
      vehicleLibraryChanged.update((n) => n + 1);
      libState = 'saved';
    } catch (e) {
      libError = String(e);
      libState = 'error';
    }
  }
</script>

<PanelShell variant="info" title={$t('nav.uavInfo')}>
  {#snippet body()}
    {#if connStatus === "connected" && fcInfo}
      <section class="panel-section">
        <h4 class="section-heading">{$t('uavInfo.flightController')}</h4>
        <div class="fc-info-grid">
          <span class="fc-label">{$t('uavInfo.craftName')}</span>
          <span class="fc-value">{fcInfo.craft_name || $t('uavInfo.craftNameUnset')}</span>
          <span class="fc-label">{$t('uavInfo.variant')}</span>
          <span class="fc-value">{fcInfo.fc_variant}</span>
          <span class="fc-label">{$t('uavInfo.version')}</span>
          <span class="fc-value">{fcInfo.fc_version}</span>
          <span class="fc-label">{$t('uavInfo.board')}</span>
          <span class="fc-value">{fcInfo.board_id}</span>
          <span class="fc-label">{$t('uavInfo.type')}</span>
          <span class="fc-value type-row">
            <select class="type-select" value={fcInfo.platform_type} onchange={changePlatformType} disabled={typeBusy} title={$t('uavInfo.typeHint')}>
              {#each PLATFORM_OPTIONS as pt}
                <option value={pt}>{getPlatformLabel(pt)}</option>
              {/each}
            </select>
            {#if overridden}
              <span class="type-detected">{$t('uavInfo.typeDetected', { values: { type: getPlatformLabel($detectedPlatformType) } })}</span>
            {/if}
          </span>
          <span class="fc-label">{$t('uavInfo.api')}</span>
          <span class="fc-value">{fcInfo.api_version}</span>
          {#if fcInfo.hardware_revision > 0}
            <span class="fc-label">{$t('uavInfo.hwRev')}</span>
            <span class="fc-value">{fcInfo.hardware_revision}</span>
          {/if}
          {#if fcInfo.fc_uid}
            <span class="fc-label">{$t('uavInfo.uid')}</span>
            <span class="fc-value"><FcUidChip uid={fcInfo.fc_uid} /></span>
          {/if}
        </div>
      </section>

      {#if fcInfo.features}
        <section class="panel-section" class:panel-section-last={!canSave}>
          <h4 class="section-heading">{$t('uavInfo.features')}</h4>
          <div class="feature-list">
            <span class="feature-badge available">{$t('uavInfo.telemetry')}</span>
            <span class="feature-badge" class:available={fcInfo.features.autoland_config} class:unavailable={!fcInfo.features.autoland_config} title="INAV 7.1+">{$t('uavInfo.autoland')}</span>
            <span class="feature-badge" class:available={fcInfo.features.geozones} class:unavailable={!fcInfo.features.geozones} title="INAV 8.0+">{$t('uavInfo.geozones')}</span>
            <span class="feature-badge" class:available={fcInfo.features.msp_rc} class:unavailable={!fcInfo.features.msp_rc} title="INAV 8.0+">{$t('uavInfo.mspRc')}</span>
            <span class="feature-badge" class:available={fcInfo.features.aux_rc} class:unavailable={!fcInfo.features.aux_rc} title="INAV 9.1+">{$t('uavInfo.auxRc')}</span>
            <span class="feature-badge" class:available={fcInfo.features.adsb_msp} class:unavailable={!fcInfo.features.adsb_msp} title="INAV 8.0+">{$t('uavInfo.adsb')}</span>
          </div>
        </section>
      {/if}

      {#if canSave}
        <section class="panel-section panel-section-last">
          {#if libState === 'exists' || libState === 'saved'}
            <button class="lib-btn" onclick={viewInLibrary}>
              {libState === 'saved' ? $t('uavInfo.savedToLibrary') : $t('uavInfo.viewInLibrary')}
            </button>
          {:else}
            <button class="lib-btn" onclick={saveToLibrary} disabled={libState === 'saving'}>
              {$t('uavInfo.saveToLibrary')}
            </button>
          {/if}
          {#if libState === 'error'}
            <div class="lib-error">{$t('uavInfo.saveFailed', { values: { error: libError } })}</div>
          {/if}
        </section>
      {/if}
    {:else}
      <div class="panel-empty">
        <span class="panel-empty-icon">⊘</span>
        <span>{$t('uavInfo.notConnected')}</span>
      </div>
    {/if}
  {/snippet}
</PanelShell>

<style>
  .panel-section {
    margin-bottom: 16px;
  }
  /* The shell field is content-sized in `info`; trim the trailing gap. */
  .panel-section-last {
    margin-bottom: 0;
  }

  .section-heading {
    margin: 0 0 8px 0;
    font-size: 11px;
    font-weight: 600;
    color: #37a8db;
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .panel-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 32px 24px;
    color: #555;
    font-size: 12px;
  }

  .panel-empty-icon {
    font-size: 28px;
    opacity: 0.4;
  }

  .fc-info-grid {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 6px 10px;
    font-size: 12px;
    align-items: center;
  }

  .fc-label {
    color: #949494;
  }

  .fc-value {
    color: #e0e0e0;
    font-weight: 600;
  }

  .type-row {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .type-select {
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
  .type-select:hover {
    border-color: rgba(55, 168, 219, 0.6);
  }
  .type-select:focus {
    border-color: #37a8db;
  }
  .type-select:disabled {
    opacity: 0.6;
  }

  .type-detected {
    font-size: 10px;
    font-weight: 400;
    color: #949494;
  }

  .lib-btn {
    width: 100%;
    padding: 5px 10px;
    background: rgba(55, 168, 219, 0.12);
    border: 1px solid rgba(55, 168, 219, 0.5);
    border-radius: 4px;
    color: #37a8db;
    font-size: 11px;
    font-weight: 600;
    font-family: inherit;
    cursor: pointer;
  }
  .lib-btn:hover:not(:disabled) {
    background: rgba(55, 168, 219, 0.22);
  }
  .lib-btn:disabled {
    opacity: 0.6;
    cursor: default;
  }

  .lib-error {
    margin-top: 4px;
    font-size: 10px;
    color: #d40000;
  }

  /* Fixed 2 columns so the panel doesn't grow wide with the feature count (wraps to 2×N rows). */
  .feature-list {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 4px;
  }

  .feature-badge {
    padding: 3px 8px;
    border-radius: 3px;
    font-size: 10px;
    font-weight: 600;
    text-align: center;
  }

  .feature-badge.available {
    background: rgba(89, 170, 41, 0.2);
    color: #59aa29;
    border: 1px solid rgba(89, 170, 41, 0.4);
  }

  .feature-badge.unavailable {
    background: rgba(80, 80, 80, 0.2);
    color: #555;
    border: 1px solid #444;
    text-decoration: line-through;
  }
</style>
