<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- Flight Mode widget — shows current flight mode as colored badge -->
<script lang="ts">
  import type { TelemetryData } from "$lib/stores/telemetry";
  import { modeLabel, modeColor, modeShort, modeCategory } from "$lib/helpers/flightModeRegistry";
  import { mission, replayActive } from "$lib/stores/mission";
  import { replayWpTotal } from "$lib/stores/navStatus";

  let { telem, size = 9 }: { telem: TelemetryData; size?: number } = $props();

  // Canonical, protocol-agnostic flight mode (see flightModeRegistry).
  let primary = $derived(telem.flightMode.primary);
  let label = $derived(modeLabel(primary));
  let color = $derived(modeColor(primary));
  let modifiers = $derived(telem.flightMode.modifiers);

  // In a MISSION-category mode, show the FC's current target waypoint as "WP N/X"
  // (N = active waypoint number, X = total waypoints). Only when an active WP number is known.
  let inMission = $derived(modeCategory(primary) === 'mission');
  // X (total): in replay use the flown mission's count (linked library mission or Blackbox
  // header); live uses the loaded planner mission's length.
  let wpTotal = $derived($replayActive ? ($replayWpTotal ?? 0) : $mission.waypoints.length);
  let wpText = $derived.by(() => {
    if (!inMission) return null;
    const n = telem.activeWpNumber;
    if (n <= 0) return null;
    return wpTotal > 0 ? `WP ${n}/${wpTotal}` : `WP ${n}`;
  });
</script>

<div class="widget-card" style="--ws: {size}px">
  <span class="w-label">MODE</span>
  <span class="w-mode" style="background: {color}; color: {color === '#c0c0c0' ? '#1a1a1a' : '#fff'}">
    {label}
  </span>
  {#if modifiers.length > 0}
    <div class="w-mods">
      {#each modifiers as mod}
        <span class="w-mod-tag">{modeShort(mod)}</span>
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
