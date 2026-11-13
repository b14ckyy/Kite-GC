<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- GroupCommandBar — bottom-centre overlay over the map (lives in +page's `.app-toasts` band) hosting
     the shared group command set (GroupCommandButtons) plus the selection count. Rendered only while
     2+ MAVLink vehicles are connected. The same buttons also sit in the fleet panel footer.
     See docs/02-design/features/fleet-control.design.md §4.2. -->
<script lang="ts">
  import { t } from 'svelte-i18n';
  import GroupCommandButtons from '$lib/components/GroupCommandButtons.svelte';
  import { vehicles } from '$lib/stores/vehicles';
  import { selectedVehicleIds, clearSelection } from '$lib/stores/fleetSelection';
  import { orderedVehicles, isMavlinkVehicle, vehicleSystem } from '$lib/helpers/fleetStatus';

  const mavVehicles = $derived(orderedVehicles($vehicles).filter((v) => isMavlinkVehicle(v) && vehicleSystem(v) != null));
  const visible = $derived(mavVehicles.length >= 2);
  const selCount = $derived(mavVehicles.filter((v) => $selectedVehicleIds.has(v.vehicleId)).length);
</script>

{#if visible}
  <div class="gcb" role="toolbar" aria-label={$t('fleet.group.title')}>
    <div class="gcb-sel" title={$t('fleet.group.hint')}>
      <span class="gcb-count" class:ready={selCount >= 2}>{$t('fleet.selectedCount', { values: { n: selCount } })}</span>
      {#if selCount > 0}
        <button class="gcb-clear" onclick={clearSelection} title={$t('fleet.group.clearSelection')}>✕</button>
      {/if}
    </div>
    <div class="gcb-body">
      <GroupCommandButtons />
    </div>
  </div>
{/if}

<style>
  .gcb {
    position: absolute;
    /* Bottom-centre of the frame, above the status bar / bottom widget dock. */
    bottom: calc(var(--grid-bottom-height, 0px) + 44px);
    left: calc(var(--toast-dock-inset, 0px) + 8px);
    right: 8px;
    margin-inline: auto;
    z-index: 470; /* under the STATUSTEXT toasts (480) and the radar banner (500) */
    width: max-content;
    max-width: calc(100vw - var(--toast-dock-inset, 0px) - 32px);
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 8px 6px 10px;
    background: rgba(46, 46, 46, 0.92);
    backdrop-filter: var(--glass-blur, blur(6px));
    border: 1px solid rgba(55, 168, 219, 0.5);
    border-radius: 8px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.5);
    color: #e0e0e0;
    font-family: 'Segoe UI', Tahoma, sans-serif;
    font-size: 12px;
    pointer-events: auto;
  }
  :global(html.is-mobile) .gcb { bottom: calc(var(--grid-bottom-height, 0px) + var(--safe-bottom, 0px) + 44px); }

  .gcb-sel {
    display: flex;
    align-items: center;
    gap: 6px;
    white-space: nowrap;
  }
  .gcb-count {
    font-weight: 600;
    color: #9a9a9a;
  }
  .gcb-count.ready { color: #37a8db; }
  .gcb-clear {
    width: 18px;
    height: 18px;
    padding: 0;
    background: transparent;
    border: 1px solid #555;
    border-radius: 3px;
    color: #cfcfcf;
    font-size: 10px;
    cursor: pointer;
  }
  .gcb-clear:hover { border-color: #37a8db; color: #37a8db; }

  .gcb-body {
    padding-left: 10px;
    border-left: 1px solid #4a4a4a;
    min-width: 0;
  }
</style>
