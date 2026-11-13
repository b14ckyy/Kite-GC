<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- MapSearch — "go to a place / coordinates" for the 2D and the 3D map, plus "to my location".
     One text line, a short hit list, a jump — not an address finder. A magnifier button opens the
     line beside it (to its left), with the location button left of the line. Search runs on Enter or
     on the magnifier while the line is open — never while typing (Nominatim's usage policy).
     Coordinates are recognised locally and work offline; see controllers/mapSearch.ts.
       variant 'toolbar' — desktop / tablet title bar: toolbar controls, the hit list drops below.
       variant 'phone'   — the phone's map control column: glass buttons, the hit list rises above
                           (the line sits near the bottom edge). The parent places the root. -->
<script lang="ts">
  import { tick } from 'svelte';
  import { t } from 'svelte-i18n';
  import { runMapSearch, jumpToHit, goToMyLocation, type PlaceHit } from '$lib/controllers/mapSearch';
  import { isMobile } from '$lib/platform';

  let { variant }: { variant: 'toolbar' | 'phone' } = $props();

  type Status = 'idle' | 'searching' | 'hits' | 'none' | 'offline' | 'refused' | 'failed' | 'locating' | 'locationFailed';

  let open = $state(false);
  let text = $state('');
  let status = $state<Status>('idle');
  let hits = $state<PlaceHit[]>([]);
  let rootEl = $state<HTMLDivElement | null>(null);
  let inputEl = $state<HTMLInputElement | null>(null);
  /** Bumped by every search, locate and close: an answer that arrives later belongs to nothing. */
  let seq = 0;

  const STATUS_KEYS: Partial<Record<Status, string>> = {
    searching: 'mapSearch.searching',
    none: 'mapSearch.noHits',
    offline: 'mapSearch.offline',
    refused: 'mapSearch.refused',
    failed: 'mapSearch.failed',
    locating: 'mapSearch.locating',
    locationFailed: 'mapSearch.locationFailed',
  };

  /** Phone: an open side panel lies over the map layer this line lives in, so a line that reaches under
   *  it loses its left end (the location button, the start of the text). Measure the panel and cap the
   *  line to the free strip between it and the magnifier; without an open panel the page's cap applies. */
  function fitBesidePanel() {
    if (variant !== 'phone' || !rootEl) return;
    const panelRight = document.querySelector('.ps')?.getBoundingClientRect().right ?? 0;
    const free = rootEl.getBoundingClientRect().right - 38 - 6 - panelRight - 8;
    if (panelRight > 0 && free > 0) rootEl.style.setProperty('--ms-line-max', `${Math.max(140, Math.round(free))}px`);
    else rootEl.style.removeProperty('--ms-line-max');
  }

  async function openLine() {
    fitBesidePanel();
    open = true;
    status = 'idle';
    // With a touch screen the focus waits for a tap into the line: focusing it here would raise the soft
    // keyboard every time, also for someone who only wants the location button.
    if (isMobile) return;
    await tick();
    inputEl?.focus();
    inputEl?.select();
  }

  function close() {
    seq++;
    open = false;
    status = 'idle';
    hits = [];
  }

  async function search() {
    if (!text.trim()) return;
    const mine = ++seq;
    status = 'searching';
    hits = [];
    let r: Awaited<ReturnType<typeof runMapSearch>>;
    try {
      r = await runMapSearch(text, () => mine === seq);
    } catch (e) {
      // Nothing in there should throw; if it does, the line must not sit on "Searching…" for ever.
      console.warn('[map-search] search failed', e instanceof Error ? e.name : 'error');
      if (mine === seq) status = 'failed';
      return;
    }
    if (mine !== seq) return; // closed or superseded meanwhile
    if (r.kind === 'jumped') { close(); return; }
    if (r.kind === 'hits') { hits = r.hits; status = 'hits'; return; }
    status = r.kind;
  }

  function pick(hit: PlaceHit) {
    jumpToHit(hit);
    close();
  }

  async function locate() {
    const mine = ++seq;
    status = 'locating';
    hits = [];
    const r = await goToMyLocation(() => mine === seq);
    if (mine !== seq) return;
    if (r.ok) { close(); return; }
    // The toast banner says it; only when the "System Messages" filter hid it does the line.
    status = r.noticeShown ? 'idle' : 'locationFailed';
  }

  /** Magnifier: opens the line; while it is open, it searches (or closes an empty line). */
  function onMagnifier() {
    if (!open) void openLine();
    else if (text.trim()) void search();
    else close();
  }

  function onKeydown(e: KeyboardEvent) {
    // The line owns the keyboard while it has the focus: no app shortcut may fire from here.
    e.stopPropagation();
    // An Enter that confirms an IME candidate (Chinese, Japanese, Korean input) is not a search.
    if (e.isComposing || e.keyCode === 229) return;
    if (e.key === 'Enter') {
      e.preventDefault();
      // On a phone or tablet the soft keyboard covers most of a landscape screen, the hit list included:
      // let it go once the search is sent.
      if (isMobile) inputEl?.blur();
      void search();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      close();
    }
  }

  // Tapping / clicking anywhere else closes the line; so does Escape while the focus is on a hit
  // (the input handles its own Escape and never lets it through).
  $effect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (rootEl && e.target instanceof Node && !rootEl.contains(e.target)) close();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && rootEl && e.target instanceof Node && rootEl.contains(e.target)) close();
    };
    window.addEventListener('pointerdown', onDown, true);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('pointerdown', onDown, true);
      window.removeEventListener('keydown', onKey);
    };
  });

  const magnifierLabel = $derived(open && text.trim() ? $t('mapSearch.search') : open ? $t('mapSearch.close') : $t('mapSearch.open'));
  const statusText = $derived(STATUS_KEYS[status] ? $t(STATUS_KEYS[status] as string) : '');
</script>

<div class="map-search v-{variant}" class:open bind:this={rootEl}>
  {#if open}
    <div class="ms-panel">
      <div class="ms-line">
        <button class="ms-btn" onclick={locate} title={$t('mapSearch.myLocation')} aria-label={$t('mapSearch.myLocation')}>
          <!-- location target: ring, centre dot, four ticks -->
          <svg viewBox="1 1 22 22" aria-hidden="true">
            <circle cx="12" cy="12" r="7" fill="none" stroke="currentColor" stroke-width="2" />
            <circle cx="12" cy="12" r="2.6" fill="currentColor" />
            <path d="M12 2v3M12 19v3M2 12h3M19 12h3" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" />
          </svg>
        </button>
        <input
          bind:this={inputEl}
          bind:value={text}
          class="ms-input"
          type="text"
          inputmode="search"
          enterkeyhint="search"
          autocomplete="off"
          autocapitalize="off"
          spellcheck="false"
          placeholder={$t('mapSearch.placeholder')}
          aria-label={$t('mapSearch.placeholder')}
          onkeydown={onKeydown}
        />
      </div>
      {#if status === 'hits'}
        <ul class="ms-drop ms-hits">
          {#each hits as hit, i (i)}
            <li>
              <!-- The name only: the hit's `kind` is an English OSM tag, not text for the UI. -->
              <button class="ms-hit" onclick={() => pick(hit)} title={hit.name}>
                <span class="ms-hit-name">{hit.name}</span>
              </button>
            </li>
          {/each}
          <li class="ms-credit">{$t('mapSearch.attribution')}</li>
        </ul>
      {:else if statusText}
        <div class="ms-drop ms-status" class:busy={status === 'searching' || status === 'locating'} role="status">{statusText}</div>
      {/if}
    </div>
  {/if}
  <button class="ms-btn ms-toggle" class:active={open} onclick={onMagnifier} title={magnifierLabel} aria-label={magnifierLabel}>
    <!-- magnifier -->
    <svg viewBox="2.4 2.4 20.4 20.4" aria-hidden="true">
      <circle cx="10.5" cy="10.5" r="6.5" fill="none" stroke="currentColor" stroke-width="2.2" />
      <path d="M15.3 15.3 21 21" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" />
    </svg>
  </button>
</div>

<style>
  .map-search {
    position: relative;
    display: flex;
    align-items: center;
    flex: none;
  }

  /* Opens to the left of the magnifier, over whatever lies there (a transient popover, so the
     title bar's collapse measurement and the map column never re-flow for it). */
  .ms-panel {
    position: absolute;
    right: calc(100% + 6px);
    display: flex;
    gap: 6px;
    z-index: 5;
  }
  .ms-line {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .ms-input {
    flex: 1;
    min-width: 0;
    box-sizing: border-box;
    color: #e0e0e0;
    font-family: inherit;
    outline: none;
  }
  .ms-input::placeholder { color: #949494; }

  .ms-btn {
    flex: none;
    box-sizing: border-box;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    cursor: pointer;
  }
  .ms-btn svg {
    display: block;
    flex: none;
  }

  .ms-drop {
    box-sizing: border-box;
    margin: 0;
    background: rgba(46, 46, 46, 0.96);
    border: 1px solid rgba(55, 168, 219, 0.45);
    border-radius: 6px;
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.45);
    color: #cfcfcf;
    font-size: 12px;
  }
  .ms-status {
    padding: 8px 10px;
    line-height: 1.35;
  }
  .ms-status.busy { color: #949494; }
  .ms-hits {
    list-style: none;
    padding: 4px;
    overflow-y: auto;
  }
  .ms-hit {
    display: flex;
    align-items: baseline;
    gap: 8px;
    width: 100%;
    box-sizing: border-box;
    padding: 7px 8px;
    background: transparent;
    border: 0;
    border-radius: 4px;
    color: #e0e0e0;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .ms-hit:hover,
  .ms-hit:focus-visible {
    background: rgba(55, 168, 219, 0.18);
    outline: none;
  }
  .ms-hit-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
  }
  .ms-credit {
    padding: 4px 8px 2px;
    color: #949494;
    font-size: 10px;
    text-align: right;
  }

  /* ── Title bar (desktop / tablet): the toolbar control look, 28 px; the list hangs below the bar. */
  .v-toolbar .ms-panel {
    top: 0;
    flex-direction: column;
    width: min(320px, 55vw);
  }
  /* The bar's own ground around the line, so the indicators it covers do not peek through. */
  .v-toolbar .ms-line {
    background: #2e2e2e;
    box-shadow: 0 0 0 6px #2e2e2e;
  }
  .v-toolbar .ms-btn {
    width: 28px;
    height: 28px;
    background: #434343;
    border: 1px solid #555;
    border-radius: 4px;
    color: #cfcfcf;
    transition: background-color 0.2s, color 0.2s, border-color 0.2s;
  }
  .v-toolbar .ms-btn:hover {
    background: rgba(55, 168, 219, 0.18);
    color: #e0e0e0;
  }
  .v-toolbar .ms-toggle.active {
    background: rgba(55, 168, 219, 0.22);
    border-color: #37a8db;
    color: #37a8db;
  }
  /* Icons fill 75 % of the button, like the map buttons (--map-btn-icon). */
  .v-toolbar .ms-btn svg {
    width: 21px;
    height: 21px;
  }
  .v-toolbar .ms-input {
    height: 28px;
    padding: 0 8px;
    background: #434343;
    border: 1px solid #555;
    border-radius: 4px;
    font-size: 12px;
  }
  .v-toolbar .ms-input:focus { border-color: #37a8db; }
  /* Clear of the bar's lower edge (its accent border included). */
  .v-toolbar .ms-drop {
    margin-top: 12px;
    max-height: min(60vh, 320px);
  }

  /* ── Phone: the map's glass buttons (38 px, as the corner controls); the list rises above. ── */
  .v-phone .ms-panel {
    bottom: 0;
    flex-direction: column-reverse;
    width: min(340px, var(--ms-line-max, 60vw));
  }
  .v-phone .ms-btn {
    width: 38px;
    height: 38px;
    background: rgba(46, 46, 46, 0.9);
    border: 2px solid rgba(55, 168, 219, 0.5);
    border-radius: 6px;
    color: #37a8db;
    backdrop-filter: var(--glass-blur, blur(6px));
    -webkit-backdrop-filter: var(--glass-blur, blur(6px));
    transition: background 0.2s, border-color 0.2s;
  }
  .v-phone .ms-btn:hover,
  .v-phone .ms-toggle.active {
    background: var(--btn-active-bg);
    border-color: #37a8db;
  }
  .v-phone .ms-btn svg {
    width: var(--map-btn-icon, 28.5px);
    height: var(--map-btn-icon, 28.5px);
  }
  .v-phone .ms-input {
    height: 38px;
    padding: 0 10px;
    background: rgba(46, 46, 46, 0.92);
    border: 2px solid rgba(55, 168, 219, 0.5);
    border-radius: 6px;
    font-size: 15px; /* ≥ 15 px: no zoom-on-focus on iOS, readable at arm's length */
  }
  .v-phone .ms-input:focus { border-color: #37a8db; }
  .v-phone .ms-drop {
    max-height: var(--ms-drop-max, 50vh);
    font-size: 13px;
  }
  .v-phone .ms-hit { padding: 9px 8px; } /* finger-sized rows */
</style>
