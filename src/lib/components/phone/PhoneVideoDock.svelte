<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- PhoneVideoDock — the phone's stand-in for the floating video window (Dev-Docs
     active/PHONE_VIDEO.md D2–D4, D7). A chromeless frame docked to the bottom-right of the map
     area (left of the corner controls, bottom-aligned with the chip row): no drag, no resize. One
     toggle button above the corner controls parks it — the frame slides right, behind the widget
     column and off the screen — and brings it back. Parked = unmounted: nothing renders, the
     source stays open, so the return is instant. Double-tap = map ⇄ video swap, as everywhere.
     The frame is the `floating` surface (store field, map location, native-sink id), so +page's
     swap plumbing works unchanged; +page computes the rect (it also places the swapped map into
     it) and passes it down. -->
<script lang="ts">
  import { t } from 'svelte-i18n';
  import { videoStream, videoState, bindVideoEl, setMapLocation, toggleFloating, reportMjpegError } from '$lib/stores/video';
  import { canvasSink, mjpegSink } from '$lib/controllers/mjpegSink';
  import { nativeSurface, activeNativeSurface } from '$lib/controllers/nativeVideo';
  import { doubleTap, mouseDoubleClick } from '$lib/helpers/doubleTap';
  import VideoReconnectOverlay from '$lib/components/video/VideoReconnectOverlay.svelte';

  let {
    left,
    top,
    width,
    height,
    /** The video widget is active in the phone grid → no dock, no button (D4/D5). */
    widgetActive = false,
  }: {
    left: number;
    top: number;
    width: number;
    height: number;
    widgetActive?: boolean;
  } = $props();

  const live = $derived($videoState.status === 'live');
  /** The map sits in this frame (swapped) — +page renders it top-level, the body is omitted. */
  const mapHere = $derived($videoState.mapLocation === 'floating');
  const showButton = $derived(live && !widgetActive);
  const open = $derived($videoState.floating && live && !widgetActive);

  // Mounted lags `open` by one slide: on close the frame stays in the DOM (class `parked`) until its
  // transform transition ends, then unmounts; on open it mounts parked and un-parks a frame later so
  // the slide-in animates. Parking with the map in the frame swaps the map back first — a parked
  // frame must never hold the map.
  let mounted = $state(false);
  let parked = $state(true);
  let frameEl = $state<HTMLDivElement | null>(null);
  $effect(() => {
    if (open) {
      mounted = true;
      requestAnimationFrame(() => { parked = false; });
    } else {
      parked = true;
      if (!mounted) return;
      if (!frameEl) { mounted = false; return; }
      const el = frameEl;
      const done = () => { el.removeEventListener('transitionend', done); if (parked) mounted = false; };
      el.addEventListener('transitionend', done);
      // Backstop: a display:none ancestor never fires transitionend.
      setTimeout(done, 450);
    }
  });

  // The button parks / recalls the frame, whatever it holds. Video primary (the map in the frame):
  // parking hides the mini map — +page moves its in-frame map layer out with the frame — and the
  // button becomes a map icon in the corner the 2D/3D controls vacate (they are hidden while the
  // map is a mini map). Double-tap the full-screen video to bring the map back to the main view.
  function toggle() {
    toggleFloating();
  }
  const label = $derived(
    mapHere
      ? ($videoState.floating ? $t('video.dockMapHide') : $t('video.dockMapShow'))
      : ($videoState.floating ? $t('video.dockHide') : $t('video.dockShow')),
  );

  let videoEl = $state<HTMLVideoElement | null>(null);
  $effect(() => {
    bindVideoEl(videoEl, $videoStream);
  });
</script>

{#if showButton}
  <button
    class="dock-btn"
    class:open={$videoState.floating}
    class:map-mode={mapHere}
    onclick={toggle}
    title={label}
    aria-label={label}
  >
    {#if mapHere}
      <!-- map -->
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M3 6.5 9 4l6 2.5 6-2.5v13.5L15 20l-6-2.5L3 20z" />
        <path d="M9 4v13.5M15 6.5V20" />
      </svg>
    {:else}
      <!-- camera -->
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <rect x="3" y="7" width="13" height="10" rx="2" />
        <path d="M16 10.5 21 8v8l-5-2.5" />
      </svg>
    {/if}
  </button>
{/if}

{#if mounted}
  <!-- No z-index on the wrapper: layers compose with the top-level in-frame map (see
       FloatingVideoWindow). Sits below the widget column (z 100), which is what the slide-out hides
       behind; the native sink is clipped at the column edge by the surface router's right bound. -->
  <div
    bind:this={frameEl}
    class="dock-win"
    class:parked
    style="left:{left}px; top:{top}px; width:{width}px; height:{height}px;"
  >
    <div class="dw-bg" class:nv-active={$activeNativeSurface === 'floating'}></div>
    {#if !mapHere}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="dw-body"
        class:nv-active={$activeNativeSurface === 'floating'}
        ondblclick={mouseDoubleClick(() => setMapLocation('floating'))}
        use:doubleTap={() => setMapLocation('floating')}
      >
        {#if live && $videoState.nativeSink}
          <div class="native-hole" class:armed={$activeNativeSurface === 'floating'} use:nativeSurface={'floating'}>
            {#if $activeNativeSurface !== 'floating'}<span>{$t('video.sinkElsewhere')}</span>{/if}
          </div>
        {:else if live && $videoState.mjpegUrl}
          {#if $canvasSink}
            <canvas use:mjpegSink class:mirror={$videoState.mirror} class:rot180={$videoState.rotate180}></canvas>
          {:else}
            <!-- svelte-ignore a11y_missing_attribute -->
            <img src={$videoState.mjpegUrl} class:mirror={$videoState.mirror} class:rot180={$videoState.rotate180} onerror={reportMjpegError} />
          {/if}
        {:else if live}
          <!-- svelte-ignore a11y_media_has_caption -->
          <video bind:this={videoEl} autoplay muted playsinline class:mirror={$videoState.mirror} class:rot180={$videoState.rotate180}></video>
        {/if}
        <VideoReconnectOverlay />
      </div>
    {/if}
  </div>
{/if}

<style>
  /* Toggle: above the corner controls (2 × 38px buttons + 2 × 8px gaps), riding along with the
     widget column when the replay player pushes it aside (--phone-shift, set by +page). */
  .dock-btn {
    position: absolute;
    right: calc(var(--phone-panel-w, 0px) + 8px - var(--phone-shift, 0px));
    bottom: calc(8px + var(--safe-bottom, 0px) + 92px);
    z-index: 110;
    box-sizing: border-box;
    width: 38px;
    height: 38px;
    padding: 7px;
    background: rgba(46, 46, 46, 0.9);
    border: 2px solid rgba(55, 168, 219, 0.5);
    border-radius: 6px;
    color: #37a8db;
    cursor: pointer;
    backdrop-filter: blur(8px);
    -webkit-backdrop-filter: blur(8px);
    transition: right 0.3s ease, background 0.2s, border-color 0.2s;
    pointer-events: auto;
  }
  .dock-btn.open {
    background: rgba(55, 168, 219, 0.25);
    border-color: #37a8db;
  }
  /* Video primary: the 2D/3D + follow buttons are hidden (mini map), the map toggle takes their
     corner. */
  .dock-btn.map-mode {
    bottom: calc(8px + var(--safe-bottom, 0px));
  }
  .dock-btn svg {
    width: 100%;
    height: 100%;
    fill: none;
    stroke: currentColor;
    stroke-width: 2;
    stroke-linejoin: round;
  }

  .dock-win {
    position: absolute;
    pointer-events: none;
    transform: translateX(0);
    transition: transform 0.3s ease;
  }
  /* Parked: behind the widget column and past the screen's right edge (100vw covers both). */
  .dock-win.parked {
    transform: translateX(100vw);
  }
  .dw-bg {
    position: absolute;
    inset: 0;
    z-index: 60;
    pointer-events: none;
    background: rgba(46, 46, 46, 0.92);
    border: 1px solid rgba(55, 168, 219, 0.35);
    border-radius: 8px;
    box-shadow: 0 4px 20px rgba(0, 0, 0, 0.4);
  }
  .dw-body {
    position: absolute;
    inset: 0;
    z-index: 61;
    pointer-events: auto;
    background: #000;
    overflow: hidden;
    border-radius: 8px;
    touch-action: none;
  }
  .dw-bg.nv-active,
  .dw-body.nv-active {
    background: transparent;
  }
  .native-hole {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #888;
    font-size: 12px;
    text-align: center;
    background: #000;
    border-radius: 8px; /* read by the surface router — the hole is cut with these corners */
  }
  .native-hole.armed {
    background: transparent;
  }
  .dw-body video,
  .dw-body img,
  .dw-body canvas {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
    will-change: transform;
  }
  .dw-body video.mirror,
  .dw-body img.mirror,
  .dw-body canvas.mirror {
    transform: scaleX(-1);
  }
  .dw-body video.rot180,
  .dw-body img.rot180,
  .dw-body canvas.rot180 {
    transform: rotate(180deg);
  }
  .dw-body video.mirror.rot180,
  .dw-body img.mirror.rot180,
  .dw-body canvas.mirror.rot180 {
    transform: scaleY(-1);
  }
</style>
