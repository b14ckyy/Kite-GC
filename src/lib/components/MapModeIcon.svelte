<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- "2D" / "3D" drawn as paths for the map's mode button: text would follow the system's font and font size. -->
<script lang="ts">
  let { mode }: { mode: '2d' | '3d' } = $props();
</script>

<svg viewBox="-0.4 -1 22 14" aria-hidden="true">
  {#if mode === '3d'}
    <path d="M2.15 2.64 A2.5 2.5 0 1 1 4.5 6 A2.9 2.5 0 1 1 1.77 9.36" />
  {:else}
    <path d="M1.2 3.8 A3.5 3.3 0 1 1 7.3 6.2 L1 11 H8.4" />
  {/if}
  <path d="M12.5 1 H15 A5 5 0 0 1 15 11 H12.5 Z" />
</svg>

<style>
  svg {
    display: block;
    width: var(--map-btn-icon, 28.5px);
    aspect-ratio: 22 / 14;
    height: auto;
    fill: none;
    stroke: currentColor;
    stroke-width: 2.3;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
</style>
