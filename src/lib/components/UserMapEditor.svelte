<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // Form of one user online map (Settings → Custom Online Provider → Add / Edit): name, URL template, max zoom,
  // attribution, plus a Test that loads one tile through the normal tile loader. Validation runs on
  // Save / Test; the template rules live in helpers/tileTemplate.ts.
  import { onMount } from 'svelte';
  import { isMobile } from '$lib/platform';
  import { t } from 'svelte-i18n';
  import { USER_TILE_SIZES, normalizeTileSize, type UserMapSpec, type UserTileSize } from '$lib/config/mapProviders';
  import { TILE_PLACEHOLDERS, validateTileTemplate, type TemplateProblem } from '$lib/helpers/tileTemplate';
  import { testUserMapTemplate, type UserMapTestResult } from '$lib/stores/userMaps';
  import NumberStepper from '$lib/components/NumberStepper.svelte';
  import Button from '$lib/components/panel/Button.svelte';

  let {
    initial = null,
    takenNames = [],
    onSave,
    onCancel,
  }: {
    /** The map to edit; null = a new one. */
    initial?: UserMapSpec | null;
    /** Names of the OTHER user maps — a name may be used once (case-insensitive). */
    takenNames?: string[];
    onSave: (spec: Omit<UserMapSpec, 'id'> & { id?: string }) => void;
    onCancel: () => void;
  } = $props();

  // svelte-ignore state_referenced_locally
  const start = {
    name: initial?.name ?? '',
    url: initial?.url ?? '',
    maxZoom: initial?.maxZoom ?? 19,
    attribution: initial?.attribution ?? '',
    tileSize: initial?.tileSize ?? (256 as UserTileSize),
  };
  let name = $state(start.name);
  let url = $state(start.url);
  let maxZoom = $state(start.maxZoom);
  let attribution = $state(start.attribution);
  let tileSize = $state<UserTileSize>(start.tileSize);
  /** Set when a Test switched the tile size to the one it measured (shown until the next edit). */
  let sizeAdjusted = $state<UserTileSize | null>(null);
  /** Anything typed since the editor opened — Escape then must not throw it away. */
  const dirty = $derived(
    name !== start.name || url !== start.url || maxZoom !== start.maxZoom || attribution !== start.attribution
      || tileSize !== start.tileSize,
  );

  /** Problems are shown once Save (all fields) or Test (the URL only) was pressed, then follow the typing. */
  let triedSave = $state(false);
  let triedUrl = $state(false);
  const nameProblem = $derived.by<'missing' | 'taken' | null>(() => {
    const n = name.trim().toLowerCase();
    if (n === '') return 'missing';
    return takenNames.some((o) => o.trim().toLowerCase() === n) ? 'taken' : null;
  });
  const urlProblem = $derived<TemplateProblem | null>(validateTileTemplate(url));

  /** Unsaved input? The dialog then ignores Escape (wherever the focus sits) instead of leaving. */
  export function isDirty(): boolean {
    return dirty;
  }

  // Opening the editor puts the cursor into the name field — not on a phone / tablet, where that
  // would pop up the on-screen keyboard over the form.
  let nameEl = $state<HTMLInputElement | null>(null);
  onMount(() => {
    if (!isMobile) nameEl?.focus();
  });

  let testing = $state(false);
  let testResult = $state<UserMapTestResult | null>(null);
  // A result belongs to the template and zoom it was made with.
  $effect(() => {
    void url;
    void maxZoom;
    testResult = null;
    sizeAdjusted = null;
  });

  function onTileSizeChange(e: Event) {
    tileSize = normalizeTileSize(Number((e.target as HTMLSelectElement).value));
    sizeAdjusted = null;
  }

  function zoomClamped(): number {
    return Math.max(1, Math.min(22, Math.round(maxZoom) || 19));
  }

  function save() {
    triedSave = true;
    if (nameProblem || urlProblem) return;
    onSave({ id: initial?.id, name: name.trim(), url: url.trim(), maxZoom: zoomClamped(), attribution: attribution.trim(), tileSize });
  }

  async function test() {
    triedUrl = true;
    if (urlProblem) return;
    testing = true;
    const tested = url;
    const zoom = zoomClamped();
    const sizeBefore = tileSize;
    try {
      const r = await testUserMapTemplate(tested, zoom);
      // Edited while it ran: the answer describes a template that is no longer in the field.
      if (url !== tested || zoomClamped() !== zoom) return;
      testResult = r;
      // The measured tile decides the tile size: a square tile of another size switches the select
      // (nearest of 128/256/512) and says so. Nothing is stored until Save; the select can be set back.
      // A size picked by hand while the test ran is left alone.
      if (r.ok && r.width === r.height && tileSize === sizeBefore) {
        const measured = normalizeTileSize(r.width);
        if (measured !== tileSize) {
          tileSize = measured;
          sizeAdjusted = measured;
        }
      }
    } finally {
      testing = false;
    }
  }
</script>

<div class="um-form">
  <label class="um-field">
    <span class="um-label">{$t('settings.userMaps.name')}</span>
    <input class="um-input" type="text" bind:this={nameEl} bind:value={name} />
  </label>
  {#if triedSave && nameProblem === 'missing'}<span class="um-err">{$t('settings.userMaps.errName')}</span>{/if}
  {#if triedSave && nameProblem === 'taken'}<span class="um-err">{$t('settings.userMaps.errNameTaken')}</span>{/if}

  <label class="um-field">
    <span class="um-label">{$t('settings.userMaps.url')}</span>
    <input class="um-input um-url" type="text" spellcheck="false" autocomplete="off" placeholder="https://…/{'{z}'}/{'{x}'}/{'{y}'}.png" bind:value={url} />
  </label>
  {#if (triedSave || triedUrl) && urlProblem}
    <span class="um-err">
      {urlProblem.kind === 'url'
        ? $t('settings.userMaps.errUrl')
        : urlProblem.kind === 'placeholders'
          ? $t('settings.userMaps.errPlaceholders')
          : $t('settings.userMaps.errUnknown', {
              values: { name: urlProblem.placeholder, allowed: TILE_PLACEHOLDERS.map((p) => `{${p}}`).join(' ') },
            })}
    </span>
  {/if}
  <!-- Placeholder tokens are code, not text: shown literally, explained via $t. -->
  <div class="um-help">
    <span><code>{'{z}'} {'{x}'} {'{y}'}</code> {$t('settings.userMaps.phXyz')}</span>
    <span><code>{'{-y}'}</code> {$t('settings.userMaps.phTms')}</span>
    <span><code>{'{s}'}</code> {$t('settings.userMaps.phSub')}</span>
    <span><code>{'{r}'}</code> {$t('settings.userMaps.phRetina')}</span>
    <span>{$t('settings.userMaps.keyHint')}</span>
  </div>

  <div class="um-field um-row">
    <span class="um-label">{$t('settings.userMaps.maxZoom')}</span>
    <NumberStepper bind:value={maxZoom} min={1} max={22} step={1} decimals={0} />
  </div>
  <span class="um-hint">{$t('settings.userMaps.maxZoomHint')}</span>

  <div class="um-field um-row">
    <label class="um-label" for="um-tile-size">{$t('settings.userMaps.tileSize')}</label>
    <select id="um-tile-size" class="um-input" value={tileSize} onchange={onTileSizeChange}>
      {#each USER_TILE_SIZES as s (s)}<option value={s}>{s} px</option>{/each}
    </select>
  </div>
  <span class="um-hint">{$t('settings.userMaps.tileSizeHint')}</span>

  <label class="um-field">
    <span class="um-label">{$t('settings.userMaps.attribution')}</span>
    <input class="um-input" type="text" placeholder={$t('settings.userMaps.attributionHint')} bind:value={attribution} />
  </label>

  <div class="um-test">
    <Button variant="standard" size="sm" icon="refresh" disabled={testing} onclick={test}>
      {testing ? $t('settings.userMaps.testing') : $t('settings.userMaps.test')}
    </Button>
    {#if testResult}
      {#if testResult.ok}
        <span class="um-ok">{$t('settings.userMaps.testOk', { values: { tile: `${testResult.z}/${testResult.x}/${testResult.y}`, w: testResult.width, h: testResult.height } })}</span>
        {#if sizeAdjusted}<span class="um-ok">{$t('settings.userMaps.tileSizeDetected', { values: { size: sizeAdjusted } })}</span>{/if}
      {:else if 'notImage' in testResult}
        <span class="um-err">{$t('settings.userMaps.testNotImage', { values: { tile: `${testResult.z}/${testResult.x}/${testResult.y}` } })}</span>
      {:else}
        <span class="um-err">{$t('settings.userMaps.testFailed', { values: { tile: `${testResult.z}/${testResult.x}/${testResult.y}`, error: testResult.error } })}</span>
      {/if}
    {/if}
  </div>

  <div class="um-buttons">
    <Button variant="standard" size="sm" onclick={onCancel}>{$t('settings.userMaps.cancel')}</Button>
    <Button variant="data" size="sm" icon="save" onclick={save}>{$t('settings.userMaps.save')}</Button>
  </div>
</div>

<style>
  .um-form { display: flex; flex-direction: column; gap: 6px; min-height: 0; }
  .um-field { display: flex; flex-direction: column; gap: 3px; }
  .um-row { flex-direction: row; align-items: center; justify-content: space-between; }
  .um-label { font-size: 12px; color: #e0e0e0; }
  .um-input {
    height: 28px;
    padding: 0 8px;
    background: #434343;
    border: 1px solid #555;
    border-radius: 4px;
    color: #e0e0e0;
    font-size: 12px;
    min-width: 0;
  }
  .um-url { font-family: Consolas, 'Courier New', monospace; }
  .um-help { display: flex; flex-direction: column; gap: 1px; font-size: 11px; color: #949494; }
  .um-help code { color: #c8c8c8; font-family: Consolas, 'Courier New', monospace; }
  .um-hint { font-size: 11px; color: #949494; margin-top: -3px; }
  .um-test { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; padding-top: 2px; }
  .um-ok { font-size: 11px; color: #59aa29; }
  .um-err { font-size: 11px; color: #d40000; word-break: break-word; }
  .um-buttons { display: flex; justify-content: flex-end; gap: 8px; padding-top: 4px; }
</style>
