<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<script lang="ts">
  /**
   * NumberStepper.svelte
   * 
   * Reusable stepper control with +/- buttons and a styled number input.
   * Uses the project's dark theme and follows the established stepper pattern.
   * 
   * Usage:
   *   <NumberStepper bind:value={myVar} min={0} max={500} step={5} />
   * 
   * Two-way binding via bind:value, or use onchange for imperative handling.
   *
   * Mouse wheel: once the field has focus (one click into it), the wheel over the control steps the
   * value — up increases, down decreases — and the page does not scroll. Without focus the wheel is
   * left alone, so scrolling a panel never edits a field by accident. Fast turning accelerates: more
   * than 4 notches within a second step 2×, more than 8 step 4× (Marc, 2026-09-14).
   *
   * This component is the ONE stepper of the app — the mission editor popups mount it too
   * (`helpers/missionEditorPopup.svelte.ts`), so a look or behaviour change here lands everywhere.
   */
  let {
    value = $bindable(0),
    min = -Infinity as number,
    max = Infinity as number,
    step = 1,
    label = '',
    unit = '',
    disabled = false,
    decimals,
    placeholder = '',
    allowEmpty = false,
    onchange,
  }: {
    value?: number;
    min?: number;
    max?: number;
    step?: number;
    label?: string;
    unit?: string;
    disabled?: boolean;
    decimals?: number;
    /** Shown when the field is empty (value is NaN, requires allowEmpty). */
    placeholder?: string;
    /** Allow an empty field → value becomes NaN (e.g. "mixed" in batch edit). */
    allowEmpty?: boolean;
    onchange?: (e: Event) => void;
  } = $props();

  function stepBy(steps: number) {
    if (disabled) return;
    const base = Number.isNaN(value) ? (Number.isFinite(min) ? min : 0) : value;
    let newVal = base + steps * step;
    newVal = Math.max(min, Math.min(max, newVal));
    // Round to sensible decimal precision
    if (decimals !== undefined) {
      newVal = Math.round(newVal * 10 ** decimals) / 10 ** decimals;
    }
    value = newVal;
    // Dispatch change event so parent can react to the change
    onchange?.(new Event('change', { bubbles: true }));
  }

  function handleBtnClick(dir: 1 | -1) {
    stepBy(dir);
  }

  // ── Wheel stepping ──────────────────────────────────────────────────────
  // Registered by hand with { passive: false }: Svelte declares wheel handlers passive, and a passive
  // handler cannot preventDefault — which is what keeps the page still and stops the browser's own
  // number-input spin from doubling every notch. (WebView2 spins a focused <input type=number> on the
  // wheel by itself, WebKitGTK does not — this handler is what makes both platforms behave alike.)
  let stepperEl = $state<HTMLDivElement | null>(null);
  let inputEl = $state<HTMLInputElement | null>(null);
  let wheelAccum = 0;           // deltaY since the last notch, so a trackpad's many small events = one notch
  let wheelLast = 0;            // time of the last wheel event, the accumulator resets after a pause
  let notchTimes: number[] = []; // recent notch timestamps, for the acceleration

  function onWheel(e: WheelEvent) {
    if (disabled || !inputEl || document.activeElement !== inputEl) return;   // no focus → the page scrolls
    e.preventDefault();
    const now = performance.now();
    if (now - wheelLast > 200) wheelAccum = 0;
    wheelLast = now;
    // A mouse notch is ~100 px in deltaMode 0; lines / pages (deltaMode 1 / 2) count one notch each.
    const notch = e.deltaMode === 0 ? 50 : 1;
    wheelAccum += e.deltaY;
    if (Math.abs(wheelAccum) < notch) return;
    const dir = wheelAccum < 0 ? 1 : -1;   // wheel up = increase
    wheelAccum = 0;
    notchTimes = notchTimes.filter((t) => now - t < 1000);
    notchTimes.push(now);
    const rate = notchTimes.length;
    stepBy(dir * (rate > 8 ? 4 : rate > 4 ? 2 : 1));
  }

  $effect(() => {
    const el = stepperEl;
    if (!el) return;
    el.addEventListener('wheel', onWheel, { passive: false });
    return () => el.removeEventListener('wheel', onWheel);
  });

  function handleInput(e: Event) {
    const target = e.target as HTMLInputElement;
    if (allowEmpty && target.value.trim() === '') {
      value = NaN;
      return;
    }
    const raw = Number(target.value);
    if (!isNaN(raw)) {
      value = Math.max(min, Math.min(max, raw));
    }
  }
</script>

<div class="ns-wrapper">
{#if label}
  <span class="ns-label">{label}</span>
{/if}
<div class="ns-stepper" class:ns-disabled={disabled} bind:this={stepperEl}>
  <button class="ns-btn ns-btn-minus" onclick={() => handleBtnClick(-1)} disabled={disabled} aria-label="-">−</button>
  <div class="ns-field">
    <input
      type="number"
      class="ns-input"
      bind:this={inputEl}
      bind:value={value}
      {min}
      {max}
      {step}
      {disabled}
      {placeholder}
      aria-label={label || undefined}
      onchange={(e) => { handleInput(e); onchange?.(e); }}
    />
    {#if unit}
      <span class="ns-unit">{unit}</span>
    {/if}
  </div>
  <button class="ns-btn ns-btn-plus" onclick={() => handleBtnClick(1)} disabled={disabled} aria-label="+">+</button>
</div>
</div>

<style>
  .ns-wrapper {
    display: inline-flex;
    flex-direction: column;
    width: fit-content;
  }

  .ns-stepper {
    display: inline-flex;
    align-items: stretch;
    gap: 0;
    border: 1px solid #555;
    border-radius: 4px;
    overflow: hidden;
  }

  .ns-stepper.ns-disabled {
    opacity: 0.5;
    pointer-events: none;
  }

  .ns-label {
    font-size: 12px;
    color: #aaa;
    display: block;
    margin-bottom: 3px;
  }

  .ns-btn {
    background: #333;
    color: #aaa;
    border: none;
    width: 24px;
    cursor: pointer;
    font-size: 14px;
    font-weight: bold;
    line-height: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    user-select: none;
    transition: background 0.1s ease, color 0.1s ease;
    flex-shrink: 0;
  }

  .ns-btn:hover {
    background: #37a8db;
    color: #fff;
  }

  .ns-btn:active {
    background: #2d8ab8;
  }

  .ns-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .ns-btn:disabled:hover {
    background: #333;
    color: #aaa;
  }

  /* Field wrapper: a single bordered cell. The number stays centered; the unit
     is overlaid right-aligned inside the cell, independent of the number. */
  .ns-field {
    position: relative;
    display: block;
    background: #434343;
    border-left: 1px solid #555;
    border-right: 1px solid #555;
  }

  .ns-stepper:focus-within {
    border-color: #37a8db;
  }

  .ns-input {
    display: block;
    padding: 3px 4px;
    background: transparent;
    border: none;
    color: #e0e0e0;
    font-size: 11px;
    width: 74px;
    text-align: center;
    color-scheme: dark;
    appearance: textfield;
    -moz-appearance: textfield;
    outline: none;
    min-height: 22px;
  }

  .ns-input::-webkit-inner-spin-button,
  .ns-input::-webkit-outer-spin-button {
    -webkit-appearance: none;
    margin: 0;
  }

  .ns-unit {
    position: absolute;
    right: 5px;
    top: 50%;
    transform: translateY(-50%);
    font-size: 11px;
    color: #888;
    pointer-events: none;
    white-space: nowrap;
  }
</style>
