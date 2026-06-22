<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // Telemetry Relay panel — drops down under the connection bar. Top: a basic LINK diagnostic (active
  // protocol + RX/TX byte rate + msgs/s). Below: the configured relays (≈ a copy of the connect bar),
  // each re-encoding the live telemetry into a wire protocol and emitting it out a second link. See
  // docs/active/TELEMETRY_FORWARDING.md.
  import { onMount } from 'svelte';
  import { t } from 'svelte-i18n';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { connection, connectionProtocol, availablePorts, bleDevices } from '$lib/stores/connection';
  import { settings } from '$lib/stores/settings';
  import { relayStats, relayResults, newRelay, type RelayConfig } from '$lib/stores/relay';
  import { reconfigureRelays } from '$lib/controllers/relayController';
  import { startBleScan, stopBleScan, startBleDeviceListener, stopBleDeviceListener, refreshSerialPorts } from '$lib/controllers/connectionController';

  let { open }: { open: boolean } = $props();

  const BAUD_RATES = [9600, 19200, 38400, 57600, 115200, 230400];

  const connected = $derived($connection.status === 'connected');
  const relays = $derived($settings.relays ?? []);

  // Listen for BLE discovery events whenever the panel is open. This is pure event subscription (no
  // adapter use), so it's always safe and lets devices found by ANY scan — the main connect's or ours —
  // populate the relay's BLE picker.
  $effect(() => {
    if (!open) return;
    void startBleDeviceListener();
    return () => stopBleDeviceListener();
  });

  // Run our OWN BLE scan to populate the picker — but ONLY when connected via a non-BLE transport, so we
  // never fight the main connection's BLE scan (while disconnected the user may be picking a BLE primary)
  // or share the adapter with a BLE primary link. Refresh the serial list at the same time so both
  // device pickers are self-sufficient (the relay panel no longer depends on the main bar having scanned).
  $effect(() => {
    if (open && $connection.status === 'connected' && $connection.transportType !== 'ble') {
      void refreshSerialPorts(''); // return value (the auto-selected port) is irrelevant here
      void startBleScan();
      return () => {
        void stopBleScan();
      };
    }
  });

  // ── LINK diagnostics (reuse the Debug Monitor stat events; one protocol active at a time) ──
  let link = $state({ rxBps: 0, txBps: 0, msgRx: 0, msgTx: 0 });
  onMount(() => {
    // Backfill defaults for relays persisted before the tcp/udp fields existed (so a config that only
    // shows the ?? fallback in the UI actually carries the value to the backend).
    const cur = $settings.relays ?? [];
    let changed = false;
    const fixed = cur.map((r) => {
      const o = r.output;
      if (o.baud === undefined || o.bleDeviceId === undefined || o.listenPort === undefined || o.host === undefined || o.udpPort === undefined) {
        changed = true;
        return {
          ...r,
          output: { ...o, baud: o.baud ?? 115200, bleDeviceId: o.bleDeviceId ?? '', listenPort: o.listenPort ?? 5760, host: o.host ?? '', udpPort: o.udpPort ?? 14550 },
        };
      }
      return r;
    });
    if (changed) {
      settings.patch({ relays: fixed });
      void reconfigureRelays(); // re-apply with the fixed config if already connected
    }

    const unlisteners: UnlistenFn[] = [];
    const sub = async () => {
      // MSP + MAVLink feed the always-on `link-stats` meter (compiled in release too); the passive
      // 'telemetry' mode reports its byte rate on the always-emitted `debug-telemetry-stats`. Only one
      // protocol is active at a time, so a single shared `link` state is fine.
      unlisteners.push(
        await listen<{ bytes_per_sec_rx: number; bytes_per_sec_tx: number; msg_per_sec_rx: number; msg_per_sec_tx: number }>(
          'link-stats',
          (e) => (link = { rxBps: e.payload.bytes_per_sec_rx, txBps: e.payload.bytes_per_sec_tx, msgRx: e.payload.msg_per_sec_rx, msgTx: e.payload.msg_per_sec_tx }),
        ),
      );
      unlisteners.push(
        await listen<{ bytes_per_sec: number }>(
          'debug-telemetry-stats',
          (e) => (link = { rxBps: e.payload.bytes_per_sec, txBps: 0, msgRx: 0, msgTx: 0 }),
        ),
      );
    };
    void sub();
    return () => unlisteners.forEach((u) => u());
  });

  function fmtBytes(b: number): string {
    if (b >= 1024) return `${(b / 1024).toFixed(1)} kB/s`;
    return `${Math.round(b)} B/s`;
  }

  // ── Relay row editing — persist immediately + re-apply live when connected ──
  function patchRelay(id: string, patch: Partial<RelayConfig>) {
    const next = relays.map((r) => (r.id === id ? { ...r, ...patch } : r));
    settings.patch({ relays: next });
    void reconfigureRelays();
  }
  function patchOutput(id: string, patch: Partial<RelayConfig['output']>) {
    const next = relays.map((r) => (r.id === id ? { ...r, output: { ...r.output, ...patch } } : r));
    settings.patch({ relays: next });
    void reconfigureRelays();
  }
  // ── Port guards ──────────────────────────────────────────────────────────────
  // TCP listen ports are local binds → must be unique (a duplicate makes the 2nd relay fail to bind).
  // UDP targets must be a unique host:port pair (same port to different hosts is fine). We auto-bump to
  // the next free port so a duplicate can't be configured.
  function nextFreeTcpPort(start: number, excludeId: string): number {
    let p = Math.max(1, Math.min(65535, start || 5760));
    const used = (port: number) =>
      relays.some((r) => r.id !== excludeId && r.output.kind === 'tcp' && r.output.listenPort === port);
    while (used(p) && p < 65535) p++;
    return p;
  }
  function nextFreeUdpPort(host: string, start: number, excludeId: string): number {
    let p = Math.max(1, Math.min(65535, start || 14550));
    const used = (port: number) =>
      relays.some(
        (r) => r.id !== excludeId && r.output.kind === 'udp' && (r.output.host ?? '') === host && r.output.udpPort === port,
      );
    while (used(p) && p < 65535) p++;
    return p;
  }

  // Change output kind, ensuring the fields that kind needs always carry a value (not just the UI
  // fallback) so the backend never sees a missing field, and that ports don't collide with other relays.
  function setKind(r: RelayConfig, kind: RelayConfig['output']['kind']) {
    const host = r.output.host ?? '';
    patchOutput(r.id, {
      kind,
      baud: r.output.baud ?? 115200,
      bleDeviceId: r.output.bleDeviceId ?? '',
      host,
      listenPort: kind === 'tcp' ? nextFreeTcpPort(r.output.listenPort ?? 5760, r.id) : (r.output.listenPort ?? 5760),
      udpPort: kind === 'udp' ? nextFreeUdpPort(host, r.output.udpPort ?? 14550, r.id) : (r.output.udpPort ?? 14550),
    });
  }

  function addRelay() {
    settings.patch({ relays: [...relays, newRelay()] });
  }
  function removeRelay(id: string) {
    settings.patch({ relays: relays.filter((r) => r.id !== id) });
    void reconfigureRelays();
  }

  // Status dot per relay: active (green) / error (red) / waiting (amber) / idle (grey).
  function rowState(id: string): { cls: string; label: string; detail: string } {
    if (!connected) return { cls: 'idle', label: $t('relay.idle'), detail: '' };
    const stat = $relayStats.find((s) => s.id === id);
    if (stat) {
      if (!stat.ok) return { cls: 'err', label: $t('relay.error'), detail: `${stat.errors}` };
      if (stat.waiting) return { cls: 'waiting', label: $t('relay.waiting'), detail: '' };
      return { cls: 'active', label: $t('relay.active'), detail: fmtBytes(stat.bytesPerSec) };
    }
    const res = $relayResults[id];
    if (res && !res.ok) return { cls: 'err', label: $t('relay.deviceMissing'), detail: res.error ?? '' };
    return { cls: 'waiting', label: $t('relay.waiting'), detail: '' };
  }
</script>

{#if open}
  <div class="relay-panel">
    <!-- LINK diagnostics -->
    <div class="link-row">
      <span class="link-label">{$t('relay.link')}</span>
      {#if connected}
        <span class="link-proto">{$connectionProtocol.primary || '—'}</span>
        <span class="sep">·</span>
        <span class="link-stat">RX {fmtBytes(link.rxBps)}</span>
        {#if link.msgRx > 0}<span class="link-stat dim">{link.msgRx.toFixed(0)} msg/s</span>{/if}
        <span class="sep">·</span>
        <span class="link-stat">TX {fmtBytes(link.txBps)}</span>
      {:else}
        <span class="link-stat dim">{$t('relay.notConnected')}</span>
      {/if}
    </div>

    <!-- Relays -->
    <div class="section-label">{$t('relay.relays')}</div>
    {#if relays.length === 0}
      <div class="empty">{$t('relay.none')}</div>
    {/if}
    {#each relays as r (r.id)}
      {@const st = rowState(r.id)}
      <div class="relay-row">
        <input
          type="checkbox"
          class="enable"
          checked={r.enabled}
          title={$t('relay.enable')}
          onchange={(e) => patchRelay(r.id, { enabled: e.currentTarget.checked })}
        />
        <select class="r-select proto" value={r.protocol} onchange={(e) => patchRelay(r.id, { protocol: e.currentTarget.value as RelayConfig['protocol'] })}>
          <option value="ltm">LTM</option>
          <option value="mavlink">MAVLink</option>
          <option value="crsf">CRSF</option>
          <option value="smartport">SmartPort</option>
        </select>
        <select class="r-select kind" value={r.output.kind} onchange={(e) => setKind(r, e.currentTarget.value as RelayConfig['output']['kind'])}>
          <option value="serial">{$t('relay.serial')}</option>
          <option value="ble">{$t('relay.ble')}</option>
          <option value="tcp">TCP</option>
          <option value="udp">UDP</option>
        </select>
        {#if r.output.kind === 'serial'}
          <select class="r-select port" value={r.output.port ?? ''} onchange={(e) => patchOutput(r.id, { port: e.currentTarget.value, baud: r.output.baud ?? 115200 })}>
            <option value="">{$t('relay.selectDevice')}</option>
            {#each $availablePorts as p}
              <option value={p.path}>{p.label}</option>
            {/each}
          </select>
          <select class="r-select baud" value={r.output.baud ?? 115200} onchange={(e) => patchOutput(r.id, { baud: Number(e.currentTarget.value) })}>
            {#each BAUD_RATES as b}
              <option value={b}>{b}</option>
            {/each}
          </select>
        {:else if r.output.kind === 'ble'}
          <select class="r-select port" value={r.output.bleDeviceId ?? ''} onchange={(e) => patchOutput(r.id, { bleDeviceId: e.currentTarget.value })}>
            <option value="">{$t('relay.selectDevice')}</option>
            {#each $bleDevices as d}
              <option value={d.id}>{d.name}{d.profile && d.profile !== 'Unknown' ? ` (${d.profile})` : ''}</option>
            {/each}
          </select>
        {:else if r.output.kind === 'tcp'}
          <input
            class="r-input port-num"
            type="number"
            min="1"
            max="65535"
            placeholder={$t('relay.listenPort')}
            value={r.output.listenPort ?? 5760}
            onchange={(e) => patchOutput(r.id, { listenPort: nextFreeTcpPort(Number(e.currentTarget.value), r.id) })}
          />
        {:else if r.output.kind === 'udp'}
          <input
            class="r-input host"
            type="text"
            placeholder={$t('relay.host')}
            value={r.output.host ?? ''}
            onchange={(e) => {
              const host = e.currentTarget.value;
              patchOutput(r.id, { host, udpPort: nextFreeUdpPort(host, r.output.udpPort ?? 14550, r.id) });
            }}
          />
          <input
            class="r-input port-num"
            type="number"
            min="1"
            max="65535"
            placeholder={$t('relay.port')}
            value={r.output.udpPort ?? 14550}
            onchange={(e) => patchOutput(r.id, { udpPort: nextFreeUdpPort(r.output.host ?? '', Number(e.currentTarget.value), r.id) })}
          />
        {/if}
        <span class="status {st.cls}" title={st.detail}>
          <span class="dot"></span>{st.label}{#if st.detail}<span class="detail">{st.detail}</span>{/if}
        </span>
        <button class="remove" title={$t('relay.remove')} onclick={() => removeRelay(r.id)}>✕</button>
      </div>
    {/each}
    <button class="add" onclick={addRelay}>+ {$t('relay.addRelay')}</button>
  </div>
{/if}

<style>
  .relay-panel {
    position: absolute;
    top: 50px;
    right: 8px;
    width: 520px;
    max-width: calc(100vw - 16px);
    background: rgba(46, 46, 46, 0.97);
    border: 1px solid #272727;
    border-top: 2px solid #37a8db;
    border-radius: 0 0 8px 8px;
    box-shadow: 0 6px 16px rgba(0, 0, 0, 0.45);
    backdrop-filter: blur(8px);
    padding: 10px 12px;
    z-index: 199;
    font-family: 'Segoe UI', Tahoma, sans-serif;
    color: #e0e0e0;
  }

  .link-row {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    padding-bottom: 8px;
    border-bottom: 1px solid #272727;
    flex-wrap: wrap;
  }
  .link-label { font-weight: 700; color: #37a8db; letter-spacing: 0.04em; }
  .link-proto { font-weight: 600; }
  .link-stat { color: #cfcfcf; font-variant-numeric: tabular-nums; }
  .link-stat.dim, .sep { color: #949494; }

  .section-label {
    font-size: 10px;
    font-weight: 700;
    color: #949494;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    margin: 8px 0 6px;
  }
  .empty { font-size: 12px; color: #949494; padding: 2px 0 6px; }

  .relay-row {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 6px;
  }
  .enable { width: 15px; height: 15px; accent-color: #37a8db; flex: none; }

  .r-select {
    height: 28px;
    box-sizing: border-box;
    padding: 0 6px;
    background: #434343;
    border: 1px solid #555;
    border-radius: 4px;
    color: #e0e0e0;
    font-size: 12px;
  }
  .r-select.proto { width: 92px; }
  .r-select.kind { width: 76px; }
  .r-select.port { flex: 1 1 auto; min-width: 0; text-overflow: ellipsis; }
  .r-select.baud { width: 84px; }

  .r-input {
    height: 28px;
    box-sizing: border-box;
    padding: 0 6px;
    background: #434343;
    border: 1px solid #555;
    border-radius: 4px;
    color: #e0e0e0;
    font-size: 12px;
  }
  .r-input::placeholder { color: #777; }
  .r-input.host { flex: 1 1 auto; min-width: 0; }
  .r-input.port-num { width: 84px; appearance: textfield; -moz-appearance: textfield; }
  .r-input.port-num::-webkit-inner-spin-button,
  .r-input.port-num::-webkit-outer-spin-button { -webkit-appearance: none; margin: 0; }

  .status {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    min-width: 92px;
  }
  .status .dot { width: 8px; height: 8px; border-radius: 50%; background: #949494; flex: none; }
  .status .detail { color: #949494; font-variant-numeric: tabular-nums; }
  .status.active .dot { background: #59aa29; box-shadow: 0 0 5px rgba(89, 170, 41, 0.7); }
  .status.active { color: #59aa29; }
  .status.err .dot { background: #d40000; box-shadow: 0 0 5px rgba(212, 0, 0, 0.7); }
  .status.err { color: #d40000; }
  .status.waiting .dot { background: #f5a623; }
  .status.waiting { color: #f5a623; }
  .status.idle { color: #777; }

  .remove {
    background: none;
    border: none;
    color: #777;
    cursor: pointer;
    font-size: 13px;
    padding: 2px 4px;
    border-radius: 3px;
  }
  .remove:hover { color: #d40000; background: rgba(212, 0, 0, 0.12); }

  .add {
    margin-top: 4px;
    background: rgba(55, 168, 219, 0.12);
    border: 1px solid rgba(55, 168, 219, 0.4);
    color: #37a8db;
    font-size: 12px;
    padding: 5px 10px;
    border-radius: 4px;
    cursor: pointer;
  }
  .add:hover { background: rgba(55, 168, 219, 0.22); }
</style>
