# Panel Framework

> 📦 ARCHIVED (2026-06-21) — Shipped + in daily use for weeks; all 15 nav-rail panels on `PanelShell`, no legacy base remaining (verified). Living reference; architecture in ADR-029.

> Status: **Complete** (2026-06-04). Phase 0 (shell + control library) signed off; **all** nav-rail
> panels migrated onto `PanelShell`, cut over, the legacy panels deleted, and the components renamed
> to their canonical names (see "Cutover done + finalised" below). This doc now serves as the
> **living reference** for the framework + control library (architecture in ADR-029). Per-panel
> migration record:
> - **UAV Info** → `info` variant (content-sized, unframed).
> - **Flight Logbook** → live `info` (minimized card) / `compact` (list) / `advanced` (list +
>   FlightDetail 1:2). All chrome + FlightDetail footer buttons use the `<Button>` library.
> - **Battery Manager** → its **own** framework panel (`BatteryManager`, own `PanelShell`,
>   `compact ↔ advanced` 1:2 split): pack list in the main field, pack detail in the detail
>   field; toolbar = Back · New · Import, pack actions above the linked-flights list, Export in
>   the detail toolbar; delete dialog doubles as Retire / Mark-Damaged. Built **parallel** to the
>   legacy `BatteryManager` (still used by the old logbook) — both deleted at the logbook cutover.
> - **Mission system** → `MissionPanel` (thin switcher) → `InavMissionPanel` /
>   `ArduMissionPanel` (each `compact`, own `PanelShell`; header = title + shared
>   `AutopilotSelect`; toolbar = edit/manager/undo/redo/pattern/clear; field = multi-mission tabs
>   (INAV only) + WP table; footer = selected-WP detail + FC/EEPROM/file controls). The library
>   view is **`MissionManager`** (own shell, `compact ↔ advanced` 1:2, like the Battery Manager),
>   rendered by the INAV editor when opened. Built **parallel** to the legacy mission components.
> - **Terrain Analyzer** → `fullscreen ↔ wide-compact` PanelShell, converted **in place** (the
>   panel is standalone — no shared leaf — so duplicating ~980 lines wasn't worth it; reachable via
>   the existing `terrain` rail button, the redundant `terrain-v2` button was dropped). MSL/AGL +
>   Waypoints/Track + correction mode are `SegmentedToggle`s, Show Map uses the flat `map` icon, and
>   the readouts + hover info moved into the new **fullscreen footer slot** (`ps-fs-foot`). Header
>   actions are left-aligned (fullscreen title is content-width).
> - **Video** → `VideoPanel` (`compact`): header = Start/Stop; content = preview + source /
>   resolution / mirror settings; footer = Floating Window (`mode` button, active = on, left) +
>   Video Window detach (plain button, right — the PiP can't be closed from inside the app).
>   Parallel build.
>
> - **Settings** → `SettingsPanel` (`compact`), reorganised into **two tabs via a `SegmentedToggle`**
>   (Interface / Data), each grouped into labelled subsections:
>   - *Interface*: UI (language, scale) · Map (provider, altitude curtain) · Units (all unit
>     selects) · Widgets (all HUD widget toggles).
>   - *Data*: Map (tile cache + Cesium token) · Telemetry (attitude/GPS rate, airspeed) · Flight
>     Logbook (all logging settings + DB path) · Mission Control (default WP alt, PH time) ·
>     Alerts (altitude).
>   All on/off switches use the shared `Toggle`, selects match the 28px control height, and **all
>   tiny italic hints were dropped except the Cesium-token one** (bumped to a readable size). No
>   footer. The `+page` settings-patch handler is extracted to `applySettingsPatch`.
>
> **Cutover done + finalised.** The regular rail tabs render the framework panels directly; the
> legacy panels (9 components) and the duplicate "v2" rail group were deleted, a persisted `*-v2`
> tab id is normalised back to its base on load, and the components were renamed from `*V2` to
> their canonical names. Cross-links needed no rewiring — the requestOpenFlight/Mission handlers
> set `activeTab` to `logbook`/`mission`, which now resolve to the framework panels. The DEV
> playground stays (dev-only). **The framework migration is complete.**
>
> One small, additive PanelShell behaviour landed: the `advanced` **detail column renders only
> when detail content exists** (`detail`/`detailToolbar`/… or `detailTitle`); with none, the
> main column fills the full width (graceful single-column degrade).
>
> FlightDetail's inline link affordances were migrated to the shared `Button` `compact` variant
> (mission/battery jump chips + their link/unlink/save controls; new `link` registry icon). The
> ✎ edit pencils (craft name / pilot / weather) stay as-is (a different affordance, not link chips).
>
> Possible follow-ups (not blocking): a shared `Field`/`Select` primitive; theming-by-FC.

## Problem
The app has 6 nav-rail panels in 4 recurring formats, but **every panel rolls its own
markup, sizing and buttons** — there is no single source of truth. Evidence: ~116 button
usages across 25 files, each component re-defining its own `.btn-*` classes; `.nav-panel`
width classes hard-coded per panel (360 / 414 / 430 / 920 / 280 px). Result: editing one
panel's layout must be hand-replicated to all, and they still drift (slightly different
sizes/spacing despite being the "same type").

## Goal
A small, reusable **panel framework**: one `PanelShell` (header / toolbar / body / detail /
footer slots + a `variant`) plus a shared **Button system** and **design tokens**. Panels
become "content placed onto the shell" — consistent by construction, drift-proof, and
edited in one place.

## Format taxonomy (the `variant`s) — specs

**`info`** — UAV Info, replay Log info.
- Width **dynamic (content-sized)**, **capped** at the `compact`/Settings fixed width.
- Height **dynamic (content-sized)**.
- Just panel chrome + body; no fixed framed regions.

**`compact`** — Settings, Logbook list (no selection), Mission Planner, Mission Manager
(compact), Camera, Battery list. *Template = the compact Flight Logbook without a selection;
reference dimensions = the current Mission Editor panel.*
- Width **fixed** (≈ Mission Editor, ~414–430 px → standardise one value).
- Height **fills the full panel area** (the widget-safe area), content scrolls inside.
- Three stacked regions:
  1. **Header** — title + general/main-function buttons (if any) + search field, etc.
  2. **Framed content field** — a thin-line-framed box (like the logbook list-entry frame)
     holding all dynamic working content (waypoint list, log list, …); this is what scrolls.
  3. **Footer** — button area (like the Mission editor's bottom buttons).

**`advanced` / wide** — Logbook expanded, Battery expanded, Mission Manager expanded.
*Reference = Flight Logbook wide (~920 px), dynamic full-panel-area height.*
- **Split screen, 1 : 2** (left : right — right wider, for WP/mission previews, maps, charts).
- **Left** = the full `compact` layout (header / framed field / footer).
- **Right** = a **second thin-framed content region with its own header-button area and its
  own bottom-button area**.

**`fullscreen`** — Terrain Analyzer (current implementation is the baseline). *"Fullscreen" =
**almost** full-screen floating overlay*, not edge-to-edge: an even ~62 px inset on all sides
(map + widgets stay visible around it), standard panel top offset. Header top, **parameter
area on the left**, one large content field, and an optional **bottom footer bar** (`footer`
snippet → `ps-fs-foot`, e.g. the terrain readouts + hover info).

**`wide-compact`** — an alternative mode of `fullscreen` (toggle). A short, wide strip docked
below the toolbar that stops before the side widget dock, so the **map stays visible** to
compare data against it. Already used by the Terrain Analyzer's compact mode; will also serve
flight-log analysis. Same internal layout as `fullscreen` (header / params / content);
**Fullscreen ↔ Wide-Compact animates** (height + right).

**Standardise one width per variant** (exact px tuned in Phase 0) — kill the 360/414/430 drift.
Per-panel exceptions only if truly justified, via a documented override.

**Multi-format panels switch `variant` live by state** (Logbook: info ↔ compact ↔ advanced;
Battery / Mission Manager: compact ↔ advanced). The shell must animate these transitions
**elegantly and robustly** (no layout jump). We already have `transition: width 0.25s` on
`.nav-panel` and the logbook minimize/expand flow — generalise and harden that.

## Component API (Svelte 5 runes + snippets)
```svelte
<PanelShell
  variant="compact"            {/* 'info' | 'compact' | 'advanced' | 'fullscreen' | 'wide-compact' */}
  title="Settings"             {/* optional header title */}
>
  {#snippet toolbar()} … {/snippet}   {/* optional action row under the header */}
  {#snippet body()} … {/snippet}      {/* main scrollable content (or default children) */}
  {#snippet detail()} … {/snippet}    {/* second region, only rendered in `advanced` */}
  {#snippet footer()} … {/snippet}    {/* optional pinned footer (primary actions) */}
</PanelShell>
```
- The shell owns: outer frame (glass panel, border, radius, blur), header, the scroll
  container + vertical bounding (the `100%`-of-scaled-container rule from UI scaling), the
  variant width, and the variant transitions. **It scales with `--ui-scale` for free** (it
  lives in `.ui-scale`).
- Slots are **optional** — `info` typically uses only `body`; `advanced` adds `detail`.
- No "panel DSL" / over-abstraction: one shell + variant + snippets covers all four.
- **No in-panel close button.** Every panel is closed via the nav rail's ✕ (the same control
  that opens it) — the shell renders no close affordance, so there's nothing to pass.

### Control library (`src/lib/components/panel/`)
Shared, self-contained controls — one definition each, so every panel looks identical. They
live in `.ui-scale`, so all px sizes scale with `--ui-scale`. (Svelte = one component per file;
non-component code like the icon registry is shared via a component's `module` script.)

**`Button.svelte`** — `<Button variant size icon active disabled full title onclick>`:
- variants: **standard** (neutral), **mode** (view toggle, `active` = current), **data** (FC
  up/download, EEPROM, im/export — blue fill), **danger** (destructive), **warning**
  (cautionary), **compact** (slim inline / auto-generated link chip).
- sizes `sm` / `md` with **fixed height** (md 28 / sm 24 / compact 20 px) so buttons align;
  **width stays dynamic** (content / translations); `full` stretches to fill the row.
- **Flat-SVG icon registry** lives here (`ICONS` + `iconSvg()` + `ButtonIcon` type), monochrome
  via `currentColor`. Icons are optional (no reserved space when absent) and on by default.
  Reused by other controls (e.g. `SegmentedToggle`). Add icons to the one `ICONS` map.

**`SegmentedToggle.svelte`** — `<SegmentedToggle options value onchange size full>`: a small
multi-position slide switch placed as ONE element with a sliding highlight (e.g. Replay:
Recording ↔ Blackbox track). Options may carry a registry icon. **Segments are content-sized**
(each fits its label — no truncation of longer labels like "MAVLink"), and the highlight tracks
the active segment's measured offset/width (so it slides across unequal widths). **`full`** stretches
the toggle to the parent's width and distributes segments evenly (`flex: 1`, with a graceful
ellipsis fallback) — for short-label, fixed-width tab bars (e.g. the Settings Interface/Data tabs).

**`Toggle.svelte`** — `<Toggle checked onchange disabled id title>`: the on/off slide switch,
centralised from the settings panel's repeated `.toggle-switch` markup; `checked` is bindable.

**Form controls** (`<select>`, text inputs) — match the **md button height (28px)**, font 12px,
4px radius, so a toolbar row of selects + buttons aligns with no vertical jog. (Not yet a shared
component; standardised via the panels' `.setting-select` / `.setting-input` until a `Field`
primitive is extracted.)

### Design tokens
Promote the theme constants (currently inline per component) to CSS custom properties on
the root: `--panel-bg`, `--panel-border`, `--accent (#37a8db)`, `--danger (#d40000)`,
`--success (#59aa29)`, `--muted (#949494)`, a spacing scale (`--sp-1…`), radii (`--r-1…`).
Buttons + shell consume these; future theming-by-FC (roadmap) becomes trivial.

## Migration strategy — parallel "strangler" build
The old UI keeps working the entire time; the new panels are built alongside and swapped in
panel-by-panel. (Pre-release, single developer → no need to DEV-gate or hide the scaffolding.)

1. **Duplicate the 6 rail icons** as a second group at the bottom of the nav rail (a
   separator between). Top group = old panels, bottom group = new framework panels. The rail
   is already centralised in one place (`allTabs` + `NavRail.svelte`) — no refactor needed.
   New buttons use a parallel tab id (e.g. `*-v2`) so both can be open/compared.
2. **Phase 0 — framework only, empty panels.** Build `PanelShell` + the 4 variants + Button
   system + tokens. Behind each new rail button, render an **empty placeholder** in its target
   variant, plus throwaway buttons to **switch variant live** (to test info↔compact↔advanced
   transitions). Validate layout, sizing, vertical bounding, UI-scale behaviour, and the
   transitions **before any wiring**.
3. **Phase 1…n — migrate one panel at a time.** Rebuild each panel's markup on the shell and
   move its wiring (props/stores/handlers) over. Reuse the existing controllers/stores
   unchanged (both old + new read the same stores → no conflict). Keep functional content
   transfer **after** the framework is signed off.
4. **Cutover per panel.** When a new panel matches/exceeds the old one, delete the old panel
   + its (top-group) rail button. Don't let duplicates linger.
5. **Finish.** When all are migrated, remove the duplicate (v2) rail group; new panels take
   the original rail positions. **Kept permanently (dev-only):** a `DEV` text button at the end
   of the rail opens `PanelPlayground` (the empty framework reference), gated on
   `import.meta.env.DEV` so it never ships in release builds.

### Suggested order (simple → complex, to harden the API early)
1. UAV Info (`info`) — simplest, exercises the minimal shell.
2. Settings (`compact`) — forms + the new Button/field primitives.
3. Camera (`compact`).
4. Battery (`compact` ↔ `advanced`) — first live variant switch.
5. Logbook (`info` ↔ `compact` ↔ `advanced`) — the 3-format stress test + transitions.
6. Mission Planner + Mission Manager (`compact` ↔ `advanced`) — heaviest wiring.
7. Terrain Analyzer (`fullscreen`) — distinct variant, likely last.

## Shared primitives (extract as needed during migration)
`SectionHeader`, `FieldRow` (label ‖ control), `ListRow` (selectable list item). Build them
when the second panel needs them — not speculatively.

## Decided (from the spec)
- `advanced` = true two-column split with **fixed field widths**: left field **380px**, right
  field **500px** (panel ≈ 914px). Right region has its own framed header + toolbar + content +
  footer (kept vertically aligned with the left). **No vertical divider line** between the
  columns — separation is by spacing only (cleaner). With no detail content the main column
  fills the full width (graceful single-column degrade).
- Footer is a **pinned** button area (header + footer fixed, only the framed field scrolls) —
  *until* the panel gets too short: the field keeps a **200px minimum height**, and when
  header + field + footer no longer fit the available height the **whole column scrolls**
  (header/footer included). `info` is content-sized and exempt from the minimum.
- Header is **always present** (its content is panel-defined and may be minimal).

## Transitions
- **All variants are left-anchored and sized by width/height/top** (no `right`), so the shell
  morphs between *any* two variants. The shell **instance persists across rail switches** (no
  `{#key}` remount), so switching panels animates width/height too.
- **First open** uses a horizontal slide-in + fade.
- **`info` (intrinsic `max-content` / `auto`)** animates via `interpolate-size: allow-keywords`
  (Chromium 129+ / WebView2). On an older engine it degrades gracefully (info snaps; everything
  else still animates).

## Standardised widths (tuned against the legacy layouts)
- **Driven by the field** (the thin-framed working box), not the panel: `compact` main field
  **380px** (panel 398px); `advanced` left field **380px** + right field **500px** (panel 914px).
  `info` is content-sized, capped at 360px (≈ Settings width).

## Out of scope
Functional behaviour changes — this is a pure structural/visual refactor; panels must behave
identically after migration. New features ride on top later.
