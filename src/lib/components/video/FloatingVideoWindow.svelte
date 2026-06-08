<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // Floating video window — an in-app overlay sink for the video router.
  //  • snaps to the bottom-left corner (above the status bar; the bottom widget
  //    dock reflows out of the way — handled in +page.svelte) or floats freely
  //  • drag the header to move (away from the corner un-snaps; dropping near the
  //    corner re-snaps); corner-drag to resize (aspect-locked, 10–30 % of vh)
  //  • double-click the video to swap it with the map (→ videoPrimary)
  //  • ✕ closes the window (swaps back first if it was primary)
  //
  // Layering: the frame is drawn as separate absolutely-positioned layers that
  // share the page stacking context (the .float-win wrapper has no z-index, so it
  // creates no stacking context). That lets the map (rendered top-level in
  // +page when swapped) sit *between* the frame's blurred background (z 60) and
  // its header/resize chrome (z 62) at z 61 — so the map is fully interactive
  // while the header/resize stay usable. Frame styling matches the NavRail panels.
  import { t } from 'svelte-i18n';
  import {
    videoStream,
    videoState,
    setFloatPos,
    setFloatSnapped,
    setFloatHeightFrac,
    setVideoPrimary,
    toggleFloating,
  } from '$lib/stores/video';

  let vw = $state(typeof window !== 'undefined' ? window.innerWidth : 1280);
  let vh = $state(typeof window !== 'undefined' ? window.innerHeight : 720);

  let videoEl = $state<HTMLVideoElement | null>(null);
  $effect(() => {
    if (videoEl) videoEl.srcObject = $videoStream;
  });

  const MARGIN = 8;
  const SNAP_BOTTOM = 30; // align the snapped bottom with the widgets (above the 24px status bar)
  const SNAP_THRESHOLD = 56;

  const aspect = $derived($videoState.aspect || 16 / 9);
  const height = $derived(Math.round($videoState.floatHeightFrac * vh));
  const width = $derived(Math.min(Math.round(height * aspect), Math.round(vw * 0.7)));
  const left = $derived($videoState.floatSnapped ? MARGIN : $videoState.floatX);
  const top = $derived($videoState.floatSnapped ? vh - height - SNAP_BOTTOM : $videoState.floatY);

  // ── Drag (header) ──────────────────────────────────────────────────
  let pendingDrag = false;
  let moved = false;
  let startX = 0;
  let startY = 0;
  let baseLeft = 0;
  let baseTop = 0;

  function onHeaderPointerDown(e: PointerEvent) {
    if ((e.target as HTMLElement).closest('.fw-btn')) return; // let buttons click
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

  // ── Resize (bottom-right handle) ───────────────────────────────────
  let resizing = false;
  let resizeStartY = 0;
  let startFrac = 0;
  function onResizePointerDown(e: PointerEvent) {
    e.stopPropagation();
    resizing = true;
    resizeStartY = e.clientY;
    startFrac = $videoState.floatHeightFrac;
    window.addEventListener('pointermove', onResizeMove);
    window.addEventListener('pointerup', onResizeUp);
  }
  function onResizeMove(e: PointerEvent) {
    if (!resizing) return;
    setFloatHeightFrac(startFrac + (e.clientY - resizeStartY) / vh);
  }
  function onResizeUp() {
    resizing = false;
    window.removeEventListener('pointermove', onResizeMove);
    window.removeEventListener('pointerup', onResizeUp);
  }
</script>

<svelte:window bind:innerWidth={vw} bind:innerHeight={vh} />

{#if $videoState.floating}
  <!-- No z-index on the wrapper → no stacking context; the layers below compose
       with the top-level map (z 61) rendered in +page when swapped. -->
  <div class="float-win" style="left:{left}px; top:{top}px; width:{width}px; height:{height}px;">
    <!-- frame background (drawn first / behind), frosted like the NavRail panels -->
    <div class="fw-bg"></div>

    <!-- header strip (on top) — drag handle + close -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="fw-header" onpointerdown={onHeaderPointerDown}>
      <span class="fw-title">{$videoState.videoPrimary ? $t('nav.video') : $t('video.title')}</span>
      <button
        class="fw-btn"
        onclick={() => ($videoState.videoPrimary ? setVideoPrimary(false) : toggleFloating())}
        title={$t('video.close')}
      >✕</button>
    </div>

    <!-- content: the video (normal). When swapped, the map (top-level in +page)
         fills this area at z 61 instead. -->
    {#if !$videoState.videoPrimary}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="fw-body" ondblclick={() => setVideoPrimary(true)}>
        {#if $videoState.status === 'live'}
          <!-- svelte-ignore a11y_media_has_caption -->
          <video bind:this={videoEl} autoplay muted playsinline class:mirror={$videoState.mirror}></video>
        {:else}
          <div class="fw-ph">{$videoState.status === 'starting' ? $t('video.starting') : $t('video.off')}</div>
        {/if}
      </div>
    {/if}

    <!-- resize grip (on top, usable in both modes) -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="fw-resize" onpointerdown={onResizePointerDown} title="Resize"></div>
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
  .fw-header {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    height: 20px;
    z-index: 62;
    pointer-events: auto;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 4px 0 8px;
    box-sizing: border-box;
    cursor: grab;
  }
  .fw-header:active {
    cursor: grabbing;
  }
  .fw-title {
    flex: 1;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: #9ad0e8;
  }
  .fw-btn {
    width: 16px;
    height: 16px;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 11px;
    line-height: 1;
    color: #aaa;
    background: transparent;
    border: none;
    border-radius: 3px;
    cursor: pointer;
  }
  .fw-btn:hover {
    background: rgba(255, 255, 255, 0.12);
    color: #fff;
  }
  .fw-body {
    position: absolute;
    left: 0;
    right: 0;
    top: 20px;
    bottom: 0;
    z-index: 61;
    pointer-events: auto;
    background: #000;
    overflow: hidden;
    border-radius: 0 0 7px 7px;
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
  .fw-resize {
    position: absolute;
    right: 0;
    bottom: 0;
    width: 16px;
    height: 16px;
    z-index: 62;
    pointer-events: auto;
    cursor: nwse-resize;
    background: linear-gradient(135deg, transparent 50%, rgba(55, 168, 219, 0.5) 50%);
  }
</style>
