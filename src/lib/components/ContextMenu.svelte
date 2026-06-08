<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // App-root context-menu host. Mount once. Suppresses the native WebView menu
  // app-wide (except inside real text inputs, to keep copy/paste), and renders
  // the active custom menu from the contextMenu store.
  import { onMount } from 'svelte';
  import { contextMenu, closeContextMenu } from '$lib/stores/contextMenu';
  import ContextMenuList from './ContextMenuList.svelte';

  onMount(() => {
    const onNativeContext = (e: MouseEvent) => {
      const el = e.target as HTMLElement | null;
      // Keep the native menu in editable fields (copy/paste/spellcheck).
      if (el?.closest('input, textarea, [contenteditable="true"], [contenteditable=""]')) return;
      e.preventDefault();
    };
    window.addEventListener('contextmenu', onNativeContext);
    return () => window.removeEventListener('contextmenu', onNativeContext);
  });

  function onPointerDown(e: PointerEvent) {
    if (!$contextMenu.open) return;
    const el = e.target as HTMLElement | null;
    if (el?.closest('.cm-menu')) return; // click inside the menu
    closeContextMenu();
  }
  function onKey(e: KeyboardEvent) {
    if (e.key === 'Escape') closeContextMenu();
  }
</script>

<svelte:window
  onpointerdown={onPointerDown}
  onkeydown={onKey}
  onresize={closeContextMenu}
  onwheel={closeContextMenu}
/>

{#if $contextMenu.open}
  <ContextMenuList items={$contextMenu.items} x={$contextMenu.x} y={$contextMenu.y} onpick={closeContextMenu} />
{/if}
