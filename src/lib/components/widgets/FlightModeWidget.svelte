<!-- Flight Mode widget — shows current flight mode as colored badge -->
<script lang="ts">
  import type { TelemetryData } from "$lib/stores/telemetry";
  import { classifyMode, isArduPilot, FLIGHT_MODE } from "$lib/helpers/trackColors";

  let { telem, size = 9 }: { telem: TelemetryData; size?: number } = $props();

  let flags = $derived(telem.activeFlightModeFlags);
  let fcVariant = $derived(telem.fcVariant ?? 'INAV');
  let isArdu = $derived(isArduPilot(fcVariant));
  let mode = $derived(classifyMode(flags, fcVariant));

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

<div class="widget-card" style="--ws: {size}vmin">
  <span class="w-label">MODE</span>
  <span class="w-mode" style="background: {mode.color}; color: {mode.color === '#e8c820' || mode.color === '#c0c0c0' ? '#1a1a1a' : '#fff'}">
    {mode.label}
  </span>
  {#if modifiers().length > 0}
    <div class="w-mods">
      {#each modifiers() as mod}
        <span class="w-mod-tag">{mod}</span>
      {/each}
    </div>
  {/if}
  <span class="w-flags">0x{flags.toString(16).toUpperCase().padStart(5, '0')}</span>
</div>

<style>
  .widget-card {
    width: var(--ws);
    height: var(--ws);
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    background: rgba(30, 30, 30, 0.75);
    backdrop-filter: blur(10px);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: calc(var(--ws) * 0.08);
    gap: calc(var(--ws) * 0.03);
    box-sizing: border-box;
    padding: calc(var(--ws) * 0.04);
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
  .w-flags {
    font-size: calc(var(--ws) * 0.09);
    font-family: monospace;
    color: #666;
  }
</style>
