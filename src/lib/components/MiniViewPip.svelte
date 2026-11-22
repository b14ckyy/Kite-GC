<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // Android picture-in-picture: covers the whole page with the mini view while MainActivity reports PiP.
  // The app stays mounted underneath; opaque on purpose, it also hides the native video's hole punch.
  import { onMount } from 'svelte';
  import MiniView from '$lib/components/MiniView.svelte';
  import { buildMiniSnapshot } from '$lib/controllers/miniView';
  import { MINI_PIP_EVENT, MINI_SNAPSHOT_MS, type MiniSnapshot } from '$lib/helpers/miniSnapshot';

  let active = $state(false);
  let snap = $state<MiniSnapshot | null>(null);

  onMount(() => {
    const onPip = (e: Event) => {
      active = e instanceof CustomEvent && e.detail === true;
    };
    window.addEventListener(MINI_PIP_EVENT, onPip);
    return () => window.removeEventListener(MINI_PIP_EVENT, onPip);
  });

  $effect(() => {
    if (!active) return;
    snap = buildMiniSnapshot();
    const id = setInterval(() => (snap = buildMiniSnapshot()), MINI_SNAPSHOT_MS);
    return () => clearInterval(id);
  });
</script>

{#if active}
  <div class="mv-pip">
    <MiniView {snap} />
  </div>
{/if}

<style>
  .mv-pip {
    position: fixed;
    inset: 0;
    z-index: 2147483000;
  }
</style>
