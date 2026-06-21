<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- ManualStates.svelte — PX4 MANUAL_CONTROL output monitor (left/control stage of RcControlPanel for
     the PX4 platform). Shows the four normalised sticks (x/y/z/r), any mapped aux axes and the active
     MANUAL_CONTROL button bits — the live equivalent of ChannelStates for the channel platforms. -->
<script lang="ts">
  import { t } from 'svelte-i18n';
  import { manualOutput, rcManual } from '$lib/stores/rcManual';
  import { rcEngaged } from '$lib/stores/rcEngage';

  // Axis rows: label + value (-1000..1000). x=pitch, y=roll, z=thrust, r=yaw.
  const axes = $derived([
    { key: 'roll', val: $manualOutput.y },
    { key: 'pitch', val: $manualOutput.x },
    { key: 'throttle', val: $manualOutput.z },
    { key: 'yaw', val: $manualOutput.r },
  ]);
  // Centred fill: 0 = middle, ±1000 = edges.
  const pct = (v: number) => Math.max(0, Math.min(100, (v + 1000) / 2000 * 100));

  // Only show aux axes that are actually mapped.
  const auxRows = $derived(
    ($rcManual.aux ?? []).map((a, i) => ({ i, mapped: !!a.input, val: $manualOutput.aux[i] ?? 0 }))
      .filter((a) => a.mapped),
  );

  // Active MANUAL_CONTROL button numbers (1..32) from the two bitfields.
  const activeButtons = $derived(
    $rcManual.buttons
      .map((b) => b.button)
      .filter((n) => n >= 1 && n <= 32)
      .sort((a, b) => a - b)
      .filter((n) => {
        const bits = n <= 16 ? $manualOutput.buttons : $manualOutput.buttons2;
        const bit = n <= 16 ? n - 1 : n - 17;
        return (bits & (1 << bit)) !== 0;
      }),
  );
  const mappedButtons = $derived(
    [...new Set($rcManual.buttons.map((b) => b.button).filter((n) => n >= 1 && n <= 32))].sort((a, b) => a - b),
  );
</script>

<div class="ms">
  <div class="ms-source" class:live={$rcEngaged.on}>{$rcEngaged.on ? $t('rc.outLive') : $t('rc.outPreview')}</div>

  {#each axes as ax (ax.key)}
    <div class="ms-row">
      <span class="ms-label">{$t(`rc.manual.${ax.key}`)}</span>
      <div class="ms-bar">
        <span class="ms-centre"></span>
        <span class="ms-fill" style="left:{Math.min(50, pct(ax.val))}%; width:{Math.abs(pct(ax.val) - 50)}%"></span>
      </div>
      <span class="ms-val">{ax.val}</span>
    </div>
  {/each}

  {#if auxRows.length}
    <div class="ms-group">{$t('rc.manual.aux')}</div>
    {#each auxRows as a (a.i)}
      <div class="ms-row">
        <span class="ms-label">AUX{a.i + 1}</span>
        <div class="ms-bar">
          <span class="ms-centre"></span>
          <span class="ms-fill" style="left:{Math.min(50, pct(a.val))}%; width:{Math.abs(pct(a.val) - 50)}%"></span>
        </div>
        <span class="ms-val">{a.val}</span>
      </div>
    {/each}
  {/if}

  {#if mappedButtons.length}
    <div class="ms-group">{$t('rc.manual.buttons')}</div>
    <div class="ms-buttons">
      {#each mappedButtons as n (n)}
        <span class="ms-btn" class:on={activeButtons.includes(n)}>{n}</span>
      {/each}
    </div>
  {/if}
</div>

<style>
  .ms { display: flex; flex-direction: column; gap: 5px; }
  .ms-source { color: #6f6f6f; font-size: 10px; font-style: italic; margin-bottom: 2px; }
  .ms-source.live { color: #7ec850; font-style: normal; font-weight: 700; }
  .ms-group {
    color: #37a8db; font-size: 10px; font-weight: 700; text-transform: uppercase; letter-spacing: 0.5px;
    margin: 6px 0 2px; padding-bottom: 3px; border-bottom: 1px solid rgba(55, 168, 219, 0.25);
  }
  .ms-row { display: grid; grid-template-columns: 80px 1fr 48px; align-items: center; gap: 8px; }
  .ms-label { color: #cfcfcf; font-size: 11px; }
  .ms-val { color: #e0e0e0; font-size: 11px; font-variant-numeric: tabular-nums; text-align: right; }
  .ms-bar {
    position: relative; height: 13px; background: #1f1f1f; border: 1px solid #333;
    border-radius: 3px; overflow: hidden;
  }
  .ms-centre { position: absolute; left: 50%; top: 0; bottom: 0; width: 1px; background: #4a4a4a; }
  .ms-fill { position: absolute; top: 0; bottom: 0; background: #37a8db; }
  .ms-buttons { display: flex; flex-wrap: wrap; gap: 4px; }
  .ms-btn {
    min-width: 22px; height: 20px; padding: 0 5px; display: inline-flex; align-items: center;
    justify-content: center; font-size: 10px; border-radius: 3px;
    background: #2a2a2a; color: #777; border: 1px solid #3a3a3a; font-variant-numeric: tabular-nums;
  }
  .ms-btn.on { background: rgba(89, 170, 41, 0.25); color: #7ec850; border-color: #59aa29; }
</style>
