<!-- BatteryManager.svelte
     Battery library view — rendered inside the Flight Logbook panel as a list-view toggle
     (like the Mission Manager is to the Mission Planner). Grouped/flat pack list on the left,
     pack detail (editable identity + lifetime stats + linked flights) on the right.
     See docs/dev/BATTERY_MANAGEMENT.md.
-->
<script lang="ts">
  import { t } from 'svelte-i18n';
  import { get } from 'svelte/store';
  import { settings } from '$lib/stores/settings';
  import {
    batteryDbList, batteryDbCreate, batteryDbUpdate, batteryDbDelete,
    batteryDbFindBySerial, batteryDbAddUsage, batteryDbAggregate, batteryDbFlights,
    formatDurationSec,
  } from '$lib/stores/flightlog';
  import type { BatteryPack, BatteryPackInput, BatteryAggregate, FlightSummary } from '$lib/stores/flightlogTypes';
  import {
    batteryManagerSelectedId, batteryGroupMode, batteryLeafAsc, batterySearchQuery, batterySortField,
  } from '$lib/stores/batteryManager';
  import { requestOpenFlightId } from '$lib/stores/missionManager';
  import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
  import NumberStepper from '$lib/components/NumberStepper.svelte';

  let confirmDialog: ReturnType<typeof ConfirmDialog>;

  let batteries = $state<BatteryPack[]>([]);
  let aggregate = $state<BatteryAggregate | null>(null);
  let linkedFlights = $state<FlightSummary[]>([]);
  // Tracks which groups are OPEN (empty = all collapsed → groups start collapsed on first open,
  // consistent with the Flight Logbook tree).
  let open = $state<Set<string>>(new Set());
  let statusMessage = $state('');

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
      serial: form.serial.trim(),
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

  async function deleteBattery() {
    if (!selected) return;
    const b = selected;
    const count = linkedFlights.length;
    const ans = await confirmDialog.show({
      title: $t('batteryMgr.deleteTitle'),
      message: count > 0
        ? $t('batteryMgr.deleteMsgLinked', { values: { count: String(count) } })
        : $t('batteryMgr.deleteMsg'),
      buttons: [{ label: $t('batteryMgr.deleteYes'), value: 'delete', danger: true }],
    });
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

<div class="bat-layout" class:bat-layout-detail={selected != null}>
  <!-- List -->
  <div class="bat-list">
    <div class="bat-list-head">
      <button class="cache-clear-btn" onclick={startCreate}>＋ {$t('batteryMgr.new')}</button>
      <div class="bat-head-right">
        {#if $batteryGroupMode === 'flat'}
          <select class="bat-sort-select" bind:value={$batterySortField} title={$t('batteryMgr.sortField')}>
            <option value="serial">{$t('batteryMgr.serial')}</option>
            <option value="cell">{$t('batteryMgr.cells')}</option>
            <option value="capacity">{$t('batteryMgr.capacity')}</option>
          </select>
        {/if}
        <button class="cache-clear-btn bat-asc" onclick={() => batteryLeafAsc.update((v) => !v)} title={$t('batteryMgr.sortDir')}>
          {$batteryLeafAsc ? '▲' : '▼'}
        </button>
      </div>
    </div>

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
  </div>

  <!-- Detail -->
  {#if selected}
    {@const b = selected}
    <div class="bat-detail">
        <div class="det-actions">
          <button class="cache-clear-btn" onclick={startEdit}>✎ {$t('batteryMgr.edit')}</button>
          <button class="cache-clear-btn" onclick={startUsage}>＋ {$t('batteryMgr.addUsage')}</button>
          <button class="cache-clear-btn logbook-danger" onclick={deleteBattery}>🗑 {$t('batteryMgr.delete')}</button>
        </div>

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

        {#if b.notes}<div class="bat-notes">{b.notes}</div>{/if}

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
      <input class="fld-input" type="text" bind:value={form.serial} disabled={!isCreate} />
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
    <button class="cache-clear-btn" onclick={saveForm}>{$t('batteryMgr.save')}</button>
    <button class="cache-clear-btn" onclick={cancelEdit}>{$t('batteryMgr.cancel')}</button>
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
      <button class="cache-clear-btn" onclick={saveUsage}>{$t('batteryMgr.uAdd')}</button>
      <button class="cache-clear-btn" onclick={cancelUsage}>{$t('batteryMgr.cancel')}</button>
    </div>
  </div>
{/snippet}

<style>
  .section-heading { margin: 8px 0 6px 0; font-size: 11px; font-weight: 600; color: #37a8db; text-transform: uppercase; letter-spacing: 0.5px; }

  .bat-layout { display: grid; grid-template-columns: 1fr; gap: 12px; min-height: 420px; }
  .bat-layout.bat-layout-detail { grid-template-columns: 300px minmax(0, 1fr); }

  .bat-list { box-sizing: border-box; max-height: 560px; overflow: auto; border: 1px solid #555; border-radius: 4px; background: rgba(0, 0, 0, 0.12); padding: 6px; }
  .bat-list-head { display: flex; gap: 6px; margin-bottom: 6px; align-items: center; justify-content: space-between; }
  .bat-head-right { display: flex; gap: 6px; align-items: center; }
  .bat-asc { padding: 4px 8px; }
  .bat-sort-select { padding: 4px 6px; background: #434343; border: 1px solid #555; border-radius: 3px; color: #e0e0e0; font-size: 11px; }

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

  .bat-detail { box-sizing: border-box; border: 1px solid #555; border-radius: 4px; background: rgba(0, 0, 0, 0.12); padding: 10px; overflow-y: auto; overflow-x: hidden; max-height: 560px; display: flex; flex-direction: column; gap: 6px; }
  .det-actions { display: flex; gap: 6px; flex-wrap: wrap; }
  .bat-title { font-size: 14px; font-weight: 600; color: #fff; margin-top: 2px; }
  .bat-notes { font-size: 12px; color: #cfcfcf; white-space: pre-wrap; overflow-wrap: anywhere; }

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

  .bat-status { padding: 4px 6px; font-size: 11px; color: #f39c12; text-align: center; }

  /* Unified app button style (matches the Mission Manager / logbook). */
  .cache-clear-btn { font-size: 11px; padding: 4px 10px; background: #434343; border: 1px solid #555; border-radius: 3px; color: #ccc; cursor: pointer; transition: background 0.15s; white-space: nowrap; }
  .cache-clear-btn:hover:not(:disabled) { background: #37a8db; border-color: #37a8db; color: #fff; }
  .cache-clear-btn:disabled { opacity: 0.5; cursor: default; }
  .logbook-danger { background: #7a2020; border-color: #8b2525; color: #e8c0c0; }
  .cache-clear-btn.logbook-danger:hover:not(:disabled) { background: #9b1f1f; border-color: #9b1f1f; color: #fff; }

  @media (max-width: 760px) {
    .bat-layout.bat-layout-detail { grid-template-columns: 1fr; }
    .form-grid { grid-template-columns: 1fr; }
  }
</style>
