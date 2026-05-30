<script lang="ts">
  // Video widget (2×1 wide) — a router sink showing the shared video feed.
  // Crop-to-fill (object-fit: cover) so the 2:1 tile is always full (too small
  // to read OSD anyway). Standard widget card with a thin rounded frame around
  // the video. No settings — the NavRail Video panel owns all control.
  import { t } from 'svelte-i18n';
  import { videoStream, videoState } from '$lib/stores/video';

  let { width = 300, height = 150 }: { width?: number; height?: number } = $props();

  let videoEl = $state<HTMLVideoElement | null>(null);
  $effect(() => {
    if (videoEl) videoEl.srcObject = $videoStream;
  });
</script>

<div class="widget-card" style="width:{width}px; height:{height}px;">
  {#if $videoState.status === 'live'}
    <!-- svelte-ignore a11y_media_has_caption -->
    <video
      bind:this={videoEl}
      autoplay
      muted
      playsinline
      class:mirror={$videoState.mirror}
    ></video>
  {:else}
    <div class="placeholder">
      {$videoState.status === 'starting' ? $t('video.starting') : $t('video.off')}
    </div>
  {/if}
</div>

<style>
  .widget-card {
    box-sizing: border-box;
    background: rgba(30, 30, 30, 0.75);
    backdrop-filter: blur(10px);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 8px;
    padding: 3px;
    overflow: hidden;
  }
  video,
  .placeholder {
    width: 100%;
    height: 100%;
    border-radius: 5px;
    border: 1px solid rgba(255, 255, 255, 0.12);
    background: #000;
    display: block;
    box-sizing: border-box;
  }
  video {
    object-fit: cover; /* crop to fill the 2:1 tile */
  }
  video.mirror {
    transform: scaleX(-1);
  }
  .placeholder {
    display: flex;
    align-items: center;
    justify-content: center;
    color: #888;
    font-size: 12px;
  }
</style>
