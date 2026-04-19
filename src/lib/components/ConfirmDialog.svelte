<script lang="ts" module>
  import { t } from 'svelte-i18n';

  export interface DialogButton {
    label: string;
    value: string;
    danger?: boolean;
    primary?: boolean;
  }

  export interface DialogOptions {
    title: string;
    message: string;
    buttons?: DialogButton[];
  }
</script>

<script lang="ts">
  import { t as tLocal } from 'svelte-i18n';

  let open = $state(false);
  let title = $state('');
  let message = $state('');
  let buttons = $state<DialogButton[]>([]);
  let resolver: ((value: string | null) => void) | null = null;

  /**
   * Show the dialog and return a promise that resolves to the selected button value
   * or null if cancelled / backdrop-clicked / Escape pressed.
   * For info-only dialogs, pass no buttons — only OK + Cancel are shown.
   */
  export function show(opts: DialogOptions): Promise<string | null> {
    title = opts.title;
    message = opts.message;
    buttons = opts.buttons ?? [];
    open = true;
    return new Promise<string | null>((resolve) => {
      resolver = resolve;
    });
  }

  function close(value: string | null) {
    open = false;
    if (resolver) {
      resolver(value);
      resolver = null;
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') close(null);
  }
</script>

{#if open}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="dialog-backdrop" onclick={() => close(null)} onkeydown={handleKeydown}>
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="dialog-box" onclick={(e) => e.stopPropagation()}>
      {#if title}
        <div class="dialog-title">{title}</div>
      {/if}
      <div class="dialog-message">{message}</div>
      <div class="dialog-buttons">
        {#if buttons.length === 0}
          <!-- Info-only: just an OK button -->
          <button class="dialog-btn dialog-btn-primary" onclick={() => close('ok')}>OK</button>
        {:else}
          <button class="dialog-btn dialog-btn-cancel" onclick={() => close(null)}>{$tLocal('dialog.cancel')}</button>
          {#each buttons as btn}
            <button
              class="dialog-btn"
              class:dialog-btn-danger={btn.danger}
              class:dialog-btn-primary={btn.primary}
              onclick={() => close(btn.value)}
            >{btn.label}</button>
          {/each}
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .dialog-backdrop {
    position: fixed;
    inset: 0;
    z-index: 9999;
    background: rgba(0, 0, 0, 0.55);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .dialog-box {
    background: #2e2e2e;
    border: 1px solid rgba(55, 168, 219, 0.45);
    border-radius: 8px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
    padding: 20px 24px 16px;
    min-width: 340px;
    max-width: 480px;
  }

  .dialog-title {
    font-size: 14px;
    font-weight: 700;
    color: #e0e0e0;
    margin-bottom: 10px;
  }

  .dialog-message {
    font-size: 12px;
    color: #bbb;
    line-height: 1.5;
    white-space: pre-line;
    margin-bottom: 16px;
  }

  .dialog-buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    flex-wrap: wrap;
  }

  .dialog-btn {
    padding: 6px 14px;
    font-size: 12px;
    font-weight: 600;
    border-radius: 4px;
    border: 1px solid #555;
    background: #434343;
    color: #e0e0e0;
    cursor: pointer;
    transition: background 0.15s;
  }

  .dialog-btn:hover {
    background: #505050;
  }

  .dialog-btn-cancel {
    color: #999;
  }

  .dialog-btn-danger {
    background: #8b2020;
    border-color: #a03030;
    color: #fff;
  }

  .dialog-btn-danger:hover {
    background: #a52a2a;
  }

  .dialog-btn-primary {
    background: #1a6b94;
    border-color: #2590c8;
    color: #fff;
  }

  .dialog-btn-primary:hover {
    background: #237fae;
  }
</style>
