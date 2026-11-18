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
  import { invoke } from '@tauri-apps/api/core';

  // Initialize i18n with the persisted locale (or browser default)
  const saved = get(settings);
  initI18n(saved.locale);

  // Block FRAME-level zoom app-wide (trackpad pinch, Ctrl/Cmd +/-) — it scales the whole WebView; preventDefault
  // stops only the page zoom, so the map's own handlers still zoom. WebView2 sends a pinch as ctrl+wheel, WKWebView
  // as gesture* events; WebKitGTK ignores preventDefault, so lib.rs pins its zoom level at 1.0 instead.
  onMount(() => {
    // Uncaught frontend errors go to the backend file log (log_frontend) — a release build has no console.
    const report = (kind: string, detail: unknown) => {
      const msg = detail instanceof Error ? `${detail.message}
${detail.stack ?? ''}` : String(detail);
      void invoke('log_frontend', { level: 'error', area: 'ui', message: `${kind}: ${msg}` }).catch(() => {});
    };
    const onError = (e: ErrorEvent) => report('uncaught', e.error ?? e.message);
    const onRejection = (e: PromiseRejectionEvent) => report('unhandled rejection', e.reason);
    window.addEventListener('error', onError);
    window.addEventListener('unhandledrejection', onRejection);

    const onWheel = (e: WheelEvent) => { if (e.ctrlKey) e.preventDefault(); };
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && ['+', '-', '=', '0'].includes(e.key)) e.preventDefault();
      // Block the WebView's page reload (F5, Ctrl/Cmd+R) in a release build: it restarts only the frontend,
      // and the live track, the mission in the editor and Kite's RC stream would be gone. `e.code` covers
      // layouts whose R key types another letter; never with Alt (AltGr + R is a character on some layouts).
      const reloadKey = e.key === 'F5' || e.key === 'BrowserRefresh'
        || ((e.ctrlKey || e.metaKey) && !e.altKey && (e.key.toLowerCase() === 'r' || e.code === 'KeyR'));
      if (!import.meta.env.DEV && reloadKey) e.preventDefault();
    };
    const onGesture: EventListener = (e) => e.preventDefault();
    const gestureEvents = ['gesturestart', 'gesturechange', 'gestureend'];

    window.addEventListener('wheel', onWheel, { passive: false, capture: true });
    window.addEventListener('keydown', onKey, { capture: true });
    for (const name of gestureEvents) window.addEventListener(name, onGesture, { passive: false });

    return () => {
      window.removeEventListener('error', onError);
      window.removeEventListener('unhandledrejection', onRejection);
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
