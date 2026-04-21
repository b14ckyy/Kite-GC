<!-- MissionLayer.svelte — thin switcher
     Delegates to INAV or ArduPilot map layer based on active autopilot system.
     Usage: <MissionLayer {map} />
-->
<script lang="ts">
  import { onDestroy } from 'svelte';
  import { get } from 'svelte/store';
  import { autopilotSystem, type AutopilotSystem } from '$lib/stores/autopilotContext';
  import InavMissionLayer from './InavMissionLayer.svelte';
  import ArduMissionLayer from './ArduMissionLayer.svelte';
  import type L from 'leaflet';

  interface Props { map: L.Map; }
  let { map }: Props = $props();

  let currentSystem = $state<AutopilotSystem>(get(autopilotSystem));
  const unsub = autopilotSystem.subscribe(s => { currentSystem = s; });
  onDestroy(() => unsub());
</script>

{#if currentSystem === 'inav'}
  <InavMissionLayer {map} />
{:else}
  <ArduMissionLayer {map} />
{/if}
