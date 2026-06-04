<script lang="ts" module>
  export type PanelVariant = 'info' | 'compact' | 'advanced' | 'fullscreen' | 'wide-compact';
</script>

<script lang="ts">
  import type { Snippet } from 'svelte';

  // Reusable panel frame for the whole app (see docs/dev/PANEL_FRAMEWORK.md). The shell owns
  // the frame, positioning, sizing per variant, the scroll/bounding of the framed field, and
  // the live variant transition. Callers only place content into the snippet slots. It lives
  // inside `.ui-scale`, so it scales with --ui-scale for free.
  let {
    variant = 'compact',
    title = '',
    onClose = undefined,
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
    onClose?: () => void;
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
</script>

<div class="ps ps-{variant}">
  {#if variant === 'fullscreen' || variant === 'wide-compact'}
    <header class="ps-head">
      <span class="ps-title">{title}</span>
      {#if headerActions}<div class="ps-head-actions">{@render headerActions()}</div>{/if}
      {#if onClose}<button class="ps-close" onclick={onClose} aria-label="Close">✕</button>{/if}
    </header>
    <div class="ps-fs-row">
      {#if params}<aside class="ps-params">{@render params()}</aside>{/if}
      <div class="ps-fs-content">
        {#if body}{@render body()}{:else if children}{@render children()}{/if}
      </div>
    </div>
  {:else}
    <section class="ps-col ps-col-main">
      <header class="ps-head">
        <span class="ps-title">{title}</span>
        {#if headerActions}<div class="ps-head-actions">{@render headerActions()}</div>{/if}
        {#if onClose}<button class="ps-close" onclick={onClose} aria-label="Close">✕</button>{/if}
      </header>
      {#if toolbar}<div class="ps-toolbar">{@render toolbar()}</div>{/if}
      <div class="ps-field">
        {#if body}{@render body()}{:else if children}{@render children()}{/if}
      </div>
      {#if footer}<footer class="ps-foot">{@render footer()}</footer>{/if}
    </section>

    {#if variant === 'advanced'}
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
    backdrop-filter: blur(12px);
    /* Live variant transitions + smooth panel switch (the shell instance persists across rail
       switches, so width/height/top animate between any two variants). `interpolate-size`
       (Chromium 129+ / WebView2) lets width/height animate even to/from the `info` variant's
       intrinsic `max-content` / `auto` sizes. */
    interpolate-size: allow-keywords;
    transition: width 0.25s ease, height 0.25s ease, top 0.25s ease;
    animation: ps-in 0.18s ease-out;
  }

  /* Bounding height shared by compact/advanced (and as a cap for info): full panel area,
     i.e. below the toolbar, above the bottom dock + status bar. 100% = the (scaled) .app. */
  .ps-info,
  .ps-compact,
  .ps-advanced {
    flex-direction: column;
  }
  .ps-compact,
  .ps-advanced {
    height: calc(100% - 53px - var(--grid-bottom-height) - 24px - 12px);
  }

  .ps-info {
    width: max-content;
    max-width: 360px;
    height: auto;
    max-height: calc(100% - 53px - var(--grid-bottom-height) - 24px - 12px);
  }
  .ps-compact {
    width: 420px;
  }
  .ps-advanced {
    flex-direction: row;
    width: 920px;
    max-width: calc(100% - 62px - var(--grid-side-width) - 54px - 12px);
  }

  /* Fullscreen + Wide-Compact: floating overlays (terrain-analyzer style), NOT edge-to-edge.
     All variants are left-anchored and sized by width/height (no `right`) so any variant can
     animate into any other. Fullscreen = even ~62px inset all round; Wide-Compact = a short
     top strip that stops before the side widget dock so the map stays visible for comparison. */
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
    backdrop-filter: blur(14px);
  }
  .ps-fullscreen {
    width: calc(100% - 124px);
    height: calc(100% - 124px);
  }
  .ps-wide-compact {
    width: calc(100% - 62px - var(--grid-side-width) - 54px - 6px);
    height: max(20vh, 160px);
  }

  /* ── Columns (compact = 1, advanced = 1:2; right wider for previews/maps) ── */
  .ps-col {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    padding: 8px;
    gap: 6px;
  }
  .ps-compact .ps-col-main,
  .ps-info .ps-col-main {
    flex: 1;
    width: 100%;
  }
  .ps-advanced .ps-col-main {
    flex: 1;
    border-right: 1px solid rgba(255, 255, 255, 0.08);
  }
  .ps-advanced .ps-col-detail {
    flex: 2;
  }

  /* ── Header / toolbar / framed field / footer ────────────── */
  .ps-head {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
    min-height: 26px;
  }
  .ps-title {
    flex: 1;
    font-weight: 600;
    font-size: 13px;
    color: #37a8db;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ps-head-actions {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
  }
  .ps-close {
    flex-shrink: 0;
    width: 22px;
    height: 22px;
    border: 1px solid #555;
    border-radius: 4px;
    background: #2a2a2a;
    color: #ccc;
    cursor: pointer;
    line-height: 1;
  }
  .ps-close:hover {
    background: rgba(55, 168, 219, 0.18);
    color: #e0e0e0;
  }

  .ps-toolbar {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    flex-shrink: 0;
  }

  /* The thin-line framed working field (matches the Flight Logbook list frame). */
  .ps-field {
    flex: 1;
    min-height: 0;
    overflow: auto;
    border: 1px solid #555;
    border-radius: 4px;
    background: rgba(0, 0, 0, 0.12);
    padding: 6px;
  }
  /* Info panels are unframed and content-sized. */
  .ps-info .ps-field {
    flex: 0 0 auto;
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
    width: 280px;
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

  /* Enter animation on first open (horizontal slide + fade). */
  @keyframes ps-in {
    from { opacity: 0; transform: translateX(-14px); }
    to { opacity: 1; transform: translateX(0); }
  }
</style>
