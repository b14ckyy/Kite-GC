<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- "Compact Database" popup: shown while the full VACUUM runs (minutes on a hard disk or an SD card).
     Deliberately modal and self-dismissing like the hi-res parse popup — no buttons, no Escape, no
     outside click: the compact cannot be cancelled, and the user must see that the wait is expected.
     SQLite reports no progress for a VACUUM, so the bar is indeterminate; the elapsed time shows that
     it is still alive. The page makes everything else `inert` meanwhile (no Tab / Enter behind it),
     so this component must stay outside those subtrees (+page.svelte, panels layer). -->
<script lang="ts">
  import { t } from 'svelte-i18n';

  const started = Date.now();
  let elapsedSec = $state(0);

  $effect(() => {
    const timer = setInterval(() => {
      elapsedSec = Math.floor((Date.now() - started) / 1000);
    }, 1000);
    return () => clearInterval(timer);
  });

  const elapsed = $derived(`${Math.floor(elapsedSec / 60)}:${String(elapsedSec % 60).padStart(2, '0')}`);
</script>

<div class="dialog-backdrop">
  <div
    class="dialog-box"
    role="alertdialog"
    aria-modal="true"
    aria-labelledby="compact-db-title"
    aria-describedby="compact-db-hint"
  >
    <div class="dialog-title" id="compact-db-title">{$t('settings.compactDbRunningTitle')}</div>
    <div class="dialog-message" id="compact-db-hint">{$t('settings.compactDbRunningHint')}</div>
    <div class="compact-track" aria-hidden="true">
      <div class="compact-bar"></div>
    </div>
    <div class="compact-elapsed">{$t('settings.compactDbElapsed', { values: { time: elapsed } })}</div>
  </div>
</div>

<style>
  .dialog-backdrop {
    position: fixed;
    inset: 0;
    z-index: 10000; /* above ConfirmDialog & co. (9999): nothing behind it may take input meanwhile */
    background: rgba(0, 0, 0, 0.55);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .dialog-box {
    background: #2e2e2e;
    border: 1px solid rgba(55, 168, 219, 0.45);
    border-radius: 8px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
    padding: 20px 24px 16px;
    box-sizing: border-box;
    width: min(420px, calc(100% - 32px));
  }

  .dialog-title {
    font-size: 14px;
    font-weight: 700;
    color: #e0e0e0;
    margin-bottom: 10px;
  }

  .dialog-message {
    font-size: 12px;
    color: #bbb;
    line-height: 1.5;
    margin-bottom: 14px;
  }

  .compact-track {
    position: relative;
    height: 8px;
    background: #434343;
    border-radius: 4px;
    overflow: hidden;
  }

  /* Indeterminate: a segment sweeping across the track (transform only — no layout per frame). The
     translate is relative to the segment's own width: -100% starts it left of the track, 334% (a bit
     more than 100 / 30) ends it right of it. */
  .compact-bar {
    position: absolute;
    top: 0;
    bottom: 0;
    left: 0;
    width: 30%;
    background: #37a8db;
    border-radius: 4px;
    animation: compact-sweep 1.4s ease-in-out infinite;
  }

  @keyframes compact-sweep {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(334%);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .compact-bar {
      animation-duration: 4s;
    }
  }

  .compact-elapsed {
    margin-top: 6px;
    font-size: 11px;
    color: #949494;
    min-height: 14px;
    font-variant-numeric: tabular-nums;
  }
</style>
