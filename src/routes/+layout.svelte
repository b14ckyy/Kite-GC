<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  import { onMount } from 'svelte';
  import { initI18n } from '$lib/i18n';
  import { isLoading } from 'svelte-i18n';
  import { settings } from '$lib/stores/settings';
  import { get } from 'svelte/store';

  // Initialize i18n with the persisted locale (or browser default)
  const saved = get(settings);
  initI18n(saved.locale);

  // Block FRAME-level zoom gestures app-wide. A trackpad pinch (or Ctrl/Cmd +/-) otherwise zooms the
  // whole WebView — scaling the widgets out of view — instead of the map. The map keeps its own
  // zoom: preventDefault() stops only the browser's page zoom; it does NOT stop propagation, so the
  // map's wheel/pinch handlers still receive the event and zoom the map.
  //   - Chromium / WebView2 (Windows): a trackpad pinch arrives as `wheel` with `ctrlKey` set.
  //   - WebKitGTK (Linux): a trackpad pinch fires the WebKit `gesture*` events instead.
  //   - Ctrl/Cmd +/-/0/= keyboard zoom.
  onMount(() => {
    const onWheel = (e: WheelEvent) => { if (e.ctrlKey) e.preventDefault(); };
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && ['+', '-', '=', '0'].includes(e.key)) e.preventDefault();
    };
    const onGesture: EventListener = (e) => e.preventDefault();
    const gestureEvents = ['gesturestart', 'gesturechange', 'gestureend'];

    window.addEventListener('wheel', onWheel, { passive: false, capture: true });
    window.addEventListener('keydown', onKey, { capture: true });
    for (const name of gestureEvents) window.addEventListener(name, onGesture, { passive: false });

    return () => {
      window.removeEventListener('wheel', onWheel, { capture: true });
      window.removeEventListener('keydown', onKey, { capture: true });
      for (const name of gestureEvents) window.removeEventListener(name, onGesture);
    };
  });
</script>

{#if $isLoading}
  <!-- Wait for locale to load before rendering the app -->
{:else}
  <slot />
{/if}
