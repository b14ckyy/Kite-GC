<script lang="ts">
  // Autopilot-system picker for the mission panel headers (INAV / ArduPilot / PX4). Shared by
  // both V2 mission editors so the control lives in one place. Locked while connected.
  import { t } from 'svelte-i18n';
  import { get } from 'svelte/store';
  import { autopilotSystem, autopilotLocked, setAutopilotSystem, type AutopilotSystem } from '$lib/stores/autopilotContext';

  let system = $state<AutopilotSystem>(get(autopilotSystem));
  let locked = $state<boolean>(get(autopilotLocked));
  const unsubSystem = autopilotSystem.subscribe((s) => { system = s; });
  const unsubLocked = autopilotLocked.subscribe((l) => { locked = l; });
  import { onDestroy } from 'svelte';
  onDestroy(() => { unsubSystem(); unsubLocked(); });

  function systemLabel(s: AutopilotSystem): string {
    if (s === 'ardupilot') return 'ArduPilot';
    if (s === 'px4') return 'PX4';
    return 'INAV';
  }
</script>

{#if locked}
  <span class="ap-locked" title={$t('mission.autopilotSystem')}>🔒 {systemLabel(system)}</span>
{:else}
  <select
    class="ap-select"
    value={system}
    title={$t('mission.autopilotSystem')}
    onchange={(e) => setAutopilotSystem((e.target as HTMLSelectElement).value as AutopilotSystem)}
  >
    <option value="inav">INAV</option>
    <option value="ardupilot">ArduPilot</option>
    <option value="px4">PX4</option>
  </select>
{/if}

<style>
  /* Matches the framework form-control height (28px md button). */
  .ap-select {
    height: 28px;
    padding: 0 8px;
    background: #434343;
    border: 1px solid #555;
    border-radius: 4px;
    color: #e0e0e0;
    font-size: 12px;
    cursor: pointer;
  }
  .ap-locked { font-size: 12px; color: #f39c12; font-weight: 600; white-space: nowrap; }
</style>
