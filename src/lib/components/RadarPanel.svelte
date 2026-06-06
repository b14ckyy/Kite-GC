<script lang="ts">
  // Radar panel (foreign-vehicle tracking) on the panel framework. Phase 0: dynamic per-system tabs
  // on the left (settings field — source config tables land in a later phase) and the consolidated,
  // grouped vehicle list on the right. A Compact button collapses the panel to the `info` variant
  // (list only). See docs/active/RADAR_TRACKING_PANEL_AND_MAP.md.
  import { t } from 'svelte-i18n';
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import PanelShell from '$lib/components/panel/PanelShell.svelte';
  import Button from '$lib/components/panel/Button.svelte';
  import Toggle from '$lib/components/panel/Toggle.svelte';
  import SegmentedToggle, { type SegOption } from '$lib/components/panel/SegmentedToggle.svelte';
  import { radarVehicles, radarAdsbStatus, enrichList, type EnrichedVehicle } from '$lib/stores/radarTracking';
  import { BUILTIN_ADSB_PROVIDERS } from '$lib/stores/settings';
  import type { AppSettings, RadarSettings, InterfaceSettings } from '$lib/stores/settings';
  import type { PortInfo } from '$lib/stores/connection';
  import { convertSpeed, convertAltitude, convertDistance, formatConverted } from '$lib/utils/units';

  let { radar, interfaceSettings, referencePoint = null, onPatch = (_p: Partial<AppSettings>) => {} }: {
    radar: RadarSettings;
    interfaceSettings: InterfaceSettings;
    /** Distance/bearing reference: connected UAV (valid fix) else the GCS location. */
    referencePoint?: { lat: number; lon: number } | null;
    onPatch?: (patch: Partial<AppSettings>) => void;
  } = $props();

  const RADIUS_STEPS = [10, 25, 50, 75, 100];
  const POLL_STEPS = [2, 5, 10, 30]; // provider limit ≈ 1 req/s, so ≥2 s
  const COMPACT_ADSB_MAX = 10; // info view shows only the nearest N ADS-B contacts

  // Patch helpers — edit the nested radar settings (onPatch merges shallowly at the top level).
  const patchRadar = (partial: Partial<RadarSettings>) => onPatch({ radar: { ...radar, ...partial } });
  const patchAdsb = (partial: Partial<RadarSettings['adsb']>) => patchRadar({ adsb: { ...radar.adsb, ...partial } });
  const setBuiltinEnabled = (name: string, on: boolean) =>
    patchAdsb({ builtins: { ...radar.adsb.builtins, [name]: on } });
  const updateProvider = (i: number, patch: Partial<RadarSettings['adsb']['online'][number]>) =>
    patchAdsb({ online: radar.adsb.online.map((p, j) => (j === i ? { ...p, ...patch } : p)) });
  const removeProvider = (i: number) =>
    patchAdsb({ online: radar.adsb.online.filter((_, j) => j !== i) });
  const addProvider = () =>
    patchAdsb({ online: [...radar.adsb.online, { name: '', url: 'https://api.adsb.lol/v2/point/{lat}/{lon}/{dist}', enabled: false }] });
  const inputVal = (e: Event) => (e.target as HTMLInputElement).value;

  // Local hardware receivers (Phase 2: serial MAVLink).
  const BAUD_STEPS = [57600, 115200, 9600, 38400, 230400, 921600];
  const updateLocal = (i: number, patch: Partial<RadarSettings['adsb']['local'][number]>) =>
    patchAdsb({ local: radar.adsb.local.map((l, j) => (j === i ? { ...l, ...patch } : l)) });
  const removeLocal = (i: number) =>
    patchAdsb({ local: radar.adsb.local.filter((_, j) => j !== i) });
  const addLocal = () => {
    void refreshPorts();
    patchAdsb({ local: [...radar.adsb.local, { name: 'Receiver', transport: 'serial', port: '', baud: 57600, enabled: false }] });
  };
  /** If another local source already uses `path`, return its name (for a disabled "(in use)" option). */
  const portConflict = (i: number, path: string): string | null => {
    if (!path) return null;
    const other = radar.adsb.local.find((l, j) => j !== i && l.port === path);
    return other ? (other.name || '—') : null;
  };

  // Available serial ports for the local-source picker (refreshed on mount + on demand).
  let serialPorts = $state<PortInfo[]>([]);
  async function refreshPorts() {
    try {
      serialPorts = await invoke<PortInfo[]>('list_serial_ports');
    } catch (e) {
      console.warn('list_serial_ports failed:', e);
    }
  }
  onMount(refreshPorts);

  let compact = $state(false);

  // Enabled systems → groups (label + enriched, distance-sorted list). Disabled systems are hidden.
  const groups = $derived(
    [
      { key: 'adsb', enabled: radar.adsb.enabled, label: $t('radar.adsb'), list: enrichList($radarVehicles.adsb, referencePoint) },
      { key: 'formationFlight', enabled: radar.formationFlight.enabled, label: $t('radar.formationFlight'), list: enrichList($radarVehicles.formationFlight, referencePoint) },
      { key: 'radio', enabled: radar.radio.enabled, label: $t('radar.radio'), list: enrichList($radarVehicles.radio, referencePoint) },
    ].filter((g) => g.enabled),
  );

  // Dynamic tabs derived from the enabled systems (SegmentedToggle takes `options` directly — no
  // framework change needed).
  const tabOptions = $derived<SegOption[]>(groups.map((g) => ({ value: g.key, label: g.label })));
  let activeSys = $state('adsb');
  // Keep the selected tab valid when systems are toggled on/off.
  $effect(() => {
    if (tabOptions.length && !tabOptions.some((o) => o.value === activeSys)) {
      activeSys = tabOptions[0].value;
    }
  });

  const variant = $derived(compact ? 'info' : 'advanced');

  // ── Formatting helpers (display units) ────────────────────────────
  // formatConverted() already appends the unit — don't add it again.
  const fmtDist = (m: number | null) => {
    if (m == null) return '—';
    const d = convertDistance(m, interfaceSettings.distanceUnit);
    return formatConverted(d, d.value < 10 ? 1 : 0);
  };
  const fmtSpeed = (ms: number | null) =>
    ms == null ? '—' : formatConverted(convertSpeed(ms, interfaceSettings.speedUnit), 0);
  const fmtAlt = (m: number | null) =>
    m == null ? '—' : formatConverted(convertAltitude(m, interfaceSettings.altitudeUnit), 0);
  const fmtBrg = (b: number | null) => (b == null ? '—' : `${Math.round(b)}°`);
  const fmtAge = (lastSeenMs: number) => `${Math.max(0, Math.round((Date.now() - lastSeenMs) / 1000))}s`;
  const label = (v: EnrichedVehicle) => v.callsign?.trim() || v.id;

  // ADS-B emitter category code (A1…C7) → short type abbreviation (weight class / heli / glider / …).
  const CATEGORY_ABBREV: Record<string, string> = {
    A1: 'LGT', A2: 'SML', A3: 'LRG', A4: 'LRG+', A5: 'HVY', A6: 'HPF', A7: 'HELI',
    B1: 'GLD', B2: 'LTA', B3: 'PARA', B4: 'ULT', B6: 'UAV', B7: 'SPC',
    C1: 'SURF', C2: 'SURF', C3: 'OBST', C4: 'OBST', C5: 'OBST', C6: 'OBST', C7: 'OBST',
  };
  const categoryAbbrev = (cat: string | null | undefined) => (cat ? (CATEGORY_ABBREV[cat] ?? '') : '');
</script>

<PanelShell
  {variant}
  title={$t('radar.title')}
  detailTitle={$t('radar.vehicles')}
  toolbar={compact ? undefined : toolbarSnip}
  detailToolbar={compact ? undefined : detailToolbarSnip}
  body={compact ? compactBody : sourcesPane}
  detail={compact ? undefined : vehicleList}
/>

{#snippet detailToolbarSnip()}
  <!-- Compact collapses the panel to the info variant (list only). Right-aligned, on the detail
       toolbar row (same level as the tab switcher on the left). Left chevron = collapse leftward. -->
  <div class="rt-right">
    <Button variant="standard" size="sm" onclick={() => (compact = true)}>← {$t('radar.compact')}</Button>
  </div>
{/snippet}

{#snippet compactBody()}
  <!-- Info variant: no button — clicking the panel re-expands it (like the logbook). It does NOT
       auto-collapse (we may want full-panel map interactions later). -->
  <div
    class="rt-compact"
    role="button"
    tabindex="0"
    title={$t('radar.expand')}
    onclick={() => (compact = false)}
    onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); compact = false; } }}
  >
    {@render vehicleList()}
  </div>
{/snippet}

{#snippet toolbarSnip()}
  {#if tabOptions.length > 1}
    <SegmentedToggle options={tabOptions} value={activeSys} size="sm" onchange={(v) => (activeSys = v)} />
  {/if}
{/snippet}

{#snippet sourcesPane()}
  <!-- Source configuration for the selected system. ADS-B: online providers + radius (Phase 1).
       FormationFlight / Radio: source tables arrive in a later phase. -->
  {#if tabOptions.length === 0}
    <p class="radar-hint">{$t('radar.noSystems')}</p>
  {:else if activeSys === 'adsb'}
    <div class="src-block">
      <div class="src-row">
        <span class="src-label">{$t('radar.radius')}</span>
        <select
          class="src-select"
          value={radar.adsb.radiusKm}
          onchange={(e) => patchAdsb({ radiusKm: Number((e.target as HTMLSelectElement).value) })}
        >
          {#each RADIUS_STEPS as km}<option value={km}>{km} km</option>{/each}
        </select>
      </div>
      <div class="src-row">
        <span class="src-label">{$t('radar.pollInterval')}</span>
        <select
          class="src-select"
          value={radar.adsb.pollSec}
          onchange={(e) => patchAdsb({ pollSec: Number((e.target as HTMLSelectElement).value) })}
        >
          {#each POLL_STEPS as s}<option value={s}>{s} s</option>{/each}
        </select>
      </div>

      <!-- Built-in providers: toggle only (fixed URL, no key, not removable). -->
      <p class="src-head">{$t('radar.onlineSources')}</p>
      {#each BUILTIN_ADSB_PROVIDERS as b (b.name)}
        {@const st = $radarAdsbStatus[b.name]}
        {@const on = radar.adsb.builtins[b.name] ?? true}
        <div class="src-row">
          <span class="src-name" title={b.url}>{b.name}</span>
          {#if on && st}
            {#if st.ok}<span class="src-stat ok" title={$t('radar.contacts')}>{st.count}</span>
            {:else}<span class="src-stat err" title={$t('radar.sourceError')}>✕</span>{/if}
          {/if}
          <Toggle checked={on} onchange={(c) => setBuiltinEnabled(b.name, c)} />
        </div>
      {/each}

      <!-- Custom providers: editable + removable (same heading — all are online sources). Delete on
           the left, toggle on the right to match the built-in rows. -->
      {#each radar.adsb.online as p, i (i)}
        {@const st = $radarAdsbStatus[p.name]}
        <div class="src-card">
          <div class="src-card-head">
            {#if !p.enabled}
              <button class="src-del" title={$t('radar.removeSource')} onclick={() => removeProvider(i)} aria-label={$t('radar.removeSource')}>✕</button>
              <input
                class="src-input src-input-name"
                placeholder={$t('radar.providerName')}
                value={p.name}
                onchange={(e) => updateProvider(i, { name: inputVal(e) })}
              />
            {:else}
              <span class="src-name" title={p.url}>{p.name || '—'}</span>
              {#if st}
                {#if st.ok}<span class="src-stat ok" title={$t('radar.contacts')}>{st.count}</span>
                {:else}<span class="src-stat err" title={$t('radar.sourceError')}>✕</span>{/if}
              {/if}
            {/if}
            <Toggle checked={p.enabled} onchange={(c) => updateProvider(i, { enabled: c })} />
          </div>
          {#if !p.enabled}
            <input
              class="src-input"
              placeholder={$t('radar.providerUrl')}
              value={p.url}
              onchange={(e) => updateProvider(i, { url: inputVal(e) })}
            />
            <input
              class="src-input"
              placeholder={$t('radar.providerKey')}
              value={p.apiKey ?? ''}
              onchange={(e) => updateProvider(i, { apiKey: inputVal(e) || undefined })}
            />
          {/if}
        </div>
      {/each}
      <Button variant="standard" size="sm" full icon="add" onclick={addProvider}>{$t('radar.addSource')}</Button>

      <!-- Local hardware receivers (serial MAVLink; TCP later). -->
      <p class="src-head">{$t('radar.localSources')}</p>
      {#each radar.adsb.local as l, i (i)}
        {@const st = $radarAdsbStatus[l.name]}
        <div class="src-card">
          <div class="src-card-head">
            {#if !l.enabled}
              <button class="src-del" title={$t('radar.removeSource')} onclick={() => removeLocal(i)} aria-label={$t('radar.removeSource')}>✕</button>
              <input
                class="src-input src-input-name"
                placeholder={$t('radar.providerName')}
                value={l.name}
                onchange={(e) => updateLocal(i, { name: inputVal(e) })}
              />
            {:else}
              <span class="src-name">{l.name || '—'}{#if l.port}<span class="src-sub"> · {l.port}</span>{/if}</span>
              {#if st}
                {#if st.ok}<span class="src-stat ok" title={$t('radar.contacts')}>{st.count}</span>
                {:else}<span class="src-stat err" title={$t('radar.sourceError')}>✕</span>{/if}
              {/if}
            {/if}
            <Toggle checked={l.enabled} onchange={(c) => updateLocal(i, { enabled: c })} />
          </div>
          {#if !l.enabled}
            <div class="src-row2">
              <select class="src-select src-input-port" value={l.port} onchange={(e) => updateLocal(i, { port: (e.target as HTMLSelectElement).value })}>
                <option value="">{$t('radar.selectPort')}</option>
                {#each serialPorts as sp}
                  {@const conflict = portConflict(i, sp.path)}
                  <option value={sp.path} disabled={!!conflict}>{sp.label || sp.path}{conflict ? ` (${$t('radar.inUse')}: ${conflict})` : ''}</option>
                {/each}
                {#if l.port && !serialPorts.some((sp) => sp.path === l.port)}<option value={l.port}>{l.port} ({$t('radar.portMissing')})</option>{/if}
              </select>
              <button class="src-refresh" title={$t('radar.refreshPorts')} onclick={refreshPorts} aria-label={$t('radar.refreshPorts')}>⟳</button>
              <select class="src-select" value={l.baud} onchange={(e) => updateLocal(i, { baud: Number((e.target as HTMLSelectElement).value) })}>
                {#each BAUD_STEPS as b}<option value={b}>{b}</option>{/each}
              </select>
            </div>
          {/if}
        </div>
      {/each}
      <Button variant="standard" size="sm" full icon="add" onclick={addLocal}>{$t('radar.addLocalSource')}</Button>
    </div>
  {:else}
    <p class="radar-hint">{$t('radar.sourcesPlaceholder')}</p>
  {/if}
{/snippet}

{#snippet vehicleList()}
  {#if groups.length === 0}
    <p class="radar-hint">{$t('radar.noSystems')}</p>
  {:else}
    <div class="radar-list">
      {#each groups as g (g.key)}
        <div class="radar-group">
          <div class="radar-group-head">
            <span class="radar-group-name">{g.label}</span>
            <span class="radar-group-count">{g.list.length}</span>
          </div>
          {#if g.list.length === 0}
            <p class="radar-empty">{$t('radar.noContacts')}</p>
          {:else}
            <!-- Info view caps ADS-B to the nearest COMPACT_ADSB_MAX (sorted by distance). -->
            {@const rows = compact && g.key === 'adsb' ? g.list.slice(0, COMPACT_ADSB_MAX) : g.list}
            {#each rows as v (v.id)}
              <div class="radar-row" class:compact>
                <span class="r-call">{label(v)}</span>
                {#if !compact}<span class="r-type" title={v.category ?? ''}>{categoryAbbrev(v.category)}</span>{/if}
                <span class="r-dist">{fmtDist(v.distanceM)}</span>
                <span class="r-brg">{fmtBrg(v.bearingDeg)}</span>
                <span class="r-alt">{fmtAlt(v.altM)}</span>
                {#if !compact}
                  <span class="r-spd">{fmtSpeed(v.groundSpeedMs)}</span>
                  <span class="r-age">{fmtAge(v.lastSeenMs)}</span>
                {/if}
              </div>
            {/each}
          {/if}
        </div>
      {/each}
    </div>
  {/if}
{/snippet}

<style>
  .radar-hint { color: #949494; font-size: 12px; line-height: 1.5; margin: 4px 2px; }

  .rt-right { margin-left: auto; display: flex; }
  /* Info variant: the whole list is clickable to re-expand. */
  .rt-compact { cursor: pointer; }

  /* ── ADS-B sources pane ── */
  .src-block { display: flex; flex-direction: column; gap: 4px; }
  .src-row { display: flex; align-items: center; justify-content: space-between; gap: 8px; padding: 3px 2px; }
  .src-head { color: #949494; font-size: 11px; text-transform: uppercase; letter-spacing: 0.4px; margin: 8px 2px 2px; }
  .src-label { color: #cdd6da; font-size: 12px; }
  .src-name { flex: 1; color: #e0e0e0; font-size: 12px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .src-stat {
    flex-shrink: 0;
    min-width: 22px;
    height: 18px;
    padding: 0 6px;
    border-radius: 9px;
    font-size: 11px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
  .src-stat.ok { background: rgba(89, 170, 41, 0.2); color: #7ed957; border: 1px solid #59aa29; }
  .src-stat.err { background: rgba(212, 0, 0, 0.2); color: #ff6b6b; border: 1px solid #d40000; }
  .src-select {
    height: 28px;
    padding: 0 8px;
    background: #434343;
    border: 1px solid #555;
    border-radius: 4px;
    color: #e0e0e0;
    font-size: 12px;
  }
  .src-card {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 6px;
    margin-bottom: 6px;
    border: 1px solid #444;
    border-radius: 5px;
    background: rgba(255, 255, 255, 0.03);
  }
  .src-card-head { display: flex; align-items: center; gap: 6px; }
  .src-input {
    box-sizing: border-box;
    width: 100%;
    height: 26px;
    padding: 0 7px;
    background: #1f1f1f;
    border: 1px solid #444;
    border-radius: 4px;
    color: #e0e0e0;
    font-size: 11px;
    font-family: 'Segoe UI', Tahoma, sans-serif;
  }
  .src-input-name { flex: 1; font-weight: 600; }
  .src-sub { color: #949494; font-weight: 400; }
  .src-row2 { display: flex; gap: 6px; align-items: center; }
  .src-input-port { flex: 1; min-width: 0; }
  .src-refresh {
    flex-shrink: 0;
    width: 26px;
    height: 26px;
    border: 1px solid #555;
    border-radius: 4px;
    background: #434343;
    color: #cdd6da;
    cursor: pointer;
    font-size: 14px;
    line-height: 1;
  }
  .src-refresh:hover { background: #4d4d4d; color: #fff; }
  .src-del {
    flex-shrink: 0;
    width: 24px;
    height: 24px;
    border: 1px solid #6a3030;
    border-radius: 4px;
    background: rgba(212, 0, 0, 0.12);
    color: #e88;
    cursor: pointer;
    font-size: 12px;
    line-height: 1;
  }
  .src-del:hover { background: rgba(212, 0, 0, 0.25); color: #fff; }

  .radar-list { display: flex; flex-direction: column; gap: 10px; }
  .radar-group { display: flex; flex-direction: column; gap: 2px; }
  .radar-group-head {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 2px 2px 4px;
    border-bottom: 1px solid #3a3a3a;
    margin-bottom: 2px;
  }
  .radar-group-name { color: #37a8db; font-weight: 600; font-size: 12px; letter-spacing: 0.3px; }
  .radar-group-count {
    color: #c7dfe8;
    font-variant-numeric: tabular-nums;
    background: rgba(55, 168, 219, 0.15);
    border-radius: 8px;
    padding: 0 7px;
    font-size: 11px;
  }
  .radar-empty { color: #6f6f6f; font-size: 11px; font-style: italic; margin: 2px 4px 6px; }

  .radar-row {
    display: grid;
    /* call · type · dist · brg · alt · spd · age — age is narrow (secs are ≤2 digits). */
    grid-template-columns: 1.25fr 0.55fr 0.85fr 0.6fr 0.95fr 0.9fr 0.42fr;
    gap: 6px;
    align-items: center;
    padding: 4px 4px;
    border-radius: 4px;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }
  .radar-row:nth-child(even) { background: rgba(255, 255, 255, 0.03); }
  /* Info view: call · dist · brg · alt — dist + alt wide enough for 4-digit km / >10 000 m (no wrap). */
  .radar-row.compact { grid-template-columns: 1.15fr 1.1fr 0.55fr 1.1fr; }
  .r-call { color: #e8e8e8; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .r-type { color: #8fb4c5; font-size: 11px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .r-dist, .r-brg, .r-alt, .r-spd, .r-age { color: #b8b8b8; text-align: right; white-space: nowrap; }
</style>
