<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->
<!--
  Shown over any video sink (panel preview / widget / floating window) while the RTSP feed is in its
  infinite auto-reconnect loop, so the pilot always sees that the link dropped and is being retried —
  with the attempt counter and an explicit Stop. The parent must be `position: relative`.
-->
<script lang="ts">
  import { t } from 'svelte-i18n';
  import { videoState, stopVideo } from '$lib/stores/video';
</script>

{#if $videoState.reconnecting}
  <div class="rc-overlay">
    <div class="rc-box">
      <span class="rc-spinner"></span>
      <span class="rc-text">
        {$t('video.reconnecting')}{$videoState.reconnectAttempt ? ` (${$videoState.reconnectAttempt})` : ''}
      </span>
      <button class="rc-stop" onclick={stopVideo}>{$t('video.reconnectStop')}</button>
    </div>
  </div>
{/if}

<style>
  .rc-overlay {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    pointer-events: none; /* let clicks through to the video except on the Stop button */
    z-index: 5;
  }
  .rc-box {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 12px;
    background: rgba(20, 20, 20, 0.82);
    border: 1px solid rgba(245, 166, 35, 0.7);
    border-radius: 8px;
    pointer-events: auto;
    max-width: 90%;
  }
  .rc-spinner {
    flex: 0 0 auto;
    width: 14px;
    height: 14px;
    border: 2px solid rgba(245, 166, 35, 0.35);
    border-top-color: #f5a623;
    border-radius: 50%;
    animation: rc-spin 0.8s linear infinite;
  }
  /* WebKitGTK → the shared 1 Hz blink instead of spinning (see stores/pulseBlink.ts). Included
     despite looking transient: the RTSP reconnect loop retries indefinitely, so this can turn for
     minutes while a link is down — and a spinner is a looping animation like any other. */
  :global(html.kite-blink-mode) .rc-spinner {
    animation: none;
    opacity: 0.4;
  }
  :global(html.kite-blink-mode.kite-blink) .rc-spinner {
    opacity: 1;
  }
  .rc-text {
    color: #f5a623;
    font-size: 13px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .rc-stop {
    flex: 0 0 auto;
    background: none;
    border: 1px solid #666;
    color: #ccc;
    border-radius: 4px;
    padding: 3px 8px;
    font-size: 11px;
    cursor: pointer;
  }
  .rc-stop:hover {
    border-color: #d40000;
    color: #ff5555;
  }
  @keyframes rc-spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .rc-spinner {
      animation: none;
    }
  }
</style>
