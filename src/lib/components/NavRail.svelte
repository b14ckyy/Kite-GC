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
    tabs,
    onToggle,
    onSelectTab,
  }: {
    open: boolean;
    activeTab: string;
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
  .nav-rail {
    position: absolute;
    top: 65px;
    left: 12px;
    display: flex;
    flex-direction: column;
    gap: 0;
    z-index: 100;
    transition: left 0.3s ease;
    /* Bound the rail to the space between the toolbar and the status bar and let it scroll, instead
       of overflowing and being silently clipped by `.ui-root`'s `overflow: hidden`.
       With every tab enabled the open rail is 42 + 4 + 10x38 + 9x2 = 444px, so it needs ~539 logical
       pixels of height; at a UI scale of 1.5 that is 808 real pixels and at 2.0 it is 1078, and any
       window shorter than that silently drops the last icons off the bottom edge with no scrollbar
       and no other indication that they exist.
       Expressed in the chrome's own logical pixels: `.ui-scale` is exactly `100vh / --ui-scale` tall,
       so this stays correct at every UI scale without depending on which ancestor is the offset
       parent (65px = toolbar 53 + 12 gap, 30px = status bar 24 + 6 gap). The scrollbar is hidden so
       the rail keeps its exact 42px width and current look; it scrolls by wheel/drag. */
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
  /* iPhone: keep the rail (incl. the settings/close button) above the HUD dock so the tiles never
     cover it. The dock is z-index 100; lift the rail over it. */
  :global(html.is-phone) .nav-rail {
    z-index: 120;
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
    backdrop-filter: blur(8px);
  }

  .hamburger-btn:hover {
    background: rgba(55, 168, 219, 0.25);
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
    backdrop-filter: blur(8px);
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
