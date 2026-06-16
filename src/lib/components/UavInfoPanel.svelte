<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // UAV Info on the panel framework (docs/active/PANEL_FRAMEWORK.md): the `info` variant —
  // content-sized, unframed.
  import { t } from 'svelte-i18n';
  import type { FcInfo } from '$lib/stores/connection';
  import PanelShell from './panel/PanelShell.svelte';

  let {
    connStatus,
    fcInfo,
  }: {
    connStatus: string;
    fcInfo: FcInfo | null;
  } = $props();

  function getPlatformLabel(type: number): string {
    const keys: Record<number, string> = {
      0: 'platform.multirotor', 1: 'platform.airplane', 2: 'platform.helicopter',
      3: 'platform.tricopter', 4: 'platform.rover', 5: 'platform.boat', 6: 'platform.other',
      7: 'platform.vtol', 255: 'platform.generic',
    };
    return keys[type] ? $t(keys[type]) : $t('platform.unknown', { values: { type } });
  }
</script>

<PanelShell variant="info" title={$t('nav.uavInfo')}>
  {#snippet body()}
    {#if connStatus === "connected" && fcInfo}
      <section class="panel-section">
        <h4 class="section-heading">{$t('uavInfo.flightController')}</h4>
        <div class="fc-info-grid">
          <span class="fc-label">{$t('uavInfo.craftName')}</span>
          <span class="fc-value">{fcInfo.craft_name || $t('uavInfo.craftNameUnset')}</span>
          <span class="fc-label">{$t('uavInfo.variant')}</span>
          <span class="fc-value">{fcInfo.fc_variant}</span>
          <span class="fc-label">{$t('uavInfo.version')}</span>
          <span class="fc-value">{fcInfo.fc_version}</span>
          <span class="fc-label">{$t('uavInfo.board')}</span>
          <span class="fc-value">{fcInfo.board_id}</span>
          <span class="fc-label">{$t('uavInfo.type')}</span>
          <span class="fc-value">{getPlatformLabel(fcInfo.platform_type)}</span>
          <span class="fc-label">{$t('uavInfo.api')}</span>
          <span class="fc-value">{fcInfo.api_version}</span>
          {#if fcInfo.hardware_revision > 0}
            <span class="fc-label">{$t('uavInfo.hwRev')}</span>
            <span class="fc-value">{fcInfo.hardware_revision}</span>
          {/if}
        </div>
      </section>

      {#if fcInfo.features}
        <section class="panel-section panel-section-last">
          <h4 class="section-heading">{$t('uavInfo.features')}</h4>
          <div class="feature-list">
            <span class="feature-badge available">{$t('uavInfo.telemetry')}</span>
            <span class="feature-badge" class:available={fcInfo.features.autoland_config} class:unavailable={!fcInfo.features.autoland_config} title="INAV 7.1+">{$t('uavInfo.autoland')}</span>
            <span class="feature-badge" class:available={fcInfo.features.geozones} class:unavailable={!fcInfo.features.geozones} title="INAV 8.0+">{$t('uavInfo.geozones')}</span>
            <span class="feature-badge" class:available={fcInfo.features.msp_rc} class:unavailable={!fcInfo.features.msp_rc} title="INAV 8.0+">{$t('uavInfo.mspRc')}</span>
            <span class="feature-badge" class:available={fcInfo.features.aux_rc} class:unavailable={!fcInfo.features.aux_rc} title="INAV 9.1+">{$t('uavInfo.auxRc')}</span>
            <span class="feature-badge" class:available={fcInfo.features.adsb_msp} class:unavailable={!fcInfo.features.adsb_msp} title="INAV 8.0+">{$t('uavInfo.adsb')}</span>
          </div>
        </section>
      {/if}
    {:else}
      <div class="panel-empty">
        <span class="panel-empty-icon">⊘</span>
        <span>{$t('uavInfo.notConnected')}</span>
      </div>
    {/if}
  {/snippet}
</PanelShell>

<style>
  .panel-section {
    margin-bottom: 16px;
  }
  /* The shell field is content-sized in `info`; trim the trailing gap. */
  .panel-section-last {
    margin-bottom: 0;
  }

  .section-heading {
    margin: 0 0 8px 0;
    font-size: 11px;
    font-weight: 600;
    color: #37a8db;
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .panel-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 32px 24px;
    color: #555;
    font-size: 12px;
  }

  .panel-empty-icon {
    font-size: 28px;
    opacity: 0.4;
  }

  .fc-info-grid {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 6px 10px;
    font-size: 12px;
  }

  .fc-label {
    color: #949494;
  }

  .fc-value {
    color: #e0e0e0;
    font-weight: 600;
  }

  /* Fixed 2 columns so the panel doesn't grow wide with the feature count (wraps to 2×N rows). */
  .feature-list {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 4px;
  }

  .feature-badge {
    padding: 3px 8px;
    border-radius: 3px;
    font-size: 10px;
    font-weight: 600;
    text-align: center;
  }

  .feature-badge.available {
    background: rgba(89, 170, 41, 0.2);
    color: #59aa29;
    border: 1px solid rgba(89, 170, 41, 0.4);
  }

  .feature-badge.unavailable {
    background: rgba(80, 80, 80, 0.2);
    color: #555;
    border: 1px solid #444;
    text-decoration: line-through;
  }
</style>
