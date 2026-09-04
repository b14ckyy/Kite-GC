<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- StatusTextToasts.svelte
     FC system messages (MAVLink STATUSTEXT) as a single compact banner at the top edge: one line per
     message, colour-coded by severity, newest at the bottom; the field scrolls to the latest. Each
     message expires individually 20 s after it arrived (the store drops lines one by one); the banner
     disappears once the last line is gone. Verbosity is controlled by settings.systemMessages; audio
     cue is played in the store.
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
          {#if msg.repeats > 1}<span class="m-count">×{msg.repeats}</span>{/if}
        </div>
      {/each}
    </div>
  </div>
{/if}

<style>
  .msg-banner {
    position: absolute;
    top: 56px;
    /* Centre within the area to the RIGHT of any open left-docked panel (issue #10). --toast-dock-inset
       is the panel's viewport right edge (0 when none); left + right + margin:auto centres the
       fixed-width banner in that band, and the max-width cap keeps it inside the band on narrow screens
       so it can never slide back over the panel. */
    left: calc(var(--toast-dock-inset, 0px) + 8px);
    right: 8px;
    margin-inline: auto;
    z-index: 480; /* below the radar conflict banner (500) */
    width: max-content;
    max-width: min(640px, calc(100vw - var(--toast-dock-inset, 0px) - 32px));
    background: rgba(30, 30, 30, 0.82);
    backdrop-filter: blur(10px);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 6px;
    box-shadow: 0 3px 12px rgba(0, 0, 0, 0.5);
    padding: 4px 6px;
    pointer-events: auto;
    font-family: 'Segoe UI', Tahoma, sans-serif;
  }

  /* Mobile: the phone toolbar wraps onto several rows while disconnected/expanded, so a fixed 56px
     lands *inside* it and the messages cover the connection banner. Track the live bar height instead —
     the same --toolbar-h the map layer and nav-rail follow. The parent .app-toasts layer is already
     offset by --safe-top and --toolbar-h includes that same inset (the toolbar pads itself down into
     the visible strip), so subtract it here or the status-bar strip is counted twice. */
  :global(html.is-mobile) .msg-banner {
    top: calc(var(--toolbar-h, 53px) - var(--safe-top, 0px) + 8px);
  }
  /* Phone: the parent .app-toasts IS the band between the corner buttons and already sits at
     --safe-top, so start in the buttons' row and let the band — not the viewport — cap the width.
     --toast-dock-inset is a viewport x; the band's own left edge (--toast-band-left) comes off it
     first, so a banner beside an open panel still centres in what is left of the band. */
  :global(html.is-phone) .msg-banner {
    top: 8px;
    left: calc(max(var(--toast-dock-inset, 0px) - var(--toast-band-left, 0px), 0px) + 8px);
    max-width: min(640px, calc(100% - max(var(--toast-dock-inset, 0px) - var(--toast-band-left, 0px), 0px) - 16px));
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
  /* Repeat tally for a message the FC keeps re-sending — dimmed so it reads as metadata, not content. */
  .m-count { flex: 0 0 auto; margin-left: auto; padding-left: 8px; font-size: 11px; opacity: 0.55; font-variant-numeric: tabular-nums; }

  .msg-line.info    { color: #cfe7f3; }
  .msg-line.info .m-icon { color: #37a8db; }
  .msg-line.warning { color: #f6e3b0; }
  .msg-line.warning .m-icon { color: #f4c020; }
  .msg-line.error   { color: #f6c9c9; background: rgba(120, 30, 30, 0.45); }
  .msg-line.error .m-icon { color: #ff5252; }
</style>
