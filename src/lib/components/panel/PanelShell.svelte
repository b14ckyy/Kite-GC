<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts" module>
  export type PanelVariant = 'info' | 'compact' | 'advanced' | 'fullscreen' | 'wide-compact';
</script>

<script lang="ts">
  import type { Snippet } from 'svelte';
  import { panelDockRight, panelAlertLevel } from '$lib/stores/panelDock';

  // Reusable panel frame (docs/active/PANEL_FRAMEWORK.md): owns frame, positioning, per-variant sizing, the
  // framed field's scroll/bounds and the live variant transition; callers fill the snippet slots. Lives inside
  // `.ui-scale`, so it scales with --ui-scale.
  let {
    variant = 'compact',
    title = '',
    headerActions = undefined,
    toolbar = undefined,
    body = undefined,
    footer = undefined,
    detailTitle = '',
    detailActions = undefined,
    detailToolbar = undefined,
    detail = undefined,
    detailFooter = undefined,
    params = undefined,
    children = undefined,
  }: {
    variant?: PanelVariant;
    title?: string;
    /** Extra controls in the (left/main) header, right of the title. */
    headerActions?: Snippet;
    /** Optional action row under the header. */
    toolbar?: Snippet;
    /** Main framed working area (or use default children). */
    body?: Snippet;
    /** Pinned footer button area. */
    footer?: Snippet;
    /** advanced: right-region title. */
    detailTitle?: string;
    /** advanced: right-region header controls. */
    detailActions?: Snippet;
    /** advanced: right-region action row (keeps the two fields vertically aligned). */
    detailToolbar?: Snippet;
    /** advanced: right-region framed content. */
    detail?: Snippet;
    /** advanced: right-region pinned footer. */
    detailFooter?: Snippet;
    /** fullscreen: left parameter column. */
    params?: Snippet;
    children?: Snippet;
  } = $props();

  // Publish the panel's live right edge (viewport px, transform-aware) for frame-pinned overlays (stores/panelDock).
  // ResizeObserver fires on observe(), so a tab switch self-corrects even if the old shell's destroy runs last;
  // the teardown reset covers plain close.
  let root = $state<HTMLElement>();
  $effect(() => {
    const el = root;
    if (!el) return;
    const publish = () => panelDockRight.set(el.getBoundingClientRect().right);
    const ro = new ResizeObserver(publish);
    ro.observe(el);
    // The observer's first call lands in the entry animation, which shifts the shell sideways.
    el.addEventListener('animationend', publish);
    return () => { ro.disconnect(); el.removeEventListener('animationend', publish); panelDockRight.set(0); };
  });
</script>

<div class="ps ps-{variant}" class:ps-alert-warning={$panelAlertLevel === 'warning'} class:ps-alert-caution={$panelAlertLevel === 'caution'} bind:this={root}>
  {#if variant === 'fullscreen' || variant === 'wide-compact'}
    {#if title || headerActions}
      <header class="ps-head">
        <span class="ps-title">{title}</span>
        {#if headerActions}<div class="ps-head-actions">{@render headerActions()}</div>{/if}
      </header>
    {/if}
    <div class="ps-fs-row">
      {#if params}<aside class="ps-params">{@render params()}</aside>{/if}
      <div class="ps-fs-content">
        {#if body}{@render body()}{:else if children}{@render children()}{/if}
      </div>
    </div>
    {#if footer}<footer class="ps-fs-foot">{@render footer()}</footer>{/if}
  {:else}
    <section class="ps-col ps-col-main">
      <header class="ps-head">
        <span class="ps-title">{title}</span>
        {#if headerActions}<div class="ps-head-actions">{@render headerActions()}</div>{/if}
      </header>
      {#if toolbar}<div class="ps-toolbar">{@render toolbar()}</div>{/if}
      <div class="ps-field">
        {#if body}{@render body()}{:else if children}{@render children()}{/if}
      </div>
      {#if footer}<footer class="ps-foot">{@render footer()}</footer>{/if}
    </section>

    {#if variant === 'advanced' && (detail || detailToolbar || detailActions || detailFooter || detailTitle)}
      <section class="ps-col ps-col-detail">
        <header class="ps-head">
          <span class="ps-title">{detailTitle}</span>
          {#if detailActions}<div class="ps-head-actions">{@render detailActions()}</div>{/if}
        </header>
        {#if detailToolbar}<div class="ps-toolbar">{@render detailToolbar()}</div>{/if}
        <div class="ps-field">{#if detail}{@render detail()}{/if}</div>
        {#if detailFooter}<footer class="ps-foot">{@render detailFooter()}</footer>{/if}
      </section>
    {/if}
  {/if}
</div>

<style>
  /* ── Frame + positioning ─────────────────────────────────── */
  .ps, .ps :global(*) { box-sizing: border-box; }

  .ps {
    position: absolute;
    top: 65px;
    left: 62px;
    z-index: 150;
    display: flex;
    overflow: hidden;
    color: #e0e0e0;
    font-size: 13px;
    background: rgba(46, 46, 46, 0.92);
    border: 1px solid rgba(55, 168, 219, 0.35);
    border-radius: 8px;
    box-shadow: 0 4px 20px rgba(0, 0, 0, 0.4);
    backdrop-filter: var(--panel-blur, blur(10px));
    -webkit-backdrop-filter: var(--panel-blur, blur(10px));
    /* Live variant transitions (the shell persists across rail switches). `interpolate-size` (Chromium 129+) lets
       width/height animate to/from the `info` variant's intrinsic sizes. */
    interpolate-size: allow-keywords;
    transition: width 0.25s ease, height 0.25s ease, top 0.25s ease, transform 0.25s ease, opacity 0.25s ease;
    animation: ps-in 0.18s ease-out;
  }
  /* Low-power mode switches the blur off, and with it the panel's own compositor layer: re-rastered after each
     transition, which over a transparent native-video window showed as the panel vanishing for a frame. Keeping
     the layer costs one texture. */
  :global(html.kite-low-power) .ps {
    will-change: transform;
  }
  /* Parked: the rail's active button was clicked again — the panel slides out to the left with
     its state intact (+page sets .panels-hidden on the host); the next click slides it back. */
  :global(.panels-hidden) .ps {
    transform: translateX(calc(-100% - 80px));
    opacity: 0;
    pointer-events: none;
  }

  /* Bounding height for compact/advanced (cap for info): below the toolbar, above `--panel-bottom-reserve` and
     the status bar. +page sets the reserve (docked video, dock tiles, or the rail's 6px gap); the fallback is
     for hosts without the var (PanelPlayground). */
  .ps-info,
  .ps-compact,
  .ps-advanced {
    flex-direction: column;
  }
  .ps-compact,
  .ps-advanced {
    height: calc(100% - 53px - var(--panel-bottom-reserve, var(--grid-bottom-height)) - 24px - 12px);
  }

  .ps-info {
    width: max-content;
    max-width: 360px;
    height: auto;
    max-height: calc(100% - 53px - var(--panel-bottom-reserve, var(--grid-bottom-height)) - 24px - 12px);
  }

  /* Mobile, any orientation: shift panels clear of a landscape side-notch. Harmless on iPad / portrait
     (--safe-left is 0), so it does not change those layouts. */
  :global(html.is-mobile) .ps {
    left: calc(62px + var(--safe-left, 0px));
  }
  /* Portrait phone only: the toolbar collapses/expands, so the panel tracks its live height. iPad and
     landscape phone keep the fixed top (single-row bar). */
  @media (max-width: 600px) {
    :global(html.is-phone) .ps {
      top: calc(var(--toolbar-h, 65px) + 8px);
    }
  }
  /* Tablet: the toolbar is taller (iOS status bar + safe-top), so track the live bar height and size the panel
     to the gap above the bottom dock. Same `--panel-bottom-reserve` rule as the desktop: on a window too short
     for the nav rail the panel may overlay the dock. */
  :global(html.is-tablet) .ps {
    top: calc(var(--toolbar-h, 65px) + 8px);
  }
  :global(html.is-tablet) .ps-compact,
  :global(html.is-tablet) .ps-advanced {
    height: calc(100% - var(--toolbar-h, 65px) - var(--panel-bottom-reserve, var(--grid-bottom-height)) - 24px - 20px);
  }
  :global(html.is-tablet) .ps-info {
    max-height: calc(100% - var(--toolbar-h, 65px) - var(--panel-bottom-reserve, var(--grid-bottom-height)) - 24px - 20px);
  }
  /* Phone layout (Dev-Docs archive/PHONE_UI.md): no toolbar — the panel starts at the top safe inset, full
     height. Width as on the desktop, bounded by the SCREEN (rail 62px left, 8px right): panels may cover the
     widget column (z 150+ over 100), since the map area alone is too narrow for the advanced layouts. */
  :global(html.is-phone) .ps {
    top: calc(8px + var(--safe-top, 0px));
  }
  :global(html.is-phone) .ps-compact {
    width: min(398px, calc(100vw - var(--safe-left, 0px) - 62px - 8px));
  }
  :global(html.is-phone) .ps-info {
    max-width: min(360px, calc(100vw - var(--safe-left, 0px) - 62px - 8px));
  }
  :global(html.is-phone) .ps-advanced {
    max-width: calc(100vw - var(--safe-left, 0px) - 62px - 8px);
  }
  :global(html.is-phone) .ps-compact,
  :global(html.is-phone) .ps-advanced {
    height: calc(100% - 16px - var(--safe-top, 0px) - var(--safe-bottom, 0px));
  }
  :global(html.is-phone) .ps-info {
    max-height: calc(100% - 16px - var(--safe-top, 0px) - var(--safe-bottom, 0px));
  }
  /* Widths are driven by the *field* size (the thin-framed working box), which the panel layouts
     were tuned against: 380px main field, 500px detail field. Panel width = field + the column's
     8px padding each side (+ the 1px shell border): 380 + 16 = 396, 500 + 16 = 516. */
  .ps-compact {
    width: 398px;
  }
  .ps-advanced {
    flex-direction: row;
    width: 914px;
    max-width: calc(100% - 62px - var(--grid-side-width) - 54px - 12px);
  }

  /* Fullscreen + Wide-Compact: floating overlays, left-anchored and sized by width/height (no `right`) so any
     variant can animate into any other. Fullscreen = ~62px inset all round; Wide-Compact = a short top strip
     that stops before the side dock. */
  .ps-fullscreen,
  .ps-wide-compact {
    top: 62px;
    left: 62px;
    z-index: 160;
    flex-direction: column;
    border-radius: 10px;
    border: 1px solid rgba(55, 168, 219, 0.4);
    background: rgba(40, 40, 40, 0.96);
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.55);
    backdrop-filter: var(--panel-blur, blur(10px));
    -webkit-backdrop-filter: var(--panel-blur, blur(10px));
  }
  .ps-fullscreen {
    width: calc(100% - 124px);
    height: calc(100% - 124px);
  }
  .ps-wide-compact {
    width: calc(100% - 62px - var(--grid-side-width) - 54px - 6px);
    height: max(20vh, 160px);
  }
  /* Phone (PHONE_UI.md): no toolbar / status bar / side dock — the fullscreen panel spans safe inset to safe
     inset and from the rail to the right screen edge (over the widget column). */
  :global(html.is-phone) .ps-fullscreen,
  :global(html.is-phone) .ps-wide-compact {
    top: calc(8px + var(--safe-top, 0px));
    left: calc(62px + var(--safe-left, 0px));
  }
  :global(html.is-phone) .ps-fullscreen {
    width: calc(100% - 62px - var(--safe-left, 0px) - 8px);
    height: calc(100% - 16px - var(--safe-top, 0px) - var(--safe-bottom, 0px));
  }
  :global(html.is-phone) .ps-wide-compact {
    width: calc(100% - 62px - var(--safe-left, 0px) - 8px);
  }

  /* ── Columns (compact = 1, advanced = 1:2; right wider for previews/maps) ── */
  .ps-col {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    padding: 8px;
    gap: 6px;
    /* When the field can't shrink below its 200px minimum and header+field+footer no longer fit
       the available height, the whole column scrolls (header/footer included). With room to
       spare the field flexes to fill and only it scrolls (footer stays pinned). */
    overflow-y: auto;
  }
  .ps-compact .ps-col-main,
  .ps-info .ps-col-main {
    flex: 1;
    width: 100%;
  }
  /* Advanced: fixed-width main column (380px field + 16px padding); the detail column fills the
     rest → a 500px field. Separation is by spacing only — no vertical divider line. */
  .ps-advanced .ps-col-main {
    flex: 0 0 396px;
  }
  /* Sole column (no detail content, e.g. the wide Battery Manager subview) → fill the width. */
  .ps-advanced .ps-col-main:last-child {
    flex: 1;
  }
  .ps-advanced .ps-col-detail {
    flex: 1;
  }

  /* ── Header / toolbar / framed field / footer ────────────── */
  .ps-head {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
    min-height: 26px;
  }
  /* A traffic alert whose banner lies behind this panel: the title bar carries its colour. */
  .ps-alert-warning .ps-head,
  .ps-alert-caution .ps-head {
    border-radius: 4px;
  }
  /* Only the column heads have room around them; the full-width heads bring their own padding. */
  .ps-alert-warning .ps-col > .ps-head,
  .ps-alert-caution .ps-col > .ps-head {
    margin: -4px -6px 4px;
    padding: 4px 6px;
  }
  .ps-alert-warning .ps-head { background: #d40000; }
  .ps-alert-caution .ps-head { background: #f4c020; }
  .ps-alert-warning .ps-title { color: #fff; }
  .ps-alert-caution .ps-title { color: #1a1400; }

  .ps-title {
    flex: 1;
    font-weight: 600;
    font-size: 13px;
    color: #37a8db;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  /* Fullscreen/wide-compact: title is content-width so the header actions sit left, right after
     it (like the original terrain layout) — the free space stays on the right. */
  .ps-fullscreen .ps-title,
  .ps-wide-compact .ps-title {
    flex: 0 1 auto;
  }
  .ps-head-actions {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
  }

  .ps-toolbar {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    flex-shrink: 0;
  }

  /* The thin-line framed working field (matches the Flight Logbook list frame). Keeps a minimum
     usable height; below that the column (not the field) scrolls — see .ps-col. */
  .ps-field {
    flex: 1;
    min-height: 200px;
    overflow: auto;
    border: 1px solid #555;
    border-radius: 4px;
    background: rgba(0, 0, 0, 0.12);
    padding: 6px;
  }
  /* Info panels are unframed and content-sized (no minimum height). */
  .ps-info .ps-field {
    flex: 0 0 auto;
    min-height: 0;
    border: none;
    background: none;
    padding: 0;
    overflow: visible;
  }

  .ps-foot {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    flex-shrink: 0;
  }

  /* ── Fullscreen / Wide-Compact (header top, params left, content fill) ───── */
  .ps-fullscreen .ps-head,
  .ps-wide-compact .ps-head {
    padding: 8px 12px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
  }
  .ps-fs-row {
    display: flex;
    flex: 1;
    min-height: 0;
  }
  .ps-params {
    width: 240px; /* the terrain controls fit with tighter spacing, the graph gets the rest */
    flex-shrink: 0;
    overflow: auto;
    padding: 10px;
    border-right: 1px solid rgba(255, 255, 255, 0.08);
  }
  .ps-fs-content {
    flex: 1;
    min-width: 0;
    overflow: auto;
    padding: 10px;
  }
  /* The strip is short: its height goes to the chart. */
  .ps-wide-compact .ps-params {
    width: auto;
    padding: 8px;
  }
  .ps-wide-compact .ps-fs-content {
    padding: 2px 4px 2px 0;
    overflow: hidden;
  }
  /* Fullscreen / Wide-Compact bottom bar (e.g. terrain readouts + hover info). */
  .ps-fs-foot {
    flex: 0 0 auto;
    border-top: 1px solid rgba(255, 255, 255, 0.08);
  }

  /* Enter animation on first open (horizontal slide + fade). */
  @keyframes ps-in {
    from { opacity: 0; transform: translateX(-14px); }
    to { opacity: 1; transform: translateX(0); }
  }
</style>
