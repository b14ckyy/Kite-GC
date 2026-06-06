<script lang="ts">
  // Radar panel (foreign-vehicle tracking) on the panel framework. Phase 0: dynamic per-system tabs
  // on the left (settings field — source config tables land in a later phase) and the consolidated,
  // grouped vehicle list on the right. A Compact button collapses the panel to the `info` variant
  // (list only). See docs/active/RADAR_TRACKING_PANEL_AND_MAP.md.
  import { t } from 'svelte-i18n';
  import PanelShell from '$lib/components/panel/PanelShell.svelte';
  import Button from '$lib/components/panel/Button.svelte';
  import SegmentedToggle, { type SegOption } from '$lib/components/panel/SegmentedToggle.svelte';
  import { radarVehicles, enrichList, type EnrichedVehicle } from '$lib/stores/radarTracking';
  import type { RadarSettings, InterfaceSettings } from '$lib/stores/settings';
  import { convertSpeed, convertAltitude, convertDistance, formatConverted } from '$lib/utils/units';

  let { radar, interfaceSettings, userLocation = null }: {
    radar: RadarSettings;
    interfaceSettings: InterfaceSettings;
    userLocation?: { lat: number; lon: number } | null;
  } = $props();

  let compact = $state(false);

  // Enabled systems → groups (label + enriched, distance-sorted list). Disabled systems are hidden.
  const groups = $derived(
    [
      { key: 'adsb', enabled: radar.adsb.enabled, label: $t('radar.adsb'), list: enrichList($radarVehicles.adsb, userLocation) },
      { key: 'formationFlight', enabled: radar.formationFlight.enabled, label: $t('radar.formationFlight'), list: enrichList($radarVehicles.formationFlight, userLocation) },
      { key: 'radio', enabled: radar.radio.enabled, label: $t('radar.radio'), list: enrichList($radarVehicles.radio, userLocation) },
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
  const fmtDist = (m: number | null) => {
    if (m == null) return '—';
    const d = convertDistance(m, interfaceSettings.distanceUnit);
    return `${formatConverted(d, d.value < 10 ? 1 : 0)} ${d.unit}`;
  };
  const fmtSpeed = (ms: number | null) => {
    if (ms == null) return '—';
    const s = convertSpeed(ms, interfaceSettings.speedUnit);
    return `${formatConverted(s, 0)} ${s.unit}`;
  };
  const fmtAlt = (m: number | null) => {
    if (m == null) return '—';
    const a = convertAltitude(m, interfaceSettings.altitudeUnit);
    return `${formatConverted(a, 0)} ${a.unit}`;
  };
  const fmtBrg = (b: number | null) => (b == null ? '—' : `${Math.round(b)}°`);
  const fmtAge = (lastSeenMs: number) => `${Math.max(0, Math.round((Date.now() - lastSeenMs) / 1000))}s`;
  const label = (v: EnrichedVehicle) => v.callsign?.trim() || v.id;
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
  <!-- Source configuration (per selected system). Phase 0: placeholder — the source tables
       (online + hard sources) arrive in a later phase. -->
  {#if tabOptions.length === 0}
    <p class="radar-hint">{$t('radar.noSystems')}</p>
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
            {#each g.list as v (v.id)}
              <div class="radar-row" class:compact>
                <span class="r-call">{label(v)}</span>
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
    grid-template-columns: 1.4fr 0.9fr 0.7fr 0.9fr 0.9fr 0.6fr;
    gap: 6px;
    align-items: center;
    padding: 4px 4px;
    border-radius: 4px;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }
  .radar-row:nth-child(even) { background: rgba(255, 255, 255, 0.03); }
  .radar-row.compact { grid-template-columns: 1.4fr 0.9fr 0.7fr 0.9fr; }
  .r-call { color: #e8e8e8; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .r-dist, .r-brg, .r-alt, .r-spd, .r-age { color: #b8b8b8; text-align: right; }
</style>
