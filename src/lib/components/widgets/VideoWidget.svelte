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
  import { videoStream, videoState, bindVideoEl, setMapLocation, setWidgetRect, reportMjpegError } from '$lib/stores/video';
  import { canvasSink, mjpegSink } from '$lib/controllers/mjpegSink';
  import { nativeSurface, activeNativeSurface } from '$lib/controllers/nativeVideo';
  import VideoReconnectOverlay from '$lib/components/video/VideoReconnectOverlay.svelte';

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
<div bind:this={cardEl} class="widget-card" data-nv-clip style="width:{width}px; height:{height}px;" ondblclick={swapHere}>
  {#if mapHere}
    <!-- The map is overlaid here by +page (top-level). Keep an empty sized tile underneath. -->
    <div class="placeholder map-here"></div>
  {:else if $videoState.status === 'live' && $videoState.nativeSink}
    <!-- Native decode sink (hole punch): the video is a hardware layer BELOW the WebView; this
         div is the transparent hole it shows through (the surface router clips the card + map
         behind it). Only one surface at a time can hold the hole — see controllers/nativeVideo. -->
    <div class="native-hole" class:armed={$activeNativeSurface === 'widget'} use:nativeSurface={'widget'}>
      {#if $activeNativeSurface !== 'widget'}<span>{$t('video.sinkElsewhere')}</span>{/if}
    </div>
  {:else if $videoState.status === 'live' && $videoState.mjpegUrl}
    <!-- Native / MJPEG feed (no MediaStream): drawn by the off-thread reader where the WebView
         allows it, otherwise the plain <img> multipart stream. -->
    {#if $canvasSink}
      <canvas use:mjpegSink class:mirror={$videoState.mirror}></canvas>
    {:else}
      <!-- svelte-ignore a11y_missing_attribute -->
      <img src={$videoState.mjpegUrl} class:mirror={$videoState.mirror} onerror={reportMjpegError} />
    {/if}
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
  <VideoReconnectOverlay />
</div>

<style>
  .widget-card {
    box-sizing: border-box;
    position: relative; /* anchor for the reconnect overlay */
    background: rgba(30, 30, 30, 0.75);
    backdrop-filter: blur(10px);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 8px;
    padding: 3px;
    overflow: hidden;
  }
  video,
  img,
  canvas,
  .placeholder {
    width: 100%;
    height: 100%;
    border-radius: 5px;
    border: 1px solid rgba(255, 255, 255, 0.12);
    background: #000;
    display: block;
    box-sizing: border-box;
  }
  video,
  img,
  canvas {
    object-fit: cover; /* crop to fill the 2:1 tile */
    /* Own compositing layer: a 60 fps feed then only re-rasters/uploads its own rect instead of
       dirtying the shared content layer's tiles every frame (matters on WebKitGTK/Pi; <video> gets a
       layer anyway, the MJPEG <img> does NOT unless promoted). */
    will-change: transform;
  }
  video.mirror,
  img.mirror,
  canvas.mirror {
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
  /* Native-sink hole: transparent while this tile holds the hardware video layer (the frame
     border stays as the bezel), an opaque placeholder while another surface has it. */
  .native-hole {
    width: 100%;
    height: 100%;
    box-sizing: border-box;
    border-radius: 5px;
    border: 1px solid rgba(255, 255, 255, 0.12);
    background: #000;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #888;
    font-size: 12px;
    text-align: center;
  }
  .native-hole.armed {
    background: transparent;
  }
</style>
