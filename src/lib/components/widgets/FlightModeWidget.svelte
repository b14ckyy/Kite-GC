<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- Flight Mode widget — shows current flight mode as colored badge -->
<script lang="ts">
  import type { TelemetryData } from "$lib/stores/telemetry";
  import { classifyMode, isArduPilot, FLIGHT_MODE } from "$lib/helpers/trackColors";
  import { mission, replayActive } from "$lib/stores/mission";
  import { replayWpTotal } from "$lib/stores/navStatus";

  let { telem, size = 9 }: { telem: TelemetryData; size?: number } = $props();

  let flags = $derived(telem.activeFlightModeFlags);
  let fcVariant = $derived(telem.fcVariant ?? 'INAV');
  let isArdu = $derived(isArduPilot(fcVariant));
  let mode = $derived(classifyMode(flags, fcVariant));

  // In MISSION (NAV_WP) mode, show the FC's current target waypoint as "WP N/X"
  // (N = active waypoint number, X = total waypoints). With no mission / no active
  // WP, INAV falls back to RTH — show "WP-RTH" instead of a number. INAV only.
  let inMission = $derived(!isArdu && (flags & FLIGHT_MODE.NAV_WP) !== 0);
  // X (total): in replay use the flown mission's count (linked library mission or Blackbox
  // header); live uses the loaded planner mission's length.
  let wpTotal = $derived($replayActive ? ($replayWpTotal ?? 0) : $mission.waypoints.length);
  let wpText = $derived.by(() => {
    if (!inMission) return null;
    const n = telem.activeWpNumber;
    if (n <= 0) return 'WP-RTH';
    return wpTotal > 0 ? `WP ${n}/${wpTotal}` : `WP ${n}`;
  });

  // Show active modifier flags as small tags (INAV only — ArduPilot uses flat mode numbers)
  let modifiers = $derived(() => {
    if (isArdu) return [];
    const mods: string[] = [];
    if (flags & FLIGHT_MODE.NAV_ALTHOLD)    mods.push('ALT');
    if (flags & FLIGHT_MODE.HEADING)        mods.push('HDG');
    if (flags & FLIGHT_MODE.HEADFREE)       mods.push('HFREE');
    if (flags & FLIGHT_MODE.SOARING)        mods.push('SOAR');
    if (flags & FLIGHT_MODE.AUTO_TUNE)      mods.push('TUNE');
    if (flags & FLIGHT_MODE.FLAPERON)       mods.push('FLAP');
    if (flags & FLIGHT_MODE.NAV_FW_AUTOLAND) mods.push('LAND');
    return mods;
  });
</script>

<div class="widget-card" style="--ws: {size}px">
  <span class="w-label">MODE</span>
  <span class="w-mode" style="background: {mode.color}; color: {mode.color === '#c0c0c0' ? '#1a1a1a' : '#fff'}">
    {mode.label}
  </span>
  {#if modifiers().length > 0}
    <div class="w-mods">
      {#each modifiers() as mod}
        <span class="w-mod-tag">{mod}</span>
      {/each}
    </div>
  {/if}
  {#if wpText}
    <span class="w-wp">{wpText}</span>
  {/if}
</div>

<style>
  .widget-card {
    width: var(--ws);
    height: var(--ws);
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: flex-start;
    background: rgba(30, 30, 30, 0.75);
    backdrop-filter: blur(10px);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: calc(var(--ws) * 0.08);
    gap: calc(var(--ws) * 0.03);
    box-sizing: border-box;
    padding: calc(var(--ws) * 0.05) calc(var(--ws) * 0.05) calc(var(--ws) * 0.04);
  }
  .w-label {
    font-size: calc(var(--ws) * 0.13);
    font-weight: 600;
    color: #37a8db;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .w-mode {
    font-size: calc(var(--ws) * 0.16);
    font-weight: 700;
    padding: calc(var(--ws) * 0.02) calc(var(--ws) * 0.06);
    border-radius: calc(var(--ws) * 0.04);
    text-transform: uppercase;
    letter-spacing: 0.03em;
    white-space: nowrap;
    margin-top: calc(var(--ws) * 0.015);
  }
  .w-mods {
    display: flex;
    gap: calc(var(--ws) * 0.02);
    flex-wrap: wrap;
    justify-content: center;
  }
  .w-mod-tag {
    font-size: calc(var(--ws) * 0.09);
    font-weight: 600;
    color: #949494;
    background: rgba(255, 255, 255, 0.08);
    padding: calc(var(--ws) * 0.01) calc(var(--ws) * 0.03);
    border-radius: calc(var(--ws) * 0.02);
  }
  .w-wp {
    margin-top: auto;
    margin-bottom: calc(var(--ws) * 0.02);
    font-size: calc(var(--ws) * 0.15);
    font-weight: 700;
    color: #37a8db;
    letter-spacing: 0.04em;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
</style>
