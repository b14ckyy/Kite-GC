# Panel Framework

> Status: **Phase 0 done** (2026-06-04). `PanelShell` + the **5** empty variant layouts
> (info / compact / advanced / wide-compact / fullscreen) are built, reviewed and approved via
> the duplicate (bottom) rail group; all variant + panel-switch transitions animate. Next: the
> **Button system** (documented below), then per-panel migration (Phase 1+). No real panel
> wiring yet.

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
area on the left**, the rest is one large content field.

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
  variant="compact"            {/* 'info' | 'compact' | 'advanced' | 'fullscreen' */}
  title="Settings"             {/* optional header title */}
  onClose={...}                {/* optional; shows a header close affordance */}
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

### Button system — **deferred until the panel shell stands**
Defined separately *after* the framework, reduced to a few basic types (rough guideline,
to be detailed then):
- **Mode switch** — toggles a view within a panel (e.g. Logbook ↔ Battery Manager).
- **Compact / inline** — small auto-generated inline links (e.g. the mission-link chips in
  the Logbook flight detail).
- **Standard** — usual actions.
- **Danger** — destructive (delete).
- **Warning** — cautionary actions.
- **Data transfer** — FC Upload / Download, EEPROM, import/export.

One shared definition per type → all panels identical. (Sizes/variants finalised later.)

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
5. **Finish.** When all are migrated, remove the duplicate-rail scaffolding; new panels take
   the original rail positions.

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
- `advanced` = true two-column split, **1 : 2** (left : right, right wider); right region has
  its own framed header + toolbar + content + footer (kept vertically aligned with the left).
- Footer is a **pinned** button area (header + footer fixed, only the framed field scrolls).
- Header is **always present** (its content is panel-defined and may be minimal).

## Transitions
- **All variants are left-anchored and sized by width/height/top** (no `right`), so the shell
  morphs between *any* two variants. The shell **instance persists across rail switches** (no
  `{#key}` remount), so switching panels animates width/height too.
- **First open** uses a horizontal slide-in + fade.
- **`info` (intrinsic `max-content` / `auto`)** animates via `interpolate-size: allow-keywords`
  (Chromium 129+ / WebView2). On an older engine it degrades gracefully (info snaps; everything
  else still animates).

## Open (tune during Phase 0)
- Exact standardised px widths per variant (`info` cap = Settings width; `compact` ≈ Mission
  Editor; `advanced` ≈ Logbook wide ~920).

## Out of scope
Functional behaviour changes — this is a pure structural/visual refactor; panels must behave
identically after migration. New features ride on top later.
