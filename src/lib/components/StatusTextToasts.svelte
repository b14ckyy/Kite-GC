<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- StatusTextToasts.svelte
     FC system messages (MAVLink STATUSTEXT) as single-line toasts pinned to the top edge — up to 5,
     newest at the bottom, colour-coded by severity. The whole stack fades out 60 s after the last
     message (handled by the store clearing the list). Audio cue is played in the store.
-->
<script lang="ts">
  import { fade, fly } from 'svelte/transition';
  import { statusTexts, type StatusTextLevel } from '$lib/stores/statusText';

  const ICON: Record<StatusTextLevel, string> = { error: '⚠', warning: '▲', info: 'ⓘ' };
</script>

{#if $statusTexts.length}
  <div class="toast-stack">
    {#each $statusTexts as msg (msg.id)}
      <div
        class="toast {msg.level}"
        role={msg.level === 'info' ? 'status' : 'alert'}
        in:fly={{ y: -8, duration: 160 }}
        out:fade={{ duration: 300 }}
      >
        <span class="t-icon">{ICON[msg.level]}</span>
        <span class="t-text">{msg.text}</span>
      </div>
    {/each}
  </div>
{/if}

<style>
  .toast-stack {
    position: absolute;
    top: 56px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 480; /* below the radar conflict banner (500) */
    display: flex;
    flex-direction: column;
    gap: 4px;
    width: max-content;
    max-width: min(640px, 80%);
    pointer-events: none;
  }

  .toast {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 12px;
    border-radius: 6px;
    border-left: 3px solid;
    font-family: 'Segoe UI', Tahoma, sans-serif;
    font-size: 13px;
    line-height: 1.3;
    background: rgba(30, 30, 30, 0.82);
    backdrop-filter: blur(10px);
    box-shadow: 0 3px 12px rgba(0, 0, 0, 0.5);
  }

  .t-icon { font-size: 13px; line-height: 1; flex: 0 0 auto; }
  .t-text {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .toast.info    { border-left-color: #37a8db; color: #cfe7f3; }
  .toast.info .t-icon { color: #37a8db; }
  .toast.warning { border-left-color: #f4c020; color: #f6e3b0; }
  .toast.warning .t-icon { color: #f4c020; }
  .toast.error   { border-left-color: #d40000; color: #f6c9c9; background: rgba(60, 20, 20, 0.85); }
  .toast.error .t-icon { color: #ff5252; }
</style>
