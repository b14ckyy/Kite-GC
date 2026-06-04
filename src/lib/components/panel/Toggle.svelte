<script lang="ts">
  // Reusable on/off slide switch (see docs/dev/PANEL_FRAMEWORK.md), centralised from the
  // settings panel's repeated `.toggle-switch` markup. `checked` is bindable; `onchange` gets
  // the new boolean.
  let {
    checked = $bindable(false),
    disabled = false,
    id = undefined,
    title = '',
    onchange = undefined,
  }: {
    checked?: boolean;
    disabled?: boolean;
    id?: string;
    title?: string;
    onchange?: (checked: boolean) => void;
  } = $props();

  function handle(e: Event) {
    const c = (e.currentTarget as HTMLInputElement).checked;
    checked = c;
    onchange?.(c);
  }
</script>

<label class="toggle-switch" class:disabled {title}>
  <input type="checkbox" {id} {checked} {disabled} onchange={handle} />
  <span class="toggle-slider"></span>
</label>

<style>
  .toggle-switch {
    position: relative;
    display: inline-block;
    width: 36px;
    height: 20px;
    flex-shrink: 0;
  }
  .toggle-switch.disabled { opacity: 0.45; }

  .toggle-switch input {
    opacity: 0;
    width: 0;
    height: 0;
  }

  .toggle-slider {
    position: absolute;
    cursor: pointer;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background-color: #434343;
    border: 1px solid #555;
    border-radius: 20px;
    transition: background-color 0.2s;
  }
  .toggle-switch.disabled .toggle-slider { cursor: default; }

  .toggle-slider::before {
    content: '';
    position: absolute;
    height: 14px;
    width: 14px;
    left: 2px;
    bottom: 2px;
    background-color: #949494;
    border-radius: 50%;
    transition: transform 0.2s, background-color 0.2s;
  }

  .toggle-switch input:checked + .toggle-slider {
    background-color: rgba(55, 168, 219, 0.3);
    border-color: #37a8db;
  }
  .toggle-switch input:checked + .toggle-slider::before {
    transform: translateX(16px);
    background-color: #37a8db;
  }
</style>
