<script lang="ts">
  // Recursive menu list — renders items at (x, y), clamped to the viewport, and
  // opens submenus as side flyouts (self-recursion). Used by ContextMenu.svelte.
  import type { ContextMenuItem } from '$lib/stores/contextMenu';
  import Self from './ContextMenuList.svelte';

  let {
    items,
    x,
    y,
    onpick,
  }: {
    items: ContextMenuItem[];
    x: number;
    y: number;
    onpick: () => void;
  } = $props();

  let menuEl = $state<HTMLDivElement | null>(null);
  // svelte-ignore state_referenced_locally
  let pos = $state({ x, y });
  let openSub = $state<number | null>(null);
  let subPos = $state({ x: 0, y: 0 });

  // Clamp to the viewport once the menu has a measured size.
  $effect(() => {
    void items;
    if (!menuEl) {
      pos = { x, y };
      return;
    }
    const r = menuEl.getBoundingClientRect();
    const nx = x + r.width > window.innerWidth ? Math.max(4, x - r.width) : x;
    const ny = y + r.height > window.innerHeight ? Math.max(4, window.innerHeight - r.height - 4) : y;
    pos = { x: nx, y: ny };
  });

  function onEnter(i: number, e: MouseEvent) {
    const item = items[i];
    if (!item.submenu?.length) {
      openSub = null;
      return;
    }
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    subPos = { x: r.right - 3, y: r.top - 5 };
    openSub = i;
  }

  function pick(item: ContextMenuItem) {
    if (item.disabled || item.separator || item.submenu) return;
    item.action?.();
    onpick();
  }
</script>

<div class="cm-menu" bind:this={menuEl} style="left:{pos.x}px; top:{pos.y}px;">
  {#each items as item, i}
    {#if item.separator}
      <div class="cm-sep"></div>
    {:else}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div
        class="cm-item"
        class:disabled={item.disabled}
        class:danger={item.danger}
        onmouseenter={(e) => onEnter(i, e)}
        onclick={() => pick(item)}
      >
        {#if item.icon}<span class="cm-icon">{item.icon}</span>{/if}
        <span class="cm-label">{item.label}</span>
        {#if item.submenu}<span class="cm-arrow">›</span>{/if}
      </div>
    {/if}
  {/each}
</div>

{#if openSub != null && items[openSub]?.submenu}
  <Self items={items[openSub].submenu ?? []} x={subPos.x} y={subPos.y} {onpick} />
{/if}

<style>
  /* Mix: side-panel frame (thin blue border, rounded) + widget background
     (darker, slightly transparent, blurred). */
  .cm-menu {
    position: fixed;
    z-index: 2000;
    min-width: 168px;
    padding: 4px;
    background: rgba(30, 30, 30, 0.75);
    backdrop-filter: blur(10px);
    border: 1px solid rgba(55, 168, 219, 0.35);
    border-radius: 8px;
    box-shadow: 0 6px 24px rgba(0, 0, 0, 0.45);
    user-select: none;
  }
  .cm-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    border-radius: 5px;
    font-size: 13px;
    color: #e0e0e0;
    cursor: pointer;
    white-space: nowrap;
  }
  .cm-item:hover {
    background: rgba(55, 168, 219, 0.18);
  }
  .cm-item.disabled {
    color: #777;
    cursor: default;
  }
  .cm-item.disabled:hover {
    background: transparent;
  }
  .cm-item.danger {
    color: #e74c3c;
  }
  .cm-item.danger:hover {
    background: rgba(231, 76, 60, 0.16);
  }
  .cm-icon {
    width: 16px;
    text-align: center;
    flex-shrink: 0;
  }
  .cm-label {
    flex: 1;
  }
  .cm-arrow {
    color: #8aa;
    font-size: 15px;
    line-height: 1;
  }
  .cm-sep {
    height: 1px;
    background: rgba(255, 255, 255, 0.1);
    margin: 4px 6px;
  }
</style>
