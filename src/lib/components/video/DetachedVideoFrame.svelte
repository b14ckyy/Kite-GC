<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // The detached video window's whole UI (VIDEO_MULTISINK_WINDOW.md PR B, §5.2). It runs in a
  // SECOND WebView — its own JS context, no app stores, no map, no telemetry — so everything it
  // knows arrives over the event channel from the main window (controllers/detachedVideo.ts).
  //
  // It looks like the in-app floating window (D4): the same glass ring around the picture, the same
  // lighter top-right corner for resizing, and the picture itself is the drag handle. The window is
  // transparent and undecorated, so the ring IS the window's edge.
  //
  // Its chrome is hover-only (D5): close in the top-left — the corner the app's unplug button sits
  // in, so the same corner takes the picture out and brings it back — and fullscreen bottom-right,
  // clear of the resize corner. Double-click does nothing (D7).
  //
  // The picture is the native decode sink's hardware layer showing through a transparent hole, the
  // same as in the app: this window runs its own surface router, publishing ONE surface (`floating`)
  // under its own window label. See controllers/nativeVideo.ts.
  import { onMount } from 'svelte';
  import { t } from 'svelte-i18n';
  import { getCurrentWindow, PhysicalSize } from '@tauri-apps/api/window';
  import { emitTo, listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { invoke } from '@tauri-apps/api/core';
  import {
    nativeSurface,
    activeNativeSurfaces,
    startNativeSurfaceRouter,
    stopNativeSurfaceRouter,
  } from '$lib/controllers/nativeVideo';
  import type { DetachedState } from '$lib/controllers/detachedVideo';

  /** The ring around the picture, per side (css px): the body's 4 px box-shadow plus its 1 px
   *  border — the in-app frame's FLOAT_BEZEL_PX in the same two parts. The PICTURE carries the
   *  stream's aspect, so the aspect snap has to know how much of the window is not picture. */
  const RING_PX = 5;
  /** How long the window may resize before we snap its height back onto the stream's aspect. Live
   *  snapping fights the OS resize loop (it owns the pointer during a drag), so it happens once the
   *  drag settles — the picture letterboxes for those few frames instead. */
  const SNAP_IDLE_MS = 140;
  /** Move/resize reports to the main window (which persists the box) — coarse on purpose. */
  const REPORT_IDLE_MS = 400;

  const win = getCurrentWindow();

  // NOT named `state`: `$state` would then read as that variable's store subscription.
  let feed = $state<DetachedState>({
    status: 'starting',
    error: null,
    aspect: 16 / 9,
    nativeSink: true,
    reconnecting: false,
    reconnectAttempt: 0,
    replayGap: false,
  });
  let fullscreen = $state(false);
  /** Is [`fullscreen`] the window's real state yet? Until it is, NOTHING may set the aspect lock:
   *  a lock set while the window is ALREADY fullscreen squeezes it — GTK applies the aspect to the
   *  fullscreen configure as well — and lifting it afterwards does not give the height back. The
   *  picture then sits in a window 80 px shorter than the screen, with the desktop above and below
   *  it where the letterbox belongs (Marc, Pi 5, 2026-09-08). */
  let stateKnown = $state(false);
  /** The windowed box (physical px) — kept across a fullscreen trip, which must not be saved as it. */
  let box = $state<{ x: number; y: number; w: number; h: number } | null>(null);
  /** The overlay controls and the resize corner — their rects go to the backend (`pushZones`). */
  let closeEl = $state<HTMLElement | undefined>(undefined);
  let fsEl = $state<HTMLElement | undefined>(undefined);
  let gripEl = $state<HTMLElement | undefined>(undefined);
  /** The frame's inner box — the picture is fitted into it (see [`stage`]). */
  let bodyEl = $state<HTMLElement | undefined>(undefined);
  /** The picture's box inside that frame (css px). The hole IS the picture, never the whole frame:
   *  the hardware layer letterboxes the stream inside whatever box it is given, and every pixel of
   *  the hole the picture does not cover is window transparency — the desktop shone through it
   *  (Marc, Pi 5, 2026-09-08). The mat behind the hole paints those pixels black instead, which is
   *  also where fullscreen's bars come from on a screen whose shape is not the stream's. */
  let stage = $state({ w: 0, h: 0 });

  // In a gap the hole goes: the layer below still holds the last picture before it.
  const live = $derived(feed.status === 'live' && feed.nativeSink && !feed.replayGap);
  const armed = $derived($activeNativeSurfaces.has('floating'));

  // The hardware layer only follows a hole while the router runs; it runs only while there is a
  // picture to place.
  $effect(() => {
    if (live) startNativeSurfaceRouter();
    else stopNativeSurfaceRouter();
  });

  // Fit the picture into the frame on every shape change of either. A ResizeObserver rather than
  // the window's own resize event: fullscreen, the ring and the window all change this box, and the
  // element knows about all three.
  $effect(() => {
    const el = bodyEl;
    const aspect = feed.aspect || 16 / 9;
    if (!el) return;
    const fit = (): void => {
      const w = Math.min(el.clientWidth, el.clientHeight * aspect);
      stage = { w, h: w / aspect };
    };
    fit();
    const ro = new ResizeObserver(fit);
    ro.observe(el);
    return () => ro.disconnect();
  });

  let snapTimer = 0;
  let reportTimer = 0;
  /** Once per window: the picture's first frame is also the one the Pi's GPU driver gets wrong —
   *  see `video_detached_nudge` for what that is and why only this window needs it. */
  let nudged = false;

  /** Overlay chrome visible? Driven by pointer activity, not by CSS `:hover`: WebKitGTK does not
   *  reliably deliver a leave event when the pointer leaves the window, so the buttons stayed on
   *  screen for good (Marc, Linux, 2026-09-08). Movement shows them, [`CHROME_IDLE_MS`] of quiet or
   *  losing focus hides them again — the way a video player behaves anyway. */
  let chrome = $state(false);
  let chromeTimer = 0;
  const CHROME_IDLE_MS = 1800;

  function wakeChrome(): void {
    chrome = true;
    clearTimeout(chromeTimer);
    chromeTimer = window.setTimeout(() => (chrome = false), CHROME_IDLE_MS);
  }
  function sleepChrome(): void {
    clearTimeout(chromeTimer);
    chrome = false;
  }

  // A saved box carries the aspect of whatever stream was running when it was saved, and the default
  // box ignores the ring — so the frame is fitted to the picture once the stream's aspect is known,
  // and again whenever it changes (a different source, a resolution switch).
  $effect(() => {
    const aspect = feed.aspect;
    if (!aspect || fullscreen) return;
    clearTimeout(snapTimer);
    snapTimer = window.setTimeout(() => void snapAspect(), SNAP_IDLE_MS);
  });

  // The picture is on screen: clear the framebuffer the driver may have filled with garbage.
  $effect(() => {
    if (!armed || nudged) return;
    nudged = true;
    void invoke('video_detached_nudge').catch(() => {});
  });

  // Whenever the page's own hit areas change — the chrome coming and going, fullscreen swallowing
  // them — the backend needs the new rects. A resize moves them too: see `onGeometryChanged`.
  $effect(() => {
    pushZones();
  });

  onMount(() => {
    document.documentElement.classList.add('detached-video');
    const offs: UnlistenFn[] = [];
    void listen<DetachedState>('video-detached-state', (e) => {
      feed = e.payload;
    }).then((u) => offs.push(u));
    // The main window may have pushed the state before this page had a listener — ask for it.
    void emitTo('main', 'video-detached-ready', {});
    // The fullscreen state FIRST: `readBox` must know not to record the screen as the window's box.
    void win
      .isFullscreen()
      .then((f) => {
        fullscreen = f;
        stateKnown = true;
        return readBox();
      })
      .catch(() => {});
    void win.onResized(onGeometryChanged).then((u) => offs.push(u));
    void win.onMoved(onGeometryChanged).then((u) => offs.push(u));
    // Focus gone = pointer gone, as far as the chrome is concerned.
    void win.onFocusChanged(({ payload }) => {
      if (!payload) sleepChrome();
    }).then((u) => offs.push(u));
    return () => {
      clearTimeout(snapTimer);
      clearTimeout(reportTimer);
      clearTimeout(chromeTimer);
      for (const u of offs) u();
      stopNativeSurfaceRouter();
      document.documentElement.classList.remove('detached-video');
    };
  });

  function onGeometryChanged(): void {
    pushZones();
    clearTimeout(snapTimer);
    clearTimeout(reportTimer);
    snapTimer = window.setTimeout(() => void snapAspect(), SNAP_IDLE_MS);
    reportTimer = window.setTimeout(() => void reportBox(), REPORT_IDLE_MS);
  }

  /** Remember the current windowed box (skipped while fullscreen — that box is the screen). */
  async function readBox(): Promise<void> {
    if (fullscreen) return;
    try {
      const pos = await win.outerPosition();
      const size = await win.innerSize();
      box = { x: pos.x, y: pos.y, w: size.width, h: size.height };
    } catch {
      /* the window is going away */
    }
  }

  async function reportBox(): Promise<void> {
    await readBox();
    if (!box) return;
    void emitTo('main', 'video-detached-geometry', { ...box, fullscreen }).catch(() => {});
  }

  /** Hand the backend the shape to hold while the user drags. The OS owns the resize loop, so a
   *  live aspect lock can only live there (`video_detached_aspect`); this page keeps the last word
   *  through [`snapAspect`], which now has next to nothing left to correct. Re-sent when the
   *  stream's aspect changes, on the way in and out of fullscreen (0 releases the window), and
   *  after every settle — the ring around the picture shifts the WINDOW's own ratio a little, and
   *  by more the smaller it is. */
  async function pushAspect(aspect: number): Promise<void> {
    try {
      const ring = 2 * RING_PX * (await win.scaleFactor());
      await invoke('video_detached_aspect', { aspect, ring });
    } catch {
      /* the window is going away */
    }
  }

  $effect(() => {
    // Read all three synchronously — an effect tracks nothing an await hides.
    const known = stateKnown;
    const aspect = fullscreen ? 0 : feed.aspect;
    if (!known) return;
    void pushAspect(aspect);
  });

  /** Snap the height so the PICTURE (the box inside the ring) carries the stream's aspect exactly —
   *  the same inside-out sizing the in-app frame uses, so neither ever shows bars. */
  async function snapAspect(): Promise<void> {
    if (fullscreen || !feed.aspect) return;
    try {
      const size = await win.innerSize();
      const ring = 2 * RING_PX * (await win.scaleFactor());
      // BOTH axes drive the result — the grip is a corner, so a diagonal pull has to land where
      // the pointer is. Always deriving the height from the width made the corner feel one-axis:
      // pulling it down grew the window and the snap pulled it straight back (Marc, 2026-09-08).
      // The picture keeps whichever side reaches FURTHER, so either direction leads and the other
      // follows — and no memory of where the drag began is needed.
      const pic = Math.max(size.width - ring, (size.height - ring) * feed.aspect);
      const next = { w: Math.round(pic + ring), h: Math.round(pic / feed.aspect + ring) };
      if (Math.abs(next.w - size.width) > 2 || Math.abs(next.h - size.height) > 2) {
        await win.setSize(new PhysicalSize(Math.max(200, next.w), Math.max(120, next.h)));
      }
      void pushAspect(feed.aspect);
    } catch {
      /* the window is going away */
    }
  }

  // Windows and macOS take their window gestures from here. On Linux these never fire: the GTK
  // press handler has already claimed the press (see `pushZones` and video::linux_drag).
  function onBodyPointerDown(e: PointerEvent): void {
    wakeChrome();
    if (e.button !== 0 || fullscreen) return;
    void win.startDragging();
  }

  function onGripPointerDown(e: PointerEvent): void {
    e.stopPropagation();
    void win.startResizeDragging('NorthEast');
  }

  /** Tell the backend which parts of the window the PAGE handles: its overlay buttons while they
   *  are on screen, and the resize corner. Linux starts the move and the resize from the GTK press
   *  handler — the only place the compositor accepts them from — and that handler cannot hit-test
   *  the DOM, so it is told beforehand. A no-op on Windows and macOS. */
  function pushZones(): void {
    const rect = (el: HTMLElement | undefined): [number, number, number, number] | null => {
      if (!el) return null;
      const b = el.getBoundingClientRect();
      return [b.x, b.y, b.width, b.height];
    };
    const held: [number, number, number, number][] = [];
    if (chrome) {
      for (const el of [closeEl, fsEl]) {
        const r = rect(el);
        if (r) held.push(r);
      }
    }
    void invoke('video_detached_chrome', {
      zones: { drag: !fullscreen, grip: rect(gripEl), chrome: held },
    }).catch(() => {});
  }

  async function toggleFullscreen(): Promise<void> {
    const next = !fullscreen;
    if (next) {
      await readBox(); // keep the windowed box; fullscreen must not overwrite it
      // And let the shape go BEFORE the compositor sizes the window: the lock constrains the
      // fullscreen configure too, and releasing it afterwards leaves the window short.
      await pushAspect(0);
    }
    fullscreen = next;
    await win.setFullscreen(next);
    // macOS drops the window's level on the way out of fullscreen (see video_detached_pin_top),
    // so the always-on-top promise has to be renewed on every toggle.
    void invoke('video_detached_pin_top').catch(() => {});
    if (!next) await readBox(); // back in a window — that box is the truth again
    if (box) void emitTo('main', 'video-detached-geometry', { ...box, fullscreen: next }).catch(() => {});
  }

  /// Ask the app to take the picture back rather than closing this window ourselves. The main
  /// window stops this window's video first and destroys it afterwards — the other way round the
  /// renderer loses its widget mid-frame and the whole stream dies with it (Linux, 2026-09-08).
  async function dockBack(): Promise<void> {
    // Withdraw this window's surfaces FIRST, from here: the backend then stops this window's own
    // pipeline while its widgets are still alive, on this command's worker thread — nothing in the
    // main app waits for it. Only then ask to be taken back.
    stopNativeSurfaceRouter();
    await invoke('video_rtsp_native_sink_surfaces', { surfaces: [] }).catch(() => {});
    void emitTo('main', 'video-detached-dock', {}).catch(() => void win.close());
  }

  function onKey(e: KeyboardEvent): void {
    if (e.key === 'Escape' && fullscreen) void toggleFullscreen();
  }
</script>

<svelte:window onkeydown={onKey} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="dv-root"
  class:fs={fullscreen}
  class:chrome
  onpointermove={wakeChrome}
  onpointerenter={wakeChrome}
  onpointerleave={sleepChrome}
>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="dv-body"
    class:armed
    bind:this={bodyEl}
    onpointerdown={onBodyPointerDown}
  >
    <!-- The letterbox. `data-nv-clip` makes the router cut the picture out of it, so it paints
         black exactly where the hardware layer is NOT — and nowhere else. -->
    <div class="dv-mat" data-nv-clip></div>
    {#if live}
      <!-- The transparent hole the hardware layer shows through — see controllers/nativeVideo. -->
      <div
        class="dv-hole"
        class:armed
        style="width: {stage.w}px; height: {stage.h}px"
        use:nativeSurface={'floating'}
      >
        {#if !armed}<span>{$t('video.starting')}</span>{/if}
      </div>
    {:else}
      <div class="dv-ph">
        {#if feed.reconnecting}
          <!-- The link dropped; the app is retrying. Stopping belongs in the app, so no button. -->
          {$t('video.reconnecting')}{feed.reconnectAttempt ? ` (${feed.reconnectAttempt})` : ''}
        {:else if feed.status === 'error'}
          ⚠ {feed.error}
        {:else if feed.replayGap}
          {$t('video.replayGap')}
        {:else}
          {$t('video.starting')}
        {/if}
      </div>
    {/if}

    <!-- Hover chrome (D5): invisible until the pointer is over the frame. -->
    <button
      bind:this={closeEl}
      class="dv-btn dv-close"
      onpointerdown={(e) => e.stopPropagation()}
      onclick={() => void dockBack()}
      title={$t('video.detachedClose')}
      aria-label={$t('video.detachedClose')}
    >
      <!-- chain, whole again: the counterpart of the app's broken-chain unplug button -->
      <svg viewBox="3.3 3.3 17.4 17.4" class="filled" aria-hidden="true">
        <path d="M15.69 12.83 19.23 9.29A3.2 3.2 0 0 0 14.71 4.77L11.17 8.31 12.44 9.58 15.98 6.04A1.4 1.4 0 0 1 17.96 8.02L14.42 11.56Z" />
        <path d="M8.31 11.17 4.77 14.71A3.2 3.2 0 0 0 9.29 19.23L12.83 15.69 11.56 14.42 8.02 17.96A1.4 1.4 0 0 1 6.04 15.98L9.58 12.44Z" />
        <path d="M14.51 10.83 10.83 14.51 9.49 13.17 13.17 9.49Z" />
      </svg>
    </button>
    <button
      bind:this={fsEl}
      class="dv-btn dv-fs"
      onpointerdown={(e) => e.stopPropagation()}
      onclick={() => void toggleFullscreen()}
      title={fullscreen ? $t('video.detachedWindowed') : $t('video.detachedFullscreen')}
      aria-label={fullscreen ? $t('video.detachedWindowed') : $t('video.detachedFullscreen')}
    >
      {#if fullscreen}
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="M9 4v5H4M15 4v5h5M9 20v-5H4M15 20v-5h5" />
        </svg>
      {:else}
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5" />
        </svg>
      {/if}
    </button>
  </div>

  {#if !fullscreen}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      bind:this={gripEl}
      class="dv-grip"
      onpointerdown={onGripPointerDown}
      title={$t('video.resizeWindow')}
    ></div>
  {/if}
</div>

<style>
  /* The window is transparent and undecorated: the page IS the frame. Only this route carries the
     class, so the main window's globals are untouched. */
  :global(html.detached-video),
  :global(html.detached-video body) {
    margin: 0;
    padding: 0;
    height: 100%;
    background: transparent;
    overflow: hidden;
    font-family: 'Segoe UI', Tahoma, sans-serif;
    color: #e0e0e0;
    user-select: none;
    -webkit-user-select: none;
  }

  .dv-root {
    position: fixed;
    inset: 0;
  }
  /* Glass ring around the picture — the in-app frame's `nv-active` look, drawn as a box-shadow so
     the body's own overflow can't clip it. `backdrop-filter` is pointless here: a standalone
     window has no page behind it to blur, only the desktop (which is not part of its backdrop). */
  .dv-body {
    position: absolute;
    inset: 4px;
    /* The picture sits centred in here; the mat fills whatever is left. */
    display: flex;
    align-items: center;
    justify-content: center;
    box-sizing: border-box;
    background: #000;
    overflow: hidden;
    border-radius: 5px;
    border: 1px solid #393939;
    /* No drop shadow: it would be painted outside the window and clipped to a smear at the edge.
       The ring IS the window's border — an undecorated window has no shadow of its own. */
    box-shadow: 0 0 0 4px rgba(30, 30, 30, 0.92);
    cursor: grab;
    touch-action: none;
  }
  .dv-body:active {
    cursor: grabbing;
  }
  .dv-body.armed {
    background: transparent;
  }
  .dv-root.fs .dv-body {
    inset: 0;
    border: none;
    border-radius: 0;
    box-shadow: none;
    cursor: default;
  }

  .dv-mat {
    position: absolute;
    inset: 0;
    background: #000;
  }

  .dv-hole {
    /* Sized to the picture (see `stage`), not to the frame. */
    position: relative;
    flex: none;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #888;
    font-size: 12px;
    background: #000;
    /* The router reads this radius and rounds the hole with it. */
    border-radius: 5px;
  }
  .dv-hole.armed {
    background: transparent;
  }
  .dv-root.fs .dv-hole {
    border-radius: 0;
  }
  .dv-ph {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0 10px;
    color: #888;
    font-size: 12px;
    text-align: center;
  }

  /* Hover-only overlay buttons, in the picture's corners. */
  .dv-btn {
    position: absolute;
    width: 32px;
    height: 32px;
    padding: 6px;
    background: rgba(46, 46, 46, 0.82);
    border: 1px solid rgba(55, 168, 219, 0.5);
    border-radius: 6px;
    color: #37a8db;
    cursor: pointer;
    opacity: 0;
    transition: opacity 0.15s ease, background 0.2s;
    pointer-events: none;
  }
  .dv-root.chrome .dv-btn {
    opacity: 1;
    pointer-events: auto;
  }
  .dv-btn:hover {
    background: rgba(55, 168, 219, 0.3);
  }
  .dv-btn svg {
    width: 100%;
    height: 100%;
    fill: none;
    stroke: currentColor;
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  /* The chain glyph is a filled shape cropped to its own bounds (see the unplug button in
     FloatingVideoWindow) — hence the smaller padding: it must read at this size. */
  .dv-btn svg.filled {
    fill: currentColor;
    stroke: none;
  }
  .dv-close {
    padding: 4px;
  }
  .dv-close {
    top: 8px;
    left: 8px;
  }
  .dv-fs {
    right: 8px;
    bottom: 8px;
  }

  /* Resize corner. The hit area is far bigger than the L it draws, and it sits INSIDE the window's
     own few-pixel resize border: that border is a ONE-AXIS edge resize, handled natively before the
     page ever sees the press, so a corner drawn on top of it grabbed a single axis whenever the
     pointer missed the very corner pixel (Marc, 2026-09-08). The easy 44 px square is ours now,
     both axes at once, and only the outer rim belongs to the window. */
  .dv-grip {
    position: absolute;
    top: 8px;
    right: 8px;
    width: 44px;
    height: 44px;
    cursor: nesw-resize;
    touch-action: none;
  }
  /* The visible L — the same corner the in-app floating frame draws (`.fw-grip`), a shade lighter
     than the ring — in that square's own corner. */
  .dv-grip::after {
    content: '';
    position: absolute;
    top: 0;
    right: 0;
    width: 26px;
    height: 26px;
    box-sizing: border-box;
    border-top: 4px solid #5e5e5e;
    border-right: 4px solid #5e5e5e;
    border-top-right-radius: 8px;
  }
  .dv-grip:hover::after {
    border-color: #727272;
  }
</style>
