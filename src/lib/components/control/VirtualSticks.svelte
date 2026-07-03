<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)

  On-screen touch RC sticks for iPad (Phase 4). Mode-2 layout:
    left stick  - throttle (vertical, no spring: holds position) + yaw (horizontal, springs to centre)
    right stick - pitch (vertical) + roll (horizontal), both spring to centre
  Feeds mobileRc, which drives the shared rcStream pump. Injecting RC to a real aircraft is
  safety-critical: the sticks are inert until Engage, arming is a separate explicit toggle, and
  releasing a spring axis re-centres it immediately.
-->
<script lang="ts">
  import {
    mobileSticks, mobileArm, mobileRcActive, startMobileRc, stopMobileRc,
  } from '$lib/stores/mobileRc';
  import { onDestroy } from 'svelte';

  // Track the active pointer per stick so multi-touch (both thumbs) works independently.
  let leftPointer = $state<number | null>(null);
  let rightPointer = $state<number | null>(null);
  // Knob offsets in −1..1 for rendering (y already flipped so up = +1).
  let leftKnob = $state({ x: 0, y: -1 });   // yaw / throttle (throttle starts idle = bottom)
  let rightKnob = $state({ x: 0, y: 0 });   // roll / pitch

  function norm(e: PointerEvent, el: HTMLElement): { x: number; y: number } {
    const r = el.getBoundingClientRect();
    const x = ((e.clientX - r.left) / r.width) * 2 - 1;
    const y = ((e.clientY - r.top) / r.height) * 2 - 1;
    // Clamp to the pad and flip Y so screen-up is positive.
    return { x: Math.min(1, Math.max(-1, x)), y: -Math.min(1, Math.max(-1, y)) };
  }

  function leftMove(e: PointerEvent, el: HTMLElement) {
    if (leftPointer !== e.pointerId) return;
    const p = norm(e, el);
    leftKnob = p;
    mobileSticks.update((s) => ({ ...s, yaw: p.x, throttle: p.y })); // throttle holds
  }
  function rightMove(e: PointerEvent, el: HTMLElement) {
    if (rightPointer !== e.pointerId) return;
    const p = norm(e, el);
    rightKnob = p;
    mobileSticks.update((s) => ({ ...s, roll: p.x, pitch: p.y }));
  }

  function leftDown(e: PointerEvent, el: HTMLElement) {
    leftPointer = e.pointerId;
    el.setPointerCapture(e.pointerId);
    leftMove(e, el);
  }
  function rightDown(e: PointerEvent, el: HTMLElement) {
    rightPointer = e.pointerId;
    el.setPointerCapture(e.pointerId);
    rightMove(e, el);
  }

  function leftUp() {
    leftPointer = null;
    // Yaw springs to centre; throttle stays where it was left.
    leftKnob = { ...leftKnob, x: 0 };
    mobileSticks.update((s) => ({ ...s, yaw: 0 }));
  }
  function rightUp() {
    rightPointer = null;
    rightKnob = { x: 0, y: 0 }; // both axes spring to centre
    mobileSticks.update((s) => ({ ...s, roll: 0, pitch: 0 }));
  }

  function toggleEngage() {
    if ($mobileRcActive) stopMobileRc();
    else startMobileRc();
  }

  // Never leave RC streaming if the panel is torn down.
  onDestroy(() => stopMobileRc());
</script>

<div class="vs-root">
  <div class="vs-bar">
    <button class="engage" class:on={$mobileRcActive} onclick={toggleEngage}>
      {$mobileRcActive ? 'Disengage' : 'Engage'}
    </button>
    <button
      class="arm"
      class:armed={$mobileArm}
      disabled={!$mobileRcActive}
      onclick={() => mobileArm.update((a) => !a)}
    >
      {$mobileArm ? 'ARMED' : 'ARM'}
    </button>
    <div class="readout">
      T {Math.round(($mobileSticks.throttle + 1) * 50)}%
      · Y {$mobileSticks.yaw.toFixed(2)}
      · P {$mobileSticks.pitch.toFixed(2)}
      · R {$mobileSticks.roll.toFixed(2)}
    </div>
  </div>

  <div class="vs-pads" class:disabled={!$mobileRcActive}>
    <!-- Left: throttle (vertical) + yaw (horizontal) -->
    <div
      class="pad"
      role="application"
      aria-label="Throttle and yaw"
      onpointerdown={(e) => leftDown(e, e.currentTarget)}
      onpointermove={(e) => leftMove(e, e.currentTarget)}
      onpointerup={leftUp}
      onpointercancel={leftUp}
    >
      <div class="cross"></div>
      <div class="knob" style="left:{(leftKnob.x + 1) * 50}%; top:{(1 - leftKnob.y) * 50}%"></div>
      <span class="lbl tl">T+</span><span class="lbl bl">T-</span>
    </div>

    <!-- Right: pitch (vertical) + roll (horizontal) -->
    <div
      class="pad"
      role="application"
      aria-label="Pitch and roll"
      onpointerdown={(e) => rightDown(e, e.currentTarget)}
      onpointermove={(e) => rightMove(e, e.currentTarget)}
      onpointerup={rightUp}
      onpointercancel={rightUp}
    >
      <div class="cross"></div>
      <div class="knob" style="left:{(rightKnob.x + 1) * 50}%; top:{(1 - rightKnob.y) * 50}%"></div>
    </div>
  </div>
</div>

<style>
  /* Anchored as a full-width bottom overlay so the touch pads get a large, thumb-reachable area
     regardless of the narrow side-panel drawer the tab content normally renders into. */
  .vs-root {
    position: fixed; left: 0; right: 0; bottom: 0; height: 46vh; z-index: 400;
    display: flex; flex-direction: column; gap: 12px; padding: 12px; box-sizing: border-box;
    background: linear-gradient(#0a0a0acc, #0a0a0aee); backdrop-filter: blur(2px);
    border-top: 2px solid #37a8db;
  }
  .vs-bar { display: flex; align-items: center; gap: 12px; }
  .engage, .arm {
    font: 600 15px/1 system-ui; padding: 12px 20px; border-radius: 8px; border: 1px solid #555;
    background: #2e2e2e; color: #eee; cursor: pointer; min-width: 110px; touch-action: manipulation;
  }
  .engage.on { background: #1e6b3a; border-color: #2e9d55; }
  .arm { background: #3a2e2e; }
  .arm.armed { background: #a12; border-color: #e33; color: #fff; }
  .arm:disabled { opacity: 0.4; cursor: not-allowed; }
  .readout { font: 13px/1 ui-monospace, monospace; color: #9fd; opacity: 0.85; }
  .vs-pads { display: flex; flex: 1; gap: 24px; }
  .vs-pads.disabled { opacity: 0.35; pointer-events: none; }
  .pad {
    position: relative; flex: 1; border-radius: 16px; background: #1b1b1b;
    border: 2px solid #37a8db55; touch-action: none; overflow: hidden;
  }
  .cross {
    position: absolute; inset: 0;
    background:
      linear-gradient(#ffffff14, #ffffff14) center / 1px 100% no-repeat,
      linear-gradient(#ffffff14, #ffffff14) center / 100% 1px no-repeat;
  }
  .knob {
    position: absolute; width: 64px; height: 64px; margin: -32px 0 0 -32px; border-radius: 50%;
    background: radial-gradient(circle at 35% 35%, #7cc7ec, #2b7fa8); border: 2px solid #cfe;
    box-shadow: 0 2px 8px #000a; pointer-events: none;
  }
  .lbl { position: absolute; font: 11px/1 system-ui; color: #ffffff66; }
  .lbl.tl { top: 8px; left: 8px; }
  .lbl.bl { bottom: 8px; left: 8px; }
</style>
