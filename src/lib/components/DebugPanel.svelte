<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { onMount, onDestroy } from "svelte";
  import { t } from 'svelte-i18n';
  import { get } from "svelte/store";
  import { radarAlertDebug, type AlertLevel } from "$lib/controllers/radarAlerts";
  import { gpsInject } from "$lib/stores/telemetry";
  import { settings } from "$lib/stores/settings";

  let { onclose }: { onclose: () => void } = $props();

  type Tab = 'msp' | 'alerts';
  let tab = $state<Tab>('msp');

  // Dev GPS injection (global — visible in both tabs). Mirror the store so reopening reflects the
  // current override; write back on every change.
  let inj = $state({ ...get(gpsInject) });
  function applyInject() {
    gpsInject.set({ ...inj });
  }
  function fillFromView() {
    const c = get(settings).map.center;
    inj.lat = Number(c[0].toFixed(6));
    inj.lon = Number(c[1].toFixed(6));
    applyInject();
  }

  interface MspCodeDebug {
    code: number;
    name: string;
    is_polling: boolean;
    request_count: number;
    response_count: number;
    timeout_count: number;
    last_status: string;
    target_rate_hz: number;
    actual_rate_hz: number;
  }

  interface DebugSnapshot {
    messages: MspCodeDebug[];
    msg_per_sec_tx: number;
    msg_per_sec_rx: number;
    bytes_per_sec_tx: number;
    bytes_per_sec_rx: number;
  }

  let snapshot = $state<DebugSnapshot>({
    messages: [],
    msg_per_sec_tx: 0,
    msg_per_sec_rx: 0,
    bytes_per_sec_tx: 0,
    bytes_per_sec_rx: 0,
  });

  // Alerts tab reads the controller's live debug snapshot directly (frontend store).
  const alerts = $derived($radarAlertDebug);

  let unlisten: (() => void) | null = null;

  onMount(async () => {
    unlisten = await listen<DebugSnapshot>("debug-msp-stats", (event) => {
      snapshot = event.payload;
    });
  });

  onDestroy(() => {
    if (unlisten) unlisten();
  });

  function ledColor(status: string): string {
    switch (status) {
      case "request": return "#f5a623";
      case "response": return "#59aa29";
      case "timeout": return "#d40000";
      default: return "#555";
    }
  }

  function levelColor(level: AlertLevel | null): string {
    switch (level) {
      case "warning": return "#d40000";
      case "caution": return "#f5a623";
      default: return "#555";
    }
  }

  function formatBytes(bytes: number): string {
    if (bytes < 1024) return `${bytes} B/s`;
    return `${(bytes / 1024).toFixed(1)} KB/s`;
  }

  function formatCode(code: number): string {
    if (code > 0xFF) return `0x${code.toString(16).toUpperCase().padStart(4, "0")}`;
    return code.toString();
  }

  /** Compact metres (raw dev readout — no unit conversion). */
  function m(v: number | null): string {
    if (v == null) return "—";
    return v >= 1000 ? `${(v / 1000).toFixed(1)}k` : `${Math.round(v)}`;
  }
</script>

<div class="debug-panel">
  <div class="debug-header">
    <span class="debug-title">{$t('debug.title')}</span>
    <button class="debug-close" onclick={onclose}>✕</button>
  </div>

  <div class="debug-tabs">
    <button class="tab" class:active={tab === 'msp'} onclick={() => tab = 'msp'}>{$t('debug.tabMsp')}</button>
    <button class="tab" class:active={tab === 'alerts'} onclick={() => tab = 'alerts'}>{$t('debug.tabAlerts')}</button>
  </div>

  <div class="inject-row" class:on={inj.active}>
    <label class="inj-toggle" title={$t('debug.injGps')}>
      <input type="checkbox" bind:checked={inj.active} onchange={applyInject} />
      {$t('debug.injGps')}
    </label>
    <input class="inj-num" type="number" step="any" aria-label={$t('debug.injLat')}
      placeholder={$t('debug.injLat')} bind:value={inj.lat} onchange={applyInject} />
    <input class="inj-num" type="number" step="any" aria-label={$t('debug.injLon')}
      placeholder={$t('debug.injLon')} bind:value={inj.lon} onchange={applyInject} />
    <input class="inj-num inj-alt" type="number" step="any" aria-label={$t('debug.injAlt')}
      placeholder={$t('debug.injAlt')} bind:value={inj.altMsl} onchange={applyInject} />
    <button class="inj-btn" onclick={fillFromView} title={$t('debug.injFromView')}>⌖</button>
  </div>

  {#if tab === 'msp'}
    <div class="debug-stats">
      <div class="stat-group">
        <span class="stat-label">{$t('debug.msgPerSec')}</span>
        <span class="stat-value">TX {snapshot.msg_per_sec_tx.toFixed(1)}</span>
        <span class="stat-sep">|</span>
        <span class="stat-value">RX {snapshot.msg_per_sec_rx.toFixed(1)}</span>
      </div>
      <div class="stat-group">
        <span class="stat-label">{$t('debug.throughput')}</span>
        <span class="stat-value">TX {formatBytes(snapshot.bytes_per_sec_tx)}</span>
        <span class="stat-sep">|</span>
        <span class="stat-value">RX {formatBytes(snapshot.bytes_per_sec_rx)}</span>
      </div>
    </div>

    <div class="debug-table-wrap">
      <table class="debug-table">
        <thead>
          <tr>
            <th class="col-led"></th>
            <th class="col-code">{$t('debug.colCode')}</th>
            <th class="col-name">{$t('debug.colName')}</th>
            <th class="col-status">{$t('debug.colStatus')}</th>
            <th class="col-num">{$t('debug.colReq')}</th>
            <th class="col-num">{$t('debug.colResp')}</th>
            <th class="col-num">{$t('debug.colTimeout')}</th>
            <th class="col-rate">{$t('debug.colTarget')}</th>
            <th class="col-rate">{$t('debug.colActual')}</th>
          </tr>
        </thead>
        <tbody>
          {#each snapshot.messages as msg}
            <tr class:inactive={!msg.is_polling}>
              <td class="col-led">
                <span
                  class="led"
                  style="background: {ledColor(msg.last_status)}; box-shadow: 0 0 4px {ledColor(msg.last_status)};"
                ></span>
              </td>
              <td class="col-code">{formatCode(msg.code)}</td>
              <td class="col-name">{msg.name}</td>
              <td class="col-status">
                <span class="status-badge" class:polling={msg.is_polling} class:init={!msg.is_polling}>
                  {msg.is_polling ? "POLL" : "INIT"}
                </span>
              </td>
              <td class="col-num">{msg.request_count}</td>
              <td class="col-num">{msg.response_count}</td>
              <td class="col-num" class:has-timeouts={msg.timeout_count > 0}>{msg.timeout_count}</td>
              <td class="col-rate">{msg.target_rate_hz > 0 ? `${msg.target_rate_hz} Hz` : '—'}</td>
              <td class="col-rate" class:throttled={msg.is_polling && msg.target_rate_hz > 0 && msg.actual_rate_hz < msg.target_rate_hz * 0.85}>
                {msg.actual_rate_hz > 0 ? `${msg.actual_rate_hz} Hz` : '—'}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {:else}
    <div class="debug-stats alerts-stats">
      {#if !alerts.uavValid}
        <span class="stat-warn">{$t('debug.alNoFix')}</span>
      {:else}
        <div class="stat-group">
          <span class="stat-label">{$t('debug.alCourse')}</span>
          <span class="stat-value">{alerts.uavCourseDeg.toFixed(0)}°</span>
          <span class="gate-badge" class:stable={alerts.courseStable} class:unstable={!alerts.courseStable}>
            {alerts.courseStable ? $t('debug.alStable') : $t('debug.alUnstable')}
          </span>
        </div>
        <div class="stat-group">
          <span class="stat-label">{$t('debug.alSpeed')}</span>
          <span class="stat-value">{alerts.uavSpeedMs.toFixed(1)} m/s</span>
          <span class="stat-sep">|</span>
          <span class="stat-label">{$t('debug.alVario')}</span>
          <span class="stat-value">{alerts.uavVarioMs.toFixed(1)} m/s</span>
        </div>
        <div class="stat-group">
          <span class="stat-label">{$t('debug.alWorst')}</span>
          <span class="level-badge" style="color: {levelColor(alerts.worst)};">{alerts.worst ?? '—'}</span>
          <span class="stat-sep">|</span>
          <span class="stat-label">{$t('debug.alEval')}</span>
          <span class="stat-value">{alerts.evaluated}</span>
        </div>
      {/if}
    </div>

    <div class="debug-table-wrap">
      <table class="debug-table">
        <thead>
          <tr>
            <th class="col-led"></th>
            <th class="col-name">{$t('debug.alColContact')}</th>
            <th class="col-num">{$t('debug.alColDh')}</th>
            <th class="col-num">{$t('debug.alColDv')}</th>
            <th class="col-num">{$t('debug.alColRate')}</th>
            <th class="col-num">{$t('debug.alColTcpa')}</th>
            <th class="col-num">{$t('debug.alColMissH')}</th>
            <th class="col-num">{$t('debug.alColMissV')}</th>
            <th class="col-stage">{$t('debug.alColS1')}</th>
            <th class="col-stage">{$t('debug.alColS2')}</th>
          </tr>
        </thead>
        <tbody>
          {#each alerts.rows as row (row.id)}
            <tr class:inactive={!row.level}>
              <td class="col-led">
                <span class="led" style="background: {levelColor(row.level)}; box-shadow: 0 0 4px {levelColor(row.level)};"></span>
              </td>
              <td class="col-name">{row.callsign ?? row.id}</td>
              <td class="col-num">{m(row.dH)}</td>
              <td class="col-num">{m(row.dV)}</td>
              <td class="col-num" class:closing={row.rangeRate != null && row.rangeRate < 0}>
                {row.rangeRate == null ? '—' : row.rangeRate.toFixed(0)}
              </td>
              <td class="col-num">{row.tCpa == null ? '—' : `${row.tCpa.toFixed(0)}s`}</td>
              <td class="col-num">{m(row.missH)}</td>
              <td class="col-num">{m(row.missV)}</td>
              <td class="col-stage"><span class="dot" class:on={row.stage1Raw}></span></td>
              <td class="col-stage"><span class="dot" class:on={row.stage2Raw}></span></td>
            </tr>
          {/each}
          {#if alerts.active && alerts.rows.length === 0}
            <tr><td colspan="10" class="empty-row">{$t('debug.alNone')}</td></tr>
          {/if}
        </tbody>
      </table>
    </div>
  {/if}
</div>

<style>
  .debug-panel {
    position: absolute;
    top: 65px;
    right: 12px;
    width: 480px;
    max-height: calc(100vh - 53px - 24px - 30px);
    background: rgba(30, 30, 30, 0.95);
    border: 1px solid rgba(55, 168, 219, 0.4);
    border-radius: 8px;
    z-index: 150;
    display: flex;
    flex-direction: column;
    box-shadow: 0 4px 20px rgba(0, 0, 0, 0.5);
    backdrop-filter: blur(12px);
    animation: debug-slide-in 0.2s ease-out;
    overflow: hidden;
    font-family: "Consolas", "JetBrains Mono", "Fira Code", monospace;
  }

  @keyframes debug-slide-in {
    from { opacity: 0; transform: translateX(16px); }
    to { opacity: 1; transform: translateX(0); }
  }

  .debug-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 8px 12px;
    background: rgba(55, 168, 219, 0.1);
    border-bottom: 1px solid rgba(55, 168, 219, 0.2);
  }

  .debug-title {
    font-size: 12px;
    font-weight: 600;
    color: #37a8db;
  }

  .debug-close {
    background: none;
    border: none;
    color: #949494;
    font-size: 14px;
    cursor: pointer;
    padding: 2px 6px;
    border-radius: 3px;
  }

  .debug-close:hover {
    background: rgba(212, 0, 0, 0.3);
    color: #ff4444;
  }

  .debug-tabs {
    display: flex;
    gap: 4px;
    padding: 6px 8px 0;
    border-bottom: 1px solid rgba(255, 255, 255, 0.06);
  }

  .tab {
    background: none;
    border: none;
    border-bottom: 2px solid transparent;
    color: #949494;
    font-size: 11px;
    font-weight: 600;
    padding: 5px 12px;
    cursor: pointer;
    font-family: inherit;
  }

  .tab:hover {
    color: #e0e0e0;
  }

  .tab.active {
    color: #37a8db;
    border-bottom-color: #37a8db;
  }

  .inject-row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 12px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.06);
    background: rgba(255, 255, 255, 0.02);
  }

  .inject-row.on {
    background: rgba(245, 166, 35, 0.08);
  }

  .inj-toggle {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 10px;
    font-weight: 700;
    color: #949494;
    text-transform: uppercase;
    letter-spacing: 0.3px;
    cursor: pointer;
    white-space: nowrap;
  }

  .inject-row.on .inj-toggle {
    color: #f5a623;
  }

  .inj-toggle input {
    accent-color: #f5a623;
    cursor: pointer;
  }

  .inj-num {
    width: 0;
    flex: 1;
    min-width: 0;
    background: rgba(0, 0, 0, 0.3);
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 3px;
    color: #e0e0e0;
    font-family: inherit;
    font-size: 10px;
    padding: 3px 5px;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  .inj-num.inj-alt {
    flex: 0 0 52px;
  }

  .inj-num:focus {
    outline: none;
    border-color: #37a8db;
  }

  .inj-btn {
    background: rgba(55, 168, 219, 0.15);
    border: 1px solid rgba(55, 168, 219, 0.3);
    border-radius: 3px;
    color: #37a8db;
    cursor: pointer;
    font-size: 12px;
    line-height: 1;
    padding: 3px 6px;
  }

  .inj-btn:hover {
    background: rgba(55, 168, 219, 0.3);
  }

  .debug-stats {
    display: flex;
    gap: 16px;
    padding: 8px 12px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.06);
    font-size: 11px;
  }

  .alerts-stats {
    flex-wrap: wrap;
    gap: 10px 16px;
  }

  .stat-group {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .stat-label {
    color: #949494;
    font-weight: 600;
  }

  .stat-value {
    color: #e0e0e0;
    font-variant-numeric: tabular-nums;
  }

  .stat-sep {
    color: #555;
  }

  .stat-warn {
    color: #f5a623;
    font-size: 11px;
  }

  .level-badge {
    font-weight: 700;
    text-transform: uppercase;
    font-size: 10px;
    letter-spacing: 0.3px;
  }

  .gate-badge {
    font-size: 9px;
    font-weight: 700;
    padding: 1px 5px;
    border-radius: 3px;
  }

  .gate-badge.stable {
    background: rgba(89, 170, 41, 0.15);
    color: #59aa29;
    border: 1px solid rgba(89, 170, 41, 0.3);
  }

  .gate-badge.unstable {
    background: rgba(148, 148, 148, 0.1);
    color: #999;
    border: 1px solid rgba(148, 148, 148, 0.2);
  }

  .debug-table-wrap {
    overflow-y: auto;
    flex: 1;
  }

  .debug-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 11px;
  }

  .debug-table thead {
    position: sticky;
    top: 0;
    background: rgba(30, 30, 30, 0.98);
  }

  .debug-table th {
    padding: 6px 8px;
    text-align: left;
    color: #949494;
    font-weight: 600;
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.3px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
  }

  .debug-table td {
    padding: 4px 8px;
    color: #e0e0e0;
    border-bottom: 1px solid rgba(255, 255, 255, 0.03);
  }

  .debug-table tr:hover {
    background: rgba(55, 168, 219, 0.05);
  }

  .debug-table tr.inactive td {
    color: #777;
  }

  .col-led {
    width: 20px;
    text-align: center;
  }

  .col-code {
    width: 55px;
    font-variant-numeric: tabular-nums;
  }

  .col-name {
    white-space: nowrap;
  }

  .col-status {
    width: 45px;
  }

  .col-num {
    width: 50px;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  th.col-num {
    text-align: right;
  }

  .col-rate {
    width: 55px;
    text-align: right;
    font-variant-numeric: tabular-nums;
    font-size: 10px;
  }

  th.col-rate {
    text-align: right;
  }

  .col-stage {
    width: 28px;
    text-align: center;
  }

  th.col-stage {
    text-align: center;
  }

  .closing {
    color: #f5a623 !important;
    font-weight: 700;
  }

  .empty-row {
    text-align: center !important;
    color: #777 !important;
    padding: 12px !important;
  }

  .throttled {
    color: #f5a623 !important;
    font-weight: 700;
  }

  .led {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    transition: background-color 0.05s, box-shadow 0.05s;
  }

  .dot {
    display: inline-block;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: rgba(148, 148, 148, 0.2);
  }

  .dot.on {
    background: #37a8db;
    box-shadow: 0 0 4px #37a8db;
  }

  .status-badge {
    display: inline-block;
    padding: 1px 5px;
    border-radius: 3px;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.3px;
  }

  .status-badge.polling {
    background: rgba(89, 170, 41, 0.15);
    color: #59aa29;
    border: 1px solid rgba(89, 170, 41, 0.3);
  }

  .status-badge.init {
    background: rgba(148, 148, 148, 0.1);
    color: #777;
    border: 1px solid rgba(148, 148, 148, 0.2);
  }

  .has-timeouts {
    color: #ff4444 !important;
    font-weight: 700;
  }
</style>
