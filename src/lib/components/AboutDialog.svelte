<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // About dialog: logo + name + version/commit/build stamp + licence + bundled third-party licences.
  // Opened from the Settings panel header. `open` is bindable; Escape / backdrop / Close dismiss it.
  import { t } from 'svelte-i18n';
  import { openUrl } from '@tauri-apps/plugin-opener';
  import {
    APP_NAME, APP_TAGLINE, APP_VERSION, GIT_COMMIT, BUILD_DATE, COPYRIGHT, LICENSE, REPO_URL,
  } from '$lib/buildInfo';
  import { THIRD_PARTY_LICENSES, SUPPORTED_FIRMWARE } from '$lib/config/thirdPartyLicenses';

  let { open = $bindable(false) }: { open?: boolean } = $props();

  function close() { open = false; }
  function onKeydown(e: KeyboardEvent) { if (e.key === 'Escape') close(); }
  async function openExt(url: string) { try { await openUrl(url); } catch { /* link is shown as text too */ } }
</script>

{#if open}
  <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
  <div class="about-backdrop" onclick={close} onkeydown={onKeydown}>
    <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
    <div class="about-box" onclick={(e) => e.stopPropagation()}>
      <button class="about-x" onclick={close} title={$t('about.close')} aria-label={$t('about.close')}>×</button>

      <div class="about-head">
        <img class="about-logo" src="/logo.svg" alt="" />
        <div class="about-id">
          <div class="about-name">{APP_NAME}</div>
          <div class="about-tagline">{APP_TAGLINE}</div>
        </div>
      </div>

      <div class="about-rows">
        <div class="about-row">
          <span class="about-k">Version</span>
          <span class="about-v">v{APP_VERSION} · <span class="about-mono">{GIT_COMMIT}</span> · {BUILD_DATE}</span>
        </div>
        <div class="about-row">
          <span class="about-k">{$t('about.license')}</span>
          <span class="about-v">{LICENSE} · {COPYRIGHT}</span>
        </div>
        <div class="about-row">
          <span class="about-k">{$t('about.source')}</span>
          <button class="about-link" onclick={() => openExt(REPO_URL)}>{REPO_URL}</button>
        </div>
      </div>

      <div class="about-fw">
        <div class="about-fw-head">{$t('about.supportedFirmware')}</div>
        <div class="about-fw-list">
          {#each SUPPORTED_FIRMWARE as fw}
            <div class="about-fw-row">
              <button class="about-link" onclick={() => openExt(fw.url)}>{fw.name}</button>
              <span class="about-fw-lic">{fw.license}</span>
            </div>
          {/each}
        </div>
      </div>

      <div class="about-tp">
        <div class="about-tp-head">{$t('about.thirdParty')}</div>
        <div class="about-tp-scroll">
          {#each THIRD_PARTY_LICENSES as group}
            <div class="about-tp-group">{group.heading}</div>
            {#each group.items as it}
              <div class="about-tp-row">
                <span class="about-tp-name">{it.name}</span>
                <span class="about-tp-lic">{it.license}</span>
              </div>
            {/each}
          {/each}
        </div>
        <div class="about-tp-note">{$t('about.thirdPartyNote')}</div>
      </div>

      <div class="about-foot">
        <button class="about-close" onclick={close}>{$t('about.close')}</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .about-backdrop {
    position: fixed; inset: 0; z-index: 10000;
    background: rgba(0, 0, 0, 0.55);
    display: flex; align-items: center; justify-content: center;
  }
  .about-box {
    position: relative;
    background: #2e2e2e;
    border: 1px solid rgba(55, 168, 219, 0.45);
    border-radius: 10px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
    padding: 22px 24px 16px;
    width: 460px; max-width: calc(100vw - 32px);
    color: #e0e0e0;
    font-family: 'Segoe UI', Tahoma, sans-serif;
  }
  .about-x {
    position: absolute; top: 10px; right: 12px;
    background: none; border: none; color: #949494;
    font-size: 22px; line-height: 1; cursor: pointer; padding: 2px 6px; border-radius: 4px;
  }
  .about-x:hover { color: #e0e0e0; background: rgba(255, 255, 255, 0.06); }

  .about-head { display: flex; align-items: center; gap: 14px; margin-bottom: 16px; }
  .about-logo { width: 56px; height: 56px; flex: none; }
  .about-name { font-size: 18px; font-weight: 700; color: #fff; }
  .about-tagline { font-size: 12px; color: #949494; margin-top: 2px; }

  .about-rows { display: flex; flex-direction: column; gap: 5px; padding-bottom: 14px; border-bottom: 1px solid #272727; }
  .about-row { display: grid; grid-template-columns: 84px 1fr; align-items: baseline; gap: 10px; font-size: 12px; }
  .about-k { color: #949494; }
  .about-v { color: #e0e0e0; }
  .about-mono { font-family: 'Consolas', monospace; color: #37a8db; }
  .about-link { background: none; border: none; padding: 0; color: #37a8db; cursor: pointer; font: inherit; text-align: left; }
  .about-link:hover { text-decoration: underline; }

  .about-fw { margin-top: 14px; }
  .about-fw-head { font-size: 11px; font-weight: 700; color: #37a8db; text-transform: uppercase; letter-spacing: 0.5px; margin-bottom: 6px; }
  .about-fw-list { display: flex; flex-direction: column; gap: 3px; }
  .about-fw-row { display: grid; grid-template-columns: 1fr auto; gap: 10px; align-items: baseline; font-size: 12px; }
  .about-fw-lic { color: #949494; white-space: nowrap; }

  .about-tp { margin-top: 14px; }
  .about-tp-head { font-size: 11px; font-weight: 700; color: #37a8db; text-transform: uppercase; letter-spacing: 0.5px; margin-bottom: 6px; }
  .about-tp-scroll { max-height: 200px; overflow-y: auto; border: 1px solid #272727; border-radius: 6px; padding: 4px 8px; background: #262626; }
  .about-tp-group { font-size: 10.5px; font-weight: 700; color: #949494; text-transform: uppercase; letter-spacing: 0.4px; margin: 8px 0 2px; }
  .about-tp-group:first-child { margin-top: 2px; }
  .about-tp-row { display: grid; grid-template-columns: 1fr auto; gap: 10px; padding: 2px 0; font-size: 11.5px; }
  .about-tp-name { color: #e0e0e0; }
  .about-tp-lic { color: #949494; white-space: nowrap; }
  .about-tp-note { font-size: 10.5px; color: #6f6f6f; margin-top: 6px; line-height: 1.4; }

  .about-foot { display: flex; justify-content: flex-end; margin-top: 16px; }
  .about-close {
    padding: 6px 16px; font-size: 12px; font-weight: 600; border-radius: 4px;
    border: 1px solid #37a8db; background: #37a8db; color: #fff; cursor: pointer;
  }
  .about-close:hover { background: #2e96c5; }
</style>
