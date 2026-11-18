<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  import { t } from 'svelte-i18n';

  interface Tab {
    id: string;
    label: () => string;
    icon: string;
  }

  let {
    open,
    activeTab,
    activeHidden = false,
    tabs,
    onToggle,
    onSelectTab,
  }: {
    open: boolean;
    activeTab: string;
    /** The active panel is slid out of view (re-click of its button) — shown dimmed. */
    activeHidden?: boolean;
    tabs: Tab[];
    onToggle: () => void;
    onSelectTab: (tabId: string) => void;
  } = $props();
</script>

<div class="nav-rail" class:open>
  <!-- Hamburger button -->
  <button class="hamburger-btn" onclick={onToggle} title={open ? $t('nav.closePanel') : $t('nav.openPanel')}>
    <span class="hamburger-icon" class:open>
      <span></span>
      <span></span>
      <span></span>
    </span>
  </button>

  <!-- Tab buttons (visible only when panel is open) -->
  {#if open}
    <div class="tab-buttons">
      {#each tabs as tab}
        {#if tab.id === '__sep__'}
          <div class="tab-sep"></div>
        {:else}
          <button
            class="tab-btn"
            class:active={activeTab === tab.id}
            class:hidden-panel={activeTab === tab.id && activeHidden}
            onclick={() => onSelectTab(tab.id)}
            title={tab.label()}
          >
            <!-- icon is a glyph or an inline SVG string (trusted, app-defined) -->
            <span class="tab-icon">{@html tab.icon}</span>
            <span class="tab-label">{tab.label()}</span>
          </button>
        {/if}
      {/each}
    </div>
  {/if}
</div>

<style>
  /* Mobile: its own compositor layer, permanently — see the zone rule in +page.svelte (the floating
     video window slides past it and would promote / demote it, a frame without the rail). */
  :global(html.is-mobile) .nav-rail {
    will-change: transform;
  }
  .nav-rail {
    position: absolute;
    top: 65px;
    left: 12px;
    display: flex;
    flex-direction: column;
    gap: 0;
    z-index: 100;
    transition: left 0.3s ease;
    /* Bound the rail between toolbar and status bar and let it scroll (hidden scrollbar, exact 42px width) —
       otherwise `.ui-root`'s overflow clips the last icons silently on short windows / high UI scale. In logical
       px: `.ui-scale` is 100vh / --ui-scale tall (65 = toolbar 53 + 12, 30 = status bar 24 + 6). */
    max-height: calc(100vh / var(--ui-scale, 1) - 65px - 30px);
    overflow-y: auto;
    overflow-x: hidden;
    scrollbar-width: none;
  }

  .nav-rail::-webkit-scrollbar {
    display: none;
  }

  .nav-rail.open {
    left: 12px;
  }

  /* Mobile, any orientation: shift the rail clear of a landscape side-notch / Dynamic Island. Harmless
     on iPad and in portrait (--safe-left is 0 there), so it does not change those layouts. */
  :global(html.is-mobile) .nav-rail,
  :global(html.is-mobile) .nav-rail.open {
    left: calc(12px + var(--safe-left, 0px));
  }
  /* Phone: no toolbar (Dev-Docs archive/PHONE_UI.md D3) — the burger IS the top-left corner, lifted above the
     widget column (z 100). The rail stops above the bottom-left chip row (26 + 8 + 8px) and scrolls. */
  :global(html.is-phone) .nav-rail {
    z-index: 120;
    top: calc(8px + var(--safe-top, 0px));
    max-height: calc(100vh - 16px - 42px - var(--safe-top, 0px) - var(--safe-bottom, 0px));
  }
  /* Portrait phone only: the toolbar collapses/expands, so the rail tracks its live height. iPad and
     landscape phone keep the fixed top (their bar is a single row that the base 65px already clears). */
  @media (max-width: 600px) {
    :global(html.is-mobile) .nav-rail {
      top: calc(var(--toolbar-h, 65px) + 8px);
    }
  }
  /* Tablet (iPad): the toolbar is taller than the fixed 65px base (iOS status-bar + safe-top padding),
     which pushed the rail's top buttons under the bar. Track the live bar height so they clear it. */
  :global(html.is-tablet) .nav-rail {
    top: calc(var(--toolbar-h, 65px) + 8px);
  }

  .hamburger-btn {
    width: 42px;
    height: 42px;
    background: rgba(46, 46, 46, 0.92);
    border: 1px solid rgba(55, 168, 219, 0.5);
    border-radius: 6px;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: background-color 0.2s;
    backdrop-filter: var(--glass-blur, blur(6px));
    -webkit-backdrop-filter: var(--glass-blur, blur(6px));
  }

  .hamburger-btn:hover {
    background: var(--btn-active-bg);
  }

  .hamburger-icon {
    display: flex;
    flex-direction: column;
    gap: 4px;
    width: 20px;
    transition: transform 0.3s ease;
  }

  .hamburger-icon span {
    display: block;
    height: 2px;
    background: #37a8db;
    border-radius: 1px;
    transition: transform 0.3s ease, opacity 0.2s ease;
  }

  .hamburger-icon.open span:nth-child(1) {
    transform: translateY(6px) rotate(45deg);
  }

  .hamburger-icon.open span:nth-child(2) {
    opacity: 0;
  }

  .hamburger-icon.open span:nth-child(3) {
    transform: translateY(-6px) rotate(-45deg);
  }

  .tab-buttons {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-top: 4px;
  }

  /* Divider between the old panels (top) and the new framework panels (bottom). */
  .tab-sep {
    height: 1px;
    margin: 4px 6px;
    background: rgba(55, 168, 219, 0.4);
  }

  .tab-btn {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 42px;
    height: 38px;
    background: rgba(46, 46, 46, 0.92);
    border: 1px solid rgba(55, 168, 219, 0.3);
    border-radius: 6px;
    color: #a8a8a8;
    font-size: 11px;
    cursor: pointer;
    padding: 0;
    justify-content: center;
    overflow: hidden;
    transition: width 0.3s ease, background-color 0.2s;
    backdrop-filter: var(--glass-blur, blur(6px));
    -webkit-backdrop-filter: var(--glass-blur, blur(6px));
    white-space: nowrap;
  }

  .tab-btn:hover {
    background: rgba(55, 168, 219, 0.15);
    color: #e0e0e0;
  }

  /* Darker active fill (black 50% + the inherited blur) so the accent border + icon stay
     readable over bright maps; the blue border/icon remain the active indicator. */
  .tab-btn.active {
    background: rgba(0, 0, 0, 0.5);
    border-color: #37a8db;
    color: #37a8db;
  }
  /* Active but slid out of view: still the selected tab, just parked. */
  .tab-btn.active.hidden-panel {
    border-style: dashed;
    color: rgba(55, 168, 219, 0.6);
  }

  .tab-icon {
    font-size: 16px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  /* Inline-SVG icons fill the button (≤ ~10% margin to the frame); glyphs keep font-size. */
  .tab-icon :global(svg) {
    width: 32px;
    height: 32px;
    display: block;
  }

  .tab-label {
    display: none;
  }
</style>
