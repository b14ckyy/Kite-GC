<script lang="ts">
  // Video control panel on the panel framework (docs/dev/PANEL_FRAMEWORK.md): a `compact`
  // PanelShell. Header = Start/Stop; content = preview + source/resolution/mirror settings;
  // footer = Floating Window (on/off Toggle) + Video Window/detach (button). Parallel build
  // alongside the legacy VideoPanel; logic identical — only the chrome moves onto PanelShell.
  // Kept deliberately simple but extensible (more sinks/sources can slot into the content field).
  import { t } from 'svelte-i18n';
  import {
    videoState,
    videoStream,
    enumerateVideoDevices,
    toggleVideo,
    setVideoDevice,
    setVideoResolution,
    setVideoMirror,
    toggleFloating,
    enterPiP,
    pipSupported,
    type VideoResolution,
  } from '$lib/stores/video';
  import PanelShell from '$lib/components/panel/PanelShell.svelte';
  import Button from '$lib/components/panel/Button.svelte';
  import Toggle from '$lib/components/panel/Toggle.svelte';

  let videoEl = $state<HTMLVideoElement | null>(null);

  // Bind the shared MediaStream to the preview element.
  $effect(() => {
    if (videoEl) videoEl.srcObject = $videoStream;
  });

  // Populate the device list when the panel opens.
  $effect(() => {
    void enumerateVideoDevices();
  });

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
  <Button variant={$videoState.enabled ? 'danger' : 'data'} onclick={toggleVideo}>
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

    <div class="field-row">
      <Toggle checked={$videoState.mirror} onchange={(c) => setVideoMirror(c)} id="vp-mirror" />
      <span class="label">{$t('video.mirror')}</span>
    </div>

    {#if $videoState.devices.length === 0}
      <p class="hint">{$t('video.noDevices')}</p>
    {/if}
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
  .hint { font-size: 11px; color: #777; margin: 0; }

  .vp-footer { display: flex; align-items: center; justify-content: space-between; gap: 8px; width: 100%; }
</style>
