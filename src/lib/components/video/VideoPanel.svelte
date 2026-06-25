<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // Video control panel on the panel framework (docs/active/PANEL_FRAMEWORK.md): a `compact`
  // PanelShell. Header = Start/Stop; content = preview + source/resolution/mirror settings;
  // footer = Floating Window (mode button) + Video Window/detach (button).
  // Kept deliberately simple but extensible (more sinks/sources can slot into the content field).
  import { t } from 'svelte-i18n';
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import {
    videoState,
    videoStream,
    bindVideoEl,
    reportVideoSize,
    enumerateVideoDevices,
    toggleVideo,
    setVideoDevice,
    setVideoResolution,
    setVideoMirror,
    setVideoKind,
    setRtspUrl,
    setRtspTransport,
    toggleFloating,
    enterPiP,
    pipSupported,
    type VideoResolution,
    type VideoKind,
    type RtspTransport,
  } from '$lib/stores/video';
  import PanelShell from '$lib/components/panel/PanelShell.svelte';
  import Button from '$lib/components/panel/Button.svelte';
  import Toggle from '$lib/components/panel/Toggle.svelte';

  let videoEl = $state<HTMLVideoElement | null>(null);

  // Bind the preview element to the shared MediaStream (camera or rtsp via captureStream).
  $effect(() => {
    bindVideoEl(videoEl, $videoStream);
  });

  // Populate the device list when the panel opens.
  $effect(() => {
    void enumerateVideoDevices();
  });

  // ── go2rtc (RTSP→WebRTC engine dependency) ───────────────────────────
  let engineVer = $state<string | null>(null);
  let engineChecked = $state(false);
  let downloading = $state(false);
  let dlPct = $state(0);
  let dlMsg = $state('');

  async function checkEngine(): Promise<void> {
    try {
      engineVer = await invoke<string | null>('video_go2rtc_status');
    } catch {
      engineVer = null;
    }
    engineChecked = true;
  }

  async function downloadEngine(): Promise<void> {
    downloading = true;
    dlPct = 0;
    dlMsg = '';
    try {
      await invoke('video_go2rtc_download');
      await checkEngine();
    } catch (e) {
      dlMsg = e instanceof Error ? e.message : String(e);
    } finally {
      downloading = false;
    }
  }

  onMount(() => {
    void checkEngine();
    let unlisten: UnlistenFn | undefined;
    void listen<{ pct: number; msg: string }>('go2rtc-download-progress', (e) => {
      dlPct = e.payload.pct;
      dlMsg = e.payload.msg;
    }).then((u) => (unlisten = u));
    return () => unlisten?.();
  });

  const KINDS: VideoKind[] = ['camera', 'rtsp'];
  const TRANSPORTS: RtspTransport[] = ['auto', 'tcp', 'udp'];

  // Measured (real) frame rate via requestVideoFrameCallback.
  let measuredFps = $state(0);
  $effect(() => {
    const el = videoEl as (HTMLVideoElement & {
      requestVideoFrameCallback?: (cb: (now: number) => void) => number;
      cancelVideoFrameCallback?: (h: number) => void;
    }) | null;
    if (!el || $videoState.status !== 'live' || !el.requestVideoFrameCallback) {
      measuredFps = 0;
      return;
    }
    let frames = 0;
    let last = performance.now();
    let handle = 0;
    const tick = (now: number) => {
      frames++;
      const dt = now - last;
      if (dt >= 1000) {
        measuredFps = (frames * 1000) / dt;
        frames = 0;
        last = now;
      }
      handle = el.requestVideoFrameCallback!(tick);
    };
    handle = el.requestVideoFrameCallback(tick);
    return () => el.cancelVideoFrameCallback?.(handle);
  });

  const RESOLUTIONS: VideoResolution[] = ['auto', '720p', '1080p'];
</script>

{#snippet headerActions()}
  <Button
    variant={$videoState.enabled ? 'danger' : 'data'}
    disabled={!$videoState.enabled && $videoState.kind === 'rtsp' && engineChecked && !engineVer}
    onclick={toggleVideo}
  >
    {$videoState.enabled ? $t('video.stop') : $t('video.start')}
  </Button>
{/snippet}

{#snippet body()}
  <div class="vp-body">
    <div class="preview" style="aspect-ratio: {$videoState.aspect};">
      <!-- svelte-ignore a11y_media_has_caption -->
      <video
        bind:this={videoEl}
        autoplay
        muted
        playsinline
        class:mirror={$videoState.mirror}
        class:hidden={$videoState.status !== 'live'}
        onloadedmetadata={(e) => reportVideoSize(e.currentTarget.videoWidth, e.currentTarget.videoHeight)}
        onerror={() => console.error('[video] element error', videoEl?.error?.code, videoEl?.error?.message)}
        onloadeddata={() => console.log('[video] loadeddata, readyState', videoEl?.readyState)}
        onstalled={() => console.warn('[video] stalled')}
        onwaiting={() => console.warn('[video] waiting/buffering')}
      ></video>
      {#if $videoState.status !== 'live'}
        <div class="preview-placeholder">
          {#if $videoState.status === 'starting'}
            {$t('video.starting')}
          {:else if $videoState.status === 'error'}
            ⚠ {$videoState.error}
          {:else}
            {$t('video.off')}
          {/if}
        </div>
      {/if}
    </div>

    {#if $videoState.status === 'live'}
      <div class="info-line">
        {$videoState.width ?? '–'}×{$videoState.height ?? '–'}
        · {measuredFps ? measuredFps.toFixed(0) : '–'}{#if $videoState.frameRate}/{Math.round($videoState.frameRate)}{/if} fps
      </div>
    {/if}

    <label class="field">
      <span class="label">{$t('video.source')}</span>
      <select
        value={$videoState.kind}
        onchange={(e) => setVideoKind((e.currentTarget as HTMLSelectElement).value as VideoKind)}
      >
        {#each KINDS as k}
          <option value={k}>{$t(`video.kind.${k}`)}</option>
        {/each}
      </select>
    </label>

    {#if $videoState.kind === 'camera'}
      <label class="field">
        <span class="label">{$t('video.device')}</span>
        <select
          value={$videoState.deviceId ?? ''}
          onchange={(e) => setVideoDevice((e.currentTarget as HTMLSelectElement).value || null)}
        >
          <option value="">{$t('video.defaultDevice')}</option>
          {#each $videoState.devices as d}
            <option value={d.deviceId}>{d.label}</option>
          {/each}
        </select>
      </label>

      <label class="field">
        <span class="label">{$t('video.resolution')}</span>
        <select
          value={$videoState.resolution}
          onchange={(e) => setVideoResolution((e.currentTarget as HTMLSelectElement).value as VideoResolution)}
        >
          {#each RESOLUTIONS as r}
            <option value={r}>{r === 'auto' ? $t('video.auto') : r}</option>
          {/each}
        </select>
      </label>

      {#if $videoState.devices.length === 0}
        <p class="hint">{$t('video.noDevices')}</p>
      {/if}
    {:else}
      <label class="field">
        <span class="label">{$t('video.rtspUrl')}</span>
        <input
          class="text-input"
          type="text"
          placeholder="rtsp://192.168.1.10:554/live"
          value={$videoState.rtspUrl}
          onchange={(e) => setRtspUrl((e.currentTarget as HTMLInputElement).value)}
        />
      </label>

      <label class="field">
        <span class="label">{$t('video.transport')}</span>
        <select
          value={$videoState.rtspTransport}
          onchange={(e) => setRtspTransport((e.currentTarget as HTMLSelectElement).value as RtspTransport)}
        >
          {#each TRANSPORTS as tp}
            <option value={tp}>{$t(`video.transportOpt.${tp}`)}</option>
          {/each}
        </select>
      </label>

      {#if engineChecked && !engineVer}
        <div class="ffmpeg-box">
          <p class="hint">{$t('video.engineMissing')}</p>
          {#if downloading}
            <div class="dl-row">
              <div class="dl-bar"><div class="dl-fill" style="width:{dlPct}%"></div></div>
              <span class="dl-pct">{dlPct}%</span>
            </div>
            {#if dlMsg}<p class="hint">{dlMsg}</p>{/if}
          {:else}
            <Button variant="data" onclick={downloadEngine}>{$t('video.engineDownload')}</Button>
            {#if dlMsg}<p class="hint err">{dlMsg}</p>{/if}
          {/if}
        </div>
      {:else if $videoState.status === 'live' && $videoState.rtspEngine}
        <p class="hint">{$t(`video.via.${$videoState.rtspEngine}`)}</p>
      {:else if engineVer}
        <p class="hint">{$t('video.engineReady')}</p>
      {/if}
    {/if}

    <div class="field-row">
      <Toggle checked={$videoState.mirror} onchange={(c) => setVideoMirror(c)} id="vp-mirror" />
      <span class="label">{$t('video.mirror')}</span>
    </div>
  </div>
{/snippet}

{#snippet footer()}
  <div class="vp-footer">
    <!-- Floating window: a mode button (active = on) — can be toggled off from here. -->
    <Button variant="mode" active={$videoState.floating} onclick={() => toggleFloating()}>
      {$t('video.floatingWindow')}
    </Button>
    <!-- Detached PiP window: a one-way action (can't be closed from inside the app) → plain button. -->
    {#if pipSupported}
      <Button variant="standard" disabled={$videoState.status !== 'live'} onclick={enterPiP}>
        {$t('video.videoWindow')}
      </Button>
    {/if}
  </div>
{/snippet}

<div class="vpv2">
  <PanelShell variant="compact" title={$t('video.title')} {headerActions} {body} {footer} />
</div>

<style>
  .vp-body { display: flex; flex-direction: column; gap: 12px; }

  .preview {
    width: 100%;
    background: #000;
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 6px;
    overflow: hidden;
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .preview video { width: 100%; height: 100%; object-fit: contain; display: block; }
  .preview video.mirror { transform: scaleX(-1); }
  .preview video.hidden { visibility: hidden; }
  .preview-placeholder {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #888;
    font-size: 12px;
    text-align: center;
    padding: 0 10px;
  }
  .info-line {
    font-size: 11px;
    color: #9ad0e8;
    font-variant-numeric: tabular-nums;
    margin-top: -6px;
    letter-spacing: 0.02em;
  }

  .field { display: flex; flex-direction: column; gap: 4px; }
  .field-row { display: flex; align-items: center; gap: 8px; }
  .label { font-size: 12px; color: #aaa; }
  /* Match the framework form-control height (md button = 28px). */
  .field select {
    height: 28px;
    padding: 0 8px;
    background: #434343;
    color: #e0e0e0;
    border: 1px solid #555;
    border-radius: 4px;
    font-size: 12px;
  }
  .field .text-input {
    height: 28px;
    padding: 0 8px;
    background: #434343;
    color: #e0e0e0;
    border: 1px solid #555;
    border-radius: 4px;
    font-size: 12px;
  }
  .hint { font-size: 11px; color: #777; margin: 0; }
  .hint.err { color: #d40000; }

  .ffmpeg-box {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px;
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 4px;
  }
  .dl-row { display: flex; align-items: center; gap: 8px; }
  .dl-bar {
    flex: 1;
    height: 6px;
    background: #1d1d1d;
    border-radius: 3px;
    overflow: hidden;
  }
  .dl-fill { height: 100%; background: #37a8db; transition: width 0.2s ease; }
  .dl-pct { font-size: 11px; color: #9ad0e8; font-variant-numeric: tabular-nums; min-width: 30px; text-align: right; }

  .vp-footer { display: flex; align-items: center; justify-content: space-between; gap: 8px; width: 100%; }
</style>
