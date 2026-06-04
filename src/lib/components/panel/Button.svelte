<script lang="ts" module>
  export type ButtonVariant = 'standard' | 'mode' | 'data' | 'danger' | 'warning' | 'compact';
  export type ButtonSize = 'sm' | 'md';

  // Flat SVG button icons, defined once here for reuse (monochrome via currentColor → they
  // follow the button's text colour). Stored as inner markup; wrapped by a common <svg> below.
  // Add new ones here; reference by name via the `icon` prop.
  const ICONS = {
    upload: '<path d="M12 15V4M8 8l4-4 4 4"/><path d="M5 20h14"/>',
    download: '<path d="M12 4v11M8 11l4 4 4-4"/><path d="M5 20h14"/>',
    import: '<path d="M12 3v10M8 9l4 4 4-4"/><path d="M5 15v4a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-4"/>',
    export: '<path d="M12 13V3M8 7l4-4 4 4"/><path d="M5 15v4a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-4"/>',
    save: '<path d="M5 4h11l3 3v13H5V4Z"/><path d="M8 4v5h7V4M8 20v-6h8v6"/>',
    delete: '<path d="M5 7h14M10 4h4M6 7l1 13h10l1-13M10 11v6M14 11v6"/>',
    edit: '<path d="M4 20h4L20 8l-4-4L4 16v4Z"/><path d="M14 6l4 4"/>',
    add: '<path d="M12 5v14M5 12h14"/>',
    close: '<path d="M6 6l12 12M18 6L6 18"/>',
    check: '<path d="M5 13l4 4L19 6"/>',
    refresh: '<path d="M21 12a9 9 0 1 1-2.64-6.36"/><path d="M21 3v5h-5"/>',
    warning: '<path d="M12 3.5 21.5 20H2.5L12 3.5Z"/><path d="M12 10v4.5"/><path d="M12 17.5h.01"/>',
    battery: '<g fill="currentColor" stroke="none"><rect x="9.5" y="2.5" width="5" height="2.2" rx="0.6"/><rect x="6.5" y="4.5" width="11" height="17" rx="2.2"/></g>',
    undo: '<path d="M3 8h12a5 5 0 0 1 0 10h-5"/><path d="M7 4 3 8l4 4"/>',
    redo: '<path d="M21 8H9a5 5 0 0 0 0 10h5"/><path d="M17 4l4 4-4 4"/>',
    library: '<rect x="4" y="4" width="14" height="16" rx="1.5"/><path d="M8 4v16"/><path d="M11 8.5h4M11 12h4"/>',
    map: '<path d="M9 4 3.5 6v14L9 18l6 2 5.5-2V4L15 6 9 4Z"/><path d="M9 4v14M15 6v14"/>',
    folder: '<path d="M4 6h5l2 2h9v10a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V6Z"/>',
  };
  export type ButtonIcon = keyof typeof ICONS;

  /** Full inline <svg> for a registry icon — shared with other panel controls (e.g.
   *  SegmentedToggle) so all flat icons stay defined in one place. */
  export function iconSvg(name: ButtonIcon): string {
    return `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">${ICONS[name]}</svg>`;
  }
</script>

<script lang="ts">
  import type { Snippet } from 'svelte';

  // Shared button for the whole app (see docs/dev/PANEL_FRAMEWORK.md). One definition per type
  // so every panel looks identical. `icon` may be a glyph or an inline SVG string.
  let {
    variant = 'standard',
    size = 'md',
    icon = undefined,
    active = false,
    disabled = false,
    full = false,
    title = '',
    onclick = undefined,
    children = undefined,
  }: {
    variant?: ButtonVariant;
    size?: ButtonSize;
    /** Toggled state for `mode` (view switch) buttons. */
    active?: boolean;
    /** Flat SVG icon name (registry in this file's module script). */
    icon?: ButtonIcon;
    disabled?: boolean;
    /** Stretch to fill the row (flex: 1). */
    full?: boolean;
    title?: string;
    onclick?: (e: MouseEvent) => void;
    children?: Snippet;
  } = $props();
</script>

<button
  type="button"
  class="kbtn kbtn-{variant} kbtn-{size}"
  class:active
  class:full
  {disabled}
  {title}
  {onclick}
>
  {#if icon && ICONS[icon]}
    <span class="kbtn-icon">{@html iconSvg(icon)}</span>
  {/if}
  {#if children}<span class="kbtn-label">{@render children()}</span>{/if}
</button>

<style>
  .kbtn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    border: 1px solid transparent;
    border-radius: 4px;
    cursor: pointer;
    font-family: inherit;
    line-height: 1.2;
    white-space: nowrap;
    transition: background 0.15s, border-color 0.15s, color 0.15s;
  }
  .kbtn.full { flex: 1; width: 100%; }
  .kbtn:disabled { opacity: 0.45; cursor: default; }
  .kbtn-icon { display: inline-flex; flex-shrink: 0; }
  .kbtn-icon :global(svg) { width: 1.2em; height: 1.2em; display: block; }

  /* ── Sizes — fixed height so buttons align regardless of icon/label; px scales with
     --ui-scale. Width stays dynamic (content / translations). ──── */
  .kbtn-md { font-size: 12px; height: 28px; padding: 0 10px; }
  .kbtn-sm { font-size: 11px; height: 24px; padding: 0 8px; }

  /* ── Standard (neutral, the usual action) ───────────────── */
  .kbtn-standard {
    background: #434343;
    border-color: #555;
    color: #ccc;
  }
  .kbtn-standard:hover:not(:disabled) {
    background: rgba(55, 168, 219, 0.18);
    border-color: rgba(55, 168, 219, 0.5);
    color: #e0e0e0;
  }

  /* ── Mode switch (toggles a view; `active` = current) ───── */
  .kbtn-mode {
    background: transparent;
    border-color: rgba(55, 168, 219, 0.4);
    color: #9fd4ee;
  }
  .kbtn-mode:hover:not(:disabled) {
    background: rgba(55, 168, 219, 0.12);
    color: #fff;
  }
  .kbtn-mode.active {
    background: rgba(55, 168, 219, 0.22);
    border-color: #37a8db;
    color: #37a8db;
  }

  /* ── Data transfer (FC up/download, EEPROM, import/export) ─ */
  .kbtn-data {
    background: #1a3a5c;
    border-color: #37a8db;
    color: #37a8db;
  }
  .kbtn-data:hover:not(:disabled) {
    background: #224b73;
    color: #5bbce9;
  }

  /* ── Danger (destructive) ───────────────────────────────── */
  .kbtn-danger {
    background: #2a2a2a;
    border-color: #c0392b;
    color: #e74c3c;
  }
  .kbtn-danger:hover:not(:disabled) {
    background: rgba(192, 57, 43, 0.18);
    border-color: #e74c3c;
    color: #ff6b5b;
  }

  /* ── Warning (cautionary) ───────────────────────────────── */
  .kbtn-warning {
    background: #2a2a2a;
    border-color: #b9791b;
    color: #f39c12;
  }
  .kbtn-warning:hover:not(:disabled) {
    background: rgba(243, 156, 18, 0.16);
    border-color: #f39c12;
    color: #ffb340;
  }

  /* ── Compact (slim inline / auto-generated link chips) ──── */
  .kbtn-compact {
    height: 20px;
    padding: 0 8px;
    font-size: 11px;
    font-weight: 600;
    background: rgba(55, 168, 219, 0.12);
    border-color: rgba(55, 168, 219, 0.35);
    color: #9fd4ee;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .kbtn-compact:hover:not(:disabled) {
    background: rgba(55, 168, 219, 0.25);
    border-color: #37a8db;
    color: #fff;
  }

  .kbtn-label { overflow: hidden; text-overflow: ellipsis; }
</style>
