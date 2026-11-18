<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- PhoneVideoDock — the phone's floating video window (Dev-Docs active/PHONE_VIDEO.md D2–D4, D7): a chromeless
     frame docked bottom-right of the map area, parked/recalled by one toggle, double-tap = map ⇄ video. It is the
     `floating` surface (store field, map location, native-sink id); +page computes its rect. -->
<script lang="ts">
  import { t } from 'svelte-i18n';
  import { videoStream, videoState, bindVideoEl, setMapLocation, toggleFloating, setFloatingTransient, togglePhoneDockCompact, reportMjpegError, reportImgSize, fpsProbe } from '$lib/stores/video';
  import { canvasSink, mjpegSink } from '$lib/controllers/mjpegSink';
  import { nativeSurface, activeNativeSurfaces } from '$lib/controllers/nativeVideo';
  import { doubleTap, mouseDoubleClick } from '$lib/helpers/doubleTap';
  import { phoneEdit } from '$lib/stores/phoneEdit.svelte';
  import { untrack } from 'svelte';
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
  // Not in widget edit mode: the window is parked for it (below), and a tap on the button would
  // first end edit mode — which recalls the window — and then toggle it away again, stored.
  const showButton = $derived(live && !widgetActive && !phoneEdit.editing);
  const open = $derived($videoState.floating && live && !widgetActive);

  // Mounted once opened while a source is live; with the native sink it stays mounted while parked — re-cutting
  // the Android SurfaceView's transparent region per toggle blanked the map. Only a source going away (or the
  // widget taking the picture) unmounts. A parked frame never holds the map (parking swaps it back first).
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
      // Parked: stays mounted (see above) — with the native sink only; a DOM video (MJPEG, camera)
      // would keep decoding off screen.
      if (live && !widgetActive && $videoState.nativeSink) return;
      if (!frameEl) { mounted = false; return; }
      const el = frameEl;
      const done = () => { el.removeEventListener('transitionend', done); if (parked) mounted = false; };
      el.addEventListener('transitionend', done);
      // Backstop: a display:none ancestor never fires transitionend.
      setTimeout(done, 450);
    }
  });

  // The button parks / recalls the frame, whatever it holds. With the map in the frame, parking hides the mini
  // map (+page moves its layer with the frame) and the button becomes a map icon in the corner the 2D/3D controls
  // vacate; double-tap the full-screen video to bring the map back.
  function toggle() {
    toggleFloating();
  }

  // Widget edit mode parks the docked window (it covers the bottom slots on most screens) and brings it back
  // afterwards — only if it was out, and not stored.
  let parkedForEdit = false;
  $effect(() => {
    const editing = phoneEdit.editing;
    untrack(() => {
      if (editing && $videoState.floating) {
        parkedForEdit = true;
        setFloatingTransient(false);
      } else if (!editing && parkedForEdit) {
        parkedForEdit = false;
        setFloatingTransient(true);
      }
    });
  });
  const label = $derived(
    mapHere
      ? ($videoState.floating ? $t('video.dockMapHide') : $t('video.dockMapShow'))
      : ($videoState.floating ? $t('video.dockHide') : $t('video.dockShow')),
  );

  let videoEl = $state<HTMLVideoElement | null>(null);
  $effect(() => {
    bindVideoEl(videoEl, $videoStream);
  });

  /** Where the native hole comes to rest (NativeSurfaceSpec.rest): the frame's box without the slide
   *  transform, so the video layer is in place before the frame moves; the hole is cut once it arrived.
   *  The matrix is in the frame's layout px, the box in viewport px (--ui-scale). */
  function restRect(el: HTMLElement): DOMRect | null {
    const r = el.getBoundingClientRect();
    if (!frameEl) return r;
    const t = getComputedStyle(frameEl).transform;
    if (!t || t === 'none') return r;
    const m = new DOMMatrix(t);
    const s = frameEl.offsetWidth ? frameEl.getBoundingClientRect().width / frameEl.offsetWidth : 1;
    return new DOMRect(r.left - m.e * s, r.top - m.f * s, r.width, r.height);
  }
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
      <svg viewBox="2 2 20 20" aria-hidden="true">
        <path d="M3 6.5 9 4l6 2.5 6-2.5v13.5L15 20l-6-2.5L3 20z" />
        <path d="M9 4v13.5M15 6.5V20" />
      </svg>
    {:else}
      <!-- camera -->
      <svg viewBox="2 2 20 20" aria-hidden="true">
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
    <!-- The frame keeps its ground while the hardware layer shows (data-nv-clip: the router cuts the
         hole out of it once the frame stands still) — a sliding frame shows its ground, not the map
         underneath, and the picture appears when it has arrived. -->
    <div class="dw-bg" data-nv-clip></div>
    {#if !mapHere}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="dw-body"
        class:nv-active={live && $videoState.nativeSink}
        ondblclick={mouseDoubleClick(() => setMapLocation('floating'))}
        use:doubleTap={() => setMapLocation('floating')}
      >
        {#if live && $videoState.nativeSink}
          <!-- Nothing of its own: until the layer is armed the frame's opaque ground covers the map. A
               placeholder vanishing at that moment was one more layer change that blanked the map. -->
          <div class="native-hole" use:nativeSurface={{ id: 'floating', rest: restRect }}></div>
        {:else if live && $videoState.mjpegUrl}
          {#if $canvasSink}
            <canvas use:mjpegSink class:mirror={$videoState.mirror} class:rot180={$videoState.rotate180}></canvas>
          {:else}
            <!-- svelte-ignore a11y_missing_attribute -->
            <img src={$videoState.mjpegUrl} class:mirror={$videoState.mirror} class:rot180={$videoState.rotate180} onload={reportImgSize} onerror={reportMjpegError} />
          {/if}
        {:else if live}
          <!-- svelte-ignore a11y_media_has_caption -->
          <video bind:this={videoEl} use:fpsProbe autoplay muted playsinline class:mirror={$videoState.mirror} class:rot180={$videoState.rotate180}></video>
        {/if}
        <VideoReconnectOverlay />
      </div>
      <!-- Size toggle (normal ⇄ compact), the desktop resize-corner look mirrored to the top-left. Video mode only:
           with the map in the frame, +page draws the same corner above the map. -->
      <button class="dw-size" onclick={togglePhoneDockCompact} title={$t('video.dockSize')} aria-label={$t('video.dockSize')}>
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="M6 6l12 12M6 6h6M6 6v6M18 18h-6M18 18v-6" />
        </svg>
      </button>
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
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    background: rgba(46, 46, 46, 0.9);
    border: 2px solid rgba(55, 168, 219, 0.5);
    border-radius: 6px;
    color: #37a8db;
    cursor: pointer;
    backdrop-filter: var(--glass-blur, blur(6px));
    -webkit-backdrop-filter: var(--glass-blur, blur(6px));
    transition: right 0.3s ease, background 0.2s, border-color 0.2s;
    pointer-events: auto;
  }
  .dock-btn.open {
    background: var(--btn-active-bg);
    border-color: #37a8db;
  }
  /* Video primary: the 2D/3D + follow buttons are hidden (mini map), the map toggle takes their
     corner. */
  .dock-btn.map-mode {
    bottom: calc(8px + var(--safe-bottom, 0px));
  }
  /* The drawing fills 75 % of the button, as on the corner controls: the viewBoxes are cropped to it
     (app.html, --map-btn-icon). */
  .dock-btn svg {
    flex: none;
    width: var(--map-btn-icon, 28.5px);
    height: var(--map-btn-icon, 28.5px);
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
    will-change: transform; /* its own layer at rest too — no promotion when the slide starts */
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
  /* Native sink: transparent from the start (the frame's ground covers the map until the hole is
     cut), not from the moment the layer is armed — that switch was a paint change in the frame at
     the very frame the masks change. */
  .dw-body.nv-active {
    background: transparent;
  }
  .native-hole {
    position: absolute;
    inset: 0;
    background: transparent;
    border-radius: 8px; /* read by the surface router — the hole is cut with these corners */
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
  /* Size toggle — the translucent L of the desktop frame's resize corner, in the top-left, with
     the diagonal double arrow inside. Above the body (z 61); the box is the hit area. */
  .dw-size {
    position: absolute;
    top: 4px;
    left: 4px;
    width: 26px;
    height: 26px;
    z-index: 62;
    pointer-events: auto;
    box-sizing: border-box;
    padding: 2px 0 0 2px;
    background: transparent;
    border: 0;
    border-top: 4px solid rgba(190, 190, 190, 0.4);
    border-left: 4px solid rgba(190, 190, 190, 0.4);
    border-top-left-radius: 8px;
    color: rgba(190, 190, 190, 0.4);
    cursor: pointer;
    touch-action: none;
  }
  .dw-size:hover {
    border-color: rgba(190, 190, 190, 0.6);
    color: rgba(190, 190, 190, 0.6);
  }
  .dw-size svg {
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
