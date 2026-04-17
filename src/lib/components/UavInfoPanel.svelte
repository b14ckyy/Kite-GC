<script lang="ts">
  import { t } from 'svelte-i18n';
  import type { FcInfo } from '$lib/stores/connection';

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
    };
    return keys[type] ? $t(keys[type]) : $t('platform.unknown', { values: { type } });
  }
</script>

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
    <section class="panel-section">
      <h4 class="section-heading">{$t('uavInfo.features')}</h4>
      <div class="feature-list">
        <span class="feature-badge available">{$t('uavInfo.telemetry')}</span>
        <span class="feature-badge" class:available={fcInfo.features.autoland_config} class:unavailable={!fcInfo.features.autoland_config} title="INAV 7.1+">{$t('uavInfo.autoland')}</span>
        <span class="feature-badge" class:available={fcInfo.features.geozones} class:unavailable={!fcInfo.features.geozones} title="INAV 8.0+">{$t('uavInfo.geozones')}</span>
        <span class="feature-badge" class:available={fcInfo.features.msp_rc} class:unavailable={!fcInfo.features.msp_rc} title="INAV 8.0+">{$t('uavInfo.mspRc')}</span>
        <span class="feature-badge" class:available={fcInfo.features.aux_rc} class:unavailable={!fcInfo.features.aux_rc} title="INAV 9.1+">{$t('uavInfo.auxRc')}</span>
      </div>
    </section>
  {/if}
{:else}
  <div class="panel-empty">
    <span class="panel-empty-icon">⊘</span>
    <span>{$t('uavInfo.notConnected')}</span>
  </div>
{/if}

<style>
  .panel-section {
    margin-bottom: 16px;
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
    padding: 32px 0;
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

  .feature-list {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }

  .feature-badge {
    padding: 3px 8px;
    border-radius: 3px;
    font-size: 10px;
    font-weight: 600;
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
