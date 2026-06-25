<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // Floating video window — a chromeless in-app overlay sink for the video router.
  //  • snaps to the bottom-left corner (above the status bar; the bottom widget dock reflows out of
  //    the way — handled in +page.svelte) or floats freely
  //  • drag the video body to move (away from the corner un-snaps; dropping near the corner re-snaps)
  //  • TOP-RIGHT corner grip resizes (aspect-locked, 10–30 % of vh, touch-friendly)
  //  • TOP-LEFT ✕ closes (swaps back first if it was primary)
  //  • double-click the video to swap it with the map (→ videoPrimary)
  //
  // No title bar (space is precious on a flight display). Layering: separate absolutely-positioned
  // layers share the page stacking context (the .float-win wrapper has no z-index). The map (rendered
  // top-level in +page when swapped) composes between the frosted background (z 60) and the corner
  // controls (z 62), so the mini-map stays interactive while close/resize stay usable.
  import { t } from 'svelte-i18n';
  import {
    videoStream,
    videoState,
    bindVideoEl,
    setFloatPos,
    setFloatSnapped,
    setFloatHeightFrac,
    setMapLocation,
    toggleFloating,
  } from '$lib/stores/video';

  // True while the map occupies this floating frame (so this window shows the map, not video).
  const mapHere = $derived($videoState.mapLocation === 'floating');

  let vw = $state(typeof window !== 'undefined' ? window.innerWidth : 1280);
  let vh = $state(typeof window !== 'undefined' ? window.innerHeight : 720);

  let videoEl = $state<HTMLVideoElement | null>(null);
  $effect(() => {
    bindVideoEl(videoEl, $videoStream);
  });

  const MARGIN = 8;
  const SNAP_BOTTOM = 30; // align the snapped bottom with the widgets (above the 24px status bar)
  const SNAP_THRESHOLD = 56;
  const FRAC_MIN = 0.1;
  const FRAC_MAX = 0.3;
  // Floor the height so the mini-map's 4 stacked control buttons (4×38 + 3×8 gap + 8 bottom offset +
  // breathing room) never overflow the frame in videoPrimary mode.
  const MIN_H_PX = 200;

  let floatWinEl = $state<HTMLDivElement | null>(null);

  const aspect = $derived($videoState.aspect || 16 / 9);
  const height = $derived(
    Math.min(Math.round(FRAC_MAX * vh), Math.max(MIN_H_PX, Math.round($videoState.floatHeightFrac * vh))),
  );
  const width = $derived(Math.min(Math.round(height * aspect), Math.round(vw * 0.7)));
  const left = $derived($videoState.floatSnapped ? MARGIN : $videoState.floatX);
  const top = $derived($videoState.floatSnapped ? vh - height - SNAP_BOTTOM : $videoState.floatY);

  // This ✕ only shows while the window holds video → it closes the floating window. (When the map is
  // in the frame, +page renders its own ✕ on top that sends the map back to the main view instead.)
  function closeWindow() {
    toggleFloating();
  }

  // ── Drag (from the video body) ─────────────────────────────────────
  let pendingDrag = false;
  let moved = false;
  let startX = 0;
  let startY = 0;
  let baseLeft = 0;
  let baseTop = 0;

  function onBodyPointerDown(e: PointerEvent) {
    if ((e.target as HTMLElement).closest('.fw-corner')) return; // let the corner controls handle it
    pendingDrag = true;
    moved = false;
    startX = e.clientX;
    startY = e.clientY;
    baseLeft = left;
    baseTop = top;
    window.addEventListener('pointermove', onDragMove);
    window.addEventListener('pointerup', onDragUp);
  }
  function onDragMove(e: PointerEvent) {
    if (!pendingDrag) return;
    const dx = e.clientX - startX;
    const dy = e.clientY - startY;
    if (!moved && Math.hypot(dx, dy) < 4) return;
    if (!moved) {
      moved = true;
      setFloatSnapped(false); // first real movement detaches from the corner
    }
    const nx = Math.max(0, Math.min(baseLeft + dx, vw - width));
    const ny = Math.max(0, Math.min(baseTop + dy, vh - height));
    setFloatPos(nx, ny);
  }
  function onDragUp() {
    window.removeEventListener('pointermove', onDragMove);
    window.removeEventListener('pointerup', onDragUp);
    if (!pendingDrag) return;
    pendingDrag = false;
    if (!moved) return;
    // Re-snap if dropped near the bottom-left corner.
    const nearLeft = $videoState.floatX <= MARGIN + SNAP_THRESHOLD;
    const nearBottom = $videoState.floatY + height >= vh - SNAP_BOTTOM - SNAP_THRESHOLD;
    if (nearLeft && nearBottom) setFloatSnapped(true);
  }

  // ── Resize (top-right handle) ──────────────────────────────────────
  // Dragging the top-right corner grows the window up + right with the bottom-left anchored:
  // up = bigger, and for a free (un-snapped) window we move the top so the bottom edge stays put.
  let resizing = false;
  let resizeStartY = 0;
  let startFrac = 0;
  let startBottom = 0;
  let startSnapped = false;
  function onResizePointerDown(e: PointerEvent) {
    e.stopPropagation();
    resizing = true;
    resizeStartY = e.clientY;
    startFrac = $videoState.floatHeightFrac;
    startBottom = top + height;
    startSnapped = $videoState.floatSnapped;
    window.addEventListener('pointermove', onResizeMove);
    window.addEventListener('pointerup', onResizeUp);
  }
  function onResizeMove(e: PointerEvent) {
    if (!resizing) return;
    const delta = (resizeStartY - e.clientY) / vh; // drag up → larger
    const fracMin = Math.max(FRAC_MIN, MIN_H_PX / vh); // honour the 4-button px floor
    const newFrac = Math.min(FRAC_MAX, Math.max(fracMin, startFrac + delta));
    setFloatHeightFrac(newFrac);
    if (!startSnapped) {
      // Keep the bottom edge fixed (top-right grip): top = bottom − newHeight.
      setFloatPos($videoState.floatX, startBottom - newFrac * vh);
    }
  }
  function onResizeUp() {
    resizing = false;
    window.removeEventListener('pointermove', onResizeMove);
    window.removeEventListener('pointerup', onResizeUp);
  }

  // ── Mini-map move (videoPrimary) — right mouse / two-finger ─────────
  // In primary mode the frame holds the interactive map, so left-drag/single-touch pan the map. To
  // MOVE the frame we grab with the right mouse button (desktop) or two fingers (touch); pinch-zoom
  // is sacrificed there (the zoom buttons + follow mode cover it). Window-level capture so we run
  // before Leaflet and can stop it seeing the gesture.
  let fmActive = false;
  let fmStartX = 0;
  let fmStartY = 0;
  let fmBaseLeft = 0;
  let fmBaseTop = 0;
  let fmMoved = false;

  function pointInFrame(cx: number, cy: number): boolean {
    const r = floatWinEl?.getBoundingClientRect();
    return !!r && cx >= r.left && cx <= r.right && cy >= r.top && cy <= r.bottom;
  }
  function frameMoveStart(cx: number, cy: number) {
    fmActive = true;
    fmMoved = false;
    fmStartX = cx;
    fmStartY = cy;
    fmBaseLeft = left;
    fmBaseTop = top;
  }
  function frameMoveTo(cx: number, cy: number) {
    if (!fmActive) return;
    const dx = cx - fmStartX;
    const dy = cy - fmStartY;
    if (!fmMoved && Math.hypot(dx, dy) < 4) return;
    if (!fmMoved) {
      fmMoved = true;
      setFloatSnapped(false);
    }
    setFloatPos(
      Math.max(0, Math.min(fmBaseLeft + dx, vw - width)),
      Math.max(0, Math.min(fmBaseTop + dy, vh - height)),
    );
  }
  function frameMoveEnd() {
    if (!fmActive) return;
    fmActive = false;
    if (!fmMoved) return;
    const nearLeft = $videoState.floatX <= MARGIN + SNAP_THRESHOLD;
    const nearBottom = $videoState.floatY + height >= vh - SNAP_BOTTOM - SNAP_THRESHOLD;
    if (nearLeft && nearBottom) setFloatSnapped(true);
  }

  $effect(() => {
    if (!$videoState.floating || !mapHere) return;
    const mid = (t: TouchList) => ({
      x: (t[0].clientX + t[1].clientX) / 2,
      y: (t[0].clientY + t[1].clientY) / 2,
    });
    const onCtx = (e: MouseEvent) => {
      if (pointInFrame(e.clientX, e.clientY)) e.preventDefault(); // no context menu over the frame
    };
    const onPD = (e: PointerEvent) => {
      if (e.button === 2 && pointInFrame(e.clientX, e.clientY)) {
        e.preventDefault();
        e.stopPropagation();
        frameMoveStart(e.clientX, e.clientY);
      }
    };
    const onPM = (e: PointerEvent) => {
      if (fmActive) frameMoveTo(e.clientX, e.clientY);
    };
    const onPU = () => frameMoveEnd();
    const onTS = (e: TouchEvent) => {
      if (e.touches.length === 2) {
        const m = mid(e.touches);
        if (pointInFrame(m.x, m.y)) {
          e.preventDefault();
          e.stopPropagation();
          frameMoveStart(m.x, m.y);
        }
      }
    };
    const onTM = (e: TouchEvent) => {
      if (fmActive && e.touches.length >= 2) {
        e.preventDefault();
        e.stopPropagation();
        const m = mid(e.touches);
        frameMoveTo(m.x, m.y);
      }
    };
    const onTE = (e: TouchEvent) => {
      if (fmActive && e.touches.length < 2) frameMoveEnd();
    };
    window.addEventListener('contextmenu', onCtx, true);
    window.addEventListener('pointerdown', onPD, true);
    window.addEventListener('pointermove', onPM, true);
    window.addEventListener('pointerup', onPU, true);
    window.addEventListener('touchstart', onTS, { capture: true, passive: false });
    window.addEventListener('touchmove', onTM, { capture: true, passive: false });
    window.addEventListener('touchend', onTE, true);
    return () => {
      window.removeEventListener('contextmenu', onCtx, true);
      window.removeEventListener('pointerdown', onPD, true);
      window.removeEventListener('pointermove', onPM, true);
      window.removeEventListener('pointerup', onPU, true);
      window.removeEventListener('touchstart', onTS, true);
      window.removeEventListener('touchmove', onTM, true);
      window.removeEventListener('touchend', onTE, true);
    };
  });
</script>

<svelte:window bind:innerWidth={vw} bind:innerHeight={vh} />

{#if $videoState.floating}
  <!-- No z-index on the wrapper → no stacking context; layers compose with the top-level map. -->
  <div bind:this={floatWinEl} class="float-win" style="left:{left}px; top:{top}px; width:{width}px; height:{height}px;">
    <!-- frosted frame background (behind), like the NavRail panels -->
    <div class="fw-bg"></div>

    <!-- content: the video. When the map is in this frame, it's rendered (top-level) by +page here
         instead, and the body is omitted. Double-click the video → the map jumps into this frame. -->
    {#if !mapHere}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="fw-body" onpointerdown={onBodyPointerDown} ondblclick={() => setMapLocation('floating')}>
        {#if $videoState.status === 'live'}
          <!-- svelte-ignore a11y_media_has_caption -->
          <video bind:this={videoEl} autoplay muted playsinline class:mirror={$videoState.mirror}></video>
        {:else}
          <div class="fw-ph">{$videoState.status === 'starting' ? $t('video.starting') : $t('video.off')}</div>
        {/if}
      </div>
    {/if}

    <!-- Corner controls (video mode only). When the map fills this frame, the map (a separate unzoomed
         top-level layer) covers these, so +page renders the equivalents above it. -->
    {#if !mapHere}
      <!-- close (top-left) — overlay, touch-sized -->
      <button class="fw-corner fw-close" onclick={closeWindow} title={$t('video.close')}>✕</button>

      <!-- resize grip (top-right) — visible, touch-sized -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="fw-corner fw-resize" onpointerdown={onResizePointerDown} title="Resize"></div>
    {/if}
  </div>
{/if}

<style>
  .float-win {
    position: absolute;
    /* No z-index on purpose (see script header). */
    pointer-events: none; /* layers opt back in individually */
  }
  /* Frame background — frosted panel, same look as the NavRail floating panels */
  .fw-bg {
    position: absolute;
    inset: 0;
    z-index: 60;
    pointer-events: none;
    background: rgba(46, 46, 46, 0.92);
    backdrop-filter: blur(12px);
    border: 1px solid rgba(55, 168, 219, 0.35);
    border-radius: 8px;
    box-shadow: 0 4px 20px rgba(0, 0, 0, 0.4);
  }
  .fw-body {
    position: absolute;
    inset: 0;
    z-index: 61;
    pointer-events: auto;
    background: #000;
    overflow: hidden;
    border-radius: 8px;
    cursor: grab;
    touch-action: none; /* let pointer-drag move the window instead of scrolling/panning */
  }
  .fw-body:active {
    cursor: grabbing;
  }
  .fw-body video {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .fw-body video.mirror {
    transform: scaleX(-1);
  }
  .fw-ph {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #888;
    font-size: 12px;
  }

  /* Corner controls — overlay the content corners (no title bar); touch-sized. */
  .fw-corner {
    position: absolute;
    top: 0;
    width: 26px;
    height: 26px;
    z-index: 62;
    pointer-events: auto;
    box-sizing: border-box;
    touch-action: none;
  }
  .fw-close {
    left: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 13px;
    line-height: 1;
    color: #e0e0e0;
    background: rgba(0, 0, 0, 0.45);
    border: none;
    border-radius: 8px 0 8px 0;
    cursor: pointer;
  }
  .fw-close:hover {
    background: rgba(212, 0, 0, 0.7);
    color: #fff;
  }
  .fw-resize {
    right: 0;
    cursor: nesw-resize;
    border-radius: 0 8px 0 8px;
    /* visible grab affordance in the top-right corner */
    background: linear-gradient(225deg, rgba(55, 168, 219, 0.85) 42%, transparent 42%);
  }
  .fw-resize:hover {
    background: linear-gradient(225deg, rgba(55, 168, 219, 1) 50%, transparent 50%);
  }
</style>
