<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->
<!--
  Startup recovery prompt for an orphan temp recording session left by a crash/close (ADR-042).
  Modal — no backdrop/Escape dismiss; the user must pick one of the three explicit actions.
-->
<script lang="ts" module>
  export interface RecoveryInfo {
    temp_path: string;
    craft_name: string;
    start_time: string;
    duration_sec: number;
    sample_count: number;
  }
  export type RecoveryChoice = 'discard' | 'save' | 'continue';
  /** Why the prompt is shown — selects the heading/intro text.
   *  `orphan`: a leftover session found at startup (crash/close). `lost`: the device link just dropped. */
  export type RecoveryReason = 'orphan' | 'lost';
</script>

<script lang="ts">
  import { t } from 'svelte-i18n';
  import { formatDurationSec } from '$lib/stores/flightlog';

  let open = $state(false);
  let info = $state<RecoveryInfo | null>(null);
  let reason = $state<RecoveryReason>('orphan');
  let confirmingDiscard = $state(false);
  let resolver: ((v: RecoveryChoice) => void) | null = null;

  export function show(i: RecoveryInfo, opts: { reason?: RecoveryReason } = {}): Promise<RecoveryChoice> {
    info = i;
    reason = opts.reason ?? 'orphan';
    confirmingDiscard = false;
    open = true;
    return new Promise((resolve) => { resolver = resolve; });
  }

  function choose(c: RecoveryChoice) {
    open = false;
    confirmingDiscard = false;
    if (resolver) { resolver(c); resolver = null; }
  }

  function fmtTime(s: string): string {
    const d = new Date(s);
    return isNaN(d.getTime()) ? (s || '—') : d.toLocaleString();
  }
</script>

{#if open && info}
  <!-- Modal: no backdrop/Escape dismiss — an explicit choice is required. -->
  <div class="dialog-backdrop">
    <div class="dialog-box">
      <div class="dialog-title">{reason === 'lost' ? $t('recovery.titleLost') : $t('recovery.title')}</div>
      <div class="rec-note">{reason === 'lost' ? $t('recovery.introLost') : $t('recovery.intro')}</div>

      <div class="fc-info-grid">
        <span class="fc-label">{$t('recovery.craft')}</span><span class="fc-value">{info.craft_name || '—'}</span>
        <span class="fc-label">{$t('recovery.started')}</span><span class="fc-value">{fmtTime(info.start_time)}</span>
        <span class="fc-label">{$t('logbook.duration')}</span><span class="fc-value">{formatDurationSec(info.duration_sec)}</span>
        <span class="fc-label">{$t('recovery.samples')}</span><span class="fc-value">{info.sample_count}</span>
      </div>

      {#if confirmingDiscard}
        <div class="rec-discard-warn">{$t('recovery.discardConfirm')}</div>
        <div class="dialog-buttons">
          <button class="dialog-btn" onclick={() => (confirmingDiscard = false)}>{$t('endFlight.cancel')}</button>
          <button class="dialog-btn dialog-btn-danger" onclick={() => choose('discard')}>{$t('recovery.discardConfirmYes')}</button>
        </div>
      {:else}
        <div class="dialog-buttons">
          <button class="dialog-btn dialog-btn-danger" onclick={() => (confirmingDiscard = true)}>{$t('recovery.discard')}</button>
          <span class="rec-spacer"></span>
          <button class="dialog-btn" onclick={() => choose('save')}>{$t('recovery.saveIncomplete')}</button>
          <button class="dialog-btn dialog-btn-primary" onclick={() => choose('continue')}>{$t('recovery.continue')}</button>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .dialog-backdrop { position: fixed; inset: 0; z-index: 9999; background: rgba(0, 0, 0, 0.55); display: flex; align-items: center; justify-content: center; }
  .dialog-box { background: #2e2e2e; border: 1px solid rgba(55, 168, 219, 0.45); border-radius: 8px; box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5); padding: 20px 24px 16px; min-width: 360px; max-width: 480px; }
  .dialog-title { font-size: 14px; font-weight: 700; color: #e0e0e0; margin-bottom: 10px; }
  .rec-note { font-size: 12px; color: #949494; margin-bottom: 14px; }

  .fc-info-grid { display: grid; grid-template-columns: auto minmax(0, 1fr); gap: 6px 14px; font-size: 13px; margin-bottom: 14px; }
  .fc-label { color: #949494; }
  .fc-value { color: #e0e0e0; font-weight: 600; }

  .rec-discard-warn { font-size: 12px; color: #f0b0b0; background: rgba(212, 0, 0, 0.12); border: 1px solid rgba(212, 0, 0, 0.4); border-radius: 4px; padding: 8px 10px; margin: 0 0 12px; }

  .dialog-buttons { display: flex; justify-content: flex-end; align-items: center; gap: 8px; margin-top: 4px; }
  .rec-spacer { flex: 1 1 auto; }
  .dialog-btn { padding: 6px 14px; font-size: 12px; font-weight: 600; border-radius: 4px; border: 1px solid #555; background: #434343; color: #e0e0e0; cursor: pointer; transition: background 0.15s; }
  .dialog-btn:hover { background: #505050; }
  .dialog-btn-primary { background: #1a6b94; border-color: #2590c8; color: #fff; }
  .dialog-btn-primary:hover { background: #237fae; }
  .dialog-btn-danger { background: #5a1414; border-color: #d40000; color: #f3c5c5; }
  .dialog-btn-danger:hover { background: #7a1a1a; }
</style>
