# UI framework & theme

Kite has a small set of **shared UI building blocks** and a fixed **dark theme**. Using them is a hard
rule, not a suggestion: it keeps every panel visually consistent, keeps behaviour (sizing, scaling,
accessibility, transitions) correct for free, and avoids a drift of one-off buttons and inputs.

!!! warning "Reuse, don't reinvent"
    Don't hand-roll buttons, switches, numeric inputs or panel frames. If a shared component is missing a
    capability, **extend the shared component** rather than building a local variant.

There's a live **`PanelPlayground`** component that renders the framework pieces together — handy as a
reference while building UI.

## Buttons — `panel/Button.svelte`

The single button for the whole app. Pick a **variant** by meaning, not by colour:

| Variant | Use for |
|---|---|
| `standard` | default actions |
| `mode` | mode/selection toggles |
| `data` | the primary "connect / go" action |
| `warning` | caution actions (e.g. connecting state) |
| `danger` | destructive / disconnect actions |
| `compact` | tight toolbar contexts |

- Sizes: `sm` / `md`. `disabled` supported. Content is a snippet (`{#snippet}` / children).
- **Icons** come from a named registry in `Button.svelte` (`icon="export"`, …) or an inline SVG string;
  they're monochrome via `currentColor`, so they follow the button's text colour. Add new glyphs to the
  registry rather than embedding ad-hoc SVGs.

```svelte
<Button variant="data" onclick={connect}>{$t('connection.connect')}</Button>
<Button variant="danger" size="sm" icon="trash" onclick={remove} />
```

## Toggles

- **`panel/Toggle.svelte`** — an on/off slide switch for a **boolean / 0–1** setting. `checked` is
  bindable; `onchange` gets the new boolean. Props: `checked`, `disabled`, `id`, `title`.
  ```svelte
  <Toggle bind:checked={settings.showSafehomes} onchange={save} />
  ```
- **`panel/SegmentedToggle.svelte`** — a multi-position switch rendered as **one** element with a sliding
  highlight, for a small **enum / mode** choice. Props: `options: {value,label,icon?}[]`, `value`,
  `onchange(value)`, `size`, `full`, `disabled`. Segments size to their content (long labels like
  "MAVLink" never truncate).
  ```svelte
  <SegmentedToggle
    options={[{value:'msp',label:'MSP'},{value:'mavlink',label:'MAVLink'}]}
    value={protocol} onchange={(v) => protocol = v} />
  ```

**Rule of thumb:** boolean → `Toggle`; a few mutually exclusive options → `SegmentedToggle`; many options
→ a styled `<select>`.

## Numeric input — `NumberStepper.svelte`

**Always** use `NumberStepper` for numbers — never a raw `<input type="number">`. It has +/- buttons,
clamping, sensible rounding and the dark-theme styling. Props: `value` (bindable), `min`, `max`, `step`,
`label`, `disabled`, `placeholder`, `allowEmpty` (empty → `NaN`, for "mixed" in batch edit), `onchange`.

```svelte
<NumberStepper bind:value={radiusM} min={0} max={5000} step={10} label="Radius (m)" />
```

## Panel frame — `panel/PanelShell.svelte`

The reusable frame for app panels. The shell owns the frame, positioning, per-variant sizing, the
scroll/bounding of the content field and the live variant transition — callers only fill the snippet
slots. Variants: `info` · `compact` · `wide-compact` · `advanced` · `fullscreen`. It lives inside the
`.ui-scale` wrapper, so it scales with `--ui-scale` automatically.

## Safety-gesture controls

For destructive or arming actions, use the deliberate-gesture controls instead of a plain button:

- **`panel/HoldToConfirm.svelte`** — press-and-hold to confirm.
- **`panel/ArmSlider.svelte`** — slide-to-arm.

## Design theme

Kite follows the **INAV Configurator dark theme**. The root element is `color-scheme: dark`; the palette:

| Token | Hex | Role |
|---|---|---|
| Accent | `#37a8db` | primary accent / focus / links |
| Body | `#3d3f3e` | app background |
| Panels | `#2e2e2e` | panel / surface background |
| Borders | `#272727` | borders / dividers |
| Muted | `#949494` | secondary text |
| Success | `#59aa29` | ok / active |
| Error | `#d40000` | error / danger |

Plus the warm accent `#f5a623` (orange) used for warnings/highlights and in the brand mark.

- **Font:** `'Segoe UI', Tahoma, sans-serif`.
- **Sizing:** widget content is sized in **`vmin`** (no fixed px) so it scales with the window; the whole
  chrome scales with the global **`--ui-scale`** variable (the `.ui-scale` wrapper).
- **Panels** use glassmorphism where appropriate — `backdrop-filter: blur()` over a semi-transparent
  background.
- Use CSS variables where practical; otherwise the palette hex values above are the source of truth.

## When something's missing

If you need a control the framework doesn't have, extend the shared component (or add a new one under
`components/panel/`) so the whole app benefits — and update this page. Don't fork a private copy.
