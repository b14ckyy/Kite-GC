<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- VehicleManager.svelte
     Vehicle library on the panel framework (docs/active/PANEL_FRAMEWORK.md) — its own PanelShell
     (compact ↔ advanced 1:2 split): vehicle list grouped by type in the main field, the build-sheet
     detail (image header + structured specs + stats + linked flights) in the detail field. Opened
     from the Flight Logbook's "Vehicles" toggle. Flights soft-link by craft_name.
     See docs/active/VEHICLE_DB.md.
-->
<script lang="ts">
  import { t } from 'svelte-i18n';
  import { get } from 'svelte/store';
  import { save, open as openFileDialog } from '@tauri-apps/plugin-dialog';
  import { settings } from '$lib/stores/settings';
  import { convertAltitude, convertDistance, formatConverted } from '$lib/utils/units';
  import {
    vehicleDbList, vehicleDbCreate, vehicleDbUpdate, vehicleDbDelete,
    vehicleDbFindByCraftName, vehicleDbAggregate, vehicleDbFlights,
    vehicleDbSetBaseline, vehicleFileWrite, vehicleFileRead, formatDurationSec,
  } from '$lib/stores/flightlog';
  import type { Vehicle, VehicleInput, VehicleAggregate, VehicleFile, FlightSummary, InavStats } from '$lib/stores/flightlogTypes';
  import {
    vehicleManagerSelectedId, vehicleSearchQuery, vehicleManagerCreateCraft, normalizeCraftName,
  } from '$lib/stores/vehicleManager';
  import { requestOpenFlightId } from '$lib/stores/missionManager';
  import { connection } from '$lib/stores/connection';
  import { autopilotSystem } from '$lib/stores/autopilotContext';
  import { telemetry } from '$lib/stores/telemetry';
  import { setInavCraftName, readInavStats } from '$lib/controllers/connectionController';
  import PanelShell, { type PanelVariant } from '$lib/components/panel/PanelShell.svelte';
  import Button from '$lib/components/panel/Button.svelte';
  import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
  import NumberStepper from '$lib/components/NumberStepper.svelte';
  import Toggle from '$lib/components/panel/Toggle.svelte';

  let { onBack }: { onBack: () => void } = $props();

  let confirmDialog: ReturnType<typeof ConfirmDialog>;

  let vehicles = $state<Vehicle[]>([]);
  let aggregate = $state<VehicleAggregate | null>(null);
  let linkedFlights = $state<FlightSummary[]>([]);
  let open = $state<Set<string>>(new Set());
  let statusMessage = $state('');
  $effect(() => {
    if (!statusMessage) return;
    const id = setTimeout(() => { statusMessage = ''; }, 10000);
    return () => clearTimeout(id);
  });

  // Edit/create form: numeric fields use NaN for "empty" (NumberStepper allowEmpty).
  interface VehicleForm {
    name: string; craftName: string; vehicleType: string; status: string;
    image: string | null; notes: string;
    model: string; wingspanMm: number; lengthMm: number; weightAuwG: number; weightDryG: number;
    motors: string; props: string; esc: string;
    recommendedCells: string; recommendedCapacityMah: number;
    rx: string; vtx: string; camera: string; gimbalCamera: string; datalink: string;
    sensorAirspeed: boolean; sensorRangefinder: boolean; sensorOpticalFlow: boolean;
    sensorGps: boolean; sensorRtk: boolean; sensorCompass: boolean;
    fcModel: string; fcManufacturer: string; fcFirmware: string; fcFirmwareVersion: string;
    blackboxAvailable: boolean;
  }
  let editing = $state(false);
  let isCreate = $state(false);
  let form = $state<VehicleForm>(blankForm());

  // Import preview (.kvehicle).
  let importFile = $state<VehicleFile | null>(null);
  let importBusy = $state(false);

  // Write craft name to FC: only when an INAV FC is connected and disarmed (post-flight use).
  let fcWriteBusy = $state(false);
  let canWriteToFc = $derived(
    $connection.status === 'connected' && $autopilotSystem === 'inav' && ($telemetry.armingFlags & 0x04) === 0,
  );

  // Pending lifetime baseline to apply on save (adopted on request from the FC `stats` totals, or
  // carried in from a `.kvehicle` import). null = leave the stored baseline untouched.
  let formBaseline = $state<InavStats | null>(null);
  let fcStatsBusy = $state(false);

  const VEHICLE_TYPES = ['fixed_wing', 'flying_wing', 'vtol', 'multirotor', 'helicopter', 'rover', 'boat', 'other'];
  const VEHICLE_STATUSES = ['active', 'storage', 'retired', 'damaged', 'crashed'];
  const FIRMWARES = ['INAV', 'Betaflight', 'ArduPilot', 'PX4', 'Other'];
  const ARCHIVED = new Set(['retired', 'damaged', 'crashed']);

  function dbPath(): string { return get(settings).flightLogDbPath; }

  let selected = $derived(vehicles.find((v) => v.id === $vehicleManagerSelectedId) ?? null);
  const variant = $derived<PanelVariant>(selected != null ? 'advanced' : 'compact');

  function blankForm(): VehicleForm {
    return {
      name: '', craftName: '', vehicleType: 'multirotor', status: 'active', image: null, notes: '',
      model: '', wingspanMm: NaN, lengthMm: NaN, weightAuwG: NaN, weightDryG: NaN,
      motors: '', props: '', esc: '', recommendedCells: '', recommendedCapacityMah: NaN,
      rx: '', vtx: '', camera: '', gimbalCamera: '', datalink: '',
      sensorAirspeed: false, sensorRangefinder: false, sensorOpticalFlow: false,
      sensorGps: false, sensorRtk: false, sensorCompass: false,
      fcModel: '', fcManufacturer: '', fcFirmware: '', fcFirmwareVersion: '', blackboxAvailable: false,
    };
  }

  function strOrNull(s: string): string | null { const v = s.trim(); return v ? v : null; }
  function numOrNull(n: number): number | null { return Number.isFinite(n) ? Math.round(n) : null; }

  function formToInput(): VehicleInput {
    return {
      name: form.name.trim(),
      craft_name: strOrNull(normalizeCraftName(form.craftName)),
      vehicle_type: form.vehicleType,
      status: form.status,
      image: form.image,
      notes: strOrNull(form.notes),
      model: strOrNull(form.model),
      wingspan_mm: numOrNull(form.wingspanMm),
      length_mm: numOrNull(form.lengthMm),
      weight_auw_g: numOrNull(form.weightAuwG),
      weight_dry_g: numOrNull(form.weightDryG),
      motors: strOrNull(form.motors),
      props: strOrNull(form.props),
      esc: strOrNull(form.esc),
      recommended_cells: strOrNull(form.recommendedCells),
      recommended_capacity_mah: numOrNull(form.recommendedCapacityMah),
      rx: strOrNull(form.rx),
      vtx: strOrNull(form.vtx),
      camera: strOrNull(form.camera),
      gimbal_camera: strOrNull(form.gimbalCamera),
      datalink: strOrNull(form.datalink),
      sensor_airspeed: form.sensorAirspeed,
      sensor_rangefinder: form.sensorRangefinder,
      sensor_optical_flow: form.sensorOpticalFlow,
      sensor_gps: form.sensorGps,
      sensor_rtk: form.sensorRtk,
      sensor_compass: form.sensorCompass,
      fc_model: strOrNull(form.fcModel),
      fc_manufacturer: strOrNull(form.fcManufacturer),
      fc_firmware: strOrNull(form.fcFirmware),
      fc_firmware_version: strOrNull(form.fcFirmwareVersion),
      blackbox_available: form.blackboxAvailable,
    };
  }

  // Search filter (name / craft name / model / type / notes / FC).
  let filtered = $derived.by(() => {
    const q = $vehicleSearchQuery.trim().toLowerCase();
    if (!q) return vehicles;
    return vehicles.filter((v) =>
      [v.name, v.craft_name, v.model, v.vehicle_type, v.notes, v.fc_model, v.fc_manufacturer]
        .some((f) => f != null && f.toLowerCase().includes(q))
    );
  });

  // ── Grouping (by type; retired/damaged/crashed pulled into a trailing Archived group) ──
  interface VehSection { key: string; label: string; count: number; vehicles: Vehicle[]; special?: boolean; }
  let sections = $derived.by<VehSection[]>(() => {
    const active = filtered.filter((v) => !ARCHIVED.has(v.status));
    const archived = filtered.filter((v) => ARCHIVED.has(v.status));
    const byName = (a: Vehicle, b: Vehicle) => a.name.localeCompare(b.name, undefined, { numeric: true });
    const out: VehSection[] = [];
    for (const type of VEHICLE_TYPES) {
      const list = active.filter((v) => v.vehicle_type === type).sort(byName);
      if (list.length) out.push({ key: type, label: $t(`vehicleMgr.typeVal.${type}`), count: list.length, vehicles: list });
    }
    if (archived.length) {
      out.push({ key: '__archived', label: $t('vehicleMgr.groupArchived'), count: archived.length, vehicles: archived.sort(byName), special: true });
    }
    return out;
  });

  // ── Loading ─────────────────────────────────────────────────────────
  async function reload() {
    try {
      vehicles = await vehicleDbList(dbPath());
    } catch (e) {
      statusMessage = $t('vehicleMgr.loadFailed', { values: { error: String(e) } });
    }
  }

  async function loadDetails(v: Vehicle) {
    if (!v.craft_name) { aggregate = null; linkedFlights = []; return; }
    try {
      aggregate = await vehicleDbAggregate(v.craft_name, dbPath());
      linkedFlights = await vehicleDbFlights(v.craft_name, dbPath());
    } catch {
      aggregate = null;
      linkedFlights = [];
    }
  }

  function select(v: Vehicle) {
    vehicleManagerSelectedId.set(v.id);
    editing = false;
    void loadDetails(v);
  }

  function toggleGroup(key: string) {
    const n = new Set(open);
    if (n.has(key)) n.delete(key); else n.add(key);
    open = n;
  }

  // ── Create / edit ───────────────────────────────────────────────────
  function startCreate() {
    form = blankForm();
    formBaseline = null;
    isCreate = true;
    editing = true;
  }

  function startEdit() {
    if (!selected) return;
    const v = selected;
    form = {
      name: v.name, craftName: v.craft_name ?? '', vehicleType: v.vehicle_type, status: v.status,
      image: v.image, notes: v.notes ?? '',
      model: v.model ?? '', wingspanMm: v.wingspan_mm ?? NaN, lengthMm: v.length_mm ?? NaN,
      weightAuwG: v.weight_auw_g ?? NaN, weightDryG: v.weight_dry_g ?? NaN,
      motors: v.motors ?? '', props: v.props ?? '', esc: v.esc ?? '',
      recommendedCells: v.recommended_cells ?? '', recommendedCapacityMah: v.recommended_capacity_mah ?? NaN,
      rx: v.rx ?? '', vtx: v.vtx ?? '', camera: v.camera ?? '', gimbalCamera: v.gimbal_camera ?? '', datalink: v.datalink ?? '',
      sensorAirspeed: v.sensor_airspeed, sensorRangefinder: v.sensor_rangefinder, sensorOpticalFlow: v.sensor_optical_flow,
      sensorGps: v.sensor_gps, sensorRtk: v.sensor_rtk, sensorCompass: v.sensor_compass,
      fcModel: v.fc_model ?? '', fcManufacturer: v.fc_manufacturer ?? '', fcFirmware: v.fc_firmware ?? '',
      fcFirmwareVersion: v.fc_firmware_version ?? '', blackboxAvailable: v.blackbox_available,
    };
    formBaseline = null;
    isCreate = false;
    editing = true;
  }

  function cancelEdit() { editing = false; }

  // Read the FC `stats` lifetime totals and stage them as the pending baseline (applied on save).
  async function adoptFcStats() {
    fcStatsBusy = true;
    try {
      const s = await readInavStats();
      if (!s.enabled) { statusMessage = $t('vehicleMgr.fcStatsDisabled'); return; }
      formBaseline = s;
      statusMessage = $t('vehicleMgr.fcStatsAdopted', { values: { count: String(s.flight_count) } });
    } catch (e) {
      statusMessage = $t('vehicleMgr.fcStatsFailed', { values: { error: String(e) } });
    } finally {
      fcStatsBusy = false;
    }
  }

  async function saveForm() {
    const input = formToInput();
    if (!input.name) { statusMessage = $t('vehicleMgr.nameRequired'); return; }
    try {
      let targetId: number;
      if (isCreate) {
        targetId = await vehicleDbCreate(input, dbPath());
      } else if (selected) {
        await vehicleDbUpdate(selected.id, input, dbPath());
        targetId = selected.id;
      } else {
        return;
      }
      // Apply a freshly adopted/imported baseline (overwrites the stored one); else leave it untouched.
      if (formBaseline) {
        const b = formBaseline;
        await vehicleDbSetBaseline(targetId, b.flight_count, b.total_time_s, b.total_dist_m, b.total_energy, dbPath());
        formBaseline = null;
      }
      await reload();
      vehicleManagerSelectedId.set(targetId);
      const v = vehicles.find((x) => x.id === targetId);
      if (v) await loadDetails(v);
      editing = false;
      statusMessage = $t('vehicleMgr.saved');
    } catch (e) {
      statusMessage = $t('vehicleMgr.saveFailed', { values: { error: String(e) } });
    }
  }

  async function deleteVehicle() {
    if (!selected) return;
    const v = selected;
    const count = linkedFlights.length;
    const ans = await confirmDialog.show({
      title: $t('vehicleMgr.deleteTitle'),
      message: count > 0
        ? $t('vehicleMgr.deleteMsgLinked', { values: { count: String(count) } })
        : $t('vehicleMgr.deleteMsg'),
      buttons: [
        { label: $t('vehicleMgr.deleteYes'), value: 'delete', danger: true },
        { label: $t('vehicleMgr.cancel'), value: 'cancel' },
      ],
    });
    if (ans !== 'delete') return;
    try {
      await vehicleDbDelete(v.id, dbPath());
      vehicleManagerSelectedId.set(null);
      aggregate = null;
      linkedFlights = [];
      await reload();
      statusMessage = $t('vehicleMgr.deleted');
    } catch (e) {
      statusMessage = $t('vehicleMgr.deleteFailed', { values: { error: String(e) } });
    }
  }

  function openFlight(id: number | null) { if (id != null) requestOpenFlightId.set(id); }

  // Push the form's craft name to the connected INAV FC (MSP_SET_NAME + EEPROM) so future flights
  // auto-link. Only offered when an INAV FC is connected + disarmed (typical post-flight situation).
  async function writeCraftToFc() {
    const name = normalizeCraftName(form.craftName);
    if (!name) { statusMessage = $t('vehicleMgr.craftNameNeededForFc'); return; }
    fcWriteBusy = true;
    try {
      await setInavCraftName(name);
      statusMessage = $t('vehicleMgr.fcWritten', { values: { name } });
    } catch (e) {
      statusMessage = $t('vehicleMgr.fcWriteFailed', { values: { error: String(e) } });
    } finally {
      fcWriteBusy = false;
    }
  }

  // ── Image (downscale to a data URI so it travels with the row + .kvehicle) ──
  function downscaleToDataUrl(file: File, maxDim: number, quality: number): Promise<string> {
    return new Promise((resolve, reject) => {
      const url = URL.createObjectURL(file);
      const img = new Image();
      img.onload = () => {
        URL.revokeObjectURL(url);
        const scale = Math.min(1, maxDim / Math.max(img.width, img.height));
        const w = Math.round(img.width * scale);
        const h = Math.round(img.height * scale);
        const canvas = document.createElement('canvas');
        canvas.width = w;
        canvas.height = h;
        const ctx = canvas.getContext('2d');
        if (!ctx) { reject(new Error('no 2d context')); return; }
        ctx.drawImage(img, 0, 0, w, h);
        resolve(canvas.toDataURL('image/jpeg', quality));
      };
      img.onerror = () => { URL.revokeObjectURL(url); reject(new Error('image load failed')); };
      img.src = url;
    });
  }

  async function onPickImage(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = '';
    if (!file) return;
    try {
      form.image = await downscaleToDataUrl(file, 1024, 0.82);
    } catch (err) {
      statusMessage = $t('vehicleMgr.imageFailed', { values: { error: String(err) } });
    }
  }

  // ── Export (.kvehicle) ──────────────────────────────────────────────
  function sanitize(s: string): string { return s.replace(/[^\w\-]+/g, '_').replace(/^_+|_+$/g, ''); }

  function vehicleToInput(v: Vehicle): VehicleInput {
    const {
      id: _id, created_at: _c, updated_at: _u,
      base_flight_count: _bf, base_total_time_s: _bt, base_total_dist_m: _bd, base_total_energy: _be,
      ...rest
    } = v;
    return rest;
  }

  async function exportVehicle() {
    if (!selected) return;
    const v = selected;
    const file: VehicleFile = {
      format: 'kvehicle',
      version: 1,
      exported_at: new Date().toISOString(),
      vehicle: vehicleToInput(v),
      base_flight_count: v.base_flight_count,
      base_total_time_s: v.base_total_time_s,
      base_total_dist_m: v.base_total_dist_m,
      base_total_energy: v.base_total_energy,
    };
    try {
      const path = await save({
        title: $t('vehicleMgr.exportTitle'),
        defaultPath: `vehicle_${sanitize(v.name)}.kvehicle`,
        filters: [{ name: 'Vehicle', extensions: ['kvehicle'] }],
      });
      if (!path) return;
      await vehicleFileWrite(path, file);
      statusMessage = $t('vehicleMgr.exported');
    } catch (e) {
      statusMessage = $t('vehicleMgr.exportFailed', { values: { error: String(e) } });
    }
  }

  // ── Import (.kvehicle) ──────────────────────────────────────────────
  async function handleImport() {
    try {
      const path = await openFileDialog({ title: $t('vehicleMgr.importTitle'), multiple: false, filters: [{ name: 'Vehicle', extensions: ['kvehicle'] }] });
      if (!path || typeof path !== 'string') return;
      importFile = await vehicleFileRead(path);
      editing = false;
    } catch (e) {
      statusMessage = $t('vehicleMgr.importFailed', { values: { error: String(e) } });
    }
  }

  function cancelImport() { importFile = null; }

  async function applyImport() {
    if (!importFile || importBusy) return;
    const f = importFile;
    if (!f.vehicle.name.trim()) { statusMessage = $t('vehicleMgr.nameRequired'); return; }
    importBusy = true;
    try {
      const id = await vehicleDbCreate(f.vehicle, dbPath());
      // Restore the lifetime baseline carried in the file (0s if it was a baseline-free export).
      if (f.base_flight_count || f.base_total_time_s || f.base_total_dist_m || f.base_total_energy) {
        await vehicleDbSetBaseline(id, f.base_flight_count, f.base_total_time_s, f.base_total_dist_m, f.base_total_energy, dbPath());
      }
      importFile = null;
      await reload();
      vehicleManagerSelectedId.set(id);
      const v = vehicles.find((x) => x.id === id);
      if (v) await loadDetails(v);
      statusMessage = $t('vehicleMgr.imported');
    } catch (e) {
      statusMessage = $t('vehicleMgr.importFailed', { values: { error: String(e) } });
    } finally {
      importBusy = false;
    }
  }

  // ── Formatting ──────────────────────────────────────────────────────
  function fmtDateTime(value: string | null): string { return value ? new Date(value).toLocaleString() : '—'; }
  function typeLabel(ty: string): string { return $t(`vehicleMgr.typeVal.${ty}`); }
  function statusLabel(s: string): string { return $t(`vehicleMgr.statusVal.${s}`); }
  function fmtAlt(m: number | null): string {
    if (m == null) return '—';
    return formatConverted(convertAltitude(m, $settings.interface.altitudeUnit), 0);
  }
  function fmtDist(m: number | null): string {
    if (m == null) return '—';
    const c = convertDistance(m, $settings.interface.distanceUnit);
    return formatConverted(c, c.unit === 'm' || c.unit === 'ft' ? 0 : 2);
  }

  // Displayed lifetime = stored FC baseline + Σ(logged flights). Records (max) stay flight-derived.
  let lifetime = $derived.by(() => {
    const v = selected;
    if (!v) return null;
    const agg = aggregate;
    const hasBaseline =
      v.base_flight_count > 0 || v.base_total_time_s > 0 || v.base_total_dist_m > 0 || v.base_total_energy > 0;
    const flights = (agg?.flight_count ?? 0) + v.base_flight_count;
    if (flights === 0 && !hasBaseline) return null;
    return {
      flights,
      timeS: (agg?.sum_duration_sec ?? 0) + v.base_total_time_s,
      distM: (agg?.sum_distance_m ?? 0) + v.base_total_dist_m,
      energy: v.base_total_energy, // FC lifetime energy (mWh); logged-flight mAh is not unit-compatible
      hasBaseline,
      firstUsed: agg?.first_used ?? null,
      lastUsed: agg?.last_used ?? null,
    };
  });

  // Whether the vehicle has any field in a given spec section (to hide empty sections).
  function any(...vals: (string | number | null | boolean)[]): boolean {
    return vals.some((v) => v != null && v !== '' && v !== false);
  }

  // Notes textarea auto-grow.
  function autoResize(el: HTMLTextAreaElement) {
    el.style.height = 'auto';
    const extra = el.offsetHeight - el.clientHeight;
    el.style.height = Math.max(44, Math.min(el.scrollHeight + extra, 140)) + 'px';
  }
  function notesAutoSize(el: HTMLTextAreaElement, _v: string) {
    autoResize(el);
    return { update() { autoResize(el); } };
  }

  // External "create vehicle from this craft name" trigger: open straight into the create form
  // with name + craft name pre-filled, then consume the one-shot signal.
  $effect(() => {
    const c = $vehicleManagerCreateCraft;
    if (c == null) return;
    startCreate();
    const name = normalizeCraftName(c);
    form.name = name;
    form.craftName = name;
    vehicleManagerCreateCraft.set(null);
  });

  // Init once: load + restore selection.
  let didInit = false;
  $effect(() => {
    if (didInit) return;
    didInit = true;
    void (async () => {
      await reload();
      const id = get(vehicleManagerSelectedId);
      const v = id != null ? vehicles.find((x) => x.id === id) : undefined;
      if (v) await loadDetails(v);
      else if (id != null) vehicleManagerSelectedId.set(null);
    })();
  });
</script>

<!-- Toolbar (main column): search · button row (Back · New / Import). -->
{#snippet toolbar()}
  <div class="vmv-toolstack">
    <div class="setting-row">
      <input
        type="text"
        class="setting-input vmv-search"
        placeholder={$t('vehicleMgr.searchPlaceholder')}
        bind:value={$vehicleSearchQuery}
      />
      {#if $vehicleSearchQuery}
        <button class="vmv-search-clear" onclick={() => vehicleSearchQuery.set('')} title={$t('logbook.searchClear')}>✕</button>
      {/if}
    </div>

    <div class="vmv-toolbtns">
      <div class="tb-left">
        <Button variant="standard" icon="drone" onclick={onBack}>← {$t('vehicleMgr.back')}</Button>
        <Button variant="standard" icon="add" onclick={startCreate}>{$t('vehicleMgr.new')}</Button>
      </div>
      <div class="tb-right">
        <Button variant="data" icon="import" onclick={handleImport}>{$t('vehicleMgr.import')}</Button>
      </div>
    </div>
  </div>
{/snippet}

<!-- Main field: the grouped vehicle list (or empty state). -->
{#snippet body()}
  {#if vehicles.length === 0}
    <div class="panel-empty">
      <span class="panel-empty-icon">🛩️</span>
      <span>{$t('vehicleMgr.empty')}</span>
    </div>
  {:else}
    {#each sections as g (g.key)}
      <div class="tree-node" class:tree-special={g.special}>
        <button class="tree-toggle" onclick={() => toggleGroup(g.key)}>
          <span class="tree-caret">{open.has(g.key) ? '▾' : '▸'}</span>
          <span class="tree-label">{g.label}</span>
          <span class="tree-count">{g.count}</span>
        </button>
        {#if open.has(g.key)}
          <div class="tree-items">
            {#each g.vehicles as v (v.id)}
              {@render vehicleRow(v)}
            {/each}
          </div>
        {/if}
      </div>
    {/each}
  {/if}
{/snippet}

<!-- Detail (right) column toolbar: Export flush right. -->
{#snippet detailToolbar()}
  <div class="vmv-detail-actions">
    <Button variant="data" icon="export" onclick={exportVehicle}>{$t('vehicleMgr.export')}</Button>
  </div>
{/snippet}

<!-- Detail (right) column field: image header · build sheet · stats · linked flights. -->
{#snippet detail()}
  {#if selected}
    {@const v = selected}
    <div class="vmv-detail">
      <!-- Image frame on top (like the WP mission preview header). -->
      <div class="vmv-image-frame">
        {#if v.image}
          <img class="vmv-image" src={v.image} alt={v.name} />
        {:else}
          <div class="vmv-image-placeholder"><span>🛩️</span></div>
        {/if}
      </div>

      <div class="veh-title">{v.name}{#if v.status !== 'active'}<span class="veh-badge">{statusLabel(v.status)}</span>{/if}</div>

      <div class="fc-info-grid">
        <span class="fc-label">{$t('vehicleMgr.type')}</span><span class="fc-value">{typeLabel(v.vehicle_type)}</span>
        <span class="fc-label">{$t('vehicleMgr.craftName')}</span><span class="fc-value">{v.craft_name || '—'}</span>
        {#if v.model}<span class="fc-label">{$t('vehicleMgr.model')}</span><span class="fc-value">{v.model}</span>{/if}
        <span class="fc-label">{$t('vehicleMgr.status')}</span><span class="fc-value">{statusLabel(v.status)}</span>
      </div>

      {#if any(v.wingspan_mm, v.length_mm, v.weight_auw_g, v.weight_dry_g)}
        <div class="section-heading">{$t('vehicleMgr.airframe')}</div>
        <div class="fc-info-grid">
          {#if v.wingspan_mm != null}<span class="fc-label">{$t('vehicleMgr.wingspan')}</span><span class="fc-value">{v.wingspan_mm} mm</span>{/if}
          {#if v.length_mm != null}<span class="fc-label">{$t('vehicleMgr.length')}</span><span class="fc-value">{v.length_mm} mm</span>{/if}
          {#if v.weight_auw_g != null}<span class="fc-label">{$t('vehicleMgr.weightAuw')}</span><span class="fc-value">{v.weight_auw_g} g</span>{/if}
          {#if v.weight_dry_g != null}<span class="fc-label">{$t('vehicleMgr.weightDry')}</span><span class="fc-value">{v.weight_dry_g} g</span>{/if}
        </div>
      {/if}

      {#if any(v.motors, v.props, v.esc, v.recommended_cells, v.recommended_capacity_mah)}
        <div class="section-heading">{$t('vehicleMgr.propulsion')}</div>
        <div class="fc-info-grid">
          {#if v.motors}<span class="fc-label">{$t('vehicleMgr.motors')}</span><span class="fc-value">{v.motors}</span>{/if}
          {#if v.props}<span class="fc-label">{$t('vehicleMgr.props')}</span><span class="fc-value">{v.props}</span>{/if}
          {#if v.esc}<span class="fc-label">{$t('vehicleMgr.esc')}</span><span class="fc-value">{v.esc}</span>{/if}
          {#if v.recommended_cells}<span class="fc-label">{$t('vehicleMgr.recommendedCells')}</span><span class="fc-value">{v.recommended_cells}</span>{/if}
          {#if v.recommended_capacity_mah != null}<span class="fc-label">{$t('vehicleMgr.recommendedCapacity')}</span><span class="fc-value">{v.recommended_capacity_mah} mAh</span>{/if}
        </div>
      {/if}

      {#if any(v.rx, v.vtx, v.camera, v.gimbal_camera, v.datalink)}
        <div class="section-heading">{$t('vehicleMgr.radioFpv')}</div>
        <div class="fc-info-grid">
          {#if v.rx}<span class="fc-label">{$t('vehicleMgr.rx')}</span><span class="fc-value">{v.rx}</span>{/if}
          {#if v.vtx}<span class="fc-label">{$t('vehicleMgr.vtx')}</span><span class="fc-value">{v.vtx}</span>{/if}
          {#if v.camera}<span class="fc-label">{$t('vehicleMgr.camera')}</span><span class="fc-value">{v.camera}</span>{/if}
          {#if v.gimbal_camera}<span class="fc-label">{$t('vehicleMgr.gimbalCamera')}</span><span class="fc-value">{v.gimbal_camera}</span>{/if}
          {#if v.datalink}<span class="fc-label">{$t('vehicleMgr.datalink')}</span><span class="fc-value">{v.datalink}</span>{/if}
        </div>
      {/if}

      {#if any(v.sensor_airspeed, v.sensor_rangefinder, v.sensor_optical_flow, v.sensor_gps, v.sensor_rtk, v.sensor_compass)}
        <div class="section-heading">{$t('vehicleMgr.sensors')}</div>
        <div class="veh-sensor-tags">
          {#if v.sensor_gps}<span class="veh-tag">{$t('vehicleMgr.sensorGps')}</span>{/if}
          {#if v.sensor_rtk}<span class="veh-tag">{$t('vehicleMgr.sensorRtk')}</span>{/if}
          {#if v.sensor_compass}<span class="veh-tag">{$t('vehicleMgr.sensorCompass')}</span>{/if}
          {#if v.sensor_airspeed}<span class="veh-tag">{$t('vehicleMgr.sensorAirspeed')}</span>{/if}
          {#if v.sensor_rangefinder}<span class="veh-tag">{$t('vehicleMgr.sensorRangefinder')}</span>{/if}
          {#if v.sensor_optical_flow}<span class="veh-tag">{$t('vehicleMgr.sensorOpticalFlow')}</span>{/if}
        </div>
      {/if}

      {#if any(v.fc_model, v.fc_manufacturer, v.fc_firmware, v.fc_firmware_version, v.blackbox_available)}
        <div class="section-heading">{$t('vehicleMgr.flightController')}</div>
        <div class="fc-info-grid">
          {#if v.fc_model}<span class="fc-label">{$t('vehicleMgr.fcModel')}</span><span class="fc-value">{v.fc_model}</span>{/if}
          {#if v.fc_manufacturer}<span class="fc-label">{$t('vehicleMgr.fcManufacturer')}</span><span class="fc-value">{v.fc_manufacturer}</span>{/if}
          {#if v.fc_firmware}<span class="fc-label">{$t('vehicleMgr.fcFirmware')}</span><span class="fc-value">{v.fc_firmware}{v.fc_firmware_version ? ` ${v.fc_firmware_version}` : ''}</span>{/if}
          <span class="fc-label">{$t('vehicleMgr.blackbox')}</span><span class="fc-value">{v.blackbox_available ? $t('vehicleMgr.yes') : $t('vehicleMgr.no')}</span>
        </div>
      {/if}

      {#if v.notes}
        <div class="section-heading">{$t('vehicleMgr.notes')}</div>
        <div class="vmv-notes-field">{v.notes}</div>
      {/if}

      <!-- Stats: lifetime totals (FC baseline + logged flights) + per-flight records (flight-derived). -->
      {#if lifetime}
        <div class="section-heading">{$t('vehicleMgr.stats')}{#if lifetime.hasBaseline}<span class="veh-baseline-tag" title={$t('vehicleMgr.baselineTip')}>{$t('vehicleMgr.baselineTag')}</span>{/if}</div>
        <div class="fc-info-grid">
          <span class="fc-label">{$t('vehicleMgr.flights')}</span><span class="fc-value">{lifetime.flights}</span>
          <span class="fc-label">{$t('vehicleMgr.totalTime')}</span><span class="fc-value">{formatDurationSec(lifetime.timeS)}</span>
          <span class="fc-label">{$t('vehicleMgr.totalDistance')}</span><span class="fc-value">{fmtDist(lifetime.distM)}</span>
          {#if lifetime.energy > 0}<span class="fc-label">{$t('vehicleMgr.totalEnergy')}</span><span class="fc-value">{(lifetime.energy / 1000).toFixed(1)} Wh</span>{/if}
          <span class="fc-label">{$t('vehicleMgr.firstUsed')}</span><span class="fc-value">{fmtDateTime(lifetime.firstUsed)}</span>
          <span class="fc-label">{$t('vehicleMgr.lastUsed')}</span><span class="fc-value">{fmtDateTime(lifetime.lastUsed)}</span>
        </div>
        {#if aggregate && aggregate.flight_count > 0}
          <div class="section-heading">{$t('vehicleMgr.records')}</div>
          <div class="veh-records">
            {@render recordRow($t('vehicleMgr.maxFlightTime'), aggregate.max_flight_time_sec != null ? formatDurationSec(aggregate.max_flight_time_sec) : '—', aggregate.max_flight_time_flight_id)}
            {@render recordRow($t('vehicleMgr.maxDistance'), fmtDist(aggregate.max_distance_m), aggregate.max_distance_flight_id)}
            {@render recordRow($t('vehicleMgr.maxAltitude'), fmtAlt(aggregate.max_altitude_m), aggregate.max_altitude_flight_id)}
          </div>
        {/if}
      {/if}

      <div class="det-flights">
        <div class="vmv-veh-actions">
          <Button variant="standard" icon="edit" onclick={startEdit}>{$t('vehicleMgr.edit')}</Button>
          <Button variant="danger" icon="delete" onclick={deleteVehicle}>{$t('vehicleMgr.delete')}</Button>
        </div>
        <div class="section-heading">{$t('vehicleMgr.linkedFlights')} ({linkedFlights.length})</div>
        {#each linkedFlights as f (f.id)}
          <button class="flight-row" onclick={() => openFlight(f.id)} title={$t('vehicleMgr.openFlight')}>
            <span class="flight-name">{f.craft_name || $t('vehicleMgr.unnamed')}</span>
            <span class="flight-meta">{fmtDateTime(f.start_time)} · {formatDurationSec(f.duration_sec)}</span>
          </button>
        {/each}
        {#if linkedFlights.length === 0}
          <div class="flight-none">{v.craft_name ? $t('vehicleMgr.noFlights') : $t('vehicleMgr.noCraftLink')}</div>
        {/if}
      </div>
    </div>
  {/if}
{/snippet}

<div class="vmv">
  <PanelShell {variant} title={$t('vehicleMgr.toVehicles')} {toolbar} {body} {detailToolbar} {detail} />
</div>

<!-- Create / edit modal. -->
{#if editing}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="modal-backdrop" onclick={cancelEdit}>
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="modal-card" onclick={(e) => e.stopPropagation()}>
      {@render editForm()}
    </div>
  </div>
{/if}

<!-- Import preview modal. -->
{#if importFile}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="modal-backdrop" onclick={cancelImport}>
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="modal-card" onclick={(e) => e.stopPropagation()}>
      <div class="section-heading">{$t('vehicleMgr.importTitle')}</div>
      <div class="vmv-image-frame vmv-image-frame-sm">
        {#if importFile.vehicle.image}
          <img class="vmv-image" src={importFile.vehicle.image} alt={importFile.vehicle.name} />
        {:else}
          <div class="vmv-image-placeholder"><span>🛩️</span></div>
        {/if}
      </div>
      <div class="fc-info-grid">
        <span class="fc-label">{$t('vehicleMgr.name')}</span><span class="fc-value">{importFile.vehicle.name}</span>
        <span class="fc-label">{$t('vehicleMgr.type')}</span><span class="fc-value">{typeLabel(importFile.vehicle.vehicle_type)}</span>
        <span class="fc-label">{$t('vehicleMgr.craftName')}</span><span class="fc-value">{importFile.vehicle.craft_name || '—'}</span>
        {#if importFile.vehicle.model}<span class="fc-label">{$t('vehicleMgr.model')}</span><span class="fc-value">{importFile.vehicle.model}</span>{/if}
      </div>
      <div class="form-actions">
        <Button variant="data" icon="import" disabled={importBusy || !importFile.vehicle.name.trim()} onclick={applyImport}>{$t('vehicleMgr.importDo')}</Button>
        <Button variant="standard" onclick={cancelImport}>{$t('vehicleMgr.cancel')}</Button>
      </div>
    </div>
  </div>
{/if}

{#if statusMessage}<div class="veh-status">{statusMessage}</div>{/if}

<ConfirmDialog bind:this={confirmDialog} />

<!-- ── Snippets ────────────────────────────────────────────────────── -->
{#snippet recordRow(label: string, value: string, flightId: number | null)}
  <div class="veh-record">
    <span class="fc-label">{label}</span>
    <span class="fc-value">{value}</span>
    {#if flightId != null}
      <button class="veh-record-link" onclick={() => openFlight(flightId)} title={$t('vehicleMgr.openFlight')}>↗</button>
    {/if}
  </div>
{/snippet}

{#snippet vehicleRow(v: Vehicle)}
  <button class="lib-item" class:selected={v.id === $vehicleManagerSelectedId} onclick={() => select(v)}>
    {#if v.image}
      <img class="lib-thumb" src={v.image} alt="" />
    {:else}
      <div class="lib-thumb lib-thumb-empty">🛩️</div>
    {/if}
    <div class="lib-item-text">
      <div class="lib-item-title">{v.name}</div>
      <div class="lib-item-meta">
        <span>{typeLabel(v.vehicle_type)}</span>
        {#if v.craft_name}<span class="veh-craft">{v.craft_name}</span>{/if}
        {#if v.status !== 'active'}<span class="veh-badge-sm">{statusLabel(v.status)}</span>{/if}
      </div>
    </div>
  </button>
{/snippet}

{#snippet editForm()}
  <div class="section-heading">{isCreate ? $t('vehicleMgr.newTitle') : $t('vehicleMgr.editTitle')}</div>

  <!-- Image picker + preview. -->
  <div class="vmv-image-edit">
    <div class="vmv-image-frame vmv-image-frame-sm">
      {#if form.image}
        <img class="vmv-image" src={form.image} alt="" />
      {:else}
        <div class="vmv-image-placeholder"><span>🛩️</span></div>
      {/if}
    </div>
    <div class="vmv-image-btns">
      <label class="vmv-image-pick">
        {$t('vehicleMgr.pickImage')}
        <input type="file" accept="image/*" onchange={onPickImage} />
      </label>
      {#if form.image}
        <Button variant="standard" onclick={() => (form.image = null)}>{$t('vehicleMgr.removeImage')}</Button>
      {/if}
    </div>
  </div>

  <div class="form-grid">
    <label class="fld"><span class="fld-label">{$t('vehicleMgr.name')} *</span>
      <input class="fld-input" type="text" bind:value={form.name} />
    </label>
    <div class="fld vmv-craft-fld">
      <span class="fld-label">{$t('vehicleMgr.craftName')}</span>
      <div class="vmv-craft-row">
        <input class="fld-input" type="text" bind:value={form.craftName} placeholder={$t('vehicleMgr.craftNameHint')} />
        {#if canWriteToFc}
          <Button variant="standard" icon="save" disabled={fcWriteBusy} onclick={writeCraftToFc} title={$t('vehicleMgr.writeToFcTip')}>{$t('vehicleMgr.writeToFc')}</Button>
        {/if}
      </div>
    </div>
    <label class="fld"><span class="fld-label">{$t('vehicleMgr.type')}</span>
      <select class="fld-input" bind:value={form.vehicleType}>
        {#each VEHICLE_TYPES as ty}<option value={ty}>{$t(`vehicleMgr.typeVal.${ty}`)}</option>{/each}
      </select>
    </label>
    <label class="fld"><span class="fld-label">{$t('vehicleMgr.status')}</span>
      <select class="fld-input" bind:value={form.status}>
        {#each VEHICLE_STATUSES as s}<option value={s}>{$t(`vehicleMgr.statusVal.${s}`)}</option>{/each}
      </select>
    </label>
    <label class="fld"><span class="fld-label">{$t('vehicleMgr.model')}</span>
      <input class="fld-input" type="text" bind:value={form.model} />
    </label>
  </div>

  <div class="section-heading">{$t('vehicleMgr.airframe')}</div>
  <div class="form-grid">
    <div class="fld"><span class="fld-label">{$t('vehicleMgr.wingspan')} (mm)</span>
      <NumberStepper bind:value={form.wingspanMm} min={0} max={20000} step={10} decimals={0} allowEmpty placeholder="—" />
    </div>
    <div class="fld"><span class="fld-label">{$t('vehicleMgr.length')} (mm)</span>
      <NumberStepper bind:value={form.lengthMm} min={0} max={20000} step={10} decimals={0} allowEmpty placeholder="—" />
    </div>
    <div class="fld"><span class="fld-label">{$t('vehicleMgr.weightAuw')} (g)</span>
      <NumberStepper bind:value={form.weightAuwG} min={0} max={500000} step={10} decimals={0} allowEmpty placeholder="—" />
    </div>
    <div class="fld"><span class="fld-label">{$t('vehicleMgr.weightDry')} (g)</span>
      <NumberStepper bind:value={form.weightDryG} min={0} max={500000} step={10} decimals={0} allowEmpty placeholder="—" />
    </div>
  </div>

  <div class="section-heading">{$t('vehicleMgr.propulsion')}</div>
  <div class="form-grid">
    <label class="fld"><span class="fld-label">{$t('vehicleMgr.motors')}</span>
      <input class="fld-input" type="text" bind:value={form.motors} />
    </label>
    <label class="fld"><span class="fld-label">{$t('vehicleMgr.props')}</span>
      <input class="fld-input" type="text" bind:value={form.props} />
    </label>
    <label class="fld"><span class="fld-label">{$t('vehicleMgr.esc')}</span>
      <input class="fld-input" type="text" bind:value={form.esc} />
    </label>
    <label class="fld"><span class="fld-label">{$t('vehicleMgr.recommendedCells')}</span>
      <input class="fld-input" type="text" bind:value={form.recommendedCells} placeholder="4S–6S" />
    </label>
    <div class="fld"><span class="fld-label">{$t('vehicleMgr.recommendedCapacity')} (mAh)</span>
      <NumberStepper bind:value={form.recommendedCapacityMah} min={0} max={100000} step={100} decimals={0} allowEmpty placeholder="—" />
    </div>
  </div>

  <div class="section-heading">{$t('vehicleMgr.radioFpv')}</div>
  <div class="form-grid">
    <label class="fld"><span class="fld-label">{$t('vehicleMgr.rx')}</span>
      <input class="fld-input" type="text" bind:value={form.rx} />
    </label>
    <label class="fld"><span class="fld-label">{$t('vehicleMgr.vtx')}</span>
      <input class="fld-input" type="text" bind:value={form.vtx} />
    </label>
    <label class="fld"><span class="fld-label">{$t('vehicleMgr.camera')}</span>
      <input class="fld-input" type="text" bind:value={form.camera} />
    </label>
    <label class="fld"><span class="fld-label">{$t('vehicleMgr.gimbalCamera')}</span>
      <input class="fld-input" type="text" bind:value={form.gimbalCamera} />
    </label>
    <label class="fld"><span class="fld-label">{$t('vehicleMgr.datalink')}</span>
      <input class="fld-input" type="text" bind:value={form.datalink} placeholder={$t('vehicleMgr.datalinkHint')} />
    </label>
  </div>

  <div class="section-heading">{$t('vehicleMgr.sensors')}</div>
  <div class="vmv-sensor-grid">
    <label class="vmv-sensor"><Toggle bind:checked={form.sensorGps} /><span>{$t('vehicleMgr.sensorGps')}</span></label>
    <label class="vmv-sensor"><Toggle bind:checked={form.sensorRtk} /><span>{$t('vehicleMgr.sensorRtk')}</span></label>
    <label class="vmv-sensor"><Toggle bind:checked={form.sensorCompass} /><span>{$t('vehicleMgr.sensorCompass')}</span></label>
    <label class="vmv-sensor"><Toggle bind:checked={form.sensorAirspeed} /><span>{$t('vehicleMgr.sensorAirspeed')}</span></label>
    <label class="vmv-sensor"><Toggle bind:checked={form.sensorRangefinder} /><span>{$t('vehicleMgr.sensorRangefinder')}</span></label>
    <label class="vmv-sensor"><Toggle bind:checked={form.sensorOpticalFlow} /><span>{$t('vehicleMgr.sensorOpticalFlow')}</span></label>
  </div>

  <div class="section-heading">{$t('vehicleMgr.flightController')}</div>
  <div class="form-grid">
    <label class="fld"><span class="fld-label">{$t('vehicleMgr.fcModel')}</span>
      <input class="fld-input" type="text" bind:value={form.fcModel} />
    </label>
    <label class="fld"><span class="fld-label">{$t('vehicleMgr.fcManufacturer')}</span>
      <input class="fld-input" type="text" bind:value={form.fcManufacturer} />
    </label>
    <label class="fld"><span class="fld-label">{$t('vehicleMgr.fcFirmware')}</span>
      <select class="fld-input" bind:value={form.fcFirmware}>
        <option value="">—</option>
        {#each FIRMWARES as fw}<option value={fw}>{fw}</option>{/each}
      </select>
    </label>
    <label class="fld"><span class="fld-label">{$t('vehicleMgr.fcFirmwareVersion')}</span>
      <input class="fld-input" type="text" bind:value={form.fcFirmwareVersion} />
    </label>
    <label class="vmv-sensor vmv-sensor-fc"><Toggle bind:checked={form.blackboxAvailable} /><span>{$t('vehicleMgr.blackbox')}</span></label>
  </div>

  <!-- Lifetime baseline: on request, adopt the connected INAV FC's `stats` totals (INAV + disarmed). -->
  {#if canWriteToFc || formBaseline}
    <div class="section-heading">{$t('vehicleMgr.baseline')}</div>
    <div class="vmv-baseline-row">
      {#if canWriteToFc}
        <Button variant="standard" icon="download" disabled={fcStatsBusy} onclick={adoptFcStats}>{$t('vehicleMgr.adoptFcStats')}</Button>
      {/if}
      {#if formBaseline}
        <span class="vmv-baseline-info">{$t('vehicleMgr.baselineStaged', { values: { count: String(formBaseline.flight_count), time: formatDurationSec(formBaseline.total_time_s), dist: fmtDist(formBaseline.total_dist_m) } })}</span>
        <button type="button" class="vmv-baseline-clear" onclick={() => (formBaseline = null)} title={$t('vehicleMgr.baselineClear')}>✕</button>
      {/if}
    </div>
    <div class="vmv-baseline-hint">{$t('vehicleMgr.baselineHint')}</div>
  {/if}

  <label class="fld"><span class="fld-label">{$t('vehicleMgr.notes')}</span>
    <textarea class="fld-input fld-area" rows="2" bind:value={form.notes}
      oninput={(e) => autoResize(e.target as HTMLTextAreaElement)} use:notesAutoSize={form.notes ?? ''}></textarea>
  </label>

  <div class="form-actions">
    <Button variant="data" icon="save" onclick={saveForm}>{$t('vehicleMgr.save')}</Button>
    <Button variant="standard" onclick={cancelEdit}>{$t('vehicleMgr.cancel')}</Button>
  </div>
{/snippet}

<style>
  .vmv-toolstack { display: flex; flex-direction: column; gap: 6px; width: 100%; }
  .vmv-toolbtns { display: flex; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: 6px; }
  .tb-left { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .tb-right { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; justify-content: flex-end; }
  .vmv-detail-actions { display: flex; flex: 1; justify-content: flex-end; gap: 6px; flex-wrap: wrap; }

  .setting-row { display: flex; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 6px; }
  .setting-input { height: 28px; padding: 0 8px; background: #434343; border: 1px solid #555; border-radius: 4px; color: #e0e0e0; font-size: 12px; }
  .vmv-search { flex: 1; min-width: 0; }
  .vmv-search-clear { background: none; border: none; color: #777; cursor: pointer; font-size: 13px; padding: 2px 4px; line-height: 1; flex-shrink: 0; }
  .vmv-search-clear:hover { color: #e0e0e0; }

  .section-heading { margin: 8px 0 6px 0; font-size: 11px; font-weight: 600; color: #37a8db; text-transform: uppercase; letter-spacing: 0.5px; }

  .panel-empty { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 8px; padding: 40px 0; color: #555; font-size: 12px; }
  .panel-empty-icon { font-size: 28px; opacity: 0.4; }

  .tree-node { margin-bottom: 4px; }
  .tree-toggle { width: 100%; text-align: left; border: 1px solid #555; border-radius: 4px; background: #353535; color: #ddd; cursor: pointer; display: grid; grid-template-columns: 14px minmax(0, 1fr) auto; align-items: center; gap: 6px; padding: 5px 7px; font-size: 12px; font-weight: 600; }
  .tree-toggle:hover { border-color: #37a8db; }
  .tree-special > .tree-toggle { background: #2a2a2a; color: #b0b0b0; border-style: dashed; }
  .tree-caret { color: #9cc6d9; font-size: 11px; line-height: 1; }
  .tree-label { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tree-count { font-size: 10px; color: #8fb4c5; background: rgba(55, 168, 219, 0.12); border: 1px solid rgba(55, 168, 219, 0.32); border-radius: 999px; padding: 1px 6px; }
  .tree-items { margin-top: 4px; margin-left: 12px; }

  .lib-item { width: calc(100% - 4px); text-align: left; border: 1px solid #555; border-radius: 4px; background: #383838; color: #ddd; margin-bottom: 4px; padding: 6px; cursor: pointer; display: flex; gap: 8px; align-items: center; }
  .lib-item:hover { border-color: #37a8db; }
  .lib-item.selected { border-color: #37a8db; background: rgba(55, 168, 219, 0.18); }
  .lib-thumb { width: 40px; height: 40px; flex-shrink: 0; object-fit: cover; border-radius: 4px; background: #1f1f1f; }
  .lib-thumb-empty { display: flex; align-items: center; justify-content: center; font-size: 18px; opacity: 0.4; }
  .lib-item-text { min-width: 0; flex: 1; }
  .lib-item-title { font-size: 12px; color: #fff; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .lib-item-meta { margin-top: 2px; display: flex; flex-wrap: wrap; gap: 4px 10px; font-size: 10px; color: #aaa; align-items: center; }
  .veh-craft { color: #8fb4c5; }
  .veh-badge-sm { font-size: 9px; color: #e0b050; border: 1px solid rgba(224, 176, 80, 0.4); border-radius: 999px; padding: 0 5px; text-transform: uppercase; }

  .vmv-detail { display: flex; flex-direction: column; gap: 6px; }
  .veh-title { font-size: 14px; font-weight: 600; color: #fff; margin-top: 2px; display: flex; align-items: center; gap: 8px; }
  .veh-badge { font-size: 10px; color: #e0b050; border: 1px solid rgba(224, 176, 80, 0.4); border-radius: 999px; padding: 1px 7px; text-transform: uppercase; }

  /* Image frame on top (like the WP mission preview header). */
  .vmv-image-frame { width: 100%; aspect-ratio: 16 / 9; background: #1f1f1f; border: 1px solid #444; border-radius: 6px; overflow: hidden; display: flex; align-items: center; justify-content: center; }
  .vmv-image-frame-sm { aspect-ratio: 16 / 9; max-height: 160px; }
  .vmv-image { width: 100%; height: 100%; object-fit: cover; }
  .vmv-image-placeholder { font-size: 40px; opacity: 0.25; }

  .vmv-image-edit { display: flex; gap: 10px; align-items: center; margin-bottom: 8px; }
  .vmv-image-edit .vmv-image-frame { width: 160px; flex-shrink: 0; }
  .vmv-image-btns { display: flex; flex-direction: column; gap: 6px; }
  .vmv-image-pick { display: inline-flex; align-items: center; justify-content: center; height: 28px; padding: 0 10px; font-size: 12px; color: #e0e0e0; background: #434343; border: 1px solid #555; border-radius: 4px; cursor: pointer; }
  .vmv-image-pick:hover { border-color: #37a8db; }
  .vmv-image-pick input { display: none; }

  .vmv-notes-field {
    width: 100%; box-sizing: border-box; padding: 8px 10px; font-size: 12px; color: #d8d8d8;
    background: rgba(55, 168, 219, 0.06); border: 1px solid rgba(55, 168, 219, 0.25);
    border-radius: 4px; white-space: pre-wrap; overflow-wrap: anywhere; line-height: 1.45;
  }

  .vmv-veh-actions { display: flex; justify-content: space-between; gap: 6px; flex-wrap: wrap; margin-bottom: 8px; }

  .fc-info-grid { display: grid; grid-template-columns: auto minmax(0, 1fr); gap: 6px 10px; font-size: 12px; }
  .fc-label { color: #949494; }
  .fc-value { color: #e0e0e0; font-weight: 600; overflow-wrap: anywhere; }

  .veh-sensor-tags { display: flex; flex-wrap: wrap; gap: 5px; }
  .veh-tag { font-size: 11px; color: #9cd9b0; background: rgba(89, 170, 41, 0.12); border: 1px solid rgba(89, 170, 41, 0.4); border-radius: 999px; padding: 1px 8px; }

  .veh-baseline-tag { margin-left: 8px; font-size: 9px; color: #9cc6d9; background: rgba(55, 168, 219, 0.12); border: 1px solid rgba(55, 168, 219, 0.32); border-radius: 999px; padding: 1px 6px; text-transform: none; letter-spacing: 0; }

  .vmv-baseline-row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .vmv-baseline-info { font-size: 12px; color: #9cd9b0; }
  .vmv-baseline-clear { background: none; border: none; color: #777; cursor: pointer; font-size: 13px; padding: 2px 4px; line-height: 1; }
  .vmv-baseline-clear:hover { color: #e0e0e0; }
  .vmv-baseline-hint { margin-top: 4px; font-size: 11px; color: #949494; }

  .veh-records { display: flex; flex-direction: column; gap: 4px; }
  .veh-record { display: grid; grid-template-columns: auto minmax(0, 1fr) auto; gap: 6px 10px; align-items: center; font-size: 12px; }
  .veh-record-link { background: none; border: none; color: #37a8db; cursor: pointer; font-size: 13px; padding: 0 4px; }
  .veh-record-link:hover { color: #6fc4e8; }

  .det-flights { border-top: 1px solid #333; padding-top: 8px; }
  .flight-row { width: 100%; box-sizing: border-box; display: flex; justify-content: space-between; align-items: center; gap: 8px; font-size: 12px; color: #e0e0e0; padding: 4px 6px; background: none; border: none; border-radius: 4px; cursor: pointer; text-align: left; }
  .flight-row:hover { background: rgba(55, 168, 219, 0.15); }
  .flight-name { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .flight-meta { color: #888; flex-shrink: 0; }
  .flight-none { color: #777; font-size: 12px; padding: 4px 0; }

  .form-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 6px 10px; }
  .fld { display: block; }
  .fld-label { display: block; font-size: 11px; font-weight: 600; color: #949494; text-transform: uppercase; letter-spacing: 0.04em; margin-bottom: 3px; }
  .fld-input { box-sizing: border-box; width: 100%; padding: 5px 7px; font-size: 12px; color: #e0e0e0; background: #1f1f1f; border: 1px solid #444; border-radius: 4px; font-family: 'Segoe UI', Tahoma, sans-serif; }
  .fld-input:focus { outline: none; border-color: #37a8db; }
  .fld-area { resize: vertical; }
  .form-actions { display: flex; gap: 6px; margin-top: 8px; }

  .vmv-craft-fld { grid-column: 1 / -1; }
  .vmv-craft-row { display: flex; gap: 6px; align-items: stretch; }
  .vmv-craft-row .fld-input { flex: 1; min-width: 0; }

  .vmv-sensor-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 6px 10px; }
  .vmv-sensor { display: flex; align-items: center; gap: 8px; font-size: 12px; color: #e0e0e0; }
  .vmv-sensor-fc { grid-column: 1 / -1; }

  .modal-backdrop { position: fixed; inset: 0; background: rgba(0, 0, 0, 0.55); display: flex; align-items: center; justify-content: center; z-index: 1000; }
  .modal-card { box-sizing: border-box; background: #2e2e2e; border: 1px solid rgba(55, 168, 219, 0.35); border-radius: 8px; padding: 14px; width: min(600px, 94vw); max-height: 90vh; overflow-y: auto; box-shadow: 0 8px 30px rgba(0, 0, 0, 0.5); }

  .veh-status { position: fixed; bottom: 14px; left: 50%; transform: translateX(-50%); z-index: 1001; padding: 6px 12px; font-size: 11px; color: #f39c12; background: rgba(0, 0, 0, 0.8); border-radius: 6px; }

  @media (max-width: 760px) {
    .form-grid, .vmv-sensor-grid { grid-template-columns: 1fr; }
  }
</style>
