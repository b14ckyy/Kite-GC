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
  }

  .nav-rail.open {
    left: 12px;
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
