<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- ChannelStates.svelte — RC channel OUTPUT view (left/control stage of RcControlPanel). Shows the
     channel value our mapping produces (helpers/rcEngine). While idle this is a preview of the mapping;
     once engaged the state is seeded from the FC (no jump) and this is what we'd send. -->
<script lang="ts">
  import { t } from 'svelte-i18n';
  import { currentChannels } from '$lib/stores/rcProfiles';
  import { channelValues } from '$lib/stores/rcEngine';
  import { rcLayout } from '$lib/stores/rcLayout';
  import { rcEngaged } from '$lib/stores/rcEngage';

  const channels = $derived(Object.keys($currentChannels).map(Number).sort((a, b) => a - b));
  const rawCh = $derived(channels.filter((c) => c <= $rcLayout.rawMax));
  const auxCh = $derived(channels.filter((c) => c > $rcLayout.rawMax));
  const usFor = (ch: number) => $channelValues[ch] ?? 1500;
  const usPct = (us: number) => Math.max(0, Math.min(100, ((us - 1000) / 1000) * 100));
</script>

{#snippet chRow(ch: number)}
  {@const us = usFor(ch)}
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
{/snippet}

{#if channels.length === 0}
  <div class="cs-empty">{$t('rc.noChannels')}</div>
{:else if $rcLayout.split}
  <div class="cs">
    <div class="cs-source" class:live={$rcEngaged.on}>{$rcEngaged.on ? $t('rc.outLive') : $t('rc.outPreview')}</div>
    {#if rawCh.length}
      <div class="cs-group">{$t('rc.groupRaw')}</div>
      {#each rawCh as ch (ch)}{@render chRow(ch)}{/each}
    {/if}
    {#if auxCh.length}
      <div class="cs-group">{$t('rc.groupAux')}</div>
      {#each auxCh as ch (ch)}{@render chRow(ch)}{/each}
    {/if}
  </div>
{:else}
  <div class="cs">
    <div class="cs-source" class:live={$rcEngaged.on}>{$rcEngaged.on ? $t('rc.outLive') : $t('rc.outPreview')}</div>
    {#each channels as ch (ch)}{@render chRow(ch)}{/each}
  </div>
{/if}

<style>
  .cs-empty { color: #949494; font-size: 12px; font-style: italic; padding: 8px; }
  .cs { display: flex; flex-direction: column; gap: 5px; }
  .cs-source { color: #6f6f6f; font-size: 10px; font-style: italic; margin-bottom: 2px; }
  .cs-source.live { color: #7ec850; font-style: normal; font-weight: 700; }
  .cs-group {
    color: #37a8db; font-size: 10px; font-weight: 700; text-transform: uppercase; letter-spacing: 0.5px;
    margin: 6px 0 2px; padding-bottom: 3px; border-bottom: 1px solid rgba(55, 168, 219, 0.25);
  }
  .cs-group:first-child { margin-top: 0; }
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
