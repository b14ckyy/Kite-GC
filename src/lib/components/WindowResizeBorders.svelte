<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!--
  Thin invisible resize grips around the window edges. With `decorations: false`
  GTK drops the native resize border on Linux, so we re-add it ourselves via
  Tauri's startResizeDragging(). Each grip lives only at the extreme window edge
  (a few px) so it never interferes with the app content underneath.
-->

<script lang="ts">
  import { getCurrentWindow } from '@tauri-apps/api/window';

  type Dir = 'East' | 'North' | 'NorthEast' | 'NorthWest' | 'South' | 'SouthEast' | 'SouthWest' | 'West';

  const grips: { cls: string; dir: Dir }[] = [
    { cls: 'edge-n', dir: 'North' },
    { cls: 'edge-s', dir: 'South' },
    { cls: 'edge-w', dir: 'West' },
    { cls: 'edge-e', dir: 'East' },
    { cls: 'corner-nw', dir: 'NorthWest' },
    { cls: 'corner-ne', dir: 'NorthEast' },
    { cls: 'corner-sw', dir: 'SouthWest' },
    { cls: 'corner-se', dir: 'SouthEast' },
  ];

  function startResize(direction: Dir) {
    getCurrentWindow().startResizeDragging(direction);
  }
</script>

<div class="resize-borders" aria-hidden="true">
  {#each grips as g}
    <!-- svelte-ignore a11y_consider_explicit_label -->
    <button type="button" tabindex="-1" class="grip {g.cls}" onmousedown={() => startResize(g.dir)}></button>
  {/each}
</div>

<style>
  .resize-borders {
    position: fixed;
    inset: 0;
    z-index: 9999;
    pointer-events: none;
  }

  .grip {
    position: fixed;
    margin: 0;
    padding: 0;
    border: none;
    background: transparent;
    pointer-events: auto;
  }

  /* Edges — thin strips. */
  .edge-n { top: 0; left: 8px; right: 8px; height: 4px; cursor: ns-resize; }
  .edge-s { bottom: 0; left: 8px; right: 8px; height: 4px; cursor: ns-resize; }
  .edge-w { left: 0; top: 8px; bottom: 8px; width: 4px; cursor: ew-resize; }
  .edge-e { right: 0; top: 8px; bottom: 8px; width: 4px; cursor: ew-resize; }

  /* Corners — small squares. */
  .corner-nw { top: 0; left: 0; width: 8px; height: 8px; cursor: nwse-resize; }
  .corner-ne { top: 0; right: 0; width: 8px; height: 8px; cursor: nesw-resize; }
  .corner-sw { bottom: 0; left: 0; width: 8px; height: 8px; cursor: nesw-resize; }
  .corner-se { bottom: 0; right: 0; width: 8px; height: 8px; cursor: nwse-resize; }
</style>
