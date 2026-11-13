<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // The link editor of one local radar source (an ADS-B receiver row, the FormationFlight module) in the
  // Radar panel: a transport select, then the serial port + baud, or host + port for TCP / UDP. One
  // compact row — the panel is also used on a landscape phone. Serial is not offered where the platform
  // has no serial ports (iOS). Which serial ports are taken is the caller's answer (`portInUse`).
  import { t } from 'svelte-i18n';
  import NumberStepper from '$lib/components/NumberStepper.svelte';
  import { hasSerialPorts } from '$lib/platform';
  import type { PortInfo } from '$lib/stores/connection';
  import type { RadarLinkFields, RadarLinkTransport } from '$lib/stores/settings';

  let { link, serialPorts, portInUse, udpPortInUse = null, onTransport, onPatch, onRefreshPorts }: {
    /** The link as shown (`effectiveLink`). */
    link: RadarLinkFields;
    serialPorts: PortInfo[];
    /** Who else holds this serial port (display name), or null when it is free for this source. */
    portInUse: (path: string) => string | null;
    /** Who else listens on this link's UDP port (display name), or null. */
    udpPortInUse?: string | null;
    onTransport: (next: RadarLinkTransport) => void;
    onPatch: (patch: Partial<RadarLinkFields>) => void;
    onRefreshPorts: () => void;
  } = $props();

  const BAUD_STEPS = [57600, 115200, 9600, 38400, 230400, 921600];

  // NumberStepper's +/- fire a target-less change event: bind to a draft and commit it in onchange.
  // Reseeded from the prop (a commit reseeds to the same value → no drift).
  let netPortDraft = $state(0);
  $effect(() => {
    netPortDraft = link.netPort;
  });
  const commitNetPort = () => onPatch({ netPort: Math.round(netPortDraft) });
</script>

<div class="lr-row">
  <select
    class="lr-select"
    value={link.transport}
    title={$t('radar.transport')}
    aria-label={$t('radar.transport')}
    onchange={(e) => onTransport((e.target as HTMLSelectElement).value as RadarLinkTransport)}
  >
    {#if hasSerialPorts}<option value="serial">{$t('radar.transportSerial')}</option>{/if}
    <option value="tcp">TCP</option>
    <option value="udp">UDP</option>
  </select>
  {#if link.transport === 'serial'}
    <select class="lr-select lr-grow" value={link.port} onchange={(e) => onPatch({ port: (e.target as HTMLSelectElement).value })}>
      <option value="">{$t('radar.selectPort')}</option>
      {#each serialPorts as sp}
        {@const who = portInUse(sp.path)}
        <!-- A taken port is disabled, unless it is this source's own current port (shown as it is). -->
        <option value={sp.path} disabled={!!who && sp.path !== link.port}>{sp.label || sp.path}{who ? ` (${$t('radar.inUse')}: ${who})` : ''}</option>
      {/each}
      {#if link.port && !serialPorts.some((sp) => sp.path === link.port)}
        <option value={link.port}>{link.port} ({$t('radar.portMissing')})</option>
      {/if}
    </select>
    <button class="lr-refresh" title={$t('radar.refreshPorts')} onclick={onRefreshPorts} aria-label={$t('radar.refreshPorts')}>⟳</button>
    <select class="lr-select" value={link.baud} onchange={(e) => onPatch({ baud: Number((e.target as HTMLSelectElement).value) })}>
      {#each BAUD_STEPS as b}<option value={b}>{b}</option>{/each}
    </select>
  {:else}
    <input
      class="lr-input lr-grow"
      type="text"
      placeholder={link.transport === 'udp' ? $t('radar.hostOptional') : $t('radar.hostPlaceholder')}
      value={link.host}
      onchange={(e) => onPatch({ host: (e.target as HTMLInputElement).value.trim() })}
    />
    <NumberStepper bind:value={netPortDraft} min={1} max={65535} step={1} onchange={commitNetPort} />
  {/if}
</div>
{#if link.transport === 'udp'}
  <p class="lr-hint">{$t('radar.udpHint', { values: { port: link.netPort } })}</p>
  {#if udpPortInUse}
    <p class="lr-hint lr-warn">{$t('radar.udpPortInUse', { values: { port: link.netPort, who: udpPortInUse } })}</p>
  {/if}
{/if}

<style>
  .lr-row { display: flex; gap: 6px; align-items: center; }
  .lr-grow { flex: 1; min-width: 0; }
  .lr-select {
    height: 28px;
    padding: 0 8px;
    background: #434343;
    border: 1px solid #555;
    border-radius: 4px;
    color: #e0e0e0;
    font-size: 12px;
  }
  .lr-input {
    box-sizing: border-box;
    height: 26px;
    padding: 0 7px;
    background: #1f1f1f;
    border: 1px solid #444;
    border-radius: 4px;
    color: #e0e0e0;
    font-size: 11px;
    font-family: 'Segoe UI', Tahoma, sans-serif;
  }
  .lr-input::placeholder { color: #777; }
  .lr-refresh {
    flex-shrink: 0;
    width: 26px;
    height: 26px;
    border: 1px solid #555;
    border-radius: 4px;
    background: #434343;
    color: #cdd6da;
    cursor: pointer;
    font-size: 14px;
    line-height: 1;
  }
  .lr-refresh:hover { background: #4d4d4d; color: #fff; }
  .lr-hint { color: #949494; font-size: 11px; line-height: 1.4; margin: 2px 2px 0; }
  .lr-warn { color: #e0a030; }
</style>
