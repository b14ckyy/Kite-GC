<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // The desktop mini window's route: renders the snapshots the main window pushes, nothing else. Must not
  // pull the app in (no map, no telemetry listeners, no stores with side effects).
  import { onMount } from 'svelte';
  import { t } from 'svelte-i18n';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { invoke } from '@tauri-apps/api/core';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import MiniView from '$lib/components/MiniView.svelte';
  import { MINI_SNAPSHOT_EVENT, MINI_SNAPSHOT_TIMEOUT_MS, type MiniSnapshot } from '$lib/helpers/miniSnapshot';

  /** Movement (css px) that turns a press into a window drag instead of a click. */
  const DRAG_PX = 4;

  const win = getCurrentWindow();

  let snap = $state<MiniSnapshot | null>(null);
  let receivedAt = $state(0);
  let now = $state(Date.now());
  const stale = $derived(!!snap && now - receivedAt > MINI_SNAPSHOT_TIMEOUT_MS);

  let press: { x: number; y: number } | null = null;

  onMount(() => {
    document.documentElement.classList.add('mini-view');
    let off: UnlistenFn | undefined;
    let gone = false;
    void listen<MiniSnapshot>(MINI_SNAPSHOT_EVENT, (e) => {
      snap = e.payload;
      receivedAt = Date.now();
    }).then((u) => (gone ? u() : (off = u)));
    const id = setInterval(() => (now = Date.now()), 1000);
    return () => {
      gone = true;
      off?.();
      clearInterval(id);
      document.documentElement.classList.remove('mini-view');
    };
  });

  function onDown(e: PointerEvent): void {
    press = e.button === 0 ? { x: e.clientX, y: e.clientY } : null;
  }

  // The OS move loop takes over the pointer once dragging starts, so no pointerup follows a drag.
  function onMove(e: PointerEvent): void {
    if (!press || (e.buttons & 1) === 0) return;
    if (Math.hypot(e.clientX - press.x, e.clientY - press.y) < DRAG_PX) return;
    press = null;
    void win.startDragging();
  }

  function onUp(): void {
    if (!press) return;
    press = null;
    void invoke('mini_view_restore').catch(() => {});
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="mini-root"
  title={$t('miniView.restore')}
  onpointerdown={onDown}
  onpointermove={onMove}
  onpointerup={onUp}
>
  <MiniView {snap} {stale} />
</div>

<style>
  :global(html.mini-view),
  :global(html.mini-view body) {
    margin: 0;
    padding: 0;
    height: 100%;
    background: #2e2e2e;
    overflow: hidden;
  }
  .mini-root {
    position: fixed;
    inset: 0;
    cursor: pointer;
  }
</style>
