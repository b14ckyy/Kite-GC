<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

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
  import { invoke } from '@tauri-apps/api/core';
  import {
    mission,
    launchPoint,
    missionUpdateWp,
    missionInsertWp,
    beginUndoGroup,
    endUndoGroup,
    getTotalWpCount,
    MAX_WAYPOINTS_TOTAL,
    WpAction,
    ALT_MODE_AGL,
    ALT_MODE_AMSL,
    altFromM,
    fromDeg,
  } from '$lib/stores/mission';
  import { autopilotSystem } from '$lib/stores/autopilotContext';
  import { arduMission, MAV_CMD_NAV_TAKEOFF } from '$lib/stores/missionArdupilot';
  import { arduToInav } from '$lib/helpers/missionConverter';
  import { homePosition } from '$lib/stores/home';
  import {
    terrainAnalysis,
    terrainCursor,
    patchTerrainAnalysis,
    resetTerrainView,
    setTerrainHover,
    toggleTerrainPlaced,
    clearTerrainHover,
    clearTerrainPlaced,
    setTerrainRfRays,
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
  import { computeRfField } from '$lib/helpers/rfLink';
  import TerrainProfileChart, { type HoverInfo } from './TerrainProfileChart.svelte';
  import NumberStepper from '$lib/components/NumberStepper.svelte';
  import UnitStepper from '$lib/components/UnitStepper.svelte';
  import PanelShell from '$lib/components/panel/PanelShell.svelte';
  import Button from '$lib/components/panel/Button.svelte';
  import SegmentedToggle from '$lib/components/panel/SegmentedToggle.svelte';
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

  // Autopilot-aware mission source. INAV uses its `mission` store + the planning launch point; ArduPilot/
  // PX4 missions live in the separate `arduMission` store, converted to the shared Waypoint shape (frame
  // → alt_mode) with the FC home as the launch reference. Editing (correction APPLY / Add WP) stays
  // INAV-only for now — those write via the INAV mission store; the profile DISPLAY works for all.
  const isInav = $derived($autopilotSystem === 'inav');
  const effWaypoints = $derived(isInav ? $mission.waypoints : arduToInav($arduMission));
  const effLaunch = $derived.by<{ lat: number; lng: number } | null>(() => {
    if (isInav) return $launchPoint ? { lat: $launchPoint.lat, lng: $launchPoint.lng } : null;
    const h = $homePosition;
    return h.set ? { lat: h.lat, lng: h.lon } : null; // ArduPilot relative alt is home-referenced
  });

  // RF obstacle analysis needs a ground origin (the GCS antenna ≈ launch/home). When no FC home is set
  // (offline ArduPilot planning), fall back to a NAV_TAKEOFF waypoint's location; if neither exists we
  // can't anchor the rays → show a hint instead of guessing (a wrong origin gives misleading shadows).
  const arduTakeoff = $derived.by<{ lat: number; lng: number } | null>(() => {
    if (isInav) return null;
    const tk = $arduMission.find((w) => w.command === MAV_CMD_NAV_TAKEOFF && (w.lat !== 0 || w.lon !== 0));
    return tk ? { lat: tk.lat / 1e7, lng: tk.lon / 1e7 } : null;
  });
  const rfWpOrigin = $derived(effLaunch ?? arduTakeoff);

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
    const wps = effWaypoints;
    const lp = effLaunch;
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

  // Drop the transient hover + RF overlay when the panel closes (the pinned marker stays)
  onDestroy(() => {
    clearTerrainHover();
    setTerrainRfRays([]);
  });

  // Ground Clearance via the standard stepper (5 m steps, 1 m manual precision).
  let groundClearance = $state($terrainAnalysis.groundClearance);
  function onClearanceChange() {
    patchTerrainAnalysis({ groundClearance: Math.max(0, groundClearance) });
  }

  // Narrow derived of just the ground-clearance value. Reading `$terrainAnalysis.groundClearance`
  // directly inside the O(n) `activeRange`/`warnThreshold` would couple them to the whole store, so
  // they'd recompute on every zoom/pan (which writes viewStart/viewEnd into the store). Depending on
  // this memoised value instead means they only recompute when the clearance actually changes.
  const gcVal = $derived($terrainAnalysis.groundClearance);

  // Clearance analysis ignores the take-off climb-out and landing descent:
  // skip the leading/trailing runs that sit below clearance (we start/land on
  // the ground), so only the en-route portion drives the min-clearance alert.
  const activeRange = $derived.by(() => {
    const gc = gcVal;
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
  const warnThreshold = $derived(gcVal * 0.95);
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
  // Editing (Terrain Correction + Add WP) writes via the INAV mission store → INAV only for now. The
  // profile/clearance/RF DISPLAY still works for ArduPilot/PX4 missions.
  const showCorrection = $derived(isInav && $terrainAnalysis.viewMode === 'waypoint' && !$terrainAnalysis.compact);
  const canAddWp = $derived(isInav && $terrainCursor.placed != null && placedDist != null && data != null);

  // Combined 3-way correction control: 'off' disables it, 'follow'/'check' enable + set the mode
  // (and a default WP range on first enable). Default off.
  const correctionValue = $derived<'off' | 'follow' | 'check'>(
    $terrainAnalysis.correctionEnabled ? $terrainAnalysis.correctionMode : 'off',
  );
  function setCorrection(v: 'off' | 'follow' | 'check') {
    if (v === 'off') {
      patchTerrainAnalysis({ correctionEnabled: false });
      return;
    }
    const st = get(terrainAnalysis);
    patchTerrainAnalysis({
      correctionEnabled: true,
      correctionMode: v,
      rangeStart: st.rangeStart > 0 ? st.rangeStart : 1,
      rangeEnd: st.rangeEnd > 0 ? st.rangeEnd : maxWpNumber,
    });
  }

  let applying = $state(false);
  async function applyCorrection() {
    if (!isInav) return; // editing writes via the INAV mission store (ArduPilot edit path is a follow-up)
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
    beginUndoGroup(); // whole terrain correction = one undo step
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
      endUndoGroup();
      applying = false;
    }
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
    if (!isInav) return; // INAV mission store only (ArduPilot edit path is a follow-up)
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

  // The track carries usable RSSI (Track mode + at least one non-null, non-zero sample). "Permanent 0"
  // (field never populated) counts as unavailable → the toggle is disabled.
  const rssiAvailable = $derived(
    !!data && data.source === 'track' && data.rssi.some((v) => v != null && v !== 0),
  );
  // The RSSI line is shown when available and the toggle is on. Independent of the analysis methods.
  const rssiShown = $derived(rssiAvailable && $terrainAnalysis.rfShowRssi);

  function toggleRssi() {
    patchTerrainAnalysis({ rfShowRssi: !$terrainAnalysis.rfShowRssi });
  }

  // ── RF link / radio-shadow field (background rainbow + LOS-clearance line) ──────────
  let rfDb = $state<(number | null)[] | null>(null);
  let losClearance = $state<(number | null)[] | null>(null);
  let rfToken = 0;

  /** Home (GCS) reference for the radial RF analysis. In Track mode this is the flight's actual
   *  take-off — the track's first fix — NOT the mission launch point (which is a planning artifact
   *  and may be stale from a previously-edited mission). In Waypoint mode it is the launch point. */
  function rfHomeLatLon(
    mode: 'waypoint' | 'track',
    lp: { lat: number; lng: number } | null,
    trk: TrackPoint[],
  ): { lat: number; lon: number } | null {
    const firstFix = () => {
      for (const p of trk) if (p.lat != null && p.lon != null) return { lat: p.lat, lon: p.lon };
      return null;
    };
    if (mode === 'track') return firstFix();
    if (lp) return { lat: lp.lat, lon: lp.lng }; // INAV launch point / FC home / ArduPilot takeoff WP
    return firstFix();
  }

  // RF rays enabled in waypoint mode but no origin → tell the user how to anchor it.
  const rfNoOrigin = $derived(
    $terrainAnalysis.viewMode === 'waypoint' &&
    ($terrainAnalysis.rfLos || $terrainAnalysis.rfFresnel || $terrainAnalysis.rfTworay) &&
    !rfWpOrigin,
  );

  function setRfBand(band: '5800' | '2400' | '900' | '433') {
    patchTerrainAnalysis({ rfBand: band });
  }

  // Clutter/vegetation offset (m) added to terrain in the RF obstacle analysis.
  let rfClutter = $state($terrainAnalysis.rfClutterM);
  function onClutterChange() {
    patchTerrainAnalysis({ rfClutterM: Math.max(0, rfClutter) });
  }

  $effect(() => {
    const st = $terrainAnalysis;
    const d = data;
    const lp = rfWpOrigin; // INAV launch point / FC home / ArduPilot takeoff WP (autopilot-aware)
    const trk = track;
    // Bump the token on EVERY change (before any early return) so a still-in-flight compute from a
    // previous run can never apply after the inputs changed — e.g. toggling the last method off must
    // not let a pending result resurrect a stale rainbow.
    const token = ++rfToken;
    const anyRf = st.rfLos || st.rfFresnel || st.rfTworay;
    if (!st.open || !d || !anyRf) {
      rfDb = null;
      losClearance = null;
      setTerrainRfRays([]);
      return;
    }
    const home = rfHomeLatLon(st.viewMode, lp, trk);
    if (!home) {
      rfDb = null;
      losClearance = null;
      setTerrainRfRays([]);
      return;
    }
    const opts = { band: st.rfBand, los: st.rfLos, fresnel: st.rfFresnel, tworay: st.rfTworay, clutterM: st.rfClutterM };
    const timer = setTimeout(async () => {
      try {
        const ground = await invoke<number | null>('terrain_elevation', { lat: home.lat, lon: home.lon });
        const field = await computeRfField(d, { lat: home.lat, lon: home.lon, ground: ground ?? 0 }, opts);
        if (token === rfToken) {
          rfDb = field.db;
          losClearance = field.losClearance;
          setTerrainRfRays(field.rays);
        }
      } catch (e) {
        console.error('[terrain] RF field computation failed', e);
        if (token === rfToken) {
          rfDb = null;
          losClearance = null;
          setTerrainRfRays([]);
        }
      }
    }, 300);
    return () => clearTimeout(timer);
  });
</script>

{#snippet headerActions()}
  <Button variant="mode" active={$terrainAnalysis.compact} icon="map" onclick={toggleCompact} title={$t('terrain.showMapHint')}>
    {$t('terrain.showMap')}
  </Button>
  <SegmentedToggle
    options={[{ value: 'waypoint', label: $t('terrain.waypointMode') }, { value: 'track', label: $t('terrain.trackMode') }]}
    value={$terrainAnalysis.viewMode}
    onchange={(v) => setMode(v as 'waypoint' | 'track')}
  />
  <SegmentedToggle
    options={[{ value: 'msl', label: $t('terrain.datumMsl') }, { value: 'agl', label: $t('terrain.datumAgl') }]}
    value={$terrainAnalysis.datum}
    onchange={(v) => setDatum(v as 'msl' | 'agl')}
  />
  {#if live}
    <Button variant="mode" active={$terrainAnalysis.follow} onclick={toggleFollow} title={$t('terrain.followHint')}>
      {$t('terrain.follow')}
    </Button>
  {/if}
  <Button variant="standard" icon="refresh" onclick={resetTerrainView} title={$t('terrain.resetView')} />
{/snippet}

{#snippet params()}
  <div class="controls">
    <div class="ctrl" class:ctrl-row={$terrainAnalysis.compact}>
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
        {#if rssiShown}<span class="lg"><i class="sw rssi"></i>{$t('terrain.rssi')}</span>{/if}
      </div>

      {#if !$terrainAnalysis.compact}
      <div class="rf-section">
        <span class="rf-head">{$t('terrain.rfTitle')}</span>
        <div class="rf-methods">
          <Button variant="mode" active={$terrainAnalysis.rfLos} onclick={() => patchTerrainAnalysis({ rfLos: !$terrainAnalysis.rfLos })} title={$t('terrain.rfLosHint')}>
            {$t('terrain.rfLos')}
          </Button>
          <Button variant="mode" active={$terrainAnalysis.rfFresnel} onclick={() => patchTerrainAnalysis({ rfFresnel: !$terrainAnalysis.rfFresnel })} title={$t('terrain.rfFresnelHint')}>
            {$t('terrain.rfFresnel')}
          </Button>
          <Button variant="mode" active={$terrainAnalysis.rfTworay} onclick={() => patchTerrainAnalysis({ rfTworay: !$terrainAnalysis.rfTworay })} title={$t('terrain.rfTworayHint')}>
            {$t('terrain.rfTworay')}
          </Button>
        </div>
        <SegmentedToggle
          full
          options={[
            { value: '5800', label: '5.8G' },
            { value: '2400', label: '2.4G' },
            { value: '900', label: '900M' },
            { value: '433', label: '433M' },
          ]}
          value={$terrainAnalysis.rfBand}
          onchange={(v) => setRfBand(v as '5800' | '2400' | '900' | '433')}
        />
        <div class="rf-methods">
          <Button
            variant="mode"
            active={rssiAvailable && $terrainAnalysis.rfShowRssi}
            disabled={!rssiAvailable}
            onclick={toggleRssi}
            title={$t('terrain.rfRssiHint')}
          >
            {$t('terrain.rssi')}
          </Button>
        </div>
        <div class="ctrl">
          <span>{$t('terrain.rfClutter')}</span>
          <UnitStepper
            bind:value={rfClutter}
            kind="altitude"
            settings={interfaceSettings}
            min={0}
            step={5}
            decimals={0}
            onchange={onClutterChange}
          />
        </div>
        {#if $terrainAnalysis.rfLos && $terrainAnalysis.rfFresnel}
          <p class="rf-note">{$t('terrain.rfLosIgnored')}</p>
        {/if}
        {#if $terrainAnalysis.datum === 'agl'}
          <p class="rf-note">{$t('terrain.rfRainbowMsl')}</p>
        {/if}
        {#if rfNoOrigin}
          <p class="rf-note rf-warn">{$t('terrain.rfNoOrigin')}</p>
        {/if}
        <p class="rf-note">{$t('terrain.rfDisclaimer')}</p>
      </div>
      {/if}

      {#if showCorrection}
        <div class="correction">
          <span class="rf-head">{$t('terrain.correction')}</span>
          <SegmentedToggle
            full
            options={[
              { value: 'off', label: $t('terrain.corrOff') },
              { value: 'follow', label: $t('terrain.corrFollow') },
              { value: 'check', label: $t('terrain.corrCheck') },
            ]}
            value={correctionValue}
            onchange={(v) => setCorrection(v as 'off' | 'follow' | 'check')}
          />

          {#if $terrainAnalysis.correctionEnabled}
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
              <div class="ctrl-2col">
                <div class="ctrl">
                  <span>{$t('terrain.climbAngle')}</span>
                  <NumberStepper bind:value={$terrainAnalysis.climbAngleLimit} min={0} max={89} step={1} decimals={0} unit="°" />
                </div>
                <div class="ctrl">
                  <span>{$t('terrain.descentAngle')}</span>
                  <NumberStepper bind:value={$terrainAnalysis.descentAngleLimit} min={0} max={89} step={1} decimals={0} unit="°" />
                </div>
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
              {#if correction.changedCount > 0 && correction.climbForcedAboveClearance}<p class="corr-warn">{$t('terrain.warnClimbForced')}</p>{/if}
              {#if correction.changedCount > 0 && correction.unresolvableLeg}<p class="corr-warn">{$t('terrain.warnUnresolvable')}</p>{/if}
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

      {#if !$terrainAnalysis.compact}<p class="hint">{$t('terrain.zoomHint')}</p>{/if}
    </div>
  {/snippet}

  {#snippet body()}
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
          groundClearance={gcVal}
          {warnThreshold}
          activeStartDist={activeRange.startDist}
          activeEndDist={activeRange.endDist}
          {placedDist}
          {previewPath}
          {rfDb}
          {losClearance}
          rssi={rssiShown ? (data?.rssi ?? null) : null}
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
  {/snippet}

  {#snippet footer()}
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
  {/snippet}

  <PanelShell
    variant={$terrainAnalysis.compact ? 'wide-compact' : 'fullscreen'}
    title={$t('terrain.title')}
    {headerActions}
    {params}
    {body}
    {footer}
  />

<style>
  .spacer {
    flex: 1;
  }

  /* Params slot content (PanelShell .ps-params provides width/padding/border/scroll). */
  .controls {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .ctrl {
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: 12px;
    color: #b8b8b8;
  }
  /* Compact (wide) mode: label + stepper on one row to save vertical space so the params
     column needs no scrollbar (correction + hint are hidden in compact). */
  .ctrl.ctrl-row {
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  /* Climb + Descent angle side by side (there's room in the params column). */
  .ctrl-2col {
    display: flex;
    gap: 10px;
  }
  .ctrl-2col > .ctrl {
    flex: 1;
    min-width: 0;
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
  .sw.rssi {
    background: #e0508a;
  }
  .hint {
    margin: 0;
    font-size: 10px;
    color: #6e6e6e;
    line-height: 1.4;
  }

  /* RF link / radio-shadow analysis section */
  .rf-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
    border-top: 1px solid rgba(255, 255, 255, 0.08);
    padding-top: 11px;
  }
  .rf-head {
    font-size: 12px;
    font-weight: 600;
    color: #cdd6db;
  }
  .rf-methods {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .rf-note {
    margin: 0;
    font-size: 10px;
    color: #6e6e6e;
    line-height: 1.4;
  }
  .rf-warn {
    color: #f5a623;
  }

  /* Terrain Correction sub-panel */
  .correction {
    display: flex;
    flex-direction: column;
    gap: 9px;
    border-top: 1px solid rgba(255, 255, 255, 0.08);
    padding-top: 11px;
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
    position: relative;
    height: 100%;
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

  /* Footer slot content (PanelShell .ps-fs-foot provides the top border). */
  .readouts {
    display: flex;
    align-items: center;
    gap: 18px;
    padding: 7px 14px;
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
