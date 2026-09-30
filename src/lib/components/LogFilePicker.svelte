<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts" module>
  /** One day-file of the diagnostics log, as the `list_log_files` command reports it. */
  export interface LogFileInfo {
    /** `kite-gc-YYYY-MM-DD.log` — the date is read from here. */
    name: string;
    /** Absolute path, what `share_file` takes. */
    path: string;
    /** Bytes at the time of the listing. */
    size: number;
    /** The file the running session appends to. */
    active: boolean;
  }
</script>

<script lang="ts">
  // Picker behind the mobile "Share log file" button (Settings → Diagnostics). The log folder is
  // app-private on Android — no file manager or MTP connection reaches it — so the system share sheet
  // is the only way a log leaves the device, and after a restart the file a tester needs is
  // yesterday's, not the active one. One row per day-file, newest first; a tap hands the path to
  // `onPick`. Markup and colours follow ConfirmDialog; rows are finger-sized (≥ 44 px) and the list
  // scrolls inside a 60vh cap so the dialog fits a 5.5" landscape phone.
  import { t } from 'svelte-i18n';

  let {
    files,
    onPick,
    onClose,
  }: { files: LogFileInfo[]; onPick: (path: string) => void; onClose: () => void } = $props();

  /** `kite-gc-2026-09-16.log` → `2026-09-16`; a name outside the scheme is shown as it is. */
  function dateOf(name: string): string {
    const m = /^kite-gc-(\d{4}-\d{2}-\d{2})\.log$/.exec(name);
    return m ? m[1] : name;
  }

  /** Whole KB below 1 MB, one decimal above — the logbook's raw-log badge uses the same steps. */
  function formatSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') onClose();
  }

  // Focus the box on open so Escape (a tablet with a keyboard) reaches the handler above; the
  // keydown listener sits on the backdrop and only sees events from focus inside it.
  let box = $state<HTMLDivElement | null>(null);
  $effect(() => {
    box?.focus();
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
<div class="dialog-backdrop" onclick={onClose} onkeydown={onKeydown}>
  <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
  <div class="dialog-box" role="dialog" aria-modal="true" aria-labelledby="log-picker-title" tabindex="-1" bind:this={box} onclick={(e) => e.stopPropagation()}>
    <div class="dialog-title" id="log-picker-title">{$t('settings.logPickerTitle')}</div>
    {#if files.length === 0}
      <div class="dialog-empty">{$t('settings.logPickerEmpty')}</div>
    {:else}
      <div class="log-list">
        {#each files as f (f.path)}
          <button class="log-row" onclick={() => onPick(f.path)}>
            <span class="log-date">{dateOf(f.name)}</span>
            {#if f.active}<span class="log-tag">{$t('settings.logPickerCurrent')}</span>{/if}
            <span class="log-size">{formatSize(f.size)}</span>
          </button>
        {/each}
      </div>
    {/if}
    <div class="dialog-buttons">
      <button class="dialog-btn dialog-btn-cancel" onclick={onClose}>{$t('settings.logPickerClose')}</button>
    </div>
  </div>
</div>

<style>
  .dialog-backdrop {
    position: fixed;
    inset: 0;
    z-index: 10000;
    background: rgba(0, 0, 0, 0.55);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 16px;
  }

  .dialog-box {
    background: #2e2e2e;
    border: 1px solid rgba(55, 168, 219, 0.45);
    border-radius: 8px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
    padding: 16px 18px 12px;
    box-sizing: border-box;
    width: min(420px, 100%);
    /* % of the backdrop, not vh: the backdrop fills the --ui-scale'd panels layer, so this clamp
       holds at any zoom while a vh figure would be multiplied by the scale. */
    max-height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
    font-family: 'Segoe UI', Tahoma, sans-serif;
  }
  .dialog-box:focus { outline: none; }

  .dialog-title {
    font-size: 14px;
    font-weight: 700;
    color: #e0e0e0;
    margin-bottom: 10px;
  }

  .dialog-empty {
    font-size: 12px;
    color: #949494;
    padding: 12px 0 14px;
  }

  /* The list scrolls, the dialog does not: 30 days of files must fit under the Close button on a
     720 px-high phone screen. Shrinkable, so the box's own clamp above wins when 60vh is too tall. */
  .log-list {
    display: flex;
    flex-direction: column;
    flex: 0 1 auto;
    min-height: 0;
    max-height: 60vh;
    overflow-y: auto;
    border: 1px solid #272727;
    border-radius: 6px;
    background: #262626;
    margin-bottom: 12px;
  }

  .log-row {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 44px;
    padding: 0 12px;
    border: none;
    border-bottom: 1px solid #272727;
    background: transparent;
    color: #e0e0e0;
    font: inherit;
    font-size: 13px;
    text-align: left;
    cursor: pointer;
  }
  .log-row:last-child { border-bottom: none; }
  .log-row:hover { background: rgba(55, 168, 219, 0.12); }
  .log-row:active { background: rgba(55, 168, 219, 0.24); }

  .log-date {
    font-variant-numeric: tabular-nums;
  }

  .log-tag {
    font-size: 10.5px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.4px;
    color: #37a8db;
    border: 1px solid rgba(55, 168, 219, 0.55);
    border-radius: 3px;
    padding: 1px 5px;
  }

  .log-size {
    margin-left: auto;
    color: #949494;
    font-size: 12px;
    white-space: nowrap;
  }

  .dialog-buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  .dialog-btn {
    min-height: 40px;
    padding: 6px 16px;
    font-size: 12px;
    font-weight: 600;
    border-radius: 4px;
    border: 1px solid #555;
    background: #434343;
    color: #e0e0e0;
    cursor: pointer;
    transition: background 0.15s;
  }
  .dialog-btn:hover { background: #505050; }
  .dialog-btn-cancel { color: #999; }
</style>
