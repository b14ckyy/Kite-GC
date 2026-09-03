<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  import { untrack } from 'svelte';
  import { t } from 'svelte-i18n';
  import type { Flight, TelemetryRecord } from '$lib/stores/flightlog';
  import { getUsedFlightModes, segmentTrackByAltitude, segmentTrackBySpeed, segmentTrackBySignal, type TrackColorMode, type FlightModeInfo, type GradientResult } from '$lib/helpers/trackColors';
  import type { UavModelOverride } from '$lib/helpers/uavIcons';
  import { showMission, geoWaypoints } from '$lib/stores/mission';
  import StickOverlay from '$lib/components/sticks/StickOverlay.svelte';
  import { computeStickData } from '$lib/helpers/stickInput';
  import SegmentedToggle, { type SegOption } from '$lib/components/panel/SegmentedToggle.svelte';

  let {
    showPlayer = false,
    selectedFlight = null,
    playbackPlaying = false,
    playbackSpeed = 1,
    playbackCurrentMs = 0,
    playbackTotalMs = 0,
    trackLength = 0,
    playbackIndex = 0,
    onClose = () => {},
    onSeekToStart = () => {},
    onSeek = (_deltaMs: number) => {},
    onTogglePlayPause = () => {},
    onCycleSpeed = () => {},
    onScrub = (_index: number) => {},
    onScrubStart = () => {},
    onScrubEnd = () => {},
    trackColorMode = 'flightmode' as TrackColorMode,
    onTrackColorModeChange = (_mode: TrackColorMode) => {},
    modelOverride = 'auto' as UavModelOverride,
    onModelOverrideChange = (_v: UavModelOverride) => {},
    playbackTrack = [] as TelemetryRecord[],
    warnAltitudeM = 120,
    replaySource = 'live' as 'live' | 'blackbox',
    hasLinkedPartner = false,
    onSwitchSource = (_source: 'live' | 'blackbox') => {},
    hiresAvailable = false,
    hiresActive = false,
    hiresParsing = false,
    onHiresToggle = (_active: boolean) => {},
    hiresRecord = null as TelemetryRecord | null,
  }: {
    showPlayer?: boolean;
    selectedFlight?: Flight | null;
    playbackPlaying?: boolean;
    playbackSpeed?: number;
    playbackCurrentMs?: number;
    playbackTotalMs?: number;
    trackLength?: number;
    playbackIndex?: number;
    onClose?: () => void;
    onSeekToStart?: () => void;
    onSeek?: (deltaMs: number) => void;
    onTogglePlayPause?: () => void;
    onCycleSpeed?: () => void;
    onScrub?: (index: number) => void;
    onScrubStart?: () => void;
    onScrubEnd?: () => void;
    trackColorMode?: TrackColorMode;
    onTrackColorModeChange?: (mode: TrackColorMode) => void;
    modelOverride?: UavModelOverride;
    onModelOverrideChange?: (v: UavModelOverride) => void;
    playbackTrack?: TelemetryRecord[];
    warnAltitudeM?: number;
    replaySource?: 'live' | 'blackbox';
    hasLinkedPartner?: boolean;
    onSwitchSource?: (source: 'live' | 'blackbox') => void;
    /** Hi-res replay (HIRES_REPLAY plan): the toggle only shows when an archived log is parseable. */
    hiresAvailable?: boolean;
    hiresActive?: boolean;
    hiresParsing?: boolean;
    onHiresToggle?: (active: boolean) => void;
    /** Latest full-rate sample while hi-res is on — drives the stick overlay at tick rate. */
    hiresRecord?: TelemetryRecord | null;
  } = $props();

  const COLOR_MODES: { value: TrackColorMode; labelKey: string }[] = [
    { value: 'flightmode', labelKey: 'player.trackFlightMode' },
    { value: 'altitude',   labelKey: 'player.trackAltitude' },
    { value: 'speed',      labelKey: 'player.trackSpeed' },
    { value: 'signal',     labelKey: 'player.trackSignal' },
    { value: 'none',       labelKey: 'player.trackNone' },
  ];

  const MODEL_OPTIONS: { value: UavModelOverride; labelKey: string }[] = [
    { value: 'auto',      labelKey: 'player.modelAuto' },
    { value: 'quad',      labelKey: 'player.modelQuad' },
    { value: 'tricopter', labelKey: 'player.modelTricopter' },
    { value: 'plane',     labelKey: 'player.modelPlane' },
    { value: 'vtol',      labelKey: 'player.modelVtol' },
    { value: 'generic',   labelKey: 'player.modelGeneric' },
  ];

  function handleModelChange(event: Event) {
    onModelOverrideChange((event.target as HTMLSelectElement).value as UavModelOverride);
  }

  let usedModes = $derived(
    trackColorMode === 'flightmode' ? getUsedFlightModes(playbackTrack ?? []) : []
  );

  let gradientMeta = $derived.by(() => {
    const track = playbackTrack ?? [];
    if (track.length === 0) return null;
    if (trackColorMode === 'altitude') return segmentTrackByAltitude(track, warnAltitudeM);
    if (trackColorMode === 'speed') return segmentTrackBySpeed(track);
    if (trackColorMode === 'signal') return segmentTrackBySignal(track);
    return null;
  });

  function handleColorModeChange(event: Event) {
    const value = (event.target as HTMLSelectElement).value as TrackColorMode;
    onTrackColorModeChange(value);
  }

  function formatPlaybackTime(ms: number): string {
    const totalSec = Math.floor(ms / 1000);
    const h = Math.floor(totalSec / 3600);
    const m = Math.floor((totalSec % 3600) / 60);
    const s = totalSec % 60;
    return `${String(h).padStart(2, '0')}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
  }

  // Wall-clock time-of-day at the current playback position = flight start + elapsed, shown in the
  // flight's LOCAL time (ADR-048). `start_time` is true UTC; `utc_offset_min` is the location's offset
  // (null ⇒ UTC). We add the offset to the epoch and read UTC components to get the local wall clock
  // without involving the browser timezone.
  const logClock = $derived.by(() => {
    const s = selectedFlight?.start_time;
    if (!s) return null;
    const base = new Date(s).getTime();
    if (!Number.isFinite(base)) return null;
    const offMin = selectedFlight?.utc_offset_min ?? 0;
    const d = new Date(base + playbackCurrentMs + offMin * 60_000);
    const p = (n: number) => String(n).padStart(2, '0');
    return `${p(d.getUTCHours())}:${p(d.getUTCMinutes())}:${p(d.getUTCSeconds())}`;
  });

  function handleScrub(event: Event) {
    const target = event.currentTarget as HTMLInputElement;
    onScrub(Number(target.value));
  }

  // REC/BBX as one SegmentedToggle (its own header names this switch as the intended use):
  // both live for a linked pair, otherwise the missing source is a disabled segment.
  const sourceOptions = $derived.by((): SegOption[] => {
    if (hasLinkedPartner) {
      return [
        { value: 'live', label: 'REC' },
        { value: 'blackbox', label: 'BBX' },
      ];
    }
    if (selectedFlight?.source === 'blackbox') {
      return [
        { value: 'live', label: 'REC', disabled: true },
        { value: 'blackbox', label: 'BBX' },
      ];
    }
    return [
      { value: 'live', label: 'REC' },
      { value: 'blackbox', label: 'BBX', disabled: true, title: $t('player.bbxNotAvailable') },
    ];
  });
  const sourceValue = $derived(
    hasLinkedPartner ? replaySource : selectedFlight?.source === 'blackbox' ? 'blackbox' : 'live',
  );

  const hiresOptions = $derived.by((): SegOption[] => [
    { value: 'std', label: '10 Hz', title: $t('player.hiresStandardTitle') },
    { value: 'hires', label: 'HI-RES', title: $t('player.hiresTitle') },
  ]);

  // Stick overlay (replay-only): normalize the current sample's recorded RC channels. Null when the
  // log has no RC (e.g. .tlog / live-recorded flights) → the overlay is hidden. While hi-res is on,
  // the full-rate sample drives the sticks so they move at tick rate, not 10 Hz.
  const currentRecord = $derived(
    hiresRecord ??
      (playbackTrack.length > 0
        ? playbackTrack[Math.min(playbackIndex, playbackTrack.length - 1)]
        : null),
  );
  const stickData = $derived(
    currentRecord
      ? computeStickData(currentRecord.rc_command_json, currentRecord.rc_data_json, selectedFlight?.fc_variant)
      : null,
  );

  // Measured player-bar height so the stick overlay sits flush (top + bottom) beside it. Measured
  // on the FULL panel and kept while it is collapsed (a transform doesn't change clientHeight), so
  // the sticks keep their size and position in compact mode.
  let barHeight = $state(0);

  // ── Compact mode (Dev-Docs active/REPLAY_PANEL_COMPACT.md) ────────────────────────────
  // While playback runs the full panel collapses into a non-interactive strip under the top bar,
  // so it hides as little of the picture as possible. It is expanded whenever playback is PAUSED
  // (configuring speed/colours or scrubbing must never fight a collapsing panel), while the mouse
  // is inside the zone the full panel occupies, or while a touch tap has pinned it open.
  //   • Mouse: the zone is hit-tested from the full panel's last measured rect on window
  //     pointermove — no invisible overlay, so the map under the zone stays fully usable. While a
  //     button is held (a map drag) the test is deferred; the pointerup decides.
  //   • Touch/pen (no hover): a tap inside the zone pins, a tap anywhere else unpins.
  //   Per-event pointerType, never a device flag — hybrids behave right per input.
  let hoverInZone = $state(false);
  let touchPinned = $state(false);
  const expanded = $derived(!playbackPlaying || hoverInZone || touchPinned);
  let zoneEl = $state<HTMLDivElement>();
  let zoneRect: DOMRect | null = null;

  // The zone is measured from an invisible, NON-animated sibling laid out exactly like the full
  // panel (same top/left/width, height = the panel's measured height). Measuring the panel itself
  // was wrong: it is mid-transition (still translated up) right when a hover expands it, and that
  // stale rect became a hard-to-hit sliver under the top bar.
  function measureZone() {
    if (zoneEl) zoneRect = zoneEl.getBoundingClientRect();
  }
  function inZone(x: number, y: number): boolean {
    const r = zoneRect;
    if (r == null || x < r.left || x > r.right || y < r.top || y > r.bottom) return false;
    // A surface lying OVER the zone (a side panel, a dialog, the floating video window) blocks
    // it: only the map — or the player itself — under the pointer counts. The compact strip and
    // the stick overlay are pointer-events:none, so they never show up here.
    const el = document.elementFromPoint(x, y);
    return el != null && (el.closest('.layer-map') != null || el.closest('.log-player') != null);
  }

  $effect(() => {
    if (!zoneEl) return;
    const ro = new ResizeObserver(() => measureZone());
    ro.observe(zoneEl);
    untrack(measureZone);
    return () => ro.disconnect();
  });

  $effect(() => {
    const onMove = (e: PointerEvent) => {
      if (e.pointerType !== 'mouse') return;
      measureZone();
      if (e.buttons !== 0) return; // dragging the map — decide on release
      hoverInZone = inZone(e.clientX, e.clientY);
    };
    const onUp = (e: PointerEvent) => {
      if (e.pointerType !== 'mouse') return;
      hoverInZone = inZone(e.clientX, e.clientY);
    };
    const onDown = (e: PointerEvent) => {
      if (e.pointerType === 'mouse') return;
      touchPinned = inZone(e.clientX, e.clientY);
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    window.addEventListener('pointerdown', onDown, true);
    return () => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
      window.removeEventListener('pointerdown', onDown, true);
    };
  });

  // A new flight starts expanded and unpinned.
  $effect(() => {
    void selectedFlight?.id;
    touchPinned = false;
    hoverInZone = false;
  });

  // Compact strip content: craft · type, elapsed, progress, total.
  const PLATFORM_KEYS: Record<number, string> = {
    0: 'platform.multirotor', 1: 'platform.airplane', 2: 'platform.helicopter',
    3: 'platform.tricopter', 4: 'platform.rover', 5: 'platform.boat', 6: 'platform.other',
    7: 'platform.vtol', 255: 'platform.generic',
  };
  const compactType = $derived.by(() => {
    const pt = selectedFlight?.platform_type;
    return pt != null && PLATFORM_KEYS[pt] ? $t(PLATFORM_KEYS[pt]) : '';
  });
  const progressPct = $derived(
    trackLength > 1 ? (Math.min(playbackIndex, trackLength - 1) / (trackLength - 1)) * 100 : 0,
  );
</script>

{#if showPlayer && selectedFlight}
  <!-- Hover/tap zone: invisible twin of the full panel's box, never animated, never interactive. -->
  <div class="log-player-zone" bind:this={zoneEl} style:height={barHeight ? `${barHeight + 9}px` : undefined}></div>
  <div class="log-player" class:collapsed={!expanded} bind:clientHeight={barHeight}>
    <div class="log-player-top">
      <div class="log-player-source">
        <SegmentedToggle
          size="sm"
          options={sourceOptions}
          value={sourceValue}
          onchange={(v) => onSwitchSource(v as 'live' | 'blackbox')}
        />
        {#if hiresAvailable}
          <span class="log-player-hires">
            <SegmentedToggle
              size="sm"
              options={hiresOptions}
              value={hiresActive ? 'hires' : 'std'}
              disabled={hiresParsing}
              onchange={(v) => onHiresToggle(v === 'hires')}
            />
          </span>
        {/if}
        {#if $geoWaypoints.length > 0}
          <button
            class="log-player-source-btn log-player-mission-btn"
            class:active={$showMission}
            onclick={() => showMission.update((v) => !v)}
            title={$t('player.toggleMission')}
          >MISSION</button>
        {/if}
      </div>
      <div class="log-player-title">
        {selectedFlight.craft_name || $t('logbook.unknownCraft')}
        {#if selectedFlight.fc_variant || selectedFlight.fc_version}
          <span class="log-player-firmware">- {selectedFlight.fc_variant} {selectedFlight.fc_version}</span>
        {/if}
      </div>
      {#if logClock}
        <span class="log-player-clock" title={$t('player.logTimeOfDay')}>{logClock}</span>
      {/if}
      <button class="log-player-close" onclick={onClose} title={$t('player.close')}>X</button>
    </div>

    <div class="log-player-controls">
      <span class="log-player-time">{formatPlaybackTime(playbackCurrentMs)}</span>
      <div class="log-player-buttons">
        <button class="log-player-btn" onclick={onSeekToStart} title={$t('player.toStart')}>|&lt;</button>
        <button class="log-player-btn" onclick={() => onSeek(-300000)} title="-5min">-5m</button>
        <button class="log-player-btn" onclick={() => onSeek(-60000)} title="-1min">-1m</button>
        <button class="log-player-btn" onclick={() => onSeek(-10000)} title="-10s">-10s</button>
        <button class="log-player-btn play-btn" onclick={onTogglePlayPause} title={playbackPlaying ? $t('player.pause') : $t('player.play')}>
          {playbackPlaying ? '||' : '>'}
        </button>
        <button class="log-player-btn" onclick={() => onSeek(10000)} title="+10s">+10s</button>
        <button class="log-player-btn" onclick={() => onSeek(60000)} title="+1min">+1m</button>
        <button class="log-player-btn" onclick={() => onSeek(300000)} title="+5min">+5m</button>
        <button class="log-player-btn speed-btn" onclick={onCycleSpeed} title={$t('player.speed')}>
          {playbackSpeed}x
        </button>
      </div>
      <span class="log-player-time">{formatPlaybackTime(playbackTotalMs)}</span>
    </div>

    <div class="log-player-scrubber">
      <input
        type="range"
        min="0"
        max={Math.max(trackLength - 1, 0)}
        value={playbackIndex}
        class="log-player-slider"
        oninput={handleScrub}
        onpointerdown={onScrubStart}
        onpointerup={onScrubEnd}
      />
    </div>

    <div class="log-player-bottom">
      <div class="track-color-select">
        <select value={trackColorMode} onchange={handleColorModeChange}>
          {#each COLOR_MODES as mode}
            <option value={mode.value}>{$t(mode.labelKey)}</option>
          {/each}
        </select>
      </div>
      {#if trackColorMode === 'flightmode' && usedModes.length > 0}
        <div class="track-legend">
          {#each usedModes as mode}
            <span class="legend-item">
              <span class="legend-dot" style="background:{mode.color}"></span>
              {mode.label}
            </span>
          {/each}
        </div>
      {:else if gradientMeta && (trackColorMode === 'altitude' || trackColorMode === 'speed')}
        <div class="track-legend">
          <span class="gradient-label">0{gradientMeta.unit}</span>
          <span class="gradient-bar {trackColorMode === 'altitude' ? 'altitude-bar' : 'speed-bar'}"></span>
          <span class="gradient-label">{Math.round(gradientMeta.max)}{gradientMeta.unit}</span>
        </div>
      {:else if gradientMeta && trackColorMode === 'signal'}
        <div class="track-legend">
          <span class="gradient-label">{gradientMeta.fieldLabel}</span>
          <span class="gradient-bar signal-bar"></span>
          <span class="gradient-label">{Math.round(gradientMeta.max)}</span>
        </div>
      {/if}
      <div class="model-select">
        <select value={modelOverride} onchange={handleModelChange} title={$t('player.modelTitle')}>
          {#each MODEL_OPTIONS as m}
            <option value={m.value}>{$t(m.labelKey)}</option>
          {/each}
        </select>
      </div>
    </div>
  </div>

  {#if stickData}
    <StickOverlay data={stickData} {barHeight} compact={!expanded} />
  {/if}

  <!-- Compact strip: slides out from under the top bar while the full panel is collapsed. Never
       interactive — pointer events pass through to whatever is underneath. -->
  <div class="log-player-compact" class:shown={!expanded} aria-hidden={expanded}>
    <span class="lpc-craft">
      {selectedFlight.craft_name || $t('logbook.unknownCraft')}{#if compactType}<span class="lpc-type"> · {compactType}</span>{/if}
    </span>
    <span class="lpc-time">{formatPlaybackTime(playbackCurrentMs)}</span>
    <div class="lpc-bar"><div class="lpc-fill" style="width: {progressPct}%"></div></div>
    <span class="lpc-time">{formatPlaybackTime(playbackTotalMs)}</span>
  </div>
{/if}

<style>
  .log-player {
    position: absolute;
    top: 62px;
    left: 50%;
    transform: translateX(-50%);
    width: 800px;
    max-width: calc(100vw - 40px);
    z-index: 50;
    background: rgba(46, 46, 46, 0.92);
    backdrop-filter: blur(12px);
    -webkit-backdrop-filter: blur(12px);
    border: 1px solid rgba(55, 168, 219, 0.35);
    border-radius: 8px;
    box-shadow: 0 4px 20px rgba(0, 0, 0, 0.4);
    padding: 8px 14px 6px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    user-select: none;
    transition: transform 0.2s ease, opacity 0.2s ease;
  }

  /* Starts at the top bar's bottom edge (53px), so the 9px gap above the panel counts too — that is
     exactly where the collapsed strip hangs and where users instinctively aim. */
  .log-player-zone {
    position: absolute;
    top: 53px;
    left: 50%;
    transform: translateX(-50%);
    width: 800px;
    max-width: calc(100vw - 40px);
    pointer-events: none;
    visibility: hidden;
  }

  /* Collapsed: the full panel slides up under the top bar and fades; it stays in the DOM (its
     measured height keeps the stick overlay in place) but takes no pointer events. */
  .log-player.collapsed {
    transform: translate(-50%, -120%);
    opacity: 0;
    pointer-events: none;
  }

  /* Compact strip — one readable line, framed, rounded at the bottom only (it "hangs" from the
     top bar). Starts hidden under the bar and slides down while the full panel is collapsed. */
  .log-player-compact {
    position: absolute;
    top: 53px;
    left: 50%;
    width: 700px;
    max-width: calc(100vw - 40px);
    z-index: 50;
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 7px 18px 8px;
    background: rgba(46, 46, 46, 0.92);
    backdrop-filter: blur(12px);
    -webkit-backdrop-filter: blur(12px);
    border: 1px solid rgba(55, 168, 219, 0.35);
    border-top: none;
    border-radius: 0 0 8px 8px;
    box-shadow: 0 4px 20px rgba(0, 0, 0, 0.4);
    font-size: 16px;
    color: #e0e0e0;
    user-select: none;
    pointer-events: none;
    transform: translate(-50%, -110%);
    opacity: 0;
    transition: transform 0.2s ease, opacity 0.2s ease;
  }
  .log-player-compact.shown {
    transform: translate(-50%, 0);
    opacity: 1;
  }
  .lpc-craft {
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 40%;
    flex-shrink: 1;
  }
  .lpc-type {
    font-weight: 400;
    color: #949494;
  }
  .lpc-time {
    font-family: 'JetBrains Mono', 'Fira Code', monospace;
    font-size: 15px;
    color: #949494;
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
  }
  .lpc-bar {
    flex: 1;
    height: 8px;
    background: #434343;
    border-radius: 3px;
    overflow: hidden;
  }
  .lpc-fill {
    height: 100%;
    background: #37a8db;
    border-radius: 3px;
  }

  .log-player-top {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .log-player-source {
    display: flex;
    align-items: center;
    gap: 2px;
    flex-shrink: 0;
  }

  /* Set the resolution switch apart from the REC/BBX source group. */
  .log-player-hires {
    display: inline-flex;
    margin-left: 8px;
  }

  .log-player-source-btn {
    background: #434343;
    border: 1px solid #555;
    color: #949494;
    font-size: 10px;
    font-weight: 600;
    padding: 2px 6px;
    border-radius: 3px;
    cursor: pointer;
    line-height: 1;
  }

  .log-player-source-btn.active {
    background: #37a8db;
    color: #fff;
    border-color: #339cc1;
  }

  /* Mission visibility toggle — set apart from the REC/BBX source group. */
  .log-player-mission-btn {
    margin-left: 8px;
  }
  .log-player-mission-btn.active {
    background: #16a34a;
    border-color: #15803d;
  }

  .log-player-title {
    flex: 1;
    text-align: center;
    font-size: 13px;
    font-weight: 600;
    color: #e0e0e0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .log-player-firmware {
    font-weight: 400;
    color: #949494;
    font-size: 12px;
  }

  .log-player-clock {
    font-family: 'JetBrains Mono', 'Fira Code', monospace;
    font-size: 13px;
    font-weight: 600;
    color: #37a8db;
    flex-shrink: 0;
    letter-spacing: 0.02em;
    font-variant-numeric: tabular-nums;
  }

  .log-player-close {
    background: none;
    border: none;
    color: #949494;
    font-size: 16px;
    cursor: pointer;
    padding: 0 4px;
    line-height: 1;
    flex-shrink: 0;
  }

  .log-player-close:hover {
    color: #d40000;
  }

  .log-player-controls {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .log-player-time {
    font-family: 'JetBrains Mono', 'Fira Code', monospace;
    font-size: 12px;
    color: #949494;
    min-width: 60px;
    text-align: center;
    flex-shrink: 0;
  }

  .log-player-buttons {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 2px;
    flex: 1;
  }

  .log-player-btn {
    background: #434343;
    border: 1px solid #555;
    color: #e0e0e0;
    font-size: 11px;
    padding: 4px 7px;
    border-radius: 3px;
    cursor: pointer;
    line-height: 1;
    transition: background 0.2s ease, border-color 0.2s ease;
  }

  .log-player-btn:hover {
    background: rgba(55, 168, 219, 0.15);
    border-color: #37a8db;
  }

  .log-player-btn.play-btn {
    font-size: 14px;
    padding: 4px 10px;
    background: #37a8db;
    color: #fff;
    border-color: #339cc1;
  }

  .log-player-btn.play-btn:hover {
    background: #45bce5;
  }

  .log-player-btn.speed-btn {
    font-weight: 700;
    min-width: 32px;
    text-align: center;
    color: #37a8db;
  }

  .log-player-scrubber {
    padding: 2px 0 0;
  }

  .log-player-slider {
    width: 100%;
    height: 6px;
    -webkit-appearance: none;
    appearance: none;
    background: #434343;
    border-radius: 3px;
    outline: none;
    cursor: pointer;
  }

  .log-player-slider::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #37a8db;
    border: 2px solid #e0e0e0;
    cursor: pointer;
  }

  .log-player-slider::-moz-range-thumb {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #37a8db;
    border: 2px solid #e0e0e0;
    cursor: pointer;
  }

  .log-player-bottom {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 2px 0 0;
    flex-wrap: wrap;
  }

  .track-color-select select {
    background: #434343;
    border: 1px solid #555;
    color: #e0e0e0;
    font-size: 11px;
    padding: 2px 6px;
    border-radius: 3px;
    cursor: pointer;
    outline: none;
  }

  .track-color-select select:hover {
    border-color: #37a8db;
  }

  .model-select {
    margin-left: auto; /* push to the opposite (right) corner of the bottom row */
  }

  .model-select select {
    background: #434343;
    border: 1px solid #555;
    color: #e0e0e0;
    font-size: 11px;
    padding: 2px 6px;
    border-radius: 3px;
    cursor: pointer;
    outline: none;
    color-scheme: dark;
  }

  .model-select select:hover {
    border-color: #37a8db;
  }

  .track-legend {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    font-size: 11px;
    color: #c0c0c0;
  }

  .legend-item {
    display: flex;
    align-items: center;
    gap: 3px;
    white-space: nowrap;
  }

  .legend-dot {
    display: inline-block;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .gradient-bar {
    display: inline-block;
    width: 120px;
    height: 8px;
    border-radius: 4px;
    flex-shrink: 0;
  }

  .altitude-bar {
    background: linear-gradient(to right, hsl(240,80%,50%), hsl(120,80%,50%), hsl(60,80%,50%), hsl(0,80%,50%));
  }

  .speed-bar {
    background: linear-gradient(to right, hsl(240,80%,50%), hsl(120,80%,50%), hsl(60,80%,50%), hsl(0,80%,50%));
  }

  .signal-bar {
    background: linear-gradient(to right, hsl(0,80%,45%), hsl(60,80%,45%), hsl(120,80%,45%));
  }

  .gradient-label {
    font-size: 10px;
    color: #949494;
  }
</style>