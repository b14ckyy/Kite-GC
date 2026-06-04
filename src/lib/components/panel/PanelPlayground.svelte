<script lang="ts">
  // THROWAWAY Phase-0 scaffolding (see docs/dev/PANEL_FRAMEWORK.md): renders PanelShell with
  // placeholder content so the empty framework layouts + live variant transitions can be
  // validated before any real wiring. Removed once panels are migrated.
  import PanelShell, { type PanelVariant } from './PanelShell.svelte';

  let { initial = 'compact', label = 'Panel', onClose = () => {} }:
    { initial?: PanelVariant; label?: string; onClose?: () => void } = $props();

  // svelte-ignore state_referenced_locally
  let variant = $state<PanelVariant>(initial);
  // Re-sync to the tab's initial when the rail switches panels (the shell instance persists
  // now — no remount — so width/height/top can animate between panels). The switcher below
  // overrides locally until the next tab switch.
  $effect(() => { variant = initial; });

  const variants: PanelVariant[] = ['info', 'compact', 'advanced', 'wide-compact', 'fullscreen'];
</script>

<!-- Floating test switcher (outside the panel so it doesn't affect the panel's width). -->
<div class="pg-switcher">
  {#each variants as v}
    <button class="pg-sw" class:active={variant === v} onclick={() => (variant = v)}>{v}</button>
  {/each}
</div>

<PanelShell {variant} title={label} {onClose} detailTitle="Detail region">
  {#snippet toolbar()}
    {#if variant !== 'info'}<div class="pg-ph pg-ph-row">toolbar — title · buttons · search</div>{/if}
  {/snippet}

  {#snippet body()}
    {#if variant === 'info'}
      <div class="pg-info">
        <div class="pg-irow"><span>Status</span><span>Connected</span></div>
        <div class="pg-irow"><span>FC</span><span>INAV 8.0</span></div>
        <div class="pg-irow"><span>Board</span><span>MATEKH743</span></div>
        <div class="pg-irow"><span>Type</span><span>Airplane</span></div>
      </div>
    {:else}
      <div class="pg-ph pg-fill">
        <div class="pg-note">{variant} · framed working field (lists / waypoints / content)</div>
        {#each Array(12) as _, i}<div class="pg-line">placeholder row {i + 1}</div>{/each}
      </div>
    {/if}
  {/snippet}

  {#snippet footer()}
    {#if variant !== 'info'}<div class="pg-ph pg-ph-row">footer — action buttons</div>{/if}
  {/snippet}

  {#snippet detailActions()}
    <span class="pg-tag">detail header</span>
  {/snippet}
  {#snippet detailToolbar()}
    <div class="pg-ph pg-ph-row">detail toolbar</div>
  {/snippet}
  {#snippet detail()}
    <div class="pg-ph pg-fill">
      <div class="pg-note">detail framed field (1:2 right column)</div>
      {#each Array(10) as _, i}<div class="pg-line">detail row {i + 1}</div>{/each}
    </div>
  {/snippet}
  {#snippet detailFooter()}
    <div class="pg-ph pg-ph-row">detail footer — buttons</div>
  {/snippet}

  {#snippet params()}
    <div class="pg-note">parameter column (fullscreen left)</div>
    {#each Array(8) as _, i}<div class="pg-line">param {i + 1}</div>{/each}
  {/snippet}
</PanelShell>

<style>
  .pg-switcher {
    position: fixed;
    top: 58px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 300;
    display: flex;
    gap: 3px;
    padding: 3px;
    border-radius: 6px;
    background: rgba(0, 0, 0, 0.55);
    backdrop-filter: blur(8px);
  }
  .pg-sw {
    padding: 2px 7px; font-size: 10px; border: 1px solid #555; border-radius: 4px;
    background: #2a2a2a; color: #aaa; cursor: pointer; text-transform: uppercase;
  }
  .pg-sw:hover { background: rgba(55, 168, 219, 0.18); color: #e0e0e0; }
  .pg-sw.active { border-color: #37a8db; color: #37a8db; background: rgba(55, 168, 219, 0.15); }

  .pg-ph {
    border: 1px dashed rgba(55, 168, 219, 0.4); border-radius: 4px;
    color: #8aa; background: rgba(55, 168, 219, 0.05);
  }
  .pg-ph-row { padding: 6px 8px; font-size: 11px; text-align: center; }
  .pg-fill { height: 100%; padding: 8px; }
  .pg-note { color: #37a8db; font-size: 11px; margin-bottom: 6px; }
  .pg-line {
    padding: 5px 8px; margin-bottom: 4px; font-size: 12px; color: #bbb;
    background: #353535; border: 1px solid #444; border-radius: 4px;
  }
  .pg-tag { font-size: 10px; color: #8aa; }

  /* Info: short label : value rows (no-wrap → the panel sizes to content, capped at 360). */
  .pg-info { display: flex; flex-direction: column; gap: 6px; }
  .pg-irow {
    display: flex; justify-content: space-between; gap: 24px; white-space: nowrap;
    font-size: 12px; padding: 4px 6px; border-radius: 4px; background: #353535;
  }
  .pg-irow span:first-child { color: #949494; }
</style>
