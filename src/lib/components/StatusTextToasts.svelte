<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- StatusTextToasts.svelte
     FC system messages (MAVLink STATUSTEXT) as a single compact banner at the top edge: one line per
     message, colour-coded by severity, newest at the bottom; the field scrolls to the latest. The
     whole banner fades out 60 s after the last message (the store clears the list). Verbosity is
     controlled by settings.systemMessages; audio cue is played in the store.
-->
<script lang="ts">
  import { fade } from 'svelte/transition';
  import { statusTexts, type StatusTextLevel } from '$lib/stores/statusText';

  const ICON: Record<StatusTextLevel, string> = { error: '⚠', warning: '▲', info: 'ⓘ' };

  let scroller = $state<HTMLDivElement | undefined>(undefined);
  // Keep the newest line in view as messages arrive.
  $effect(() => {
    void $statusTexts.length;
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
  });
</script>

{#if $statusTexts.length}
  <div class="msg-banner" role="log" transition:fade={{ duration: 250 }}>
    <div class="msg-lines" bind:this={scroller}>
      {#each $statusTexts as msg (msg.id)}
        <div class="msg-line {msg.level}">
          <span class="m-icon">{ICON[msg.level]}</span>
          <span class="m-text">{msg.text}</span>
        </div>
      {/each}
    </div>
  </div>
{/if}

<style>
  .msg-banner {
    position: absolute;
    top: 56px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 480; /* below the radar conflict banner (500) */
    width: max-content;
    max-width: min(640px, 80%);
    background: rgba(30, 30, 30, 0.82);
    backdrop-filter: blur(10px);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 6px;
    box-shadow: 0 3px 12px rgba(0, 0, 0, 0.5);
    padding: 4px 6px;
    pointer-events: auto;
    font-family: 'Segoe UI', Tahoma, sans-serif;
  }

  .msg-lines {
    display: flex;
    flex-direction: column;
    gap: 1px;
    max-height: 108px; /* ~5 lines, then it scrolls to the newest */
    overflow-y: auto;
    scrollbar-width: thin;
    scrollbar-color: #555 transparent;
  }
  .msg-lines::-webkit-scrollbar { width: 6px; }
  .msg-lines::-webkit-scrollbar-thumb { background: #555; border-radius: 3px; }

  .msg-line {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 2px 6px;
    border-radius: 3px;
    font-size: 12.5px;
    line-height: 1.35;
    white-space: nowrap;
  }
  .m-icon { font-size: 11px; line-height: 1; flex: 0 0 auto; }
  .m-text { overflow: hidden; text-overflow: ellipsis; }

  .msg-line.info    { color: #cfe7f3; }
  .msg-line.info .m-icon { color: #37a8db; }
  .msg-line.warning { color: #f6e3b0; }
  .msg-line.warning .m-icon { color: #f4c020; }
  .msg-line.error   { color: #f6c9c9; background: rgba(120, 30, 30, 0.45); }
  .msg-line.error .m-icon { color: #ff5252; }
</style>
