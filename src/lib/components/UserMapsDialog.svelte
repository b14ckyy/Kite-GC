<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // Settings → Custom Online Provider: the list of the user's online tile sources (Add / Edit / Delete) and,
  // in place of the list, the editor of one (UserMapEditor). A modal like LogFilePicker, so the
  // crowded settings menu keeps a single row for the feature. A row shows the host only — the path
  // and query may hold the user's API key.
  import { tick } from 'svelte';
  import { t } from 'svelte-i18n';
  import { settings } from '$lib/stores/settings';
  import { saveUserMap, deleteUserMap } from '$lib/stores/userMaps';
  import type { UserMapSpec } from '$lib/config/mapProviders';
  import { templateHost } from '$lib/helpers/tileTemplate';
  import Button from '$lib/components/panel/Button.svelte';
  import UserMapEditor from '$lib/components/UserMapEditor.svelte';

  let { onClose }: { onClose: () => void } = $props();

  /** null = the list; 'new' = the editor of a new map; otherwise the map being edited. */
  let editing = $state<UserMapSpec | 'new' | null>(null);
  /** Delete takes two clicks: the first arms the row's button. */
  let deleteArmed = $state<string | null>(null);

  let editorRef = $state<{ isDirty: () => boolean } | null>(null);

  function onKeydown(e: KeyboardEvent) {
    if (e.key !== 'Escape') return;
    // Unsaved input in the editor: Escape does nothing (Cancel discards it on purpose).
    if (editing && editorRef?.isDirty()) return;
    if (editing) closeEditor();
    else onClose();
  }

  function remove(id: string) {
    if (deleteArmed !== id) {
      deleteArmed = id;
      return;
    }
    deleteArmed = null;
    deleteUserMap(id);
    // The pressed button is gone with its row: focus the box again so Escape still closes the list.
    void tick().then(() => box?.focus());
  }

  let box = $state<HTMLDivElement | null>(null);
  $effect(() => {
    box?.focus();
  });

  /** Back to the list. The focused editor field is gone with the editor: focus the box again, or
   *  Escape would no longer reach the backdrop's handler. */
  function closeEditor() {
    editing = null;
    void tick().then(() => box?.focus());
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
<!-- A stray tap outside must not throw away a half-typed map: the backdrop closes the list only. -->
<div class="dialog-backdrop" onclick={() => { if (!editing) onClose(); }} onkeydown={onKeydown}>
  <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
  <div class="dialog-box" role="dialog" aria-modal="true" aria-labelledby="user-maps-title" tabindex="-1" bind:this={box} onclick={(e) => e.stopPropagation()}>
    <div class="dialog-title" id="user-maps-title">
      {editing === 'new'
        ? $t('settings.userMaps.titleNew')
        : editing
          ? $t('settings.userMaps.titleEdit')
          : $t('settings.userMaps.title')}
    </div>
    <div class="dialog-body">
      {#if editing}
        {@const editId = editing === 'new' ? null : editing.id}
        <UserMapEditor
          bind:this={editorRef}
          initial={editing === 'new' ? null : editing}
          takenNames={$settings.userMapProviders.filter((m) => m.id !== editId).map((m) => m.name)}
          onSave={(spec) => { saveUserMap(spec); closeEditor(); }}
          onCancel={closeEditor}
        />
      {:else}
        {#if $settings.userMapProviders.length === 0}
          <div class="dialog-empty">{$t('settings.userMaps.empty')}</div>
        {:else}
          <div class="um-list">
            {#each $settings.userMapProviders as m (m.id)}
              <div class="um-row">
                <div class="um-text">
                  <span class="um-name">{m.name}</span>
                  <span class="um-host">{templateHost(m.url) || '—'}</span>
                </div>
                <Button variant="standard" size="sm" icon="edit" onclick={() => { deleteArmed = null; editing = m; }}>{$t('settings.userMaps.edit')}</Button>
                <Button variant={deleteArmed === m.id ? 'danger' : 'standard'} size="sm" icon="delete" onclick={() => remove(m.id)}>
                  {deleteArmed === m.id ? $t('settings.userMaps.deleteConfirm') : $t('settings.userMaps.delete')}
                </Button>
              </div>
            {/each}
          </div>
        {/if}
        <div class="dialog-buttons">
          <Button variant="standard" size="sm" icon="add" onclick={() => { deleteArmed = null; editing = 'new'; }}>{$t('settings.userMaps.add')}</Button>
          <Button variant="standard" size="sm" onclick={onClose}>{$t('settings.userMaps.close')}</Button>
        </div>
      {/if}
    </div>
  </div>
</div>

<style>
  /* Frame and colours follow LogFilePicker / ConfirmDialog. */
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
    width: min(520px, 100%);
    /* % of the backdrop (the --ui-scale'd panels layer), not vh — see LogFilePicker. */
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

  /* The body scrolls (the editor on a landscape phone), the title stays. */
  .dialog-body { flex: 0 1 auto; min-height: 0; overflow-y: auto; }

  .dialog-empty {
    font-size: 12px;
    color: #949494;
    padding: 4px 0 14px;
  }

  .um-list {
    display: flex;
    flex-direction: column;
    border: 1px solid #272727;
    border-radius: 6px;
    background: #262626;
    margin-bottom: 12px;
  }
  .um-row {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 44px;
    padding: 4px 10px;
    border-bottom: 1px solid #272727;
  }
  .um-row:last-child { border-bottom: none; }
  .um-text { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .um-name, .um-host { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .um-name { font-size: 13px; color: #e0e0e0; }
  .um-host { font-size: 11px; color: #949494; }

  .dialog-buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
