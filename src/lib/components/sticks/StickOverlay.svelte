<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- StickOverlay.svelte
     Two Mode-2 gimbal indicators (left = throttle/yaw, right = pitch/roll) shown beside the replay
     LogPlayer bar. Layout + i18n labels only; the stick math lives in helpers/stickInput.ts and the
     drawing in GimbalStick.svelte (both reusable). Render gated on having stick data.
     Usage: <StickOverlay data={stickData} />
-->
<script lang="ts">
  import { t } from 'svelte-i18n';
  import GimbalStick from './GimbalStick.svelte';
  import type { StickData } from '$lib/helpers/stickInput';

  let { data, barHeight = 0 }: { data: StickData; barHeight?: number } = $props();
</script>

<div class="stick-overlay">
  <div class="sticks-row" style:height={barHeight ? `${barHeight}px` : undefined}>
    <GimbalStick
      primary={data.primary.left}
      secondary={data.secondary?.left ?? null}
      label={$t('player.stickThrottleYaw')}
    />
    <GimbalStick
      primary={data.primary.right}
      secondary={data.secondary?.right ?? null}
      label={$t('player.stickPitchRoll')}
    />
  </div>
  {#if data.secondary}
    <div class="stick-legend">
      <span class="leg"><span class="leg-dot leg-primary"></span>{$t('player.stickCommand')}</span>
      <span class="leg"><span class="leg-dot leg-secondary"></span>{$t('player.stickRaw')}</span>
    </div>
  {/if}
</div>

<style>
  /* Anchored to the LogPlayer (centred, 800px wide): sit just right of its right edge. */
  .stick-overlay {
    position: absolute;
    top: 62px;
    left: 50%;
    transform: translateX(calc(400px + 28px)); /* clear gap to the right of the centred 800px bar */
    z-index: 50;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    pointer-events: none; /* purely informational — never steal clicks from the map/player */
  }

  .sticks-row {
    display: flex;
    align-items: stretch;   /* both panels fill the measured bar height → flush top + bottom */
    gap: 8px;
    min-height: 90px;       /* fallback until the bar height is measured */
  }

  .stick-legend {
    display: flex;
    gap: 10px;
    padding: 3px 8px;
    background: rgba(30, 30, 30, 0.5);
    backdrop-filter: blur(10px);
    -webkit-backdrop-filter: blur(10px);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 6px;
    font-size: 10px;
    color: #c0c0c0;
  }

  .leg {
    display: flex;
    align-items: center;
    gap: 4px;
    white-space: nowrap;
  }

  .leg-dot {
    display: inline-block;
    width: 9px;
    height: 9px;
    border-radius: 50%;
    box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.7);
  }

  .leg-primary { background: #37a8db; }
  .leg-secondary { background: #f39c12; opacity: 0.65; }
</style>
