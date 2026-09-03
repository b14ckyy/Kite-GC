<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- PhoneDebugButton — the dev-only Debug Monitor toggle in the phone's bottom-left row, right of
     the arming / sensor chips. It lives in the PANELS layer (+page), so it stays clickable over an
     open panel — a developer tool, unlike the chips, which the panels may cover. Publishes its own
     width (+ gap) as `--phone-debug-w` so the Leaflet attribution starts after it. -->
<script lang="ts">
  import { t } from 'svelte-i18n';

  let { debugOpen = $bindable(false) }: { debugOpen?: boolean } = $props();

  let widthPx = $state(0);
  $effect(() => {
    document.documentElement.style.setProperty('--phone-debug-w', `${Math.ceil(widthPx) + 8}px`);
    return () => document.documentElement.style.removeProperty('--phone-debug-w');
  });
</script>

<button class="debug-btn" class:open={debugOpen} bind:clientWidth={widthPx} onclick={() => (debugOpen = !debugOpen)} title="MSP Debug Monitor">
  🔧 {$t('statusBar.debug')}
</button>

<style>
  .debug-btn {
    height: 26px;
    padding: 0 8px;
    background: rgba(46, 46, 46, 0.92);
    border: 1px solid #555;
    border-radius: 4px;
    color: #949494;
    font-size: 10px;
    cursor: pointer;
    backdrop-filter: blur(8px);
    -webkit-backdrop-filter: blur(8px);
    pointer-events: auto;
  }
  .debug-btn.open {
    border-color: #37a8db;
    color: #37a8db;
  }
</style>
