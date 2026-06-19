<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // Hold-to-confirm button (control library, see docs/active/VEHICLE_CONTROL.md): fills left→right
  // while pressed; firing `onconfirm` only once the fill completes. Releasing early cancels. Touch-
  // and trackpad-friendly (no drag precision needed). Used for all vehicle-control actions except
  // arming (which uses the slide-to-confirm ArmSlider).
  import type { Snippet } from 'svelte';

  let {
    duration = 1000,
    variant = 'standard',
    disabled = false,
    busy = false,
    title = '',
    onconfirm,
    children,
  }: {
    /** Hold time in ms before firing. */
    duration?: number;
    variant?: 'standard' | 'danger' | 'warning';
    disabled?: boolean;
    /** External busy state (command in flight) — shows a pulse + blocks re-trigger. */
    busy?: boolean;
    title?: string;
    onconfirm: () => void;
    children: Snippet;
  } = $props();

  let progress = $state(0); // 0..1
  let holding = $state(false);
  let rafId: number | null = null;
  let startTs = 0;

  function tick(now: number) {
    if (!holding) return;
    const elapsed = now - startTs;
    progress = Math.min(1, elapsed / duration);
    if (progress >= 1) {
      holding = false;
      progress = 0;
      onconfirm();
      return;
    }
    rafId = requestAnimationFrame(tick);
  }

  function start(e: PointerEvent) {
    if (disabled || busy) return;
    e.preventDefault();
    (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
    holding = true;
    startTs = performance.now();
    rafId = requestAnimationFrame(tick);
  }

  function cancel() {
    holding = false;
    progress = 0;
    if (rafId != null) { cancelAnimationFrame(rafId); rafId = null; }
  }
</script>

<button
  class="htc {variant}"
  class:disabled
  class:busy
  {title}
  disabled={disabled || busy}
  onpointerdown={start}
  onpointerup={cancel}
  onpointerleave={cancel}
  onpointercancel={cancel}
>
  <span class="fill" style="width: {progress * 100}%"></span>
  <span class="label">{@render children()}</span>
</button>

<style>
  .htc {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 100%;
    min-height: 30px;
    padding: 5px 10px;
    border-radius: 5px;
    border: 1px solid var(--htc-border, #37a8db);
    background: var(--htc-bg, rgba(55, 168, 219, 0.12));
    color: var(--htc-fg, #cfe8f4);
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    overflow: hidden;
    user-select: none;
    touch-action: none;
    transition: background-color 0.15s, border-color 0.15s, opacity 0.15s;
  }
  .htc.standard { --htc-border: #37a8db; --htc-bg: rgba(55, 168, 219, 0.12); --htc-fg: #cfe8f4; --htc-fill: rgba(55, 168, 219, 0.55); }
  .htc.warning  { --htc-border: #d4a017; --htc-bg: rgba(212, 160, 23, 0.12); --htc-fg: #f0dca6; --htc-fill: rgba(212, 160, 23, 0.55); }
  .htc.danger   { --htc-border: #d40000; --htc-bg: rgba(212, 0, 0, 0.14);   --htc-fg: #f3b5b5; --htc-fill: rgba(212, 0, 0, 0.6); }

  .htc:hover:not(.disabled):not(.busy) { filter: brightness(1.15); }

  .htc.disabled, .htc.busy {
    opacity: 0.45;
    cursor: not-allowed;
  }
  .htc.busy { animation: htc-pulse 1s ease-in-out infinite; }
  @keyframes htc-pulse { 50% { opacity: 0.7; } }

  .fill {
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    background: var(--htc-fill, rgba(55, 168, 219, 0.55));
    pointer-events: none;
  }

  .label {
    position: relative;
    z-index: 1;
    display: flex;
    align-items: center;
    gap: 6px;
    white-space: nowrap;
  }
</style>
