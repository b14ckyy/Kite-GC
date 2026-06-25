<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- BatteryManager.svelte
     Battery library on the panel framework (docs/active/PANEL_FRAMEWORK.md) — its own PanelShell
     (compact ↔ advanced, true 1:2 split): grouped pack list in the 380px main field, pack detail
     in the 500px detail field. Opened from the Flight Logbook's "Batteries" toggle.
     See docs/active/BATTERY_MANAGEMENT.md.
-->
<script lang="ts">
  import { t } from 'svelte-i18n';
  import { get } from 'svelte/store';
  import { save, open as openFileDialog } from '@tauri-apps/plugin-dialog';
  import { settings } from '$lib/stores/settings';
  import {
    batteryDbList, batteryDbCreate, batteryDbUpdate, batteryDbDelete,
    batteryDbFindBySerial, batteryDbAddUsage, batteryDbAggregate, batteryDbFlights,
    batteryDbSetBaseline, batteryFileWrite, batteryFileRead,
    formatDurationSec,
  } from '$lib/stores/flightlog';
  import type { BatteryPack, BatteryPackInput, BatteryAggregate, BatteryFile, FlightSummary } from '$lib/stores/flightlogTypes';
  import {
    batteryManagerSelectedId, batteryGroupMode, batteryLeafAsc, batterySearchQuery, batterySortField,
    batteryManagerCreateSerial, normalizeSerial,
  } from '$lib/stores/batteryManager';
  import { requestOpenFlightId } from '$lib/stores/missionManager';
  import PanelShell, { type PanelVariant } from '$lib/components/panel/PanelShell.svelte';
  import Button from '$lib/components/panel/Button.svelte';
  import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
  import NumberStepper from '$lib/components/NumberStepper.svelte';

  let { onBack }: { onBack: () => void } = $props();

  let confirmDialog: ReturnType<typeof ConfirmDialog>;

  let batteries = $state<BatteryPack[]>([]);
  let aggregate = $state<BatteryAggregate | null>(null);
  let linkedFlights = $state<FlightSummary[]>([]);
  // Tracks which groups are OPEN (empty = all collapsed → groups start collapsed on first open,
  // consistent with the Flight Logbook tree).
  let open = $state<Set<string>>(new Set());
  let statusMessage = $state('');
  // Auto-clear the transient status line after 10s — persistent state is shown elsewhere.
  $effect(() => {
    if (!statusMessage) return;
    const id = setTimeout(() => { statusMessage = ''; }, 10000);
    return () => clearTimeout(id);
  });

  // Edit/create form: numeric fields use NaN for "empty" (NumberStepper allowEmpty).
  interface BatteryForm {
    serial: string; label: string; manufacturer: string; model: string;
    chemistry: string; status: string; connector: string; inService: string; notes: string;
    cellCount: number; capacityMah: number; cDischarge: number; cCharge: number;
  }
  let editing = $state(false);
  let isCreate = $state(false);
  let form = $state<BatteryForm>(blankForm());

  // Add-usage form (deltas; NaN = empty).
  let showUsage = $state(false);
  let usage = $state<{ cycles: number; hours: number; mah: number; charges: number }>(
    { cycles: NaN, hours: NaN, mah: NaN, charges: NaN }
  );

  // Import preview (.kbatt). The serial is editable ONLY when the file's serial collided at load.
  let importFile = $state<BatteryFile | null>(null);
  let importSerial = $state('');
  let importSerialEditable = $state(false);
  let importBusy = $state(false);

  const CHEMISTRIES = ['lipo', 'liion', 'life', 'lihv'];
  const STATUSES = ['active', 'storage', 'retired', 'damaged'];
  const CONNECTORS = ['XT30', 'XT60', 'XT90', 'XT60H', 'EC3', 'EC5', 'EC8', 'AS150', 'AS150U', 'MR30', 'MR60', 'MT60', 'Deans (T)', 'Other'];

  // Nominal / empty(min) / full(max) volts per cell, by chemistry. Drives the computed pack
  // nominal voltage, voltage range, and energy (Wh) capacity shown in the detail.
  const CHEM_V: Record<string, { nominal: number; min: number; max: number }> = {
    lipo: { nominal: 3.7, min: 3.2, max: 4.2 },
    liion: { nominal: 3.6, min: 2.5, max: 4.2 },
    life: { nominal: 3.3, min: 2.5, max: 3.65 },
    lihv: { nominal: 3.8, min: 3.2, max: 4.35 },
  };

  function dbPath(): string { return get(settings).flightLogDbPath; }

  let selected = $derived(batteries.find((b) => b.id === $batteryManagerSelectedId) ?? null);

  // Shell variant: list-only when nothing is selected, list + detail (1:2) when a pack is open.
  const variant = $derived<PanelVariant>(selected != null ? 'advanced' : 'compact');

  /** Computed voltage/energy spec for the selected pack (null if chemistry/cells unknown). */
  let voltSpec = $derived.by(() => {
    const b = selected;
    if (!b || !b.chemistry || b.cell_count == null || !CHEM_V[b.chemistry]) return null;
    const v = CHEM_V[b.chemistry];
    const cells = b.cell_count;
    return {
      nominal: v.nominal * cells,
      min: v.min * cells,
      max: v.max * cells,
      energyWh: b.capacity_mah != null ? v.nominal * cells * (b.capacity_mah / 1000) : null,
    };
  });

  function blankForm(): BatteryForm {
    return {
      serial: '', label: '', manufacturer: '', model: '', chemistry: 'lipo', status: 'active',
      connector: '', inService: '', notes: '', cellCount: NaN, capacityMah: NaN,
      cDischarge: NaN, cCharge: NaN,
    };
  }

  function strOrNull(s: string): string | null { const t = s.trim(); return t ? t : null; }
  function numOrNull(n: number): number | null { return Number.isFinite(n) ? Math.round(n) : null; }

  function formToInput(): BatteryPackInput {
    return {
      serial: normalizeSerial(form.serial),
      label: strOrNull(form.label),
      manufacturer: strOrNull(form.manufacturer),
      model: strOrNull(form.model),
      chemistry: form.chemistry || null,
      cell_count: numOrNull(form.cellCount),
      capacity_mah: numOrNull(form.capacityMah),
      c_rating_discharge: numOrNull(form.cDischarge),
      c_rating_charge: numOrNull(form.cCharge),
      connector: strOrNull(form.connector),
      in_service_date: strOrNull(form.inService),
      status: form.status,
      notes: strOrNull(form.notes),
    };
  }

  // Search filter (serial / label / maker / model / notes / connector / chemistry).
  let filtered = $derived.by(() => {
    const q = $batterySearchQuery.trim().toLowerCase();
    if (!q) return batteries;
    return batteries.filter((b) =>
      [b.serial, b.label, b.manufacturer, b.model, b.notes, b.connector, b.chemistry]
        .some((f) => f != null && f.toLowerCase().includes(q))
    );
  });

  // ── Grouping ────────────────────────────────────────────────────────
  interface PackSubGroup { key: string; label: string; packs: BatteryPack[]; }
  // A list section: a 2-level normal group (children), a flat special group (packs + label),
  // or the bare flat-mode active list (packs + bare).
  interface PackSection {
    key: string; label: string; count: number;
    children?: PackSubGroup[];
    packs?: BatteryPack[];
    bare?: boolean;
  }

  function cellKey(b: BatteryPack): string { return b.cell_count != null ? `${b.cell_count}S` : $t('batteryMgr.unknownCells'); }
  function capKey(b: BatteryPack): string { return b.capacity_mah != null ? `${b.capacity_mah} mAh` : $t('batteryMgr.unknownCap'); }
  function numFrom(key: string): number { const n = parseFloat(key); return isNaN(n) ? -1 : n; }

  /** Sort leaf packs by the given field + direction; serial is the tiebreak. */
  function sortPacks(packs: BatteryPack[], field: 'cell' | 'capacity' | 'serial', asc: boolean): BatteryPack[] {
    const dir = asc ? 1 : -1;
    return [...packs].sort((a, b) => {
      if (field === 'cell') {
        const x = a.cell_count ?? -1, y = b.cell_count ?? -1;
        if (x !== y) return (x - y) * dir;
      } else if (field === 'capacity') {
        const x = a.capacity_mah ?? -1, y = b.capacity_mah ?? -1;
        if (x !== y) return (x - y) * dir;
      }
      return a.serial.localeCompare(b.serial, undefined, { numeric: true }) * dir;
    });
  }

  /** Build the list sections. The ▲/▼ button sets the GROUP ordering (both levels); the leaf
   *  packs are ALWAYS serial-ascending in grouped views. Storage and Retired/Damaged packs are
   *  pulled out of the normal grouping into trailing special groups (Storage = second-to-last,
   *  Retired/Damaged = last) in every mode, including flat. */
  let sections = $derived.by<PackSection[]>(() => {
    const mode = $batteryGroupMode;
    const asc = $batteryLeafAsc;
    const active = filtered.filter((b) => b.status === 'active');
    const storage = filtered.filter((b) => b.status === 'storage');
    const archived = filtered.filter((b) => b.status === 'retired' || b.status === 'damaged');
    const out: PackSection[] = [];

    if (mode === 'flat') {
      out.push({ key: '__active', label: '', count: active.length, bare: true, packs: sortPacks(active, $batterySortField, asc) });
    } else {
      const [topFn, subFn] = mode === 'cell-capacity' ? [cellKey, capKey] : [capKey, cellKey];
      const tops = new Map<string, Map<string, BatteryPack[]>>();
      for (const b of active) {
        const tk = topFn(b), sk = subFn(b);
        if (!tops.has(tk)) tops.set(tk, new Map());
        const sub = tops.get(tk)!;
        if (!sub.has(sk)) sub.set(sk, []);
        sub.get(sk)!.push(b);
      }
      const dir = asc ? 1 : -1;
      const byNum = (a: string, b: string) => (numFrom(a) - numFrom(b)) * dir || a.localeCompare(b) * dir;
      for (const [tk, sub] of [...tops.entries()].sort((a, b) => byNum(a[0], b[0]))) {
        const children = [...sub.entries()].sort((a, b) => byNum(a[0], b[0])).map(([sk, packs]) => ({
          key: `${tk}/${sk}`, label: sk, packs: sortPacks(packs, 'serial', true),
        }));
        out.push({ key: tk, label: tk, count: children.reduce((n, c) => n + c.packs.length, 0), children });
      }
    }

    if (storage.length) out.push({ key: '__storage', label: $t('batteryMgr.groupStorage'), count: storage.length, packs: sortPacks(storage, 'serial', true) });
    if (archived.length) out.push({ key: '__archived', label: $t('batteryMgr.groupArchived'), count: archived.length, packs: sortPacks(archived, 'serial', true) });
    return out;
  });

  // ── Loading ─────────────────────────────────────────────────────────
  async function reload() {
    try {
      batteries = await batteryDbList(dbPath());
    } catch (e) {
      statusMessage = $t('batteryMgr.loadFailed', { values: { error: String(e) } });
    }
  }

  async function loadDetails(b: BatteryPack) {
    try {
      aggregate = await batteryDbAggregate(b.serial, dbPath());
      linkedFlights = await batteryDbFlights(b.serial, dbPath());
    } catch {
      aggregate = null;
      linkedFlights = [];
    }
  }

  function select(b: BatteryPack) {
    batteryManagerSelectedId.set(b.id);
    editing = false;
    showUsage = false;
    void loadDetails(b);
  }

  function toggleGroup(key: string) {
    const n = new Set(open);
    if (n.has(key)) n.delete(key); else n.add(key);
    open = n;
  }

  // ── Create / edit ───────────────────────────────────────────────────
  function startCreate() {
    form = blankForm();
    isCreate = true;
    editing = true;
    showUsage = false;
  }

  function startEdit() {
    if (!selected) return;
    const b = selected;
    form = {
      serial: b.serial, label: b.label ?? '', manufacturer: b.manufacturer ?? '', model: b.model ?? '',
      chemistry: b.chemistry ?? 'lipo', status: b.status, connector: b.connector ?? '',
      inService: b.in_service_date ?? '', notes: b.notes ?? '',
      cellCount: b.cell_count ?? NaN, capacityMah: b.capacity_mah ?? NaN,
      cDischarge: b.c_rating_discharge ?? NaN, cCharge: b.c_rating_charge ?? NaN,
    };
    isCreate = false;
    editing = true;
    showUsage = false;
  }

  function cancelEdit() { editing = false; }

  async function saveForm() {
    const input = formToInput();
    if (!input.serial) { statusMessage = $t('batteryMgr.serialRequired'); return; }
    try {
      if (isCreate) {
        const existing = await batteryDbFindBySerial(input.serial, dbPath());
        if (existing) { statusMessage = $t('batteryMgr.serialExists'); return; }
        const id = await batteryDbCreate(input, dbPath());
        await reload();
        batteryManagerSelectedId.set(id);
        const b = batteries.find((x) => x.id === id);
        if (b) await loadDetails(b);
      } else if (selected) {
        await batteryDbUpdate(selected.id, input, dbPath());
        await reload();
      }
      editing = false;
      statusMessage = $t('batteryMgr.saved');
    } catch (e) {
      statusMessage = $t('batteryMgr.saveFailed', { values: { error: String(e) } });
    }
  }

  // Update only the pack's status (Retire / Mark Damaged) — keeps all other fields. Offered as a
  // non-destructive alternative inside the delete dialog (otherwise these states are hard to find).
  async function setStatus(b: BatteryPack, status: string) {
    try {
      await batteryDbUpdate(b.id, {
        serial: b.serial, label: b.label, manufacturer: b.manufacturer, model: b.model,
        chemistry: b.chemistry, cell_count: b.cell_count, capacity_mah: b.capacity_mah,
        c_rating_discharge: b.c_rating_discharge, c_rating_charge: b.c_rating_charge,
        connector: b.connector, in_service_date: b.in_service_date, status, notes: b.notes,
      }, dbPath());
      await reload();
      const updated = batteries.find((x) => x.id === b.id);
      if (updated) await loadDetails(updated);
      statusMessage = $t('batteryMgr.statusUpdated');
    } catch (e) {
      statusMessage = $t('batteryMgr.statusUpdateFailed', { values: { error: String(e) } });
    }
  }

  async function deleteBattery() {
    if (!selected) return;
    const b = selected;
    const count = linkedFlights.length;
    // Delete dialog doubles as the status changer: Retire / Mark Damaged (non-destructive) or Delete.
    const ans = await confirmDialog.show({
      title: $t('batteryMgr.deleteTitle'),
      message: count > 0
        ? $t('batteryMgr.deleteMsgLinked', { values: { count: String(count) } })
        : $t('batteryMgr.deleteMsg'),
      buttons: [
        { label: $t('batteryMgr.deleteRetire'), value: 'retire' },
        { label: $t('batteryMgr.deleteDamaged'), value: 'damaged' },
        { label: $t('batteryMgr.deleteYes'), value: 'delete', danger: true },
      ],
    });
    if (ans === 'retire' || ans === 'damaged') {
      await setStatus(b, ans === 'retire' ? 'retired' : 'damaged');
      return;
    }
    if (ans !== 'delete') return;
    try {
      await batteryDbDelete(b.id, dbPath());
      batteryManagerSelectedId.set(null);
      aggregate = null;
      linkedFlights = [];
      await reload();
      statusMessage = $t('batteryMgr.deleted');
    } catch (e) {
      statusMessage = $t('batteryMgr.deleteFailed', { values: { error: String(e) } });
    }
  }

  // ── Add usage (additive baseline) ───────────────────────────────────
  function startUsage() { usage = { cycles: NaN, hours: NaN, mah: NaN, charges: NaN }; showUsage = true; editing = false; }
  function cancelUsage() { showUsage = false; }
  function num(v: number): number { return Number.isFinite(v) ? v : 0; }

  async function saveUsage() {
    if (!selected) return;
    try {
      await batteryDbAddUsage(
        selected.id,
        Math.round(num(usage.hours) * 3600),
        Math.round(num(usage.mah)),
        num(usage.cycles),
        Math.round(num(usage.charges)),
        dbPath(),
      );
      showUsage = false;
      await reload();
      const b = batteries.find((x) => x.id === selected!.id);
      if (b) await loadDetails(b);
      statusMessage = $t('batteryMgr.usageAdded');
    } catch (e) {
      statusMessage = $t('batteryMgr.saveFailed', { values: { error: String(e) } });
    }
  }

  function openFlight(id: number) { requestOpenFlightId.set(id); }

  // ── Export (.kbatt) ─────────────────────────────────────────────────
  function sanitize(s: string): string { return s.replace(/[^\w\-]+/g, '_').replace(/^_+|_+$/g, ''); }

  async function exportBattery() {
    if (!selected) return;
    const b = selected;
    // Consolidate (fold linked flights into the file's baseline) or base-only export.
    const ans = await confirmDialog.show({
      title: $t('batteryMgr.exportTitle'),
      message: $t('batteryMgr.exportMsg'),
      buttons: [
        { label: $t('batteryMgr.exportConsolidate'), value: 'consolidate', primary: true },
        { label: $t('batteryMgr.exportBase'), value: 'base' },
      ],
    });
    if (ans !== 'consolidate' && ans !== 'base') return;
    const consolidate = ans === 'consolidate';
    const agg = aggregate ?? { flight_count: 0, sum_duration_sec: 0, sum_mah: 0, first_used: null, last_used: null };
    const flightCycles = consolidate && b.capacity_mah && b.capacity_mah > 0 ? agg.sum_mah / b.capacity_mah : 0;

    const file: BatteryFile = {
      format: 'kbatt',
      version: 1,
      exported_at: new Date().toISOString(),
      consolidated: consolidate,
      flight_count: consolidate ? agg.flight_count : 0,
      pack: {
        serial: b.serial, label: b.label, manufacturer: b.manufacturer, model: b.model,
        chemistry: b.chemistry, cell_count: b.cell_count, capacity_mah: b.capacity_mah,
        c_rating_discharge: b.c_rating_discharge, c_rating_charge: b.c_rating_charge,
        connector: b.connector, in_service_date: b.in_service_date, status: b.status, notes: b.notes,
      },
      base_flight_seconds: b.base_flight_seconds + (consolidate ? agg.sum_duration_sec : 0),
      base_mah: b.base_mah + (consolidate ? agg.sum_mah : 0),
      base_cycles: b.base_cycles + flightCycles,
      base_charges: b.base_charges,
    };

    const labelPart = b.label ? `_${sanitize(b.label)}` : '';
    try {
      const path = await save({
        title: $t('batteryMgr.exportTitle'),
        defaultPath: `battery_${sanitize(b.serial)}${labelPart}.kbatt`,
        filters: [{ name: 'Battery', extensions: ['kbatt'] }],
      });
      if (!path) return;
      await batteryFileWrite(path, file);
      statusMessage = $t('batteryMgr.exported');
    } catch (e) {
      statusMessage = $t('batteryMgr.exportFailed', { values: { error: String(e) } });
    }
  }

  // ── Import (.kbatt) ─────────────────────────────────────────────────
  async function handleImport() {
    try {
      const path = await openFileDialog({ title: $t('batteryMgr.importTitle'), multiple: false, filters: [{ name: 'Battery', extensions: ['kbatt'] }] });
      if (!path || typeof path !== 'string') return;
      const file = await batteryFileRead(path);
      importFile = file;
      importSerial = normalizeSerial(file.pack.serial);
      // Serial is editable only if it collides with an existing pack (conflict resolution).
      importSerialEditable = batteries.some((b) => normalizeSerial(b.serial) === importSerial);
      editing = false;
      showUsage = false;
    } catch (e) {
      statusMessage = $t('batteryMgr.importFailed', { values: { error: String(e) } });
    }
  }

  function cancelImport() { importFile = null; }

  /** Does the current import serial collide with an existing pack? (drives the action buttons) */
  let importConflict = $derived(importFile != null && batteries.some((b) => normalizeSerial(b.serial) === importSerial));

  async function applyImport(mode: 'new' | 'consolidate' | 'overwrite') {
    if (!importFile || importBusy) return;
    const f = importFile;
    const serial = normalizeSerial(importSerial);
    if (!serial) { statusMessage = $t('batteryMgr.serialRequired'); return; }
    importBusy = true;
    try {
      let targetId: number;
      if (mode === 'new') {
        const id = await batteryDbCreate({ ...f.pack, serial }, dbPath());
        await batteryDbSetBaseline(id, f.base_flight_seconds, f.base_mah, f.base_cycles, f.base_charges, dbPath());
        targetId = id;
      } else {
        const existing = batteries.find((b) => normalizeSerial(b.serial) === serial);
        if (!existing) { statusMessage = $t('batteryMgr.serialRequired'); importBusy = false; return; }
        if (mode === 'consolidate') {
          await batteryDbAddUsage(existing.id, f.base_flight_seconds, f.base_mah, f.base_cycles, f.base_charges, dbPath());
        } else {
          await batteryDbUpdate(existing.id, { ...f.pack, serial: existing.serial }, dbPath());
          await batteryDbSetBaseline(existing.id, f.base_flight_seconds, f.base_mah, f.base_cycles, f.base_charges, dbPath());
        }
        targetId = existing.id;
      }
      importFile = null;
      await reload();
      batteryManagerSelectedId.set(targetId);
      const b = batteries.find((x) => x.id === targetId);
      if (b) await loadDetails(b);
      statusMessage = $t('batteryMgr.imported');
    } catch (e) {
      statusMessage = $t('batteryMgr.importFailed', { values: { error: String(e) } });
    } finally {
      importBusy = false;
    }
  }

  // ── Lifetime (baseline + Σ linked flights) ──────────────────────────
  let lifetime = $derived.by(() => {
    const b = selected;
    if (!b) return null;
    const agg = aggregate ?? { flight_count: 0, sum_duration_sec: 0, sum_mah: 0, first_used: null, last_used: null };
    const totalSeconds = b.base_flight_seconds + agg.sum_duration_sec;
    const totalMah = b.base_mah + agg.sum_mah;
    const flightCycles = b.capacity_mah && b.capacity_mah > 0 ? agg.sum_mah / b.capacity_mah : 0;
    return {
      flightCount: agg.flight_count,
      totalSeconds,
      totalMah,
      cycles: b.base_cycles + flightCycles,
      charges: b.base_charges,
      firstUsed: agg.first_used,
      lastUsed: agg.last_used,
    };
  });

  function fmtDateTime(value: string | null): string { return value ? new Date(value).toLocaleString() : '—'; }
  function chemLabel(c: string | null): string { return c ? $t(`batteryMgr.chem.${c}`) : '—'; }
  function statusLabel(s: string): string { return $t(`batteryMgr.statusVal.${s}`); }

  // Notes textarea auto-grow (same approach as the logbook / mission manager).
  function autoResize(el: HTMLTextAreaElement) {
    el.style.height = 'auto';
    const extra = el.offsetHeight - el.clientHeight;
    el.style.height = Math.max(44, Math.min(el.scrollHeight + extra, 140)) + 'px';
  }
  function notesAutoSize(el: HTMLTextAreaElement, _v: string) {
    autoResize(el);
    return { update() { autoResize(el); } };
  }

  // External "create new battery with this serial" trigger (End-Flight flow): open straight into the
  // create form with the serial pre-filled, then consume the one-shot signal.
  $effect(() => {
    const s = $batteryManagerCreateSerial;
    if (s == null) return;
    startCreate();
    form.serial = normalizeSerial(s);
    batteryManagerCreateSerial.set(null);
  });

  // Init once: load + restore selection.
  let didInit = false;
  $effect(() => {
    if (didInit) return;
    didInit = true;
    void (async () => {
      await reload();
      const id = get(batteryManagerSelectedId);
      const b = id != null ? batteries.find((x) => x.id === id) : undefined;
      if (b) await loadDetails(b);
      else if (id != null) batteryManagerSelectedId.set(null);
    })();
  });
</script>

<!-- Toolbar (main column): group/sort row · search · button row (Back · New / Import). -->
{#snippet toolbar()}
  <div class="bmv2-toolstack">
    <div class="setting-row">
      <span class="setting-label">{$t('batteryMgr.groupMode')}</span>
      <div class="bmv2-order">
        {#if $batteryGroupMode === 'flat'}
          <select class="setting-select" bind:value={$batterySortField} title={$t('batteryMgr.sortField')}>
            <option value="serial">{$t('batteryMgr.serial')}</option>
            <option value="cell">{$t('batteryMgr.cells')}</option>
            <option value="capacity">{$t('batteryMgr.capacity')}</option>
          </select>
        {/if}
        <select class="setting-select" bind:value={$batteryGroupMode}>
          <option value="cell-capacity">{$t('batteryMgr.groupCellCap')}</option>
          <option value="capacity-cell">{$t('batteryMgr.groupCapCell')}</option>
          <option value="flat">{$t('batteryMgr.groupFlat')}</option>
        </select>
        <Button variant="standard" title={$t('batteryMgr.sortDir')} onclick={() => batteryLeafAsc.update((v) => !v)}>
          {$batteryLeafAsc ? '▲' : '▼'}
        </Button>
      </div>
    </div>

    <div class="setting-row">
      <input
        type="text"
        class="setting-input bmv2-search"
        placeholder={$t('batteryMgr.searchPlaceholder')}
        bind:value={$batterySearchQuery}
      />
      {#if $batterySearchQuery}
        <button class="bmv2-search-clear" onclick={() => batterySearchQuery.set('')} title={$t('logbook.searchClear')}>✕</button>
      {/if}
    </div>

    <div class="bmv2-toolbtns">
      <div class="tb-left">
        <Button variant="standard" icon="battery" onclick={onBack}>← {$t('batteryMgr.back')}</Button>
        <Button variant="standard" icon="add" onclick={startCreate}>{$t('batteryMgr.new')}</Button>
      </div>
      <div class="tb-right">
        <Button variant="data" icon="import" onclick={handleImport}>{$t('batteryMgr.import')}</Button>
      </div>
    </div>
  </div>
{/snippet}

<!-- Main field: the grouped pack list (or empty state). -->
{#snippet body()}
  {#if batteries.length === 0}
    <div class="panel-empty">
      <span class="panel-empty-icon">🔋</span>
      <span>{$t('batteryMgr.empty')}</span>
    </div>
  {:else}
    {#each sections as g (g.key)}
      {#if g.bare}
        <!-- Flat-mode active list: bare rows, no group header. -->
        {#each g.packs ?? [] as b (b.id)}
          {@render packRow(b)}
        {/each}
      {:else if g.children}
        <!-- Normal 2-level group (active packs). -->
        <div class="tree-node" class:tree-special={g.key === '__storage' || g.key === '__archived'}>
          <button class="tree-toggle" onclick={() => toggleGroup(g.key)}>
            <span class="tree-caret">{open.has(g.key) ? '▾' : '▸'}</span>
            <span class="tree-label">{g.label}</span>
            <span class="tree-count">{g.count}</span>
          </button>
          {#if open.has(g.key)}
            <div class="tree-items">
              {#each g.children as sub (sub.key)}
                <div class="tree-node">
                  <button class="tree-toggle tree-toggle-sub" onclick={() => toggleGroup(sub.key)}>
                    <span class="tree-caret">{open.has(sub.key) ? '▾' : '▸'}</span>
                    <span class="tree-label">{sub.label}</span>
                    <span class="tree-count">{sub.packs.length}</span>
                  </button>
                  {#if open.has(sub.key)}
                    <div class="tree-items">
                      {#each sub.packs as b (b.id)}
                        {@render packRow(b)}
                      {/each}
                    </div>
                  {/if}
                </div>
              {/each}
            </div>
          {/if}
        </div>
      {:else}
        <!-- Special flat group (Storage / Retired+Damaged) — single collapsible level. -->
        <div class="tree-node tree-special">
          <button class="tree-toggle" onclick={() => toggleGroup(g.key)}>
            <span class="tree-caret">{open.has(g.key) ? '▾' : '▸'}</span>
            <span class="tree-label">{g.label}</span>
            <span class="tree-count">{g.count}</span>
          </button>
          {#if open.has(g.key)}
            <div class="tree-items">
              {#each g.packs ?? [] as b (b.id)}
                {@render packRow(b)}
              {/each}
            </div>
          {/if}
        </div>
      {/if}
    {/each}
  {/if}
{/snippet}

<!-- Detail (right) column toolbar: Export flush right (mirrors the flight view's export toolbar). -->
{#snippet detailToolbar()}
  <div class="bmv2-detail-actions">
    <Button variant="data" icon="export" onclick={exportBattery}>{$t('batteryMgr.export')}</Button>
  </div>
{/snippet}

<!-- Detail (right) column field: identity · lifetime · linked flights. -->
{#snippet detail()}
  {#if selected}
    {@const b = selected}
    <div class="bmv2-detail">
      <div class="bat-title">{b.label || b.serial}</div>

      <div class="fc-info-grid">
        <span class="fc-label">{$t('batteryMgr.serial')}</span><span class="fc-value">{b.serial}</span>
        <span class="fc-label">{$t('batteryMgr.chemistry')}</span><span class="fc-value">{chemLabel(b.chemistry)}</span>
        <span class="fc-label">{$t('batteryMgr.cells')}</span><span class="fc-value">{b.cell_count != null ? `${b.cell_count}S` : '—'}</span>
        <span class="fc-label">{$t('batteryMgr.capacity')}</span><span class="fc-value">{b.capacity_mah != null ? `${b.capacity_mah} mAh` : '—'}</span>
        <span class="fc-label">{$t('batteryMgr.cRating')}</span><span class="fc-value">{b.c_rating_discharge != null ? `${b.c_rating_discharge}C` : '—'}{b.c_rating_charge != null ? ` / ${b.c_rating_charge}C` : ''}</span>
        {#if voltSpec}
          <span class="fc-label">{$t('batteryMgr.nominalVoltage')}</span><span class="fc-value">{voltSpec.nominal.toFixed(1)} V</span>
          <span class="fc-label">{$t('batteryMgr.voltageRange')}</span><span class="fc-value">{voltSpec.min.toFixed(1)}–{voltSpec.max.toFixed(1)} V</span>
          {#if voltSpec.energyWh != null}<span class="fc-label">{$t('batteryMgr.energy')}</span><span class="fc-value">{voltSpec.energyWh.toFixed(0)} Wh</span>{/if}
        {/if}
        <span class="fc-label">{$t('batteryMgr.manufacturer')}</span><span class="fc-value">{[b.manufacturer, b.model].filter(Boolean).join(' ') || '—'}</span>
        <span class="fc-label">{$t('batteryMgr.connector')}</span><span class="fc-value">{b.connector || '—'}</span>
        <span class="fc-label">{$t('batteryMgr.inService')}</span><span class="fc-value">{b.in_service_date || '—'}</span>
        <span class="fc-label">{$t('batteryMgr.status')}</span><span class="fc-value">{statusLabel(b.status)}</span>
      </div>

      {#if b.notes}
        <div class="section-heading">{$t('batteryMgr.notes')}</div>
        <div class="bmv2-notes-field">{b.notes}</div>
      {/if}

      {#if lifetime}
        <div class="section-heading">{$t('batteryMgr.lifetime')}</div>
        <div class="fc-info-grid">
          <span class="fc-label">{$t('batteryMgr.cycles')}</span><span class="fc-value">{lifetime.cycles.toFixed(1)}</span>
          <span class="fc-label">{$t('batteryMgr.flights')}</span><span class="fc-value">{lifetime.flightCount}</span>
          <span class="fc-label">{$t('batteryMgr.flightTime')}</span><span class="fc-value">{formatDurationSec(lifetime.totalSeconds)}</span>
          <span class="fc-label">{$t('batteryMgr.totalMah')}</span><span class="fc-value">{lifetime.totalMah} mAh</span>
          <span class="fc-label">{$t('batteryMgr.charges')}</span><span class="fc-value">{lifetime.charges}</span>
          <span class="fc-label">{$t('batteryMgr.firstUsed')}</span><span class="fc-value">{fmtDateTime(lifetime.firstUsed)}</span>
          <span class="fc-label">{$t('batteryMgr.lastUsed')}</span><span class="fc-value">{fmtDateTime(lifetime.lastUsed)}</span>
        </div>
      {/if}

      <div class="det-flights">
        <!-- Pack actions above the flight list (spread left→right) so they stay put as the list grows. -->
        <div class="bmv2-pack-actions">
          <Button variant="standard" icon="edit" onclick={startEdit}>{$t('batteryMgr.edit')}</Button>
          <Button variant="standard" icon="add" onclick={startUsage}>{$t('batteryMgr.addUsage')}</Button>
          <Button variant="danger" icon="delete" onclick={deleteBattery}>{$t('batteryMgr.delete')}</Button>
        </div>
        <div class="section-heading">{$t('batteryMgr.linkedFlights')} ({linkedFlights.length})</div>
        {#each linkedFlights as f (f.id)}
          <button class="flight-row" onclick={() => openFlight(f.id)} title={$t('batteryMgr.openFlight')}>
            <span class="flight-name">{f.craft_name || $t('batteryMgr.unnamed')}</span>
            <span class="flight-meta">{fmtDateTime(f.start_time)} · {formatDurationSec(f.duration_sec)}</span>
          </button>
        {/each}
        {#if linkedFlights.length === 0}<div class="flight-none">{$t('batteryMgr.noFlights')}</div>{/if}
      </div>
    </div>
  {/if}
{/snippet}

<div class="bmv2">
  <PanelShell {variant} title={$t('batteryMgr.toBatteries')} {toolbar} {body} {detailToolbar} {detail} />
</div>

<!-- Create / edit modal (works without a selection → used by "New"). -->
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

<!-- Add-usage modal. -->
{#if showUsage}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="modal-backdrop" onclick={cancelUsage}>
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="modal-card" onclick={(e) => e.stopPropagation()}>
      {@render usageForm()}
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
      <div class="section-heading">{$t('batteryMgr.importTitle')}</div>
      {#if importFile.consolidated}
        <div class="usage-hint">{$t('batteryMgr.importConsolidatedInfo', { values: { count: String(importFile.flight_count) } })}</div>
      {:else}
        <div class="usage-hint">{$t('batteryMgr.importBaseInfo')}</div>
      {/if}

      <div class="fld">
        <span class="fld-label">{$t('batteryMgr.serial')}{importSerialEditable ? ' *' : ''}</span>
        <input
          class="fld-input"
          type="text"
          value={importSerial}
          readonly={!importSerialEditable}
          autocapitalize="characters"
          autocomplete="off"
          spellcheck="false"
          oninput={(e) => (importSerial = normalizeSerial(e.currentTarget.value))}
        />
      </div>
      {#if importSerialEditable}
        <div class="usage-hint">
          {importConflict ? $t('batteryMgr.importSerialExists') : $t('batteryMgr.importSerialFree')}
        </div>
      {/if}

      <div class="fc-info-grid">
        <span class="fc-label">{$t('batteryMgr.label')}</span><span class="fc-value">{importFile.pack.label || '—'}</span>
        <span class="fc-label">{$t('batteryMgr.chemistry')}</span><span class="fc-value">{chemLabel(importFile.pack.chemistry)}</span>
        <span class="fc-label">{$t('batteryMgr.cells')}</span><span class="fc-value">{importFile.pack.cell_count != null ? `${importFile.pack.cell_count}S` : '—'}</span>
        <span class="fc-label">{$t('batteryMgr.capacity')}</span><span class="fc-value">{importFile.pack.capacity_mah != null ? `${importFile.pack.capacity_mah} mAh` : '—'}</span>
        <span class="fc-label">{$t('batteryMgr.manufacturer')}</span><span class="fc-value">{[importFile.pack.manufacturer, importFile.pack.model].filter(Boolean).join(' ') || '—'}</span>
        <span class="fc-label">{$t('batteryMgr.connector')}</span><span class="fc-value">{importFile.pack.connector || '—'}</span>
        <span class="fc-label">{$t('batteryMgr.status')}</span><span class="fc-value">{statusLabel(importFile.pack.status)}</span>
        <span class="fc-label">{$t('batteryMgr.cycles')}</span><span class="fc-value">{importFile.base_cycles.toFixed(1)}</span>
        <span class="fc-label">{$t('batteryMgr.flightTime')}</span><span class="fc-value">{formatDurationSec(importFile.base_flight_seconds)}</span>
        <span class="fc-label">{$t('batteryMgr.totalMah')}</span><span class="fc-value">{importFile.base_mah} mAh</span>
        <span class="fc-label">{$t('batteryMgr.charges')}</span><span class="fc-value">{importFile.base_charges}</span>
      </div>

      <div class="form-actions">
        {#if importSerialEditable && importConflict}
          <Button variant="standard" disabled={importBusy} onclick={() => applyImport('consolidate')}>{$t('batteryMgr.importConsolidate')}</Button>
          <Button variant="danger" disabled={importBusy} onclick={() => applyImport('overwrite')}>{$t('batteryMgr.importOverwrite')}</Button>
        {:else}
          <Button variant="data" icon="import" disabled={importBusy || !importSerial.trim()} onclick={() => applyImport('new')}>{$t('batteryMgr.importDo')}</Button>
        {/if}
        <Button variant="standard" onclick={cancelImport}>{$t('batteryMgr.cancel')}</Button>
      </div>
    </div>
  </div>
{/if}

{#if statusMessage}<div class="bat-status">{statusMessage}</div>{/if}

<ConfirmDialog bind:this={confirmDialog} />

<!-- ── Snippets ────────────────────────────────────────────────────── -->
{#snippet packRow(b: BatteryPack)}
  <button class="lib-item" class:selected={b.id === $batteryManagerSelectedId} onclick={() => select(b)}>
    <div class="lib-item-title">{b.serial}{#if b.label}<span class="lib-item-label">{b.label}</span>{/if}</div>
    <div class="lib-item-meta">
      <span>{b.cell_count != null ? `${b.cell_count}S` : '—'}</span>
      <span>{b.capacity_mah != null ? `${b.capacity_mah} mAh` : '—'}</span>
      <span class="bat-chem">{chemLabel(b.chemistry)}</span>
      {#if b.status !== 'active'}<span class="bat-badge">{statusLabel(b.status)}</span>{/if}
    </div>
  </button>
{/snippet}

{#snippet editForm()}
  <div class="section-heading">{isCreate ? $t('batteryMgr.newTitle') : $t('batteryMgr.editTitle')}</div>
  <div class="form-grid">
    <label class="fld"><span class="fld-label">{$t('batteryMgr.serial')} *</span>
      <input
        class="fld-input"
        type="text"
        value={form.serial}
        disabled={!isCreate}
        autocapitalize="characters"
        autocomplete="off"
        spellcheck="false"
        oninput={(e) => (form.serial = normalizeSerial(e.currentTarget.value))}
      />
    </label>
    <label class="fld"><span class="fld-label">{$t('batteryMgr.label')}</span>
      <input class="fld-input" type="text" bind:value={form.label} />
    </label>
    <label class="fld"><span class="fld-label">{$t('batteryMgr.chemistry')}</span>
      <select class="fld-input" bind:value={form.chemistry}>
        {#each CHEMISTRIES as c}<option value={c}>{$t(`batteryMgr.chem.${c}`)}</option>{/each}
      </select>
    </label>
    <label class="fld"><span class="fld-label">{$t('batteryMgr.status')}</span>
      <select class="fld-input" bind:value={form.status}>
        {#each STATUSES as s}<option value={s}>{$t(`batteryMgr.statusVal.${s}`)}</option>{/each}
      </select>
    </label>
    <div class="fld"><span class="fld-label">{$t('batteryMgr.cells')}</span>
      <NumberStepper bind:value={form.cellCount} min={1} max={24} step={1} decimals={0} allowEmpty placeholder="—" />
    </div>
    <div class="fld"><span class="fld-label">{$t('batteryMgr.capacity')} (mAh)</span>
      <NumberStepper bind:value={form.capacityMah} min={0} max={100000} step={100} decimals={0} allowEmpty placeholder="—" />
    </div>
    <div class="fld"><span class="fld-label">{$t('batteryMgr.cDischarge')}</span>
      <NumberStepper bind:value={form.cDischarge} min={0} max={300} step={5} decimals={0} allowEmpty placeholder="—" />
    </div>
    <div class="fld"><span class="fld-label">{$t('batteryMgr.cCharge')}</span>
      <NumberStepper bind:value={form.cCharge} min={0} max={50} step={1} decimals={0} allowEmpty placeholder="—" />
    </div>
    <label class="fld"><span class="fld-label">{$t('batteryMgr.manufacturerField')}</span>
      <input class="fld-input" type="text" bind:value={form.manufacturer} />
    </label>
    <label class="fld"><span class="fld-label">{$t('batteryMgr.model')}</span>
      <input class="fld-input" type="text" bind:value={form.model} />
    </label>
    <label class="fld"><span class="fld-label">{$t('batteryMgr.connector')}</span>
      <select class="fld-input" bind:value={form.connector}>
        <option value="">—</option>
        {#each CONNECTORS as c}<option value={c}>{c}</option>{/each}
      </select>
    </label>
    <label class="fld"><span class="fld-label">{$t('batteryMgr.inService')}</span>
      <input class="fld-input" type="date" bind:value={form.inService} />
    </label>
  </div>
  <label class="fld"><span class="fld-label">{$t('batteryMgr.notes')}</span>
    <textarea class="fld-input fld-area" rows="2" bind:value={form.notes}
      oninput={(e) => autoResize(e.target as HTMLTextAreaElement)} use:notesAutoSize={form.notes ?? ''}></textarea>
  </label>
  <div class="form-actions">
    <Button variant="data" icon="save" onclick={saveForm}>{$t('batteryMgr.save')}</Button>
    <Button variant="standard" onclick={cancelEdit}>{$t('batteryMgr.cancel')}</Button>
  </div>
{/snippet}

{#snippet usageForm()}
  <div class="usage-box">
    <div class="section-heading">{$t('batteryMgr.addUsageTitle')}</div>
    <div class="usage-hint">{$t('batteryMgr.addUsageHint')}</div>
    <div class="form-grid">
      <div class="fld"><span class="fld-label">{$t('batteryMgr.uCycles')}</span>
        <NumberStepper bind:value={usage.cycles} min={0} step={0.5} decimals={1} allowEmpty placeholder="0" />
      </div>
      <div class="fld"><span class="fld-label">{$t('batteryMgr.uHours')}</span>
        <NumberStepper bind:value={usage.hours} min={0} step={0.5} decimals={1} allowEmpty placeholder="0" />
      </div>
      <div class="fld"><span class="fld-label">{$t('batteryMgr.uMah')}</span>
        <NumberStepper bind:value={usage.mah} min={0} step={100} decimals={0} allowEmpty placeholder="0" />
      </div>
      <div class="fld"><span class="fld-label">{$t('batteryMgr.uCharges')}</span>
        <NumberStepper bind:value={usage.charges} min={0} step={1} decimals={0} allowEmpty placeholder="0" />
      </div>
    </div>
    <div class="form-actions">
      <Button variant="data" icon="add" onclick={saveUsage}>{$t('batteryMgr.uAdd')}</Button>
      <Button variant="standard" onclick={cancelUsage}>{$t('batteryMgr.cancel')}</Button>
    </div>
  </div>
{/snippet}

<style>
  /* Toolbar rows: stacked full-width (the shell toolbar slot is a flex row). */
  .bmv2-toolstack { display: flex; flex-direction: column; gap: 6px; width: 100%; }
  .bmv2-order { display: flex; align-items: center; gap: 6px; }
  .bmv2-toolbtns { display: flex; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: 6px; }
  .tb-left { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .tb-right { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; justify-content: flex-end; }

  /* Detail actions flush right (mirrors the flight view's export toolbar). */
  .bmv2-detail-actions { display: flex; flex: 1; justify-content: flex-end; gap: 6px; flex-wrap: wrap; }

  .setting-row { display: flex; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 6px; }
  .setting-label { font-size: 12px; color: #e0e0e0; }
  /* Form controls match the md button height (28px) so toolbars align cleanly (no vertical jog). */
  .setting-select { height: 28px; padding: 0 8px; background: #434343; border: 1px solid #555; border-radius: 4px; color: #e0e0e0; font-size: 12px; }
  .setting-input { height: 28px; padding: 0 8px; background: #434343; border: 1px solid #555; border-radius: 4px; color: #e0e0e0; font-size: 12px; }
  .bmv2-search { flex: 1; min-width: 0; }
  .bmv2-search-clear { background: none; border: none; color: #777; cursor: pointer; font-size: 13px; padding: 2px 4px; line-height: 1; flex-shrink: 0; }
  .bmv2-search-clear:hover { color: #e0e0e0; }

  .section-heading { margin: 8px 0 6px 0; font-size: 11px; font-weight: 600; color: #37a8db; text-transform: uppercase; letter-spacing: 0.5px; }

  .panel-empty { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 8px; padding: 40px 0; color: #555; font-size: 12px; }
  .panel-empty-icon { font-size: 28px; opacity: 0.4; }

  .tree-node { margin-bottom: 4px; }
  .tree-toggle { width: 100%; text-align: left; border: 1px solid #555; border-radius: 4px; background: #353535; color: #ddd; cursor: pointer; display: grid; grid-template-columns: 14px minmax(0, 1fr) auto; align-items: center; gap: 6px; padding: 5px 7px; font-size: 12px; font-weight: 600; }
  .tree-toggle:hover { border-color: #37a8db; }
  .tree-toggle-sub { background: #303030; font-weight: 500; }
  .tree-special > .tree-toggle { background: #2a2a2a; color: #b0b0b0; border-style: dashed; }
  .tree-caret { color: #9cc6d9; font-size: 11px; line-height: 1; }
  .tree-label { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tree-count { font-size: 10px; color: #8fb4c5; background: rgba(55, 168, 219, 0.12); border: 1px solid rgba(55, 168, 219, 0.32); border-radius: 999px; padding: 1px 6px; }
  .tree-items { margin-top: 4px; margin-left: 12px; }

  .lib-item { width: calc(100% - 4px); text-align: left; border: 1px solid #555; border-radius: 4px; background: #383838; color: #ddd; margin-bottom: 4px; padding: 6px; cursor: pointer; }
  .lib-item:hover { border-color: #37a8db; }
  .lib-item.selected { border-color: #37a8db; background: rgba(55, 168, 219, 0.18); }
  .lib-item-title { font-size: 12px; color: #fff; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .lib-item-label { margin-left: 10px; color: #fff; font-weight: 600; }
  .lib-item-meta { margin-top: 2px; display: flex; flex-wrap: wrap; gap: 4px 10px; font-size: 10px; color: #aaa; align-items: center; }
  .bat-chem { text-transform: uppercase; }
  .bat-badge { font-size: 9px; color: #e0b050; border: 1px solid rgba(224, 176, 80, 0.4); border-radius: 999px; padding: 0 5px; text-transform: uppercase; }

  /* Detail content fills the framed detail field (the shell provides the frame). */
  .bmv2-detail { display: flex; flex-direction: column; gap: 6px; }
  .bat-title { font-size: 14px; font-weight: 600; color: #fff; margin-top: 2px; }

  /* Read-only notes field: full width, height grows with content, subtly highlighted. Hidden
     when empty (unlike the flight log, where notes are edited more often and always shown). */
  .bmv2-notes-field {
    width: 100%; box-sizing: border-box; padding: 8px 10px; font-size: 12px; color: #d8d8d8;
    background: rgba(55, 168, 219, 0.06); border: 1px solid rgba(55, 168, 219, 0.25);
    border-radius: 4px; white-space: pre-wrap; overflow-wrap: anywhere; line-height: 1.45;
  }

  /* Pack actions above the flight list (spread left→right). */
  .bmv2-pack-actions { display: flex; justify-content: space-between; gap: 6px; flex-wrap: wrap; margin-bottom: 8px; }

  .fc-info-grid { display: grid; grid-template-columns: auto minmax(0, 1fr); gap: 6px 10px; font-size: 12px; }
  .fc-label { color: #949494; }
  .fc-value { color: #e0e0e0; font-weight: 600; overflow-wrap: anywhere; }

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
  .fld-input:disabled { opacity: 0.6; }
  .fld-area { resize: vertical; }
  .form-actions { display: flex; gap: 6px; margin-top: 8px; }

  .usage-box { border: 1px solid #444; border-radius: 4px; padding: 8px; background: rgba(55, 168, 219, 0.06); }
  .usage-hint { font-size: 11px; color: #949494; margin-bottom: 6px; }

  .modal-backdrop { position: fixed; inset: 0; background: rgba(0, 0, 0, 0.55); display: flex; align-items: center; justify-content: center; z-index: 1000; }
  .modal-card { box-sizing: border-box; background: #2e2e2e; border: 1px solid rgba(55, 168, 219, 0.35); border-radius: 8px; padding: 14px; width: min(540px, 92vw); max-height: 88vh; overflow-y: auto; box-shadow: 0 8px 30px rgba(0, 0, 0, 0.5); }

  .bat-status { position: fixed; bottom: 14px; left: 50%; transform: translateX(-50%); z-index: 1001; padding: 6px 12px; font-size: 11px; color: #f39c12; background: rgba(0, 0, 0, 0.8); border-radius: 6px; }

  @media (max-width: 760px) {
    .form-grid { grid-template-columns: 1fr; }
  }
</style>
