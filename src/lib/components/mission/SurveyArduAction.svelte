<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- SurveyArduAction.svelte
     One ArduPilot/PX4 survey action slot: a dropdown of DO_ command items (camera trigger, servo,
     relay, …) + catalog-driven param fields. The cross-autopilot equivalent of an INAV UA-flag slot —
     the chosen command is inserted as a DO_ mission item at the matching pattern position (line start /
     line end / start / track / end). null = no action. -->
<script lang="ts">
  import { t } from 'svelte-i18n';
  import NumberStepper from '$lib/components/NumberStepper.svelte';
  import {
    CMD, cmdDef, cmdName, cmdDefaultParams, cmdSupportedByPx4, type ParamSpec,
  } from '$lib/helpers/arduCommandCatalog';
  import type { ArduAction } from '$lib/stores/surveyPattern.svelte';

  let { label, value = null, firmware = 'ardupilot', commands, onchange }: {
    label: string;
    value: ArduAction | null;
    firmware?: 'ardupilot' | 'px4';
    /** Override the offered command list (e.g. batch edit passes the full modifier set). Defaults to
     *  the curated survey-trigger subset below. */
    commands?: number[];
    onchange: (v: ArduAction | null) => void;
  } = $props();

  // Action commands offered for survey triggers (in this order).
  const ACTION_CMDS = [
    CMD.DO_SET_CAM_TRIGG_DIST, CMD.DO_DIGICAM_CONTROL, CMD.DO_AUX_FUNCTION,
    CMD.DO_SET_SERVO, CMD.DO_REPEAT_SERVO, CMD.DO_SET_RELAY, CMD.DO_REPEAT_RELAY,
  ];
  const options = $derived((commands ?? ACTION_CMDS).filter((id) => firmware !== 'px4' || cmdSupportedByPx4(id)));

  // Local working copy — synced from the value prop, emitted on every edit.
  let cmd = $state(0);
  let params = $state<[number, number, number, number]>([0, 0, 0, 0]);
  $effect(() => {
    cmd = value?.command ?? 0;
    params = value ? [value.param1, value.param2, value.param3, value.param4] : [0, 0, 0, 0];
  });

  const def = $derived(cmd ? cmdDef(cmd) : undefined);
  const paramIdx = $derived(def?.params ? ([1, 2, 3, 4] as const).filter((i) => def!.params![i]) : []);
  const spec = (i: number): ParamSpec | undefined => def?.params?.[i as 1 | 2 | 3 | 4];

  function emit() {
    onchange(cmd ? { command: cmd, param1: params[0], param2: params[1], param3: params[2], param4: params[3] } : null);
  }
  function selectCmd(id: number) {
    if (!id) { onchange(null); return; }
    const d = cmdDefaultParams(id);
    onchange({ command: id, param1: d.param1, param2: d.param2, param3: d.param3, param4: d.param4 });
  }
</script>

<div class="aa">
  <div class="aa-head">
    <span class="aa-label">{label}</span>
    <select
      class="aa-sel"
      value={cmd}
      onchange={(e) => selectCmd(Number((e.currentTarget as HTMLSelectElement).value))}
    >
      <option value={0}>{$t('survey.actionNone')}</option>
      {#each options as id (id)}
        <option value={id}>{cmdName(id)}</option>
      {/each}
    </select>
  </div>

  {#if def && paramIdx.length}
    <div class="aa-params">
      {#each paramIdx as i (i)}
        {@const s = spec(i)}
        {#if s}
          {#if s.enumStrings && s.enumValues}
            <label class="aa-row">
              <span class="aa-pl">{s.label}</span>
              <select
                class="aa-psel"
                value={params[i - 1]}
                onchange={(e) => { params[i - 1] = Number((e.currentTarget as HTMLSelectElement).value); emit(); }}
              >
                {#each s.enumStrings as es, k (k)}<option value={s.enumValues![k]}>{es}</option>{/each}
              </select>
            </label>
          {:else}
            <div class="aa-row">
              <span class="aa-pl">{s.label}{s.units ? ` (${s.units})` : ''}</span>
              <NumberStepper
                bind:value={params[i - 1]}
                min={s.min ?? -Infinity}
                max={s.max ?? Infinity}
                step={1}
                decimals={s.decimals ?? 0}
                onchange={emit}
              />
            </div>
          {/if}
        {/if}
      {/each}
    </div>
  {/if}
</div>

<style>
  .aa { display: flex; flex-direction: column; gap: 4px; margin-bottom: 6px; }
  .aa-head { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .aa-label { font-size: 12px; color: #c8c8c8; }
  .aa-sel {
    flex: 1; max-width: 60%; background: #2a2a2a; color: #e0e0e0;
    border: 1px solid #555; border-radius: 4px; padding: 3px 6px; font-size: 12px;
  }
  .aa-params {
    display: flex; flex-direction: column; gap: 4px;
    margin: 2px 0 4px 8px; padding-left: 8px; border-left: 2px solid #3a3a3a;
  }
  .aa-row { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .aa-pl { font-size: 11px; color: #949494; }
  .aa-psel {
    background: #2a2a2a; color: #e0e0e0; border: 1px solid #555;
    border-radius: 4px; padding: 2px 6px; font-size: 12px;
  }
</style>
