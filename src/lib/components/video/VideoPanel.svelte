<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // Video control panel on the panel framework (docs/active/PANEL_FRAMEWORK.md): a `compact`
  // PanelShell. Header = Start/Stop; content = status/info lines + source/resolution/mirror
  // settings; no footer. No preview surface either: the picture lives in the
  // floating window (docked window on the phone) / the widget / the map swap — a started source
  // appears there, and the window's own toggle parks it (PHONE_VIDEO.md D1 + §10).
  // Kept deliberately simple but extensible (more sources can slot into the content field).
  import { t } from 'svelte-i18n';
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import {
    videoState,
    videoRtcStats,
    videoElFps,
    rtspBufferFrames,
    enumerateVideoDevices,
    toggleVideo,
    setVideoDevice,
    setVideoResolution,
    setCameraFps,
    setVideoMirror,
    setVideoRotate180,
    setUnobstructedFullscreen,
    setDisableHwAccel,
    setVideoKind,
    setRtspUrl,
    setRtspTransport,
    setRtspNativeClient,
    saveRtspConnection,
    updateRtspConnection,
    removeRtspConnection,
    selectRtspConnection,
    isWebrtcAvailable,
    type RtspTransport,
    type VideoResolution,
    type VideoKind,
    type CameraFps,
    enumerateNativeDevices,
    setNativeDevice,
    setNativeResolution,
    setNativeFramerate,
    setNativeCodec,
  } from '$lib/stores/video';
  import { canvasSink, mjpegStats } from '$lib/controllers/mjpegSink';
  import { nativeSinkFps } from '$lib/stores/video';
  import {
    codecsFor,
    codecLabel,
    resolutionsFor,
    frameratesFor,
    resolutionLabel,
  } from '$lib/helpers/videoCapabilities';
  import PanelShell from '$lib/components/panel/PanelShell.svelte';
  import Button from '$lib/components/panel/Button.svelte';
  import NumberStepper from '$lib/components/NumberStepper.svelte';
  import Toggle from '$lib/components/panel/Toggle.svelte';
  import { isLinux, isMobile, isAndroid, isPhone, hasVideo } from '$lib/platform';

  // Which saved RTSP connection is being edited inline (null = none).
  let editingRtspId = $state<string | null>(null);
  const inputVal = (e: Event) => (e.currentTarget as HTMLInputElement).value;

  // Populate the getUserMedia device list. It is only consumed by the `camera` source; on Linux,
  // enumerating it drives WebKit's GStreamer/pipewire stack, which can hang ~35 s on an unreachable
  // pipewire and freeze the app — so there we enumerate it only while the camera source is selected.
  // (The native list comes from the Rust backend and is enumerated once on mount, below.)
  //
  // The condition MUST come through this narrow $derived: an effect that reads `$videoState` directly
  // depends on the WHOLE store, and every enumeration writes back into it (devices / nativeDevices) —
  // which re-triggers the effect forever. That hit Linux only, because there the `||` short-circuit
  // does not skip the store read: a permanent enumerate → patch → enumerate loop (IPC + an ffmpeg
  // probe process + a localStorage write per round, and a stream restart when the saved device is
  // stale). A $derived boolean re-evaluates but only propagates when it actually flips.
  const needCameraList = $derived(!isLinux || $videoState.kind === 'camera');
  $effect(() => {
    if (needCameraList) void enumerateVideoDevices();
  });

  // ── RTSP / V4L2 dependencies ──────────────────────────────────────────
  // MediaMTX is required (the RTSP→WebRTC engine); ffmpeg is the optional fallback reader for
  // sources its native client can't pull (e.g. obs-rtspserver), and the V4L2 path always uses it.
  // Both are downloaded on demand.
  let engineVer = $state<string | null>(null);
  let engineChecked = $state(false);
  let engineDownloading = $state(false);
  let enginePct = $state(0);
  let engineMsg = $state('');

  let ffmpegVer = $state<string | null>(null);
  let ffmpegChecked = $state(false);
  let ffmpegDownloading = $state(false);
  let ffmpegPct = $state(0);
  let ffmpegMsg = $state('');

  async function checkEngine(): Promise<void> {
    try {
      engineVer = await invoke<string | null>('video_engine_status');
    } catch {
      engineVer = null;
    }
    engineChecked = true;
  }

  async function checkFfmpeg(): Promise<void> {
    try {
      ffmpegVer = await invoke<string | null>('video_ffmpeg_status');
    } catch {
      ffmpegVer = null;
    }
    ffmpegChecked = true;
  }

  async function downloadEngine(): Promise<void> {
    engineDownloading = true;
    enginePct = 0;
    engineMsg = '';
    try {
      await invoke('video_engine_download');
      await checkEngine();
    } catch (e) {
      engineMsg = e instanceof Error ? e.message : String(e);
    } finally {
      engineDownloading = false;
    }
  }

  async function downloadFfmpeg(): Promise<void> {
    ffmpegDownloading = true;
    ffmpegPct = 0;
    ffmpegMsg = '';
    try {
      await invoke('video_ffmpeg_download');
      await checkFfmpeg();
      // On Windows/macOS enumeration itself needs ffmpeg — refresh the native device list now.
      void enumerateNativeDevices();
    } catch (e) {
      ffmpegMsg = e instanceof Error ? e.message : String(e);
    } finally {
      ffmpegDownloading = false;
    }
  }

  onMount(() => {
    void checkEngine();
    void checkFfmpeg();
    // Native capture devices: enumerated once per panel open (the Rust backend reads V4L2 sysfs /
    // DirectShow / AVFoundation). Deliberately NOT in an $effect — it writes `nativeDevices` back into
    // the video store, which would make any store-reading effect re-trigger itself (see above).
    void enumerateNativeDevices();
    const unlisteners: UnlistenFn[] = [];
    void listen<{ pct: number; msg: string }>('video-engine-download-progress', (e) => {
      enginePct = e.payload.pct;
      engineMsg = e.payload.msg;
    }).then((u) => unlisteners.push(u));
    void listen<{ pct: number; msg: string }>('ffmpeg-download-progress', (e) => {
      ffmpegPct = e.payload.pct;
      ffmpegMsg = e.payload.msg;
    }).then((u) => unlisteners.push(u));
    return () => unlisteners.forEach((u) => u());
  });

  // Native capture is available on every desktop OS (Linux V4L2 / Windows DirectShow / macOS
  // AVFoundation) — all through ffmpeg, which cannot run on mobile. There, the OS's own capture
  // devices (USB/OTG included) already arrive through the camera kind (getUserMedia), so the native
  // kind would add nothing and is not offered.
  const KINDS: VideoKind[] = isMobile ? ['camera', 'rtsp'] : ['camera', 'rtsp', 'native'];

  // MediaMTX is only needed for the WebRTC path. A WebView without it (WebKitGTK builds with WebRTC
  // compiled out — Raspberry Pi OS among them) runs RTSP entirely on ffmpeg now, so demanding the
  // engine there would block a machine that already has everything it needs.
  // …and not at all when the experimental in-process native RTSP client is active.
  const needsEngine = $derived(isWebrtcAvailable() && !$videoState.rtspNativeClient);

  // Mirror can't reach Android's hardware-decoded surface (see the toggle row's comment).
  const mirrorUnavailable = $derived(isAndroid && $videoState.nativeSink);

  const RESOLUTIONS: VideoResolution[] = ['auto', '480p', '720p', '1080p'];
  const CAMERA_FPS: CameraFps[] = ['auto', '30', '60'];

  // Native-capture cascade, derived from the device's real probed modes: Format (codec) → Resolution
  // → Framerate. Each level lists exactly what the device reports for the level(s) above it.
  const nativeCodecs = $derived(codecsFor($videoState.nativeModes));
  const nativeResolutions = $derived(
    resolutionsFor($videoState.nativeModes, $videoState.nativeSel.codec),
  );
  const nativeFramerates = $derived(
    frameratesFor(
      $videoState.nativeModes,
      $videoState.nativeSel.codec,
      $videoState.nativeSel.width,
      $videoState.nativeSel.height,
    ),
  );

  // Info-line frame rate — each path reports it from the stage that actually knows.
  //
  // `videoElFps` counts `requestVideoFrameCallback` on a visible <video> sink (store), which fires
  // through the page's render loop: it reads 50–53 on a rock-steady 60 fps feed whenever the UI is
  // busy, because it is measuring our own rendering, not the stream. So a WebRTC feed takes the
  // decoder's own rate from the inbound stats and only falls back to the sampled count where there
  // are none (camera / getUserMedia). The MJPEG reader counts its own drawn frames, which is exact on
  // every platform; the <img> fallback (WebKitGTK, which fires `load` once per multipart image) has no
  // countable rate, hence the configured rate as a last resort.
  const fpsText = $derived.by(() => {
    const s = $videoState;
    // Native decode sink: the backend counts what it actually presents (1 Hz poll).
    if (s.nativeSink) return $nativeSinkFps ? $nativeSinkFps.toFixed(0) : '–';
    const drawn = $canvasSink ? ($mjpegStats?.fpsOut ?? 0) : 0;
    if (s.kind === 'native' && s.mjpegUrl) return drawn ? drawn.toFixed(0) : String(s.nativeSel.fps);
    const cur = s.mjpegUrl ? drawn : ($videoRtcStats?.decodeFps ?? $videoElFps);
    const curStr = cur ? cur.toFixed(0) : '–';
    return s.frameRate ? `${curStr}/${Math.round(s.frameRate)}` : curStr;
  });

  // Codec + bitrate for a network stream, from whichever pipeline is carrying it. Both are facts
  // about the feed the user is looking at, and neither was visible outside the Debug Monitor.
  // On the image path this used to read "MJPEG" for every source, because MJPEG is what reaches the
  // screen — true, and not what the user is asking. The transcode verdict answers the real question:
  // the backend only gets `copy` when the mpjpeg muxer accepted the source's own packets, which no
  // codec but MJPEG survives. Anything else was decoded and re-encoded, and the only other codec Kite
  // supports over RTSP is H.264.
  //
  // Both ends are named on the transcode path, because the bitrate next to it is the MJPEG one and
  // reading a source's codec beside the pipeline's output rate is how "3 Mbit H.264" turns into a
  // puzzling 25 Mbit/s. The source's own bitrate is not on offer: ffmpeg reports what it writes, not
  // what a live RTSP input costs.
  const streamCodec = $derived.by(() => {
    const s = $videoState;
    if (s.kind !== 'rtsp' || s.status !== 'live') return null;
    if (s.nativeSink) return s.nativeSinkCodec ?? 'H.264';
    if (!s.mjpegUrl) return $videoRtcStats?.codec ?? null;
    return s.activeTranscode === 'copy' ? 'MJPEG' : 'H.264 → MJPEG';
  });
  const streamBitrate = $derived.by(() => {
    const s = $videoState;
    if (s.kind !== 'rtsp' || s.status !== 'live') return null;
    const kbps = s.mjpegUrl ? $mjpegStats?.kbps : $videoRtcStats?.kbps;
    if (!kbps) return null;
    return kbps >= 1000 ? `${(kbps / 1000).toFixed(1)} Mbit/s` : `${Math.round(kbps)} kbit/s`;
  });

  // Diagnostic: what the LIVE feed actually does. Two independent questions, so two badges:
  //   • transcode — reported by the backend for this stream (`activeTranscode`), never inferred from
  //     what the host *could* do: an MJPEG camera is stream-copied and a user can force software, so
  //     "this host has VAAPI" says nothing about the feed in front of you.
  //   • surface   — `<video>` on a hardware overlay, or the image path: a JPEG decoded per frame and
  //     drawn into a canvas. That canvas is its own compositing layer, but it is still not a hardware
  //     video surface, which is what this badge is about.
  // A `<video>` feed transcodes nothing (the WebView decodes it), so it shows only the surface badge.
  const TRANSCODE_LABEL: Record<string, string> = { vaapi: 'VAAPI', v4l2m2m: 'V4L2' };
  const pipeline = $derived.by(():
    | { method: string; transcode: string | null; transcodeHw: boolean; surfaceHw: boolean }
    | null => {
    const s = $videoState;
    if (s.status !== 'live') return null;
    if (s.nativeSink) {
      // The whole chain is hardware: RTP → depacketizer → MF decoder (DXVA) → D3D layer.
      return { method: 'Kite RTSP client → native decode', transcode: null, transcodeHw: true, surfaceHw: true };
    }
    if (s.mjpegUrl) {
      const mode = s.activeTranscode;
      const engine = mode ? TRANSCODE_LABEL[mode] : undefined;
      const via = engine ?? (mode === 'copy' ? $t('video.pipeline.copy') : undefined);
      return {
        // Always ffmpeg: the image path reads the source itself and broadcasts `-f mpjpeg` through
        // Kite's own server. It used to run through go2rtc, whose republish was measured as the
        // cause of the freezes, and naming go2rtc here now would point at the wrong component.
        method: s.rtspEngine === 'kite'
          ? 'Kite RTSP client → MJPEG'
          : `ffmpeg → MJPEG${via ? ` (${via})` : ''}`,
        transcode: mode,
        // A stream copy is better than hardware — there is nothing to accelerate — so it counts as
        // "not costing us anything", not as a software fallback.
        transcodeHw: !!engine || mode === 'copy',
        surfaceHw: false,
      };
    }
    if (s.kind === 'rtsp') {
      return { method: `MediaMTX → WebRTC (${s.rtspEngine ?? 'native'})`, transcode: null, transcodeHw: true, surfaceHw: true };
    }
    return { method: 'getUserMedia', transcode: null, transcodeHw: true, surfaceHw: true };
  });
</script>

{#snippet headerActions()}
  <Button
    variant={$videoState.enabled ? 'danger' : 'data'}
    disabled={(!$videoState.enabled && $videoState.kind === 'rtsp' && needsEngine && engineChecked && !engineVer)}
    onclick={toggleVideo}
  >
    {$videoState.enabled ? $t('video.stop') : $t('video.start')}
  </Button>
{/snippet}

{#snippet body()}
  <div class="vp-body">
    <!-- No preview surface (PHONE_VIDEO.md D1, desktop since §10): the picture is in the floating /
         docked window or the widget. What the preview used to say about a source that is not live
         is said here instead. -->
    <!-- Status block — the room the preview left goes to what the user looks for first: the state
         of the source, then resolution / fps / codec / bitrate as readable tiles. -->
    <div class="vp-status" class:live={$videoState.status === 'live'} class:starting={$videoState.status === 'starting'} class:error={$videoState.status === 'error'}>
      <div class="vp-state">
        <span class="state-dot"></span>
        {#if $videoState.status === 'live'}
          {$t('video.live')}
        {:else if $videoState.status === 'starting'}
          {$t('video.starting')}
        {:else if $videoState.status === 'error'}
          ⚠ {$videoState.error}
        {:else}
          {$t('video.off')}
        {/if}
      </div>
      {#if $videoState.status === 'live'}
        <div class="vp-tiles">
          <div class="tile">
            <span class="val">{$videoState.width ?? '–'}×{$videoState.height ?? '–'}</span>
            <span class="lbl">{$t('video.resolution')}</span>
          </div>
          <div class="tile">
            <span class="val">{fpsText}</span>
            <span class="lbl">{$t('video.framerate')}</span>
          </div>
          {#if streamCodec}
            <div class="tile">
              <span class="val">{streamCodec}</span>
              <span class="lbl">{$t('video.codec')}</span>
            </div>
          {/if}
          {#if streamBitrate}
            <div class="tile">
              <span class="val">{streamBitrate}</span>
              <span class="lbl">{$t('video.bitrate')}</span>
            </div>
          {/if}
        </div>
      {/if}
    </div>

    {#if $videoState.status === 'live'}
      {#if pipeline}
        <div class="pipeline-line" class:sw={!pipeline.transcodeHw}>
          <span class="pl-dot"></span>
          <span class="pl-method">{pipeline.method}</span>
          <span class="pl-badges">
            {#if pipeline.transcode}
              <span class="pl-badge" class:sw={!pipeline.transcodeHw}>
                {$t('video.pipeline.transcode')}:
                {pipeline.transcode === 'copy'
                  ? $t('video.pipeline.copy')
                  : pipeline.transcodeHw
                    ? $t('video.pipeline.hw')
                    : $t('video.pipeline.sw')}
              </span>
            {/if}
            <span class="pl-badge" class:sw={!pipeline.surfaceHw}>
              {$t('video.pipeline.surface')}:
              {pipeline.surfaceHw ? $t('video.pipeline.hw') : $t('video.pipeline.sw')}
            </span>
          </span>
        </div>
      {/if}
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

      <label class="field">
        <span class="label">{$t('video.framerate')}</span>
        <select
          value={$videoState.cameraFps}
          onchange={(e) => setCameraFps((e.currentTarget as HTMLSelectElement).value as CameraFps)}
        >
          {#each CAMERA_FPS as f}
            <option value={f}>{f === 'auto' ? $t('video.auto') : `${f} fps`}</option>
          {/each}
        </select>
      </label>

      {#if $videoState.devices.length === 0}
        <p class="hint">{$t('video.noDevices')}</p>
      {/if}
    {:else if $videoState.kind === 'native'}
      <label class="field">
        <span class="label">{$t('video.device')}</span>
        <select
          value={$videoState.nativeDevice ?? ''}
          onchange={(e) => setNativeDevice((e.currentTarget as HTMLSelectElement).value || null)}
        >
          {#each $videoState.nativeDevices as d}
            <option value={d.id}>{d.name}</option>
          {/each}
        </select>
      </label>

      {#if $videoState.nativeDevices.length === 0}
        <p class="hint">{$t('video.noNativeDevices')}</p>
      {:else}
        <label class="field">
          <span class="label">{$t('video.format')}</span>
          <select
            value={$videoState.nativeSel.codec}
            onchange={(e) => void setNativeCodec((e.currentTarget as HTMLSelectElement).value)}
          >
            {#each nativeCodecs as c}
              <option value={c}>{codecLabel(c)}</option>
            {/each}
          </select>
        </label>

        <label class="field">
          <span class="label">{$t('video.resolution')}</span>
          <select
            value={`${$videoState.nativeSel.width}x${$videoState.nativeSel.height}`}
            onchange={(e) => {
              const [w, h] = (e.currentTarget as HTMLSelectElement).value.split('x').map(Number);
              void setNativeResolution(w, h);
            }}
          >
            {#each nativeResolutions as r}
              <option value={`${r.width}x${r.height}`}>{resolutionLabel(r.width, r.height)}</option>
            {/each}
          </select>
        </label>

        <label class="field">
          <span class="label">{$t('video.framerate')}</span>
          <select
            value={String($videoState.nativeSel.fps)}
            onchange={(e) => setNativeFramerate(Number((e.currentTarget as HTMLSelectElement).value))}
          >
            {#each nativeFramerates as f}
              <option value={String(f)}>{f} fps</option>
            {/each}
          </select>
        </label>
      {/if}

      <!-- Native capture needs ffmpeg (no engine). -->
      {#if ffmpegChecked && !ffmpegVer}
        <div class="ffmpeg-box">
          <p class="hint">{$t('video.ffmpegNativeMissing')}</p>
          {#if ffmpegDownloading}
            <div class="dl-row">
              <div class="dl-bar"><div class="dl-fill" style="width:{ffmpegPct}%"></div></div>
              <span class="dl-pct">{ffmpegPct}%</span>
            </div>
            {#if ffmpegMsg}<p class="hint">{ffmpegMsg}</p>{/if}
          {:else}
            <Button variant="data" onclick={downloadFfmpeg}>{$t('video.ffmpegFallbackDownload')}</Button>
            {#if ffmpegMsg}<p class="hint err">{ffmpegMsg}</p>{/if}
          {/if}
        </div>
      {/if}
    {:else}
      <!-- Direct connect: URL + transport, with an explicit Save-to-list button (never auto-saved). -->
      <div class="field">
        <span class="label">{$t('video.rtspUrl')}</span>
        <div class="rtsp-url-row">
          <input
            class="text-input"
            type="text"
            placeholder="rtsp://192.168.1.10:554/cam"
            value={$videoState.rtspUrl}
            onchange={(e) => setRtspUrl(inputVal(e))}
          />
          <select
            class="rtsp-transport"
            value={$videoState.rtspTransport}
            title={$t('video.rtspTransportHint')}
            onchange={(e) => setRtspTransport((e.currentTarget as HTMLSelectElement).value as RtspTransport)}
          >
            <option value="auto">{$t('video.rtspAuto')}</option>
            <option value="udp">UDP</option>
            <option value="tcp">TCP</option>
          </select>
          <button
            class="rtsp-save"
            title={$t('video.rtspSave')}
            aria-label={$t('video.rtspSave')}
            disabled={!$videoState.rtspUrl.trim()}
            onclick={saveRtspConnection}
          >💾</button>
        </div>
      </div>

      <!-- Experimental: Kite's own in-process RTSP client (MOBILE_RTSP.md) — no MediaMTX, no
           ffmpeg. MJPEG on the multipart path; H.264/HEVC decode in hardware.
           On Android and iOS it is the ONLY route (no sidecars on mobile) — forced on in the
           store, nothing to toggle. -->
      {#if !isMobile}
        <div class="field-row" title={$t('video.nativeClientHint')}>
          <Toggle
            checked={$videoState.rtspNativeClient}
            onchange={(c) => void setRtspNativeClient(c)}
            id="vp-native-rtsp"
          />
          <span class="label">{$t('video.nativeClient')}</span>
        </div>
      {/if}

      <!-- Saved connections: single-line rows, selectable / editable / deletable (ADS-B-provider style). -->
      {#if $videoState.rtspConnections.length}
        <div class="rtsp-list">
          {#each $videoState.rtspConnections as c (c.id)}
            <div class="rtsp-item" class:active={c.url === $videoState.rtspUrl}>
              {#if editingRtspId === c.id}
                <input
                  class="rtsp-edit rtsp-edit-name"
                  placeholder={$t('video.rtspName')}
                  value={c.name}
                  onchange={(e) => updateRtspConnection(c.id, { name: inputVal(e) })}
                />
                <input
                  class="rtsp-edit"
                  placeholder="rtsp://…"
                  value={c.url}
                  onchange={(e) => updateRtspConnection(c.id, { url: inputVal(e) })}
                />
                <select
                  class="rtsp-transport"
                  value={c.transport}
                  onchange={(e) => updateRtspConnection(c.id, { transport: (e.currentTarget as HTMLSelectElement).value as RtspTransport })}
                >
                  <option value="auto">{$t('video.rtspAuto')}</option>
                  <option value="udp">UDP</option>
                  <option value="tcp">TCP</option>
                </select>
                <button class="rtsp-item-btn" title={$t('video.rtspDone')} aria-label={$t('video.rtspDone')} onclick={() => (editingRtspId = null)}>✓</button>
              {:else}
                <button class="rtsp-item-main" title={c.url} onclick={() => selectRtspConnection(c.id)}>
                  <span class="rtsp-item-name">{c.name || c.url}</span>
                  <span class="rtsp-item-transport">{c.transport === 'auto' ? $t('video.rtspAuto') : c.transport.toUpperCase()}</span>
                </button>
                <button class="rtsp-item-btn" title={$t('video.rtspEdit')} aria-label={$t('video.rtspEdit')} onclick={() => (editingRtspId = c.id)}>✎</button>
                <button class="rtsp-item-btn del" title={$t('video.rtspDelete')} aria-label={$t('video.rtspDelete')} onclick={() => removeRtspConnection(c.id)}>✕</button>
              {/if}
            </div>
          {/each}
        </div>
      {/if}

      <!-- Receive-buffer depth in frame times, applied live and persisted — to the WebRTC
           receiver, the MJPEG reader's cushion, or the native decode sink's paced
           presentation queue. 0 = minimal latency (present on arrival/decode); each step
           trades one frame time of latency for smoothing of arrival jitter. Expressed in
           frames so the same setting means the same smoothing at 30 and 60 fps. -->
      <div class="field buffer-row" title={$t('video.bufferHint')}>
        <span class="label">{$t('video.bufferLabel')}</span>
        <NumberStepper bind:value={$rtspBufferFrames} min={0} max={3} step={1} />
      </div>

      {#if needsEngine && engineChecked && !engineVer}
        <!-- MediaMTX is required for the WebRTC path only — see `needsEngine`. -->
        <div class="ffmpeg-box">
          <p class="hint">{$t('video.engineMissing')}</p>
          {#if engineDownloading}
            <div class="dl-row">
              <div class="dl-bar"><div class="dl-fill" style="width:{enginePct}%"></div></div>
              <span class="dl-pct">{enginePct}%</span>
            </div>
            {#if engineMsg}<p class="hint">{engineMsg}</p>{/if}
          {:else}
            <Button variant="data" onclick={downloadEngine}>{$t('video.engineDownload')}</Button>
            {#if engineMsg}<p class="hint err">{engineMsg}</p>{/if}
          {/if}
        </div>
      {:else if engineVer && !$videoState.rtspNativeClient}
        {#if $videoState.status === 'live' && $videoState.rtspEngine && !$videoState.mjpegUrl}
          <!-- Which reader the engine uses — a WebRTC-path question. The image path does not go
               through the engine at all, and the pipeline line above already names what it runs. -->
          <p class="hint">{$t(`video.via.${$videoState.rtspEngine}`)}</p>
        {:else}
          <p class="hint">{$t('video.engineReady')}</p>
        {/if}

        {#if ffmpegChecked && !ffmpegVer}
          <!-- ffmpeg is the optional fallback reader (e.g. obs-rtspserver). Non-blocking. -->
          <div class="ffmpeg-box">
            <p class="hint">{$t('video.ffmpegFallbackMissing')}</p>
            {#if ffmpegDownloading}
              <div class="dl-row">
                <div class="dl-bar"><div class="dl-fill" style="width:{ffmpegPct}%"></div></div>
                <span class="dl-pct">{ffmpegPct}%</span>
              </div>
              {#if ffmpegMsg}<p class="hint">{ffmpegMsg}</p>{/if}
            {:else}
              <Button variant="standard" onclick={downloadFfmpeg}>{$t('video.ffmpegFallbackDownload')}</Button>
              {#if ffmpegMsg}<p class="hint err">{ffmpegMsg}</p>{/if}
            {/if}
          </div>
        {/if}
      {/if}
    {/if}

    <!-- Mirror is unavailable on Android's HARDWARE decode path (no MediaCodec key, and both
         view and buffer transforms were tried and measurably don't reach the surface — see
         android_sink.rs); the DOM-rendered routes (MJPEG, camera) mirror fine there. -->
    <div class="field-row" title={mirrorUnavailable ? $t('video.mirrorNativeAndroid') : undefined}>
      <Toggle
        checked={$videoState.mirror}
        disabled={mirrorUnavailable}
        onchange={(c) => setVideoMirror(c)}
        id="vp-mirror"
      />
      <span class="label" class:muted={mirrorUnavailable}>{$t('video.mirror')}</span>
    </div>

    <div class="field-row">
      <Toggle checked={$videoState.rotate180} onchange={(c) => setVideoRotate180(c)} id="vp-rot180" />
      <span class="label">{$t('video.rotate180')}</span>
    </div>

    <!-- Unobstructed fullscreen: the map-swap video shrinks so occupied widget panels and the
         nav rail stay clear of the picture; an empty/hidden panel releases its edge. Off = the
         video fills the whole map zone as before. Layout math lives in +page.svelte. -->
    {#if !isPhone}
      <!-- Not on the phone: no docks to retreat from, four corner buttons at most (PHONE_VIDEO.md). -->
      <div class="field-row" title={$t('video.unobstructedHint')}>
        <Toggle
          checked={$videoState.unobstructedFullscreen}
          onchange={(c) => setUnobstructedFullscreen(c)}
          id="vp-unobstructed"
        />
        <span class="label">{$t('video.unobstructed')}</span>
      </div>
    {/if}

    <!-- Escape hatch: some driver/hardware combinations pass the backend probe but still misbehave on
         a live feed. Hardware stays the default; this forces the software transcode. It only steers
         the ffmpeg transcode path — the native RTSP client decodes through the OS (Media Foundation),
         where this switch has no say, so it is hidden there. -->
    {#if !($videoState.kind === 'rtsp' && $videoState.rtspNativeClient)}
      <div class="field-row">
        <Toggle
          checked={$videoState.disableHwAccel}
          onchange={(c) => void setDisableHwAccel(c)}
          id="vp-no-hwaccel"
        />
        <span class="label">{$t('video.disableHwAccel')}</span>
      </div>
      <p class="hint">{$t('video.disableHwAccelHint')}</p>
    {/if}
  </div>
{/snippet}

{#snippet iphoneBody()}
  <p class="hint">{$t('video.iphoneUnsupported')}</p>
{/snippet}

<div class="vpv2">
  {#if !hasVideo}
    <PanelShell variant="compact" title={$t('video.title')} body={iphoneBody} />
  {:else}
    <PanelShell variant="compact" title={$t('video.title')} {headerActions} {body} />
  {/if}
</div>

<style>
  .vp-body { display: flex; flex-direction: column; gap: 12px; }

  /* Status block: state line (dot = off grey / starting amber / live green / error red) + the
     stream facts as tiles — the panel's headline now that there is no preview. */
  .vp-status {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .vp-state {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 14px;
    font-weight: 600;
    color: #cfcfcf;
  }
  .state-dot {
    flex: 0 0 auto;
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: #949494;
  }
  .vp-status.live .state-dot { background: #59aa29; box-shadow: 0 0 6px rgba(89, 170, 41, 0.7); }
  .vp-status.live .vp-state { color: #e0e0e0; }
  .vp-status.starting .state-dot { background: #e0a53c; }
  .vp-status.error .state-dot { background: #d40000; }
  .vp-status.error .vp-state { color: #ff6b6b; font-weight: 500; }
  .vp-status:not(.live):not(.error) .vp-state { font-weight: 500; }
  .vp-tiles {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(84px, 1fr));
    gap: 6px;
  }
  .tile {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 6px 8px;
    background: rgba(0, 0, 0, 0.25);
    border: 1px solid rgba(255, 255, 255, 0.06);
    border-radius: 6px;
    min-width: 0;
  }
  .tile .val {
    font-size: 15px;
    font-weight: 600;
    color: #9ad0e8;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tile .lbl {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: #949494;
  }
  /* Diagnostic pipeline readout: dot + method + a HW/SW badge. Green = hardware-composited <video>
     (getUserMedia / engine-WebRTC); amber = the software ffmpeg→MJPEG <img> fallback. */
  .pipeline-line {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    color: #cfe6f2;
    letter-spacing: 0.02em;
  }
  .pl-dot { width: 7px; height: 7px; border-radius: 50%; background: #4fc47a; flex: 0 0 auto; }
  .pipeline-line.sw .pl-dot { background: #e0a53c; }
  .pl-method { font-variant-numeric: tabular-nums; }
  .pl-badges { margin-left: auto; display: flex; gap: 4px; flex-wrap: wrap; justify-content: flex-end; }
  .pl-badge {
    padding: 1px 6px;
    border-radius: 3px;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.03em;
    white-space: nowrap;
    color: #4fc47a;
    background: rgba(79, 196, 122, 0.14);
  }
  .pl-badge.sw { color: #e0a53c; background: rgba(224, 165, 60, 0.14); }

  .field { display: flex; flex-direction: column; gap: 4px; }
  .field-row { display: flex; align-items: center; gap: 8px; }
  .label { font-size: 12px; color: #aaa; }
  .label.muted { color: #666; }
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

  /* RTSP direct-connect row + saved-connection list */
  .rtsp-url-row { display: flex; align-items: center; gap: 6px; }
  .rtsp-url-row .text-input { flex: 1; min-width: 0; }
  .rtsp-transport {
    height: 28px;
    padding: 0 6px;
    background: #434343;
    color: #e0e0e0;
    border: 1px solid #555;
    border-radius: 4px;
    font-size: 12px;
    flex: 0 0 auto;
  }
  .rtsp-save {
    height: 28px;
    min-width: 30px;
    padding: 0 6px;
    background: #37a8db;
    color: #fff;
    border: none;
    border-radius: 4px;
    cursor: pointer;
    font-size: 13px;
  }
  .rtsp-save:disabled { opacity: 0.4; cursor: not-allowed; }

  .rtsp-list { display: flex; flex-direction: column; gap: 4px; margin-top: 6px; }
  .rtsp-item {
    display: flex;
    align-items: center;
    gap: 4px;
    background: #2e2e2e;
    border: 1px solid #272727;
    border-radius: 4px;
    padding: 3px 4px 3px 6px;
  }
  .rtsp-item.active { border-color: rgba(55, 168, 219, 0.75); }
  .rtsp-item-main {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    background: none;
    border: none;
    color: #e0e0e0;
    cursor: pointer;
    text-align: left;
    padding: 3px 2px;
    font-size: 12px;
  }
  .rtsp-item-main:hover .rtsp-item-name { color: #37a8db; }
  .rtsp-item-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .rtsp-item-transport { flex: 0 0 auto; font-size: 10px; color: #949494; letter-spacing: 0.04em; }
  .rtsp-item-btn {
    flex: 0 0 auto;
    width: 24px;
    height: 24px;
    background: none;
    border: none;
    color: #949494;
    cursor: pointer;
    border-radius: 3px;
    font-size: 12px;
  }
  .rtsp-item-btn:hover { background: #3a3a3a; color: #e0e0e0; }
  .rtsp-item-btn.del:hover { background: rgba(212, 0, 0, 0.3); color: #ff4444; }
  .rtsp-edit {
    flex: 1;
    min-width: 0;
    height: 26px;
    padding: 0 6px;
    background: #434343;
    color: #e0e0e0;
    border: 1px solid #555;
    border-radius: 4px;
    font-size: 12px;
  }
  .rtsp-edit-name { flex: 0 0 90px; }

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

</style>
