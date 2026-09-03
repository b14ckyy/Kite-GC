<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- GimbalStick.svelte
     Reusable single transmitter-gimbal indicator: a glassmorphism panel with a square stick field, a
     centre crosshair, and one or two dots. `primary` (theme blue) is the main signal; optional
     `secondary` (orange, rendered behind, dimmed) is a second layer (e.g. RAW TX vs FC command).
     Pure presentation — all values are normalized −1…+1 (+y = up) and passed as props, so this can be
     reused anywhere (replay overlay now, live HUD / dual-pilot later).
     Usage: <GimbalStick primary={{x,y}} secondary={{x,y}} />
-->
<script lang="ts">
  import type { StickPos } from '$lib/helpers/stickInput';

  let { primary, secondary = null }: {
    primary: StickPos;
    secondary?: StickPos | null;
  } = $props();

  // Normalized −1…+1 → 0…100 % box coordinate; y inverted so +1 sits at the top.
  const px = (v: number) => ((v + 1) / 2) * 100;
  const py = (v: number) => ((1 - v) / 2) * 100;
</script>

<div class="gimbal">
  <div class="gimbal-box">
    <span class="cross cross-h"></span>
    <span class="cross cross-v"></span>
    {#if secondary}
      <span class="dot dot-secondary" style="left:{px(secondary.x)}%; top:{py(secondary.y)}%"></span>
    {/if}
    <span class="dot dot-primary" style="left:{px(primary.x)}%; top:{py(primary.y)}%"></span>
  </div>
</div>

<style>
  .gimbal {
    display: flex;
    align-items: stretch;
    /* Height comes from the parent row (= measured bar height) via align-items:stretch → flush;
       the frame is square and hugs the stick field (no labels — Marc, 2026-09-03). */
    aspect-ratio: 1 / 1;
    padding: 6px;
    box-sizing: border-box;
    /* Widget-style glass, a touch more transparent. */
    background: rgba(30, 30, 30, 0.5);
    backdrop-filter: blur(10px);
    -webkit-backdrop-filter: blur(10px);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 8px;
    box-shadow: 0 4px 20px rgba(0, 0, 0, 0.35);
    user-select: none;
  }

  /* The stick field fills the square frame. */
  .gimbal-box {
    position: relative;
    flex: 1;
    min-width: 0;
    background: rgba(15, 15, 15, 0.35);
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 6px;
    overflow: hidden;
  }

  /* Centre crosshair (faint). */
  .cross {
    position: absolute;
    background: rgba(148, 148, 148, 0.35);
  }
  .cross-h { left: 0; right: 0; top: 50%; height: 1px; transform: translateY(-0.5px); }
  .cross-v { top: 0; bottom: 0; left: 50%; width: 1px; transform: translateX(-0.5px); }

  .dot {
    position: absolute;
    width: 13px;
    height: 13px;
    border-radius: 50%;
    transform: translate(-50%, -50%);
    /* thin black ring for contrast against the field / the other dot */
    box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.7);
  }

  .dot-primary {
    background: #37a8db;
    z-index: 2;
  }

  /* RAW (TX) layer — orange, dimmed, behind the primary. */
  .dot-secondary {
    background: #f39c12;
    opacity: 0.65;
    z-index: 1;
  }

</style>
