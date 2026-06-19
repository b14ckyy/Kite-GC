<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  // Slide-to-confirm control (control library) — the deliberate gesture for arming, modelled on
  // QGroundControl's arm slider. Drag the knob to the far end to fire `onconfirm`; release short of
  // the threshold and it snaps back. See docs/active/VEHICLE_CONTROL.md.
  let {
    label = '',
    variant = 'standard',
    disabled = false,
    busy = false,
    title = '',
    onconfirm,
  }: {
    label?: string;
    variant?: 'standard' | 'danger' | 'warning';
    disabled?: boolean;
    busy?: boolean;
    title?: string;
    onconfirm: () => void;
  } = $props();

  const THRESHOLD = 0.92;
  const KNOB = 30; // px

  let track: HTMLDivElement;
  let dragging = $state(false);
  let pos = $state(0); // 0..1 knob position fraction

  function maxTravel(): number {
    return Math.max(1, (track?.clientWidth ?? 1) - KNOB);
  }

  function start(e: PointerEvent) {
    if (disabled || busy) return;
    e.preventDefault();
    (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
    dragging = true;
  }

  function move(e: PointerEvent) {
    if (!dragging || !track) return;
    const rect = track.getBoundingClientRect();
    const x = e.clientX - rect.left - KNOB / 2;
    pos = Math.min(1, Math.max(0, x / maxTravel()));
  }

  function end() {
    if (!dragging) return;
    dragging = false;
    if (pos >= THRESHOLD) {
      onconfirm();
    }
    pos = 0; // always snap back; the FC's armed state drives any persistent UI
  }
</script>

<div
  class="arm-slider {variant}"
  class:disabled
  class:busy
  class:armed-zone={pos >= THRESHOLD}
  {title}
  bind:this={track}
>
  <span class="hint" style="opacity: {1 - pos * 1.4}">{label}</span>
  <button
    class="knob"
    style="transform: translateX({pos * maxTravel()}px)"
    class:dragging
    disabled={disabled || busy}
    onpointerdown={start}
    onpointermove={move}
    onpointerup={end}
    onpointerleave={end}
    onpointercancel={end}
    aria-label={label}
  >
    <span class="chev">›››</span>
  </button>
</div>

<style>
  .arm-slider {
    position: relative;
    display: flex;
    align-items: center;
    width: 100%;
    height: 34px;
    border-radius: 18px;
    border: 1px solid var(--as-border, #37a8db);
    background: var(--as-bg, rgba(55, 168, 219, 0.1));
    overflow: hidden;
    user-select: none;
    touch-action: none;
  }
  .arm-slider.standard { --as-border: #37a8db; --as-bg: rgba(55, 168, 219, 0.1);  --as-knob: #37a8db; --as-fg: #cfe8f4; }
  .arm-slider.warning  { --as-border: #d4a017; --as-bg: rgba(212, 160, 23, 0.1);  --as-knob: #d4a017; --as-fg: #f0dca6; }
  .arm-slider.danger   { --as-border: #d40000; --as-bg: rgba(212, 0, 0, 0.12);    --as-knob: #d40000; --as-fg: #f3b5b5; }

  .arm-slider.disabled, .arm-slider.busy { opacity: 0.45; }
  .arm-slider.busy { animation: as-pulse 1s ease-in-out infinite; }
  @keyframes as-pulse { 50% { opacity: 0.7; } }

  .arm-slider.armed-zone { background: color-mix(in srgb, var(--as-knob) 30%, transparent); }

  .hint {
    position: absolute;
    width: 100%;
    text-align: center;
    font-size: 11.5px;
    font-weight: 600;
    letter-spacing: 0.3px;
    color: var(--as-fg, #cfe8f4);
    pointer-events: none;
    transition: opacity 0.05s linear;
  }

  .knob {
    position: absolute;
    left: 2px;
    top: 1px;
    width: 30px;
    height: 30px;
    border-radius: 50%;
    border: none;
    background: var(--as-knob, #37a8db);
    color: #11181d;
    cursor: grab;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
  }
  .knob.dragging { cursor: grabbing; }
  .knob:disabled { cursor: not-allowed; }
  .chev { font-size: 12px; font-weight: 800; line-height: 1; }
</style>
