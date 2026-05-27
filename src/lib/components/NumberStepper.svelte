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
    onchange?: (e: Event) => void;
  } = $props();

  function handleBtnClick(dir: 1 | -1) {
    if (disabled) return;
    let newVal = value + dir * step;
    newVal = Math.max(min, Math.min(max, newVal));
    // Round to sensible decimal precision
    if (decimals !== undefined) {
      newVal = Math.round(newVal * 10 ** decimals) / 10 ** decimals;
    }
    value = newVal;
    // Dispatch change event so parent can react to the change
    onchange?.(new Event('change', { bubbles: true }));
  }

  function handleInput(e: Event) {
    const target = e.target as HTMLInputElement;
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
<div class="ns-stepper" class:ns-disabled={disabled}>
  <button class="ns-btn ns-btn-minus" onclick={() => handleBtnClick(-1)} disabled={disabled} aria-label="-">−</button>
  <input
    type="number"
    class="ns-input"
    bind:value={value}
    {min}
    {max}
    {step}
    {disabled}
    aria-label={label || undefined}
    onchange={(e) => { handleInput(e); onchange?.(e); }}
  />
  <button class="ns-btn ns-btn-plus" onclick={() => handleBtnClick(1)} disabled={disabled} aria-label="+">+</button>
  {#if unit}
    <span class="ns-unit">{unit}</span>
  {/if}
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

  .ns-input {
    padding: 3px 4px;
    background: #434343;
    border: none;
    border-left: 1px solid #555;
    border-right: 1px solid #555;
    color: #e0e0e0;
    font-size: 11px;
    width: 52px;
    text-align: center;
    color-scheme: dark;
    appearance: textfield;
    -moz-appearance: textfield;
    outline: none;
    min-height: 22px;
  }

  .ns-input:focus {
    border-color: #37a8db;
  }

  .ns-input::-webkit-inner-spin-button,
  .ns-input::-webkit-outer-spin-button {
    -webkit-appearance: none;
    margin: 0;
  }

  .ns-unit {
    font-size: 11px;
    color: #888;
    margin-left: 4px;
    align-self: center;
    white-space: nowrap;
  }
</style>
