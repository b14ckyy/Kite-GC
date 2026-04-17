<script lang="ts">
  import { t } from 'svelte-i18n';
  import type { FcInfo } from '$lib/stores/connection';
  import type { TelemetryData } from '$lib/stores/telemetry';
  import { isArmed } from '$lib/helpers/telemetry';

  let {
    connStatus,
    fcInfo,
    telem,
    connectionPort,
    devMode = false,
    debugOpen = $bindable(false),
  }: {
    connStatus: string;
    fcInfo: FcInfo | null;
    telem: TelemetryData;
    connectionPort: string;
    devMode?: boolean;
    debugOpen?: boolean;
  } = $props();

  function armed(): boolean {
    return isArmed(telem.armingFlags, telem.lastUpdate);
  }

  function getArmingLabel(): string {
    if (!telem.lastUpdate) return "";
    return armed() ? $t('arming.armed') : $t('arming.disarmed');
  }
</script>

<footer class="statusbar">
  <div class="statusbar-left">
    <span
      class="status-indicator"
      class:connected={connStatus === "connected"}
      class:disconnected={connStatus !== "connected"}
    ></span>
    <span>
      {#if connStatus === "connected" && fcInfo}
        {$t('connection.connectedOn', { values: { variant: fcInfo.fc_variant, version: fcInfo.fc_version, port: connectionPort } })}
      {:else if connStatus === "connecting"}
        {$t('connection.connecting')}
      {:else}
        {$t('connection.disconnected')}
      {/if}
    </span>
    {#if devMode}
      <button class="debug-btn" class:open={debugOpen} onclick={() => debugOpen = !debugOpen} title="MSP Debug Monitor">
        🔧 {$t('statusBar.debug')}
      </button>
    {/if}
  </div>
  <div class="statusbar-right">
    {#if connStatus === "connected" && telem.lastUpdate > 0}
      <span class="status-arming" class:armed={armed()} class:disarmed={!armed()}>
        {getArmingLabel()}
      </span>
      <span class="status-sep">|</span>
    {/if}
    <span>{$t('app.brand')}</span>
  </div>
</footer>

<style>
  .statusbar {
    position: absolute;
    bottom: 0;
    left: 0;
    right: 0;
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0 10px;
    height: 24px;
    background: #2e2e2e;
    border-top: 1px solid #272727;
    font-size: 11px;
    color: #949494;
    z-index: 200;
  }

  .statusbar-left {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .status-indicator {
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }

  .status-indicator.disconnected {
    background: #d40000;
    box-shadow: 0 0 4px rgba(212, 0, 0, 0.5);
  }

  .status-indicator.connected {
    background: #59aa29;
    box-shadow: 0 0 4px rgba(89, 170, 41, 0.5);
  }

  .statusbar-right {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .status-arming {
    font-weight: 700;
    font-size: 10px;
    letter-spacing: 0.5px;
  }

  .status-arming.armed {
    color: #ff4444;
  }

  .status-arming.disarmed {
    color: #59aa29;
  }

  .status-sep {
    color: #555;
  }

  .debug-btn {
    background: none;
    border: 1px solid transparent;
    color: #666;
    font-size: 11px;
    cursor: pointer;
    padding: 0 6px;
    border-radius: 3px;
    margin-left: 8px;
    transition: color 0.2s, border-color 0.2s, background-color 0.2s;
  }

  .debug-btn:hover {
    color: #f5a623;
    border-color: rgba(245, 166, 35, 0.3);
  }

  .debug-btn.open {
    color: #f5a623;
    border-color: rgba(245, 166, 35, 0.4);
    background: rgba(245, 166, 35, 0.1);
  }
</style>
