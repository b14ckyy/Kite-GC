<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- StatusTextToasts — FC system messages (MAVLink STATUSTEXT) as one compact top-edge banner: a line per
     message, colour-coded, newest at the bottom; lines expire individually (the store drops them). Verbosity:
     settings.systemMessages; the audio cue plays in the store. -->
<script lang="ts">
  import { fade } from 'svelte/transition';
  import { statusTexts, type StatusTextLevel } from '$lib/stores/statusText';

  const ICON: Record<StatusTextLevel, string> = { error: '⚠', warning: '▲', info: 'ⓘ', debug: '⋯' };

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
          {#if msg.vehicle}
            <span class="m-vehicle" style:--vc={msg.vehicle.color} title={msg.vehicle.name}>
              <span class="m-vdot"></span>{msg.vehicle.name}
            </span>
          {/if}
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
    /* Centre to the RIGHT of an open left-docked panel: --toast-dock-inset is its viewport right edge (0 when
       none); left + right + margin:auto centre the banner in that band, max-width keeps it off the panel. */
    left: calc(var(--toast-dock-inset, 0px) + 8px);
    right: 8px;
    margin-inline: auto;
    z-index: 480; /* below the radar conflict banner (500) */
    width: max-content;
    max-width: min(640px, calc(100vw - var(--toast-dock-inset, 0px) - 32px));
    background: rgba(30, 30, 30, 0.82);
    backdrop-filter: var(--glass-blur, blur(6px));
    -webkit-backdrop-filter: var(--glass-blur, blur(6px));
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 6px;
    box-shadow: 0 3px 12px rgba(0, 0, 0, 0.5);
    padding: 4px 6px;
    pointer-events: auto;
    font-family: 'Segoe UI', Tahoma, sans-serif;
  }

  /* Mobile: the wrapping toolbar's height varies, so track --toolbar-h; .app-toasts is already offset by
     --safe-top, which --toolbar-h includes too — subtract it to not count the status-bar strip twice. */
  :global(html.is-mobile) .msg-banner {
    top: calc(var(--toolbar-h, 53px) - var(--safe-top, 0px) + 8px);
  }
  /* Phone: the parent .app-toasts IS the band between the corner buttons (at --safe-top), so the band caps the
     width. --toast-dock-inset is a viewport x: subtract the band's left edge (--toast-band-left) first, so a
     banner beside an open panel centres in what is left. */
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
  /* Fleet: which vehicle spoke — a dot in its map colour + its name, so a failing craft is obvious. */
  .m-vehicle {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    flex: 0 0 auto;
    max-width: 140px;
    overflow: hidden;
    text-overflow: ellipsis;
    padding: 0 6px 0 4px;
    border-radius: 3px;
    background: rgba(255, 255, 255, 0.08);
    font-size: 11px;
    font-weight: 600;
    color: #e0e0e0;
  }
  .m-vdot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--vc, #888);
    box-shadow: 0 0 3px var(--vc, #888);
    flex: 0 0 auto;
  }
  /* Repeat tally for a message the FC keeps re-sending. Dimmed so it reads as metadata, not content. */
  .m-count { flex: 0 0 auto; margin-left: auto; padding-left: 8px; font-size: 11px; opacity: 0.55; font-variant-numeric: tabular-nums; }

  .msg-line.info    { color: #cfe7f3; }
  .msg-line.info .m-icon { color: #37a8db; }
  .msg-line.debug   { color: #b4b4b4; }
  .msg-line.debug .m-icon { color: #949494; }
  .msg-line.warning { color: #f6e3b0; }
  .msg-line.warning .m-icon { color: #f4c020; }
  .msg-line.error   { color: #f6c9c9; background: rgba(120, 30, 30, 0.45); }
  .msg-line.error .m-icon { color: #ff5252; }
</style>
