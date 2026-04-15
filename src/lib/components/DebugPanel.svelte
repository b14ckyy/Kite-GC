<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { onMount, onDestroy } from "svelte";

  let { onclose }: { onclose: () => void } = $props();

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

  function formatBytes(bytes: number): string {
    if (bytes < 1024) return `${bytes} B/s`;
    return `${(bytes / 1024).toFixed(1)} KB/s`;
  }

  function formatCode(code: number): string {
    if (code > 0xFF) return `0x${code.toString(16).toUpperCase().padStart(4, "0")}`;
    return code.toString();
  }
</script>

<div class="debug-panel">
  <div class="debug-header">
    <span class="debug-title">🔧 MSP Debug Monitor</span>
    <button class="debug-close" onclick={onclose}>✕</button>
  </div>

  <div class="debug-stats">
    <div class="stat-group">
      <span class="stat-label">MSG/s</span>
      <span class="stat-value">TX {snapshot.msg_per_sec_tx.toFixed(1)}</span>
      <span class="stat-sep">|</span>
      <span class="stat-value">RX {snapshot.msg_per_sec_rx.toFixed(1)}</span>
    </div>
    <div class="stat-group">
      <span class="stat-label">Throughput</span>
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
          <th class="col-code">Code</th>
          <th class="col-name">Name</th>
          <th class="col-status">Status</th>
          <th class="col-num">Req</th>
          <th class="col-num">Resp</th>
          <th class="col-num">T/O</th>
          <th class="col-rate">Target</th>
          <th class="col-rate">Actual</th>
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

  .debug-stats {
    display: flex;
    gap: 16px;
    padding: 8px 12px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.06);
    font-size: 11px;
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
    color: #666;
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
