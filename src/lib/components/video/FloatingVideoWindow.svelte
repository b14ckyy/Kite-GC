<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // Floating video window — the desktop's in-app sink for the video router, modelled on the phone's
  // docked window (PhoneVideoDock / Dev-Docs active/PHONE_VIDEO.md §10 "Desktop port"):
  //  • appears when a source starts (the store flips `floating` on Start): slides in from the left
  //    into the bottom-left corner (above the status bar; the bottom widget dock reflows out of the
  //    way — handled in +page.svelte), or to its last free position
  //  • ONE toggle button parks it — it slides left off the screen and unmounts; the source stays
  //    open, so the return is instant. The button rides on the frame's bottom-right while the window
  //    is snapped and swipes back to the bottom-left screen corner when the window parks or is moved.
  //    Visible as long as a source is active (started: starting / live / error).
  //  • thin glass bezel like the video widget; no ✕ (the toggle is the only show/hide)
  //  • the BOTTOM-LEFT corner handle (a rounded square with a four-way arrow) moves the window —
  //    the only way to move it (away from the corner un-snaps; dropping near the corner re-snaps).
  //    It used to be a body drag, and right mouse / two fingers with the map in the frame: that
  //    took the right button from the 3D map's tilt and the second finger from pinch and tilt on a
  //    touchscreen (Marc, 2026-09-10) — both now reach the map, and neither does anything on video
  //  • the bezel's lighter TOP-RIGHT corner resizes (aspect-locked, 10–30 % of vh, touch-sized hit
  //    area, drawn only in the bezel so it never covers the picture)
  //  • double-click the video to swap it with the map (→ video primary); parking with the map in
  //    the frame parks the mini map with it (+page moves its map layer), the full-screen video stays.
  //
  // Geometry comes from +page (`floatFrameRect` in the store, logical px of the zoomed chrome layer)
  // — one computation for the window, the swapped-in map and the dock reserve. Layering: separate
  // absolutely-positioned layers share the page stacking context (the .float-win wrapper has no
  // z-index). The map (rendered top-level in +page when swapped) composes between the glass frame
  // (z 60) and the corner handles (z 62), so the mini-map stays interactive while they stay usable
  // — with the map in the frame +page draws both handles above it (`.miniframe-ctl`).
  import { t } from 'svelte-i18n';
  import {
    videoStream,
    videoState,
    bindVideoEl,
    setMapLocation,
    toggleFloating,
    reportMjpegError,
    reportImgSize,
    fpsProbe,
    FLOAT_MARGIN_PX,
    FLOAT_SNAP_BOTTOM_PX,
    FLOAT_BTN_PX,
    FLOAT_BTN_GAP_PX,
  } from '$lib/stores/video';
  import { canvasSink, mjpegSink } from '$lib/controllers/mjpegSink';
  import { detachVideo } from '$lib/controllers/detachedVideo';
  import { isMobile } from '$lib/platform';
  import { nativeSurface, activeNativeSurfaces } from '$lib/controllers/nativeVideo';
  import { doubleTap, mouseDoubleClick } from '$lib/helpers/doubleTap';
  import { startFloatMove, startFloatResize } from '$lib/helpers/floatWindowGestures';
  import VideoReconnectOverlay from '$lib/components/video/VideoReconnectOverlay.svelte';

  let {
    left,
    top,
    width,
    height,
    /** Logical viewport (the zoomed chrome layer's box) — drag clamping + the parked button corner. */
    vw,
    vh,
  }: { left: number; top: number; width: number; height: number; vw: number; vh: number } = $props();

  /** A source is active (Start pressed): the window may show, the toggle button exists. While the
   *  picture is in the DETACHED window there is nothing here to show or park (D6), so the frame and
   *  its toggle button are both gone. */
  const active = $derived($videoState.enabled && !$videoState.undocked);
  /** The map sits in this frame (swapped) — +page renders it top-level, the body is omitted. */
  const mapHere = $derived($videoState.mapLocation === 'floating');
  const live = $derived($videoState.status === 'live');
  /** The unplug button: take the picture out of the app into its own window (D3). Native decode
   *  sink only (D2) — the DOM paths render into THIS WebView and cannot be handed to another one.
   *  Every desktop platform serves it now, each in its own way: a child window on Windows, a second
   *  AppKit host on macOS, a second GStreamer pipeline on Linux. */
  const canDetach = $derived(!isMobile && live && $videoState.nativeSink);
  /** Narrow derived, not a raw store read in the effect below (that would re-run it on every
   *  telemetry patch): detaching must skip the slide-out — see there. */
  const detached = $derived($videoState.undocked);
  const open = $derived($videoState.floating && active);

  // Mounted lags `open` by one slide (the phone dock's pattern): on close the frame stays in the DOM
  // (class `parked`) until its transform transition ends, then unmounts; on open it mounts parked and
  // un-parks a frame later so the slide-in animates.
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
      // Detaching moves the picture, it does not put it away: sliding out here would keep this
      // frame's surface published for the length of the animation, and the sink serves two
      // surfaces — the third (the new window's) would be dropped and its hole would stand empty.
      if (detached || !frameEl) { mounted = false; return; }
      const el = frameEl;
      const done = () => { el.removeEventListener('transitionend', done); if (parked) mounted = false; };
      el.addEventListener('transitionend', done);
      // Backstop: a display:none ancestor never fires transitionend.
      setTimeout(done, 450);
    }
  });

  // Toggle button: on the frame's bottom-right while the window is out and snapped, otherwise in
  // the bottom-left screen corner (parked window, or a window moved away from the corner). Its
  // left/top transition is the "swipe" between the two.
  const btnAtFrame = $derived(open && $videoState.floatSnapped);
  const btnLeft = $derived(btnAtFrame ? left + width + FLOAT_BTN_GAP_PX : FLOAT_MARGIN_PX);
  const btnTop = $derived(btnAtFrame ? top + height - FLOAT_BTN_PX : vh - FLOAT_SNAP_BOTTOM_PX - FLOAT_BTN_PX);
  const btnLabel = $derived(
    mapHere
      ? ($videoState.floating ? $t('video.dockMapHide') : $t('video.dockMapShow'))
      : ($videoState.floating ? $t('video.dockHide') : $t('video.dockShow')),
  );

  let videoEl = $state<HTMLVideoElement | null>(null);
  $effect(() => {
    bindVideoEl(videoEl, $videoStream);
  });

  // ── Gestures (helpers/floatWindowGestures — +page's mini-frame corners share both) ──
  const frame = () => ({ left, top, width, height, vw, vh });
  function onMovePointerDown(e: PointerEvent) {
    startFloatMove(e, frame());
  }
  function onGripPointerDown(e: PointerEvent) {
    startFloatResize(e, frame());
  }
</script>

{#if active}
  <button
    class="fw-toggle"
    class:open={$videoState.floating}
    style="left:{btnLeft}px; top:{btnTop}px;"
    onclick={() => toggleFloating()}
    title={btnLabel}
    aria-label={btnLabel}
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
  <!-- No z-index on the wrapper → no stacking context; layers compose with the top-level map. -->
  <div
    bind:this={frameEl}
    class="float-win"
    class:parked
    style="left:{left}px; top:{top}px; width:{width}px; height:{height}px;"
  >
    <!-- glass bezel (behind) — the video / map sits in its inner box -->
    <!-- The bezel keeps its border and its drop shadow while the hardware layer is armed, so it is
         a clip target too — that outline was what still crossed the widget's picture. -->
    <div
      class="fw-frame"
      class:nv-active={$activeNativeSurfaces.has('floating')}
      data-nv-clip={$activeNativeSurfaces.has('floating') ? 'floating' : undefined}
    ></div>

    <!-- content: the video. When the map is in this frame, it's rendered (top-level) by +page here
         instead, and the body is omitted. Double-click the video → the map jumps into this frame.
         `data-nv-clip="floating"` while it holds the hardware layer: the opaque bezel this box
         paints (its border + the ring around it) sits BEHIND the widget tile, whose own hole is
         transparent — so the bezel showed through the tile's picture wherever the two overlap. The
         router cuts it away with the holes of the surfaces above this one, never with its own. -->
    {#if !mapHere}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="fw-body"
        class:nv-active={$activeNativeSurfaces.has('floating')}
        data-nv-clip={$activeNativeSurfaces.has('floating') ? 'floating' : undefined}
        ondblclick={mouseDoubleClick(() => setMapLocation('floating'))}
        use:doubleTap={() => setMapLocation('floating')}
      >
        {#if live && $videoState.nativeSink}
          <!-- Native decode sink (hole punch): the video is a hardware layer BELOW the WebView;
               this div is the transparent hole it shows through. See controllers/nativeVideo. -->
          <div class="native-hole" class:armed={$activeNativeSurfaces.has('floating')} use:nativeSurface={'floating'}>
            {#if !$activeNativeSurfaces.has('floating')}<span>{$t('video.sinkElsewhere')}</span>{/if}
          </div>
        {:else if live && $videoState.mjpegUrl}
          <!-- Native / MJPEG feed (no MediaStream): drawn by the off-thread reader where the WebView
               allows it, otherwise the plain <img> multipart stream. -->
          {#if $canvasSink}
            <canvas use:mjpegSink class:mirror={$videoState.mirror} class:rot180={$videoState.rotate180}></canvas>
          {:else}
            <!-- svelte-ignore a11y_missing_attribute -->
            <img src={$videoState.mjpegUrl} class:mirror={$videoState.mirror} class:rot180={$videoState.rotate180} onload={reportImgSize} onerror={reportMjpegError} />
          {/if}
        {:else if live}
          <!-- svelte-ignore a11y_media_has_caption -->
          <video bind:this={videoEl} use:fpsProbe autoplay muted playsinline class:mirror={$videoState.mirror} class:rot180={$videoState.rotate180}></video>
        {:else}
          <div class="fw-ph">
            {#if $videoState.status === 'error'}
              ⚠ {$videoState.error}
            {:else}
              {$t('video.starting')}
            {/if}
          </div>
        {/if}
        {#if canDetach}
          <!-- Hover-only, top-left (D3): the corner the detached window's dock button sits in, so
               the same corner takes the picture out and brings it back. -->
          <button
            class="fw-unplug"
            onpointerdown={(e) => e.stopPropagation()}
            onclick={() => detachVideo()}
            title={$t('video.detach')}
            aria-label={$t('video.detach')}
          >
            <!-- broken chain: two links pulling apart, sparks at the break. Filled, not stroked —
                 a stroked chain loses the link's hole, which is what makes it read as a chain. -->
            <svg viewBox="3.3 3.3 17.4 17.4" aria-hidden="true">
              <path d="M15.69 12.83 19.23 9.29A3.2 3.2 0 0 0 14.71 4.77L11.17 8.31 12.44 9.58 15.98 6.04A1.4 1.4 0 0 1 17.96 8.02L14.42 11.56Z" />
              <path d="M8.31 11.17 4.77 14.71A3.2 3.2 0 0 0 9.29 19.23L12.83 15.69 11.56 14.42 8.02 17.96A1.4 1.4 0 0 1 6.04 15.98L9.58 12.44Z" />
              <path d="M7.91 10.67 5.68 9.11 5.19 10.63Z M8.96 8.96 7.69 6.56 6.56 7.69Z M10.67 7.91 10.63 5.19 9.11 5.68Z" />
            </svg>
          </button>
        {/if}
        <VideoReconnectOverlay />
      </div>
    {/if}

    <!-- Corner handles: the lighter L just inside the picture's top-right corner resizes, the
         rounded square with the four-way arrow in the bottom-left corner moves. Video mode only —
         with the map in the frame its layer covers the chrome, so +page draws the same corners
         above the map. -->
    {#if !mapHere}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="fw-grip" onpointerdown={onGripPointerDown} title={$t('video.resizeWindow')}></div>
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="fw-move" onpointerdown={onMovePointerDown} title={$t('video.moveWindow')}>
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="M12 3v18M3 12h18" />
          <path d="M9 6l3-3 3 3M9 18l3 3 3-3M6 9l-3 3 3 3M18 9l3 3-3 3" />
        </svg>
      </div>
    {/if}
  </div>
{/if}

<style>
  /* Toggle: above the widget dock (z 100), swipes between the frame's side and the screen corner. */
  .fw-toggle {
    position: absolute;
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
    transition: left 0.3s ease, top 0.3s ease, background 0.2s, border-color 0.2s;
    pointer-events: auto;
  }
  .fw-toggle.open {
    background: rgba(55, 168, 219, 0.25);
    border-color: #37a8db;
  }
  .fw-toggle svg {
    width: 100%;
    height: 100%;
    fill: none;
    stroke: currentColor;
    stroke-width: 2;
    stroke-linejoin: round;
  }

  .float-win {
    position: absolute;
    /* No z-index on purpose (see script header). */
    pointer-events: none; /* layers opt back in individually */
    transform: translateX(0);
    transition: transform 0.3s ease;
  }
  /* Parked: past the screen's left edge. */
  .float-win.parked {
    transform: translateX(-100vw);
  }
  /* Glass bezel — the video widget's card: 1 px border + 3 px padding = the 4 px FLOAT_BEZEL_PX
     inner offset the body and the swapped-in map use. */
  .fw-frame {
    position: absolute;
    inset: 0;
    z-index: 60;
    pointer-events: none;
    box-sizing: border-box;
    background: rgba(30, 30, 30, 0.75);
    backdrop-filter: blur(10px);
    -webkit-backdrop-filter: blur(10px);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 8px;
    box-shadow: 0 4px 20px rgba(0, 0, 0, 0.4);
  }
  .fw-body {
    position: absolute;
    inset: 4px;
    z-index: 61;
    pointer-events: auto;
    box-sizing: border-box;
    background: #000;
    overflow: hidden;
    border-radius: 5px;
    border: 1px solid rgba(255, 255, 255, 0.12);
    touch-action: none; /* nothing to scroll or pan on the picture */
  }
  /* Native-sink hole: while this window holds the hardware video layer, the glass must stop
     painting (the native layer shows through transparent DOM); the bezel is painted as an opaque
     ring OUTSIDE the body instead — a box-shadow on the body, not on the hole: the body's
     overflow:hidden would clip a shadow of its child (see VideoWidget for the same trick). */
  .fw-frame.nv-active {
    background: transparent;
    backdrop-filter: none;
    -webkit-backdrop-filter: none;
  }
  .fw-body.nv-active {
    background: transparent;
    box-shadow: 0 0 0 4px rgba(30, 30, 30, 0.9);
    /* The body's 1 px border is translucent white — over the clipped (transparent) backdrop it read
       as a hairline gap around the picture. Opaque here: white 0.12 over the ring's grey. */
    border-color: #393939;
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
    /* Matches the body's rounding — the surface router reads this radius and cuts the
       hole with rounded corners, so the map behind caps the native layer's square edges. */
    border-radius: 5px;
  }
  .native-hole.armed {
    background: transparent;
  }
  .fw-body video,
  .fw-body img,
  .fw-body canvas {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
    /* Own compositing layer — see VideoWidget: keeps the 60 fps MJPEG <img> from dirtying shared
       layer tiles every frame on WebKitGTK. */
    will-change: transform;
  }
  .fw-body video.mirror,
  .fw-body img.mirror,
  .fw-body canvas.mirror {
    transform: scaleX(-1);
  }
  .fw-body video.rot180,
  .fw-body img.rot180,
  .fw-body canvas.rot180 {
    transform: rotate(180deg);
  }
  .fw-body video.mirror.rot180,
  .fw-body img.mirror.rot180,
  .fw-body canvas.mirror.rot180 {
    transform: scaleY(-1);
  }
  /* Unplug: invisible until the pointer is over the frame, then a small overlay button in the
     picture's top-left corner. */
  .fw-unplug {
    position: absolute;
    top: 8px;
    left: 8px;
    z-index: 2;
    box-sizing: border-box;
    width: 32px;
    height: 32px;
    /* Tight padding, and the glyph's viewBox is cropped to its own bounds — the chain has to stay
       readable at this size, where a 24-unit box with the usual margin left it too small (Marc). */
    padding: 4px;
    background: rgba(46, 46, 46, 0.82);
    border: 1px solid rgba(55, 168, 219, 0.5);
    border-radius: 6px;
    color: #37a8db;
    cursor: pointer;
    opacity: 0;
    pointer-events: none;
    transition: opacity 0.15s ease, background 0.2s;
  }
  .fw-body:hover .fw-unplug {
    opacity: 1;
    pointer-events: auto;
  }
  .fw-unplug:hover {
    background: rgba(55, 168, 219, 0.3);
  }
  .fw-unplug svg {
    width: 100%;
    height: 100%;
    fill: currentColor;
    stroke: none;
  }

  .fw-ph {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #888;
    font-size: 12px;
    text-align: center;
    padding: 0 10px;
  }

  /* Resize corner — a translucent light L, set INSIDE the picture rather than drawn into the
     bezel: it reads better there, and it is the same corner the detached window shows
     (DetachedVideoFrame's `.dv-grip`, still opaque grey). Translucent so it marks the corner
     without sitting on the picture (Marc, 2026-09-10). 7 px = the bezel's 4 px plus the 3 px
     inset both use. The box is the hit area. */
  .fw-grip {
    position: absolute;
    top: 7px;
    right: 7px;
    width: 26px;
    height: 26px;
    z-index: 62;
    pointer-events: auto;
    box-sizing: border-box;
    background: transparent;
    border-top: 4px solid rgba(190, 190, 190, 0.4);
    border-right: 4px solid rgba(190, 190, 190, 0.4);
    border-top-right-radius: 8px;
    cursor: nesw-resize;
    touch-action: none;
  }
  .fw-grip:hover {
    border-color: rgba(190, 190, 190, 0.6);
  }
  /* Move handle — the resize corner's colour and weight as a rounded square in the bottom-left
     corner, a four-way arrow filling it; the only thing that moves the window (see the header). */
  .fw-move {
    position: absolute;
    bottom: 7px;
    left: 7px;
    width: 26px;
    height: 26px;
    z-index: 62;
    pointer-events: auto;
    box-sizing: border-box;
    padding: 1px;
    background: transparent;
    border: 4px solid rgba(190, 190, 190, 0.4);
    border-radius: 8px;
    color: rgba(190, 190, 190, 0.4);
    cursor: grab;
    touch-action: none;
  }
  .fw-move:hover {
    border-color: rgba(190, 190, 190, 0.6);
    color: rgba(190, 190, 190, 0.6);
  }
  .fw-move:active {
    cursor: grabbing;
  }
  .fw-move svg {
    display: block;
    width: 100%;
    height: 100%;
    fill: none;
    stroke: currentColor;
    stroke-width: 2.4;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
</style>
