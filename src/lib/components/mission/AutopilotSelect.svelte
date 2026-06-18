<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // Autopilot-system picker for the mission panel headers (INAV / ArduPilot / PX4). Shared by both
  // mission editors so the control lives in one place. A 3-way SegmentedToggle (we only have three
  // systems); read-only (dimmed) while connected — the system is then fixed to the detected FC.
  import { t } from 'svelte-i18n';
  import { get } from 'svelte/store';
  import { onDestroy } from 'svelte';
  import { autopilotSystem, autopilotLocked, setAutopilotSystem, type AutopilotSystem } from '$lib/stores/autopilotContext';
  import SegmentedToggle, { type SegOption } from '$lib/components/panel/SegmentedToggle.svelte';

  let system = $state<AutopilotSystem>(get(autopilotSystem));
  let locked = $state<boolean>(get(autopilotLocked));
  const unsubSystem = autopilotSystem.subscribe((s) => { system = s; });
  const unsubLocked = autopilotLocked.subscribe((l) => { locked = l; });
  onDestroy(() => { unsubSystem(); unsubLocked(); });

  const OPTIONS: SegOption[] = [
    { value: 'inav', label: 'INAV' },
    { value: 'ardupilot', label: 'ArduPilot' },
    { value: 'px4', label: 'PX4' },
  ];
</script>

<div title={locked ? $t('mission.autopilotLocked') : $t('mission.autopilotSystem')}>
  <SegmentedToggle
    options={OPTIONS}
    value={system}
    size="sm"
    disabled={locked}
    onchange={(v) => setAutopilotSystem(v as AutopilotSystem)}
  />
</div>
