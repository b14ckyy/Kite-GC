<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- UpdateDialog.svelte — one-shot "new version available" prompt driven by the update-check controller.
     Shows the release notes (GitHub's release body, rendered from markdown) above the choices: skip this
     version, remind me later, open the release page, or "Update and Restart" — which downloads + installs
     in place with a progress bar and relaunches (mobile builds link to their store instead). Styled like
     ConfirmDialog. -->
<script lang="ts">
  import { t } from 'svelte-i18n';
  import { marked } from 'marked';
  import DOMPurify from 'dompurify';
  import {
    pendingUpdate, updateKind, installState, currentVersion,
    openReleasePage, openStore, storeUrl, installUpdate, remindLater, skipVersion,
  } from '$lib/controllers/updateCheck';
  import { isAndroid } from '$lib/platform';

  const info = $derived($pendingUpdate);
  const busy = $derived($installState.phase === 'download' || $installState.phase === 'install');

  /** Release body → sanitised HTML. The centred banner block at the top of every release is dropped (it is
   *  the page header, not notes), and GitHub's `> [!NOTE]`-style alerts become classed blockquotes. */
  const notesHtml = $derived.by(() => {
    if (!info?.body) return '';
    const md = info.body.replace(/^\s*<p align="center">[\s\S]*?<\/p>\s*/i, '');
    const html = marked.parse(md, { async: false, gfm: true });
    const alerts = html.replace(
      /<blockquote>\s*<p>\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\]\s*(?:<br\s*\/?>)?\s*/gi,
      (_m, kind: string) => `<blockquote class="alert alert-${kind.toLowerCase()}"><p>`,
    );
    return DOMPurify.sanitize(alerts, { ADD_ATTR: ['target'] });
  });

  /** Primary button label per update kind. */
  const primaryLabel = $derived.by(() => {
    if ($updateKind !== 'mobile') return $t('update.install');
    if (!storeUrl()) return $t('update.openPage');
    return isAndroid ? $t('update.playStore') : $t('update.appStore');
  });

  function onPrimary() {
    if ($updateKind === 'mobile') void openStore();
    else void installUpdate();
  }

  function dismiss() {
    if (!busy) remindLater();
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') dismiss(); // Escape = remind me later (non-destructive)
  }
</script>

{#if info}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="dialog-backdrop" onclick={dismiss} onkeydown={onKeydown}>
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="dialog-box" onclick={(e) => e.stopPropagation()}>
      <div class="dialog-title">{$t('update.title')}</div>
      <div class="dialog-message">
        {$t('update.body')}
        <div class="upd-versions">
          <div><span class="upd-k">{$t('update.latest')}</span> <span class="upd-v">{info.version}</span>{#if info.prerelease} <span class="upd-pre">{$t('update.prerelease')}</span>{/if}</div>
          <div><span class="upd-k">{$t('update.current')}</span> <span class="upd-v">{currentVersion}</span></div>
        </div>
      </div>

      {#if notesHtml}
        <div class="upd-notes-head">{$t('update.notes')}</div>
        <div class="upd-notes">{@html notesHtml}</div>
      {/if}

      {#if busy}
        <div class="upd-progress">
          <div class="upd-progress-text">
            {#if $installState.phase === 'download'}
              {$t('update.downloading', { values: { pct: $installState.percent ?? 0 } })}
            {:else}
              {$t('update.installing')}
            {/if}
          </div>
          <div class="upd-bar" class:indeterminate={$installState.percent === null || $installState.phase === 'install'}>
            <div class="upd-bar-fill" style:width={`${$installState.percent ?? 100}%`}></div>
          </div>
        </div>
      {:else}
        {#if $installState.phase === 'failed'}
          <div class="upd-error">{$t('update.failed', { values: { error: $installState.error } })}</div>
        {/if}
        {#if $updateKind === 'portable'}
          <div class="upd-hint">{$t('update.portableNote')}</div>
        {/if}
        <div class="dialog-buttons">
          <button class="dialog-btn dialog-btn-cancel" onclick={skipVersion}>{$t('update.skip')}</button>
          <button class="dialog-btn" onclick={remindLater}>{$t('update.later')}</button>
          {#if $updateKind !== 'mobile' || storeUrl()}
            <button class="dialog-btn" onclick={openReleasePage}>{$t('update.openPage')}</button>
          {/if}
          <button class="dialog-btn dialog-btn-primary" onclick={onPrimary}>{primaryLabel}</button>
        </div>
      {/if}
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
    padding: 16px;
  }

  .dialog-box {
    background: #2e2e2e;
    border: 1px solid rgba(55, 168, 219, 0.45);
    border-radius: 8px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
    padding: 20px 24px 16px;
    width: min(640px, 100%);
    max-height: 90vh;
    display: flex;
    flex-direction: column;
    min-height: 0;
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
    margin-bottom: 12px;
  }

  .upd-versions {
    margin-top: 12px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .upd-k { color: #949494; }
  .upd-v { color: #e0e0e0; font-weight: 600; }
  .upd-pre { color: #f5a623; font-size: 11px; }

  /* ── Release notes ─────────────────────────────────────────────────────── */
  .upd-notes-head {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: #949494;
    margin-bottom: 6px;
  }

  .upd-notes {
    flex: 1 1 auto;
    min-height: 0;
    max-height: 45vh;
    overflow-y: auto;
    padding: 10px 14px;
    margin-bottom: 14px;
    border: 1px solid #272727;
    border-radius: 6px;
    background: #262626;
    font-size: 12px;
    line-height: 1.55;
    color: #c8c8c8;
    overflow-wrap: anywhere;
  }
  .upd-notes :global(h1), .upd-notes :global(h2), .upd-notes :global(h3), .upd-notes :global(h4) {
    color: #e0e0e0;
    font-weight: 700;
    line-height: 1.3;
    margin: 14px 0 6px;
  }
  .upd-notes :global(h1) { font-size: 15px; }
  .upd-notes :global(h2) { font-size: 14px; border-bottom: 1px solid #333; padding-bottom: 4px; }
  .upd-notes :global(h3) { font-size: 13px; }
  .upd-notes :global(h4) { font-size: 12px; }
  .upd-notes :global(> :first-child) { margin-top: 0; }
  .upd-notes :global(p) { margin: 0 0 8px; }
  .upd-notes :global(ul), .upd-notes :global(ol) { margin: 0 0 8px; padding-left: 20px; }
  .upd-notes :global(li) { margin: 2px 0; }
  .upd-notes :global(a) { color: #37a8db; text-decoration: none; }
  .upd-notes :global(a:hover) { text-decoration: underline; }
  .upd-notes :global(code) {
    font-family: Consolas, 'Courier New', monospace;
    font-size: 11px;
    background: #1f1f1f;
    border-radius: 3px;
    padding: 1px 4px;
  }
  .upd-notes :global(pre) {
    background: #1f1f1f;
    border-radius: 4px;
    padding: 8px 10px;
    overflow-x: auto;
  }
  .upd-notes :global(pre code) { background: none; padding: 0; }
  .upd-notes :global(table) { border-collapse: collapse; margin: 0 0 8px; }
  .upd-notes :global(th), .upd-notes :global(td) { border: 1px solid #3a3a3a; padding: 3px 8px; text-align: left; }
  .upd-notes :global(th) { color: #e0e0e0; background: #2a2a2a; }
  .upd-notes :global(hr) { border: 0; border-top: 1px solid #333; margin: 10px 0; }
  .upd-notes :global(img) { max-width: 100%; }
  .upd-notes :global(blockquote) {
    margin: 0 0 8px;
    padding: 6px 12px;
    border-left: 3px solid #555;
    background: #2a2a2a;
    color: #bbb;
  }
  .upd-notes :global(blockquote p:last-child) { margin-bottom: 0; }
  /* GitHub alert blockquotes (> [!IMPORTANT] …) — colour by kind, GitHub's palette on our dark ground. */
  .upd-notes :global(blockquote.alert-note) { border-left-color: #37a8db; }
  .upd-notes :global(blockquote.alert-tip) { border-left-color: #59aa29; }
  .upd-notes :global(blockquote.alert-important) { border-left-color: #a371f7; }
  .upd-notes :global(blockquote.alert-warning) { border-left-color: #f5a623; }
  .upd-notes :global(blockquote.alert-caution) { border-left-color: #d40000; }

  /* ── Install progress / outcome ────────────────────────────────────────── */
  .upd-progress {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 4px 0 6px;
  }
  .upd-progress-text { font-size: 12px; color: #e0e0e0; }
  .upd-bar {
    position: relative;
    height: 6px;
    border-radius: 3px;
    background: #1f1f1f;
    overflow: hidden;
  }
  .upd-bar-fill {
    height: 100%;
    background: #37a8db;
    border-radius: 3px;
    transition: width 0.15s linear;
  }
  .upd-bar.indeterminate .upd-bar-fill {
    width: 35% !important;
    animation: upd-slide 1.2s ease-in-out infinite;
  }
  @keyframes upd-slide {
    0% { transform: translateX(-100%); }
    100% { transform: translateX(300%); }
  }
  @media (prefers-reduced-motion: reduce) {
    .upd-bar.indeterminate .upd-bar-fill { animation: none; width: 100% !important; }
  }

  .upd-error {
    font-size: 12px;
    color: #ff6b6b;
    margin-bottom: 10px;
    overflow-wrap: anywhere;
  }
  .upd-hint {
    font-size: 11px;
    color: #949494;
    margin-bottom: 10px;
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
  .dialog-btn:hover { background: #505050; }
  .dialog-btn-cancel { color: #999; }

  .dialog-btn-primary {
    background: #1a6b94;
    border-color: #2590c8;
    color: #fff;
  }
  .dialog-btn-primary:hover { background: #237fae; }
</style>
