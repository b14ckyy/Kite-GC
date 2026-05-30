<script module lang="ts">
  import type { ProfileData as _ProfileData } from '$lib/helpers/terrainProfile';

  // Per-mode profile cache (survives panel remount) so switching Waypoints↔Track
  // is instant when the underlying route/track signature hasn't changed.
  interface CacheEntry {
    sig: string;
    data: _ProfileData | null;
    errorKind: 'none' | 'insufficient' | 'noTrack';
  }
  const profileCache = new Map<'waypoint' | 'track', CacheEntry>();
</script>

<script lang="ts">
  import { t } from 'svelte-i18n';
  import { onDestroy } from 'svelte';
  import { get } from 'svelte/store';
  import {
    mission,
    launchPoint,
    missionUpdateWp,
    missionInsertWp,
    getTotalWpCount,
    MAX_WAYPOINTS_TOTAL,
    WpAction,
    ALT_MODE_AGL,
    ALT_MODE_AMSL,
    altFromM,
    fromDeg,
  } from '$lib/stores/mission';
  import {
    terrainAnalysis,
    terrainCursor,
    patchTerrainAnalysis,
    resetTerrainView,
    setTerrainHover,
    toggleTerrainPlaced,
    clearTerrainHover,
    clearTerrainPlaced,
  } from '$lib/stores/terrainAnalysis';
  import {
    buildWaypointProfile,
    buildTrackProfile,
    LiveTrackProfiler,
    type ProfileData,
    type TrackPoint,
  } from '$lib/helpers/terrainProfile';
  import { liveTrack } from '$lib/stores/liveTrack';
  import { computeCorrection, type CorrectionResult } from '$lib/helpers/terrainCorrection';
  import TerrainProfileChart, { type HoverInfo } from './TerrainProfileChart.svelte';
  import NumberStepper from '$lib/components/NumberStepper.svelte';
  import UnitStepper from '$lib/components/UnitStepper.svelte';
  import { convertAltitude, convertDistance } from '$lib/utils/units';
  import type { InterfaceSettings } from '$lib/stores/settings';
  import type { DialogOptions } from '$lib/components/ConfirmDialog.svelte';

  let {
    track = [],
    live = false,
    interfaceSettings,
    confirm,
  }: {
    /** Loaded blackbox track for Track mode (non-live) */
    track?: TrackPoint[];
    /** A live FC connection is active → Track mode follows the live flown track */
    live?: boolean;
    interfaceSettings: InterfaceSettings;
    /** In-app dialog (from +page) for the APPLY confirmation */
    confirm?: (opts: DialogOptions) => Promise<string | null>;
  } = $props();

  // Unit-aware display formatters (internal values are metric)
  function fmtAlt(m: number): string {
    const c = convertAltitude(m, interfaceSettings.altitudeUnit);
    return `${Math.round(c.value)} ${c.unit}`;
  }
  function fmtDist(m: number): string {
    const c = convertDistance(m, interfaceSettings.distanceUnit);
    const digits = c.unit === 'km' || c.unit === 'mi' ? 2 : 0;
    return `${c.value.toFixed(digits)} ${c.unit}`;
  }

  let data = $state<ProfileData | null>(null);
  let loading = $state(false);
  let errorKind = $state<'none' | 'insufficient' | 'noTrack'>('none');
  let hover = $state<HoverInfo | null>(null);

  let computeToken = 0;

  // Recompute the profile only when something *structural* changes (route /
  // track / launch / mode) — not on cheap param tweaks (clearance, datum, zoom).
  // Results are cached per mode so switching Waypoints↔Track is instant.
  $effect(() => {
    const st = $terrainAnalysis;
    const wps = $mission.waypoints;
    const lp = $launchPoint;
    const trk = track;
    if (!st.open) return;
    const mode = st.viewMode;
    if (live && mode === 'track') return; // the live poll owns Track mode while connected

    const sig =
      mode === 'track'
        ? `t:${trk.length}:${trk.length ? trk[trk.length - 1].timestamp_ms ?? '' : ''}`
        : `w:${wps.length}:${wps
            .map((w) => `${w.lat},${w.lon},${w.altitude},${w.alt_mode ?? ''}`)
            .join('|')}:${lp ? `${lp.lat},${lp.lng}` : '-'}`;

    const cached = profileCache.get(mode);
    if (cached && cached.sig === sig) {
      data = cached.data;
      errorKind = cached.errorKind;
      loading = false;
      return;
    }

    const token = ++computeToken;
    loading = true;
    const timer = setTimeout(async () => {
      try {
        let result: ProfileData | null;
        let kind: 'none' | 'insufficient' | 'noTrack';
        if (mode === 'track') {
          result = await buildTrackProfile(trk);
          kind = result ? 'none' : 'noTrack';
        } else {
          result = await buildWaypointProfile(wps, lp ? { lat: lp.lat, lng: lp.lng } : null);
          kind = result ? 'none' : 'insufficient';
        }
        if (token === computeToken) {
          data = result;
          errorKind = kind;
          loading = false;
          profileCache.set(mode, { sig, data: result, errorKind: kind });
        }
      } catch (e) {
        console.error('[terrain] profile computation failed', e);
        if (token === computeToken) {
          data = null;
          errorKind = 'none';
          loading = false;
        }
      }
    }, 250);
    return () => clearTimeout(timer);
  });

  // ── Live Track mode (FC connected) ─────────────────────────────────
  const liveActive = $derived(live && $terrainAnalysis.viewMode === 'track' && $terrainAnalysis.open);
  const profiler = new LiveTrackProfiler();

  const LIVE_MIN_WINDOW = 250; // m — default live window; track builds up then scrolls

  /** Pin the view to the newest data (right edge), keeping the current window.
   *  null/null = full-zoom-out → left untouched (auto-fits the growing range). */
  function pinFollow(total: number) {
    const st = get(terrainAnalysis);
    if (!st.follow) return;
    if (st.viewStart == null && st.viewEnd == null) return; // full-fit, grows on its own
    const win = (st.viewEnd as number) - (st.viewStart as number);
    const ve = Math.max(win, total); // before the track reaches `win`, keep [0, win]
    patchTerrainAnalysis({ viewStart: Math.max(0, ve - win), viewEnd: ve });
  }

  $effect(() => {
    if (!liveActive) return;
    profiler.reset();
    data = null;
    loading = true;
    // default live window: build up from the left within a fixed window
    if (get(terrainAnalysis).follow) {
      patchTerrainAnalysis({ viewStart: 0, viewEnd: LIVE_MIN_WINDOW });
    }
    let cancelled = false;
    const tick = async () => {
      try {
        const result = await profiler.update(get(liveTrack));
        if (cancelled) return;
        data = result;
        errorKind = result ? 'none' : 'noTrack';
        loading = false;
        if (result) pinFollow(result.totalDist);
      } catch (e) {
        console.error('[terrain] live profile failed', e);
      }
    };
    void tick();
    const id = setInterval(() => void tick(), 5000);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  });

  function toggleFollow() {
    const f = !get(terrainAnalysis).follow;
    patchTerrainAnalysis({ follow: f });
    if (f && data) pinFollow(data.totalDist);
  }

  function setMode(mode: 'waypoint' | 'track') {
    if ($terrainAnalysis.viewMode === mode) return;
    resetTerrainView();
    patchTerrainAnalysis({ viewMode: mode });
  }

  function setDatum(datum: 'msl' | 'agl') {
    patchTerrainAnalysis({ datum });
  }

  function toggleCompact() {
    patchTerrainAnalysis({ compact: !$terrainAnalysis.compact });
  }

  // Mirror the chart cursor onto the map (transient hover + readouts)
  function onChartHover(h: HoverInfo | null) {
    hover = h;
    setTerrainHover(h ? { lat: h.lat, lon: h.lon } : null);
  }
  // Click pins/unpins a persistent map marker
  function onChartPick(p: { lat: number; lon: number }) {
    toggleTerrainPlaced(p);
  }

  // Drop the transient hover when the panel closes (the pinned marker stays)
  onDestroy(() => clearTerrainHover());

  // Ground Clearance via the standard stepper (5 m steps, 1 m manual precision).
  let groundClearance = $state($terrainAnalysis.groundClearance);
  function onClearanceChange() {
    patchTerrainAnalysis({ groundClearance: Math.max(0, groundClearance) });
  }

  // Clearance analysis ignores the take-off climb-out and landing descent:
  // skip the leading/trailing runs that sit below clearance (we start/land on
  // the ground), so only the en-route portion drives the min-clearance alert.
  const activeRange = $derived.by(() => {
    const gc = $terrainAnalysis.groundClearance;
    if (!data) return { startDist: -Infinity, endDist: Infinity, min: null as number | null };
    const t = data.terrain;
    const cl = data.clearance;
    let first = -1;
    let last = -1;
    for (let i = 0; i < cl.length; i++) {
      if (cl[i] != null && (cl[i] as number) >= gc) {
        first = i;
        break;
      }
    }
    for (let i = cl.length - 1; i >= 0; i--) {
      if (cl[i] != null && (cl[i] as number) >= gc) {
        last = i;
        break;
      }
    }
    // Never reaches clearance → real problem: consider the whole route.
    if (first < 0 || last < first) {
      let min: number | null = null;
      for (const c of cl) if (c != null && (min == null || c < min)) min = c;
      return { startDist: -Infinity, endDist: Infinity, min };
    }
    let min: number | null = null;
    for (let i = first; i <= last; i++) {
      const c = cl[i];
      if (c != null && (min == null || c < min)) min = c;
    }
    return { startDist: t[first].dist, endDist: t[last].dist, min };
  });

  // Warn/colour at 95% of the target (5% grace) so exact-clearance isn't already red
  const warnThreshold = $derived($terrainAnalysis.groundClearance * 0.95);
  const belowClearance = $derived(activeRange.min != null && activeRange.min < warnThreshold);

  // Distance of the pinned map marker along the current profile (nearest sample
  // by lat/lon), so the chart can show it too. Hidden if the pin isn't on the
  // current path (e.g. after switching Waypoints↔Track).
  const placedDist = $derived.by(() => {
    const p = $terrainCursor.placed;
    if (!p || !data) return null;
    let best: number | null = null;
    let bd = Infinity;
    for (const s of data.terrain) {
      const dlat = s.lat - p.lat;
      const dlon = s.lon - p.lon;
      const d2 = dlat * dlat + dlon * dlon;
      if (d2 < bd) {
        bd = d2;
        best = s.dist;
      }
    }
    // ~0.002° ≈ 200 m tolerance
    if (best == null || bd > 0.002 * 0.002) return null;
    return best;
  });

  // ── Terrain Correction (Waypoint mode) ─────────────────────────────
  const maxWpNumber = $derived(
    data && data.markers.length ? Math.max(...data.markers.map((m) => m.number)) : 1,
  );

  const correction = $derived.by<CorrectionResult | null>(() => {
    const st = $terrainAnalysis;
    if (!st.correctionEnabled || st.viewMode !== 'waypoint' || !data) return null;
    if (data.markers.length < 2) return null;
    return computeCorrection(data, {
      mode: st.correctionMode,
      groundClearance: st.groundClearance,
      rangeStart: st.rangeStart,
      rangeEnd: st.rangeEnd,
      fixedWing: st.fixedWing,
      climbAngle: st.climbAngleLimit,
      descentAngle: st.descentAngleLimit,
    });
  });

  const previewPath = $derived(correction ? correction.previewPath : null);
  const showCorrection = $derived($terrainAnalysis.viewMode === 'waypoint' && !$terrainAnalysis.compact);
  const canAddWp = $derived($terrainCursor.placed != null && placedDist != null && data != null);

  function toggleCorrection() {
    const st = get(terrainAnalysis);
    const enabling = !st.correctionEnabled;
    if (enabling) {
      patchTerrainAnalysis({
        correctionEnabled: true,
        rangeStart: st.rangeStart > 0 ? st.rangeStart : 1,
        rangeEnd: st.rangeEnd > 0 ? st.rangeEnd : maxWpNumber,
      });
    } else {
      patchTerrainAnalysis({ correctionEnabled: false });
    }
  }

  let applying = $state(false);
  async function applyCorrection() {
    const c = correction;
    if (!c || c.changedCount === 0) return;
    if (confirm) {
      const ans = await confirm({
        title: $t('terrain.applyTitle'),
        message: $t('terrain.applyConfirm', { values: { changed: c.changedCount } }),
        buttons: [{ label: $t('terrain.apply'), value: 'apply', primary: true }],
      });
      if (ans !== 'apply') return;
    }
    applying = true;
    try {
      for (const ch of c.changes) {
        const wp = get(mission).waypoints[ch.index];
        if (!wp) continue;
        await missionUpdateWp(ch.index, {
          ...wp,
          altitude: altFromM(ch.aglValue),
          alt_mode: ALT_MODE_AGL,
        });
      }
    } catch (e) {
      console.error('[terrain] apply correction failed', e);
    } finally {
      applying = false;
    }
  }

  function setCorrectionMode(m: 'follow' | 'check') {
    patchTerrainAnalysis({ correctionMode: m });
  }

  /** Interpolated mission-path MSL altitude at a distance (to place a WP on the track). */
  function pathAltAtDist(d: number): number | null {
    if (!data || data.markers.length === 0) return null;
    const m = data.markers;
    if (d <= m[0].dist) return m[0].altMsl;
    if (d >= m[m.length - 1].dist) return m[m.length - 1].altMsl;
    for (let i = 1; i < m.length; i++) {
      if (d <= m[i].dist) {
        const a = m[i - 1];
        const b = m[i];
        const t = (d - a.dist) / (b.dist - a.dist || 1);
        return a.altMsl + (b.altMsl - a.altMsl) * t;
      }
    }
    return m[m.length - 1].altMsl;
  }

  // Manually add a waypoint at the pinned marker — exactly on the current track.
  // Then the user re-runs Terrain Follow to adjust it.
  async function addWaypointAtMarker() {
    const p = get(terrainCursor).placed;
    if (!p || !data || placedDist == null) return;
    if (getTotalWpCount() >= MAX_WAYPOINTS_TOTAL) {
      if (confirm) await confirm({ title: $t('terrain.addWp'), message: $t('terrain.warnWpLimit') });
      return;
    }
    const m = data.markers;
    let leftIdx = -1;
    for (let i = 0; i < m.length - 1; i++) {
      if (placedDist >= m[i].dist && placedDist <= m[i + 1].dist) {
        leftIdx = i;
        break;
      }
    }
    if (leftIdx < 0) return;
    const afterIndex = m[leftIdx].index;
    const altMsl = pathAltAtDist(placedDist) ?? m[leftIdx].altMsl;
    await missionInsertWp(
      afterIndex + 1,
      WpAction.Waypoint,
      fromDeg(p.lat),
      fromDeg(p.lon),
      altFromM(altMsl),
      0,
      0,
      0,
      ALT_MODE_AMSL,
    );
    clearTerrainPlaced();
  }

  // Terrain availability: any defined elevation sample?
  const terrainAvailable = $derived(
    data ? data.terrain.some((s) => s.elev != null) : true,
  );
</script>

<div class="overlay" class:compact={$terrainAnalysis.compact}>
  <!-- Header -->
  <div class="header">
    <div class="title">⛰ {$t('terrain.title')}</div>

    <button
      class="map-toggle"
      class:active={$terrainAnalysis.compact}
      onclick={toggleCompact}
      title={$t('terrain.showMapHint')}
    >
      🗺 {$t('terrain.showMap')}
    </button>

    <div class="seg">
      <button class:active={$terrainAnalysis.viewMode === 'waypoint'} onclick={() => setMode('waypoint')}>
        {$t('terrain.waypointMode')}
      </button>
      <button class:active={$terrainAnalysis.viewMode === 'track'} onclick={() => setMode('track')}>
        {$t('terrain.trackMode')}
      </button>
    </div>

    <div class="seg">
      <button class:active={$terrainAnalysis.datum === 'msl'} onclick={() => setDatum('msl')}>
        {$t('terrain.datumMsl')}
      </button>
      <button class:active={$terrainAnalysis.datum === 'agl'} onclick={() => setDatum('agl')}>
        {$t('terrain.datumAgl')}
      </button>
    </div>

    {#if live}
      <button
        class="map-toggle"
        class:active={$terrainAnalysis.follow}
        onclick={toggleFollow}
        title={$t('terrain.followHint')}
      >
        ⇥ {$t('terrain.follow')}
      </button>
    {/if}

    <button class="ghost-btn" onclick={resetTerrainView} title={$t('terrain.resetView')}>⟲</button>

    <div class="spacer"></div>
  </div>

  <!-- Body -->
  <div class="body">
    <!-- Controls -->
    <div class="controls">
      <div class="ctrl">
        <span>{$t('terrain.groundClearance')}</span>
        <UnitStepper
          bind:value={groundClearance}
          kind="altitude"
          settings={interfaceSettings}
          min={0}
          step={5}
          decimals={0}
          onchange={onClearanceChange}
        />
      </div>

      <div class="legend">
        <span class="lg"><i class="sw path"></i>{$t('terrain.path')}</span>
        <span class="lg"><i class="sw terrain"></i>{$t('terrain.terrain')}</span>
        <span class="lg"><i class="sw floor"></i>{$t('terrain.clearanceFloor')}</span>
        <span class="lg"><i class="sw unsafe"></i>{$t('terrain.unsafe')}</span>
      </div>

      {#if showCorrection}
        <div class="correction">
          <label class="corr-head">
            <input
              type="checkbox"
              checked={$terrainAnalysis.correctionEnabled}
              onchange={toggleCorrection}
            />
            {$t('terrain.correction')}
          </label>

          {#if $terrainAnalysis.correctionEnabled}
            <div class="seg corr-mode">
              <button
                class:active={$terrainAnalysis.correctionMode === 'follow'}
                onclick={() => setCorrectionMode('follow')}
              >
                {$t('terrain.terrainFollow')}
              </button>
              <button
                class:active={$terrainAnalysis.correctionMode === 'check'}
                onclick={() => setCorrectionMode('check')}
              >
                {$t('terrain.clearanceCheck')}
              </button>
            </div>

            <div class="ctrl">
              <span>{$t('terrain.range')}</span>
              <div class="range-row">
                <NumberStepper bind:value={$terrainAnalysis.rangeStart} min={1} max={maxWpNumber} step={1} decimals={0} />
                <span class="dash">–</span>
                <NumberStepper bind:value={$terrainAnalysis.rangeEnd} min={1} max={maxWpNumber} step={1} decimals={0} />
              </div>
            </div>

            <label class="corr-check">
              <input type="checkbox" bind:checked={$terrainAnalysis.fixedWing} />
              {$t('terrain.fixedWing')}
            </label>
            {#if $terrainAnalysis.fixedWing}
              <div class="ctrl">
                <span>{$t('terrain.climbAngle')}</span>
                <NumberStepper bind:value={$terrainAnalysis.climbAngleLimit} min={0} max={89} step={1} decimals={0} unit="°" />
              </div>
              <div class="ctrl">
                <span>{$t('terrain.descentAngle')}</span>
                <NumberStepper bind:value={$terrainAnalysis.descentAngleLimit} min={0} max={89} step={1} decimals={0} unit="°" />
              </div>
            {/if}

            <div class="addwp-row">
              <button class="addwp-btn" disabled={!canAddWp} onclick={addWaypointAtMarker}>
                ＋ {$t('terrain.addWp')}
              </button>
              {#if !canAddWp}<span class="addwp-hint">{$t('terrain.addWpHint')}</span>{/if}
            </div>

            {#if correction}
              <div class="corr-stats">
                <span>{$t('terrain.changed')}: <b>{correction.changedCount}</b></span>
                <span>{$t('terrain.minClearance')}: <b>{correction.minClearanceAfter != null ? fmtAlt(correction.minClearanceAfter) : '—'}</b></span>
              </div>
              {#if correction.climbForcedAboveClearance}<p class="corr-warn">{$t('terrain.warnClimbForced')}</p>{/if}
              {#if correction.unresolvableLeg}<p class="corr-warn">{$t('terrain.warnUnresolvable')}</p>{/if}
              <button
                class="apply-btn"
                disabled={applying || correction.changedCount === 0}
                onclick={applyCorrection}
              >
                {$t('terrain.apply')}
              </button>
            {/if}
          {/if}
        </div>
      {/if}

      <p class="hint">{$t('terrain.zoomHint')}</p>
    </div>

    <!-- Chart -->
    <div class="chart-area">
      {#if loading}
        <div class="state">{$t('terrain.loading')}</div>
      {:else if data && terrainAvailable}
        <TerrainProfileChart
          {data}
          datum={$terrainAnalysis.datum}
          settings={interfaceSettings}
          live={liveActive}
          follow={$terrainAnalysis.follow}
          groundClearance={$terrainAnalysis.groundClearance}
          {warnThreshold}
          activeStartDist={activeRange.startDist}
          activeEndDist={activeRange.endDist}
          {placedDist}
          {previewPath}
          bind:viewStart={$terrainAnalysis.viewStart}
          bind:viewEnd={$terrainAnalysis.viewEnd}
          onhover={onChartHover}
          onpick={onChartPick}
        />
      {:else if data && !terrainAvailable}
        <div class="state warn">{$t('terrain.terrainUnavailable')}</div>
      {:else if errorKind === 'noTrack'}
        <div class="state">{$t('terrain.noTrack')}</div>
      {:else}
        <div class="state">{$t('terrain.insufficientWps')}</div>
      {/if}
    </div>
  </div>

  <!-- Readouts -->
  <div class="readouts">
    <div class="readout" class:warn={belowClearance}>
      <span class="rk">{$t('terrain.minClearance')}</span>
      <span class="rv">
        {#if activeRange.min != null}
          {fmtAlt(activeRange.min)}{#if belowClearance}&nbsp;⚠{/if}
        {:else}—{/if}
      </span>
    </div>
    <div class="readout">
      <span class="rk">{$t('terrain.maxClimb')}</span>
      <span class="rv">{data?.maxClimbAngle != null ? `${data.maxClimbAngle.toFixed(1)}°` : '—'}</span>
    </div>
    <div class="readout">
      <span class="rk">{$t('terrain.distance')}</span>
      <span class="rv">{data ? fmtDist(data.totalDist) : '—'}</span>
    </div>

    <div class="spacer"></div>

    {#if hover}
      <div class="readout cursor">
        <span class="rk">{$t('terrain.distance')}</span>
        <span class="rv">{fmtDist(hover.dist)}</span>
      </div>
      <div class="readout cursor">
        <span class="rk">{$t('terrain.terrain')}</span>
        <span class="rv">{hover.terrainElev != null ? fmtAlt(hover.terrainElev) : '—'}</span>
      </div>
      <div class="readout cursor">
        <span class="rk">{$t('terrain.altitude')}</span>
        <span class="rv">{hover.pathAlt != null ? fmtAlt(hover.pathAlt) : '—'}</span>
      </div>
      <div class="readout cursor">
        <span class="rk">{$t('terrain.clearance')}</span>
        <span class="rv">{hover.clearance != null ? fmtAlt(hover.clearance) : '—'}</span>
      </div>
    {/if}
  </div>
</div>

<style>
  .overlay {
    position: absolute;
    /* Symmetric inset: leave the same free margin on the right/bottom as the
       nav rail occupies on the left, so the map + widgets stay visible around it.
       Driven by height/right (not bottom) so full↔compact can animate. */
    top: 62px;
    left: 62px;
    right: 62px;
    height: calc(100% - 124px);
    z-index: 160;
    transition: height 0.25s ease, right 0.25s ease;
    display: flex;
    flex-direction: column;
    background: rgba(40, 40, 40, 0.96);
    border: 1px solid rgba(55, 168, 219, 0.4);
    border-radius: 10px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.55);
    backdrop-filter: blur(14px);
    overflow: hidden;
    animation: overlay-in 0.18s ease-out;
  }
  /* Compact: short top-docked strip; map stays visible above & to the right
     (right edge stops before the side widget dock so widgets aren't covered) */
  .overlay.compact {
    height: max(20vh, 160px);
    right: calc(var(--grid-side-width) + 54px + 6px);
  }
  .overlay.compact .hint {
    display: none;
  }

  @keyframes overlay-in {
    from {
      opacity: 0;
      transform: scale(0.99);
    }
    to {
      opacity: 1;
      transform: scale(1);
    }
  }

  .map-toggle {
    background: rgba(46, 46, 46, 0.6);
    border: 1px solid rgba(55, 168, 219, 0.3);
    border-radius: 6px;
    color: #b8b8b8;
    font-size: 12px;
    padding: 5px 10px;
    cursor: pointer;
    white-space: nowrap;
    transition: background-color 0.15s, color 0.15s, border-color 0.15s;
  }
  .map-toggle:hover {
    background: rgba(55, 168, 219, 0.15);
    color: #e0e0e0;
  }
  .map-toggle.active {
    background: rgba(55, 168, 219, 0.28);
    color: #37a8db;
    border-color: #37a8db;
  }

  .header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    flex: 0 0 auto;
  }
  .title {
    font-size: 14px;
    font-weight: 600;
    color: #37a8db;
  }
  .spacer {
    flex: 1;
  }

  .seg {
    display: flex;
    border: 1px solid rgba(55, 168, 219, 0.3);
    border-radius: 6px;
    overflow: hidden;
  }
  .seg button {
    background: rgba(46, 46, 46, 0.6);
    border: none;
    color: #949494;
    font-size: 12px;
    padding: 5px 12px;
    cursor: pointer;
    transition: background-color 0.15s, color 0.15s;
  }
  .seg button:hover {
    color: #e0e0e0;
    background: rgba(55, 168, 219, 0.15);
  }
  .seg button.active {
    background: rgba(55, 168, 219, 0.28);
    color: #37a8db;
  }

  .ghost-btn {
    width: 30px;
    height: 28px;
    background: rgba(46, 46, 46, 0.6);
    border: 1px solid rgba(55, 168, 219, 0.3);
    border-radius: 6px;
    color: #b8b8b8;
    cursor: pointer;
    font-size: 14px;
    transition: background-color 0.15s, color 0.15s;
  }
  .ghost-btn:hover {
    background: rgba(55, 168, 219, 0.2);
    color: #e0e0e0;
  }

  .body {
    flex: 1;
    min-height: 0;
    display: flex;
  }

  .controls {
    flex: 0 0 200px;
    padding: 12px;
    border-right: 1px solid rgba(255, 255, 255, 0.08);
    display: flex;
    flex-direction: column;
    gap: 16px;
    overflow-y: auto;
  }
  .ctrl {
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: 12px;
    color: #b8b8b8;
  }

  .legend {
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: 11px;
    color: #949494;
  }
  .lg {
    display: flex;
    align-items: center;
    gap: 7px;
  }
  .sw {
    width: 14px;
    height: 3px;
    border-radius: 2px;
    flex: 0 0 auto;
  }
  .sw.path {
    background: #37a8db;
  }
  .sw.terrain {
    height: 10px;
    background: #6e5b46;
  }
  .sw.floor {
    height: 0;
    border-top: 2px dashed rgba(231, 76, 60, 0.7);
  }
  .sw.unsafe {
    background: #e74c3c;
  }
  .hint {
    margin: 0;
    font-size: 10px;
    color: #6e6e6e;
    line-height: 1.4;
  }

  /* Terrain Correction sub-panel */
  .correction {
    display: flex;
    flex-direction: column;
    gap: 9px;
    border-top: 1px solid rgba(255, 255, 255, 0.08);
    padding-top: 11px;
  }
  .corr-head {
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: 12px;
    font-weight: 600;
    color: #cdd6db;
    cursor: pointer;
  }
  .corr-check {
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: 12px;
    color: #b8b8b8;
    cursor: pointer;
  }
  .range-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .range-row .dash {
    color: #6e6e6e;
  }
  .corr-stats {
    display: flex;
    flex-direction: column;
    gap: 3px;
    font-size: 11px;
    color: #949494;
  }
  .corr-stats b {
    color: #2ecc71;
  }
  .corr-warn {
    margin: 0;
    font-size: 11px;
    color: #e0a030;
    line-height: 1.35;
  }
  .addwp-row {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .addwp-btn {
    background: rgba(55, 168, 219, 0.15);
    border: 1px solid rgba(55, 168, 219, 0.5);
    color: #37a8db;
    border-radius: 6px;
    padding: 6px 10px;
    font-size: 12px;
    cursor: pointer;
    transition: background-color 0.15s;
  }
  .addwp-btn:hover:not(:disabled) {
    background: rgba(55, 168, 219, 0.28);
  }
  .addwp-btn:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .addwp-hint {
    font-size: 10px;
    color: #6e6e6e;
    line-height: 1.3;
  }
  .apply-btn {
    margin-top: 2px;
    background: rgba(46, 204, 113, 0.2);
    border: 1px solid #2ecc71;
    color: #2ecc71;
    border-radius: 6px;
    padding: 7px 10px;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    transition: background-color 0.15s;
  }
  .apply-btn:hover:not(:disabled) {
    background: rgba(46, 204, 113, 0.32);
  }
  .apply-btn:disabled {
    opacity: 0.45;
    cursor: default;
  }

  .chart-area {
    flex: 1;
    min-width: 0;
    position: relative;
    padding: 6px;
  }
  .state {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #949494;
    font-size: 13px;
  }
  .state.warn {
    color: #e0a030;
  }

  .readouts {
    display: flex;
    align-items: center;
    gap: 18px;
    padding: 7px 14px;
    border-top: 1px solid rgba(255, 255, 255, 0.08);
    flex: 0 0 auto;
    font-size: 12px;
  }
  .readout {
    display: flex;
    align-items: baseline;
    gap: 6px;
  }
  .readout .rk {
    color: #6e6e6e;
  }
  .readout .rv {
    color: #e0e0e0;
    font-variant-numeric: tabular-nums;
  }
  .readout.warn .rv {
    color: #e74c3c;
    font-weight: 600;
  }
  .readout.cursor .rv {
    color: #9fd4ec;
  }
</style>
