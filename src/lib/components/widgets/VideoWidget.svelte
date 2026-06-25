<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // Video widget (2×1 wide) — a router sink showing the shared video feed.
  // Crop-to-fill (object-fit: cover) so the 2:1 tile is always full (too small
  // to read OSD anyway). Standard widget card with a thin rounded frame around
  // the video. No settings — the NavRail Video panel owns all control.
  //
  // Double-click swaps: the single map instance jumps INTO this tile (locked to a 2D heading-follow
  // nav view by +page), and the other surfaces show video. To do that, +page overlays the top-level
  // map onto this tile, so we publish our on-screen rect; when the map is here we render an empty
  // tile underneath it.
  import { t } from 'svelte-i18n';
  import { onMount, onDestroy } from 'svelte';
  import { videoStream, videoState, bindVideoEl, setMapLocation, setWidgetRect } from '$lib/stores/video';

  let { width = 300, height = 150 }: { width?: number; height?: number } = $props();

  const mapHere = $derived($videoState.mapLocation === 'widget');

  let cardEl = $state<HTMLDivElement | null>(null);
  let videoEl = $state<HTMLVideoElement | null>(null);
  $effect(() => {
    bindVideoEl(videoEl, $videoStream);
  });

  // Publish the tile's screen rect so +page can overlay the map on it in `widget` mode.
  function measure() {
    if (!cardEl) return;
    const r = cardEl.getBoundingClientRect();
    setWidgetRect({ x: r.left, y: r.top, w: r.width, h: r.height });
  }
  // Re-measure when the tile's size changes (dock reflow / UI scale come through the width/height
  // props). MUST NOT read $videoState here — measure() writes it (widgetRect), which would re-trigger
  // this effect and loop. Position-only moves are caught by the ResizeObserver + window resize below.
  $effect(() => {
    void width;
    void height;
    measure();
  });
  onMount(() => {
    measure();
    let ro: ResizeObserver | undefined;
    if (cardEl && typeof ResizeObserver !== 'undefined') {
      ro = new ResizeObserver(() => measure());
      ro.observe(cardEl);
    }
    window.addEventListener('resize', measure);
    return () => {
      ro?.disconnect();
      window.removeEventListener('resize', measure);
    };
  });
  onDestroy(() => {
    setWidgetRect(null);
    if (mapHere) setMapLocation('main'); // tile gone → don't strand the map
  });

  function swapHere() {
    if ($videoState.status !== 'live' || mapHere) return;
    setMapLocation('widget');
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<div bind:this={cardEl} class="widget-card" style="width:{width}px; height:{height}px;" ondblclick={swapHere}>
  {#if mapHere}
    <!-- The map is overlaid here by +page (top-level). Keep an empty sized tile underneath. -->
    <div class="placeholder map-here"></div>
  {:else if $videoState.status === 'live'}
    <!-- svelte-ignore a11y_media_has_caption -->
    <video
      bind:this={videoEl}
      autoplay
      muted
      playsinline
      class:mirror={$videoState.mirror}
    ></video>
  {:else}
    <div class="placeholder">
      {$videoState.status === 'starting' ? $t('video.starting') : $t('video.off')}
    </div>
  {/if}
</div>

<style>
  .widget-card {
    box-sizing: border-box;
    background: rgba(30, 30, 30, 0.75);
    backdrop-filter: blur(10px);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 8px;
    padding: 3px;
    overflow: hidden;
  }
  video,
  .placeholder {
    width: 100%;
    height: 100%;
    border-radius: 5px;
    border: 1px solid rgba(255, 255, 255, 0.12);
    background: #000;
    display: block;
    box-sizing: border-box;
  }
  video {
    object-fit: cover; /* crop to fill the 2:1 tile */
  }
  video.mirror {
    transform: scaleX(-1);
  }
  .placeholder {
    display: flex;
    align-items: center;
    justify-content: center;
    color: #888;
    font-size: 12px;
  }
  .placeholder.map-here {
    color: #555; /* faint — the map is drawn on top of this tile */
  }
</style>
