<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- ChannelStates.svelte — live RC channel output view (left/control stage of RcControlPanel). Each
     configured channel's current µs value + bar, computed by the RC engine from live HID input. The
     "what are we sending" surface (display only for now; MSP streaming later). -->
<script lang="ts">
  import { t } from 'svelte-i18n';
  import { currentChannels } from '$lib/stores/rcProfiles';
  import { channelValues } from '$lib/stores/rcEngine';

  const channels = $derived(Object.keys($currentChannels).map(Number).sort((a, b) => a - b));
  const usPct = (us: number) => Math.max(0, Math.min(100, ((us - 1000) / 1000) * 100));
</script>

{#if channels.length === 0}
  <div class="cs-empty">{$t('rc.noChannels')}</div>
{:else}
  <div class="cs">
    {#each channels as ch (ch)}
      {@const us = $channelValues[ch] ?? 1500}
      {@const name = $currentChannels[ch]?.name}
      <div class="cs-row">
        <div class="cs-label">
          <span class="cs-num">CH{ch}</span>
          {#if name}<span class="cs-name" title={name}>{name}</span>{/if}
        </div>
        <div class="cs-bar">
          <span class="cs-centre"></span>
          <span class="cs-fill" style="width:{usPct(us)}%"></span>
        </div>
        <span class="cs-val">{us}</span>
      </div>
    {/each}
  </div>
{/if}

<style>
  .cs-empty { color: #949494; font-size: 12px; font-style: italic; padding: 8px; }
  .cs { display: flex; flex-direction: column; gap: 5px; }
  .cs-row { display: grid; grid-template-columns: 110px 1fr 42px; align-items: center; gap: 8px; }
  .cs-label { display: flex; align-items: baseline; gap: 6px; min-width: 0; }
  .cs-num { color: #37a8db; font-weight: 700; font-size: 12px; flex: none; }
  .cs-name { color: #cfcfcf; font-size: 11px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .cs-val { color: #e0e0e0; font-size: 11px; font-variant-numeric: tabular-nums; text-align: right; }
  .cs-bar {
    position: relative; height: 13px; background: #1f1f1f; border: 1px solid #333;
    border-radius: 3px; overflow: hidden;
  }
  .cs-centre { position: absolute; left: 50%; top: 0; bottom: 0; width: 1px; background: #4a4a4a; }
  .cs-fill { position: absolute; left: 0; top: 0; bottom: 0; background: #37a8db; }
</style>
