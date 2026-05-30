<script lang="ts">
  // Video control panel (NavRail "Video" tab) — source selection, settings and a
  // live preview. The preview is just one sink of the shared video router; the
  // dock widget, floating window and map-swap view (later steps) bind the same
  // stream.
  import { t } from 'svelte-i18n';
  import {
    videoState,
    videoStream,
    enumerateVideoDevices,
    toggleVideo,
    setVideoDevice,
    setVideoResolution,
    setVideoMirror,
    type VideoResolution,
  } from '$lib/stores/video';

  let videoEl = $state<HTMLVideoElement | null>(null);

  // Bind the shared MediaStream to the preview element.
  $effect(() => {
    if (videoEl) videoEl.srcObject = $videoStream;
  });

  // Populate the device list when the panel opens (labels fill in after the
  // first permission grant / start).
  $effect(() => {
    void enumerateVideoDevices();
  });

  // Measured (real) frame rate via requestVideoFrameCallback — surfaces the
  // actual delivered fps, which can differ a lot from the negotiated setting.
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

<div class="video-panel">
  <h2 class="title">{$t('video.title')}</h2>

  <button
    class="enable-btn"
    class:on={$videoState.enabled}
    onclick={toggleVideo}
  >
    {$videoState.enabled ? $t('video.stop') : $t('video.start')}
  </button>

  <div class="preview" style="aspect-ratio: {$videoState.aspect};">
    <!-- svelte-ignore a11y_media_has_caption -->
    <video
      bind:this={videoEl}
      autoplay
      muted
      playsinline
      class:mirror={$videoState.mirror}
      class:hidden={$videoState.status !== 'live'}
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

  <label class="field row">
    <input
      type="checkbox"
      checked={$videoState.mirror}
      onchange={(e) => setVideoMirror((e.currentTarget as HTMLInputElement).checked)}
    />
    <span class="label">{$t('video.mirror')}</span>
  </label>

  {#if $videoState.devices.length === 0}
    <p class="hint">{$t('video.noDevices')}</p>
  {/if}
</div>

<style>
  .video-panel {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 4px 2px;
  }
  .title {
    font-size: 15px;
    font-weight: 600;
    color: #37a8db;
    margin: 0;
  }
  .enable-btn {
    padding: 8px 12px;
    border-radius: 6px;
    border: 1px solid rgba(55, 168, 219, 0.5);
    background: rgba(55, 168, 219, 0.12);
    color: #cfe8f5;
    font-weight: 600;
    cursor: pointer;
    transition: background-color 0.15s;
  }
  .enable-btn:hover {
    background: rgba(55, 168, 219, 0.22);
  }
  .enable-btn.on {
    background: rgba(231, 76, 60, 0.15);
    border-color: rgba(231, 76, 60, 0.6);
    color: #f3b3ac;
  }
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
  .preview video {
    width: 100%;
    height: 100%;
    object-fit: contain;
    display: block;
  }
  .preview video.mirror {
    transform: scaleX(-1);
  }
  .preview video.hidden {
    visibility: hidden;
  }
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
    margin-top: -4px;
    letter-spacing: 0.02em;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .field.row {
    flex-direction: row;
    align-items: center;
    gap: 8px;
  }
  .label {
    font-size: 12px;
    color: #aaa;
  }
  select {
    background: rgba(20, 20, 20, 0.9);
    color: #e0e0e0;
    border: 1px solid rgba(255, 255, 255, 0.15);
    border-radius: 5px;
    padding: 6px 8px;
    font-size: 13px;
  }
  .hint {
    font-size: 11px;
    color: #777;
    margin: 0;
  }
</style>
