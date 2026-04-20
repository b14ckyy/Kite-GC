# Copilot Instructions — Kite Ground Control (INAV GCS)

These rules are loaded automatically for every Copilot session in this workspace.

---

## Communication
- User communicates in **German**, all code/comments/docs in **English**
- User does NOT write code — AI writes all code based on instructions
- User has C/embedded background, reads code but doesn't author it

## Git & Workflow
- **NEVER** commit or push unless the user explicitly says to after testing
- Always wait for user confirmation before any git operation
- Run `npm run -s check` (svelte-check) after significant frontend changes
- Run `cargo check` after significant Rust changes

## Tech Stack (locked)
- **App**: Tauri 2.0 (Rust backend + Svelte 5 frontend)
- **Frontend**: Svelte 5 with TypeScript, SvelteKit
- **Maps**: Leaflet (2D), CesiumJS (3D)
- **Backend**: Rust, rusqlite (bundled), serialport
- **i18n**: svelte-i18n with ICU Message Format (en.json, de.json)
- **Build**: `cargo tauri dev` / `npx tauri dev`

## Svelte 5 — Mandatory Patterns
- Use **runes only**: `$state`, `$derived`, `$effect`, `$props()`, `$bindable()`
- **NEVER** use legacy Svelte 4 syntax (`export let`, `$:`, `on:click`)
- Event handlers: `onclick={handler}` (not `on:click={handler}`)
- Props: `let { prop1, prop2 } = $props()`

## Frontend Architecture (ADR-009)
- **`+page.svelte` = thin orchestrator only** — no inline UI blocks, no utility functions, minimal CSS
- Extract components when they exceed ~100 lines or have distinct responsibility
- Controllers (`src/lib/controllers/`) for domain logic (connection, logbook, playback, widgets)
- Adapters (`src/lib/adapters/`) for data format transformation
- Helpers (`src/lib/helpers/`) for pure utility functions
- Stores (`src/lib/stores/`) for shared reactive state
- Components are self-contained with `$props()` — no direct store access except where needed

## CSS & Theming
- **Dark theme derived from INAV Configurator**:
  - Primary accent: `#37a8db`
  - Body bg: `#3d3f3e`, Panels: `#2e2e2e`
  - Borders: `#272727`, Text muted: `#949494`
  - Success: `#59aa29`, Error: `#d40000`
  - Font: `'Segoe UI', Tahoma, sans-serif`
- Use CSS custom properties / variables where practical
- Widget sizes: viewport-relative (`vmin` units), no fixed pixels for widget content
- Glassmorphism panels: `backdrop-filter: blur()` + semi-transparent backgrounds
- Global `color-scheme: dark` on root element

## Rust Backend
- **Debug logging**: Use `eprintln!()` liberally — keep debug prints in code, do NOT remove after fixing
- Modules: one feature = one module folder (`msp/`, `flightlog/`, `mission/`, `transport/`, `scheduler/`)
- DB migrations: incremental `PRAGMA user_version` chain — never modify earlier migrations
- Dev-only code: `#[cfg(debug_assertions)]` with zero-cost no-op stubs for release
- Error handling: `Result<T, String>` for Tauri commands, `anyhow` or custom errors internally
- Tauri events: emit to frontend, same event names regardless of protocol (MSP/MAVLink)

## Frontend Debug Logging
- Use `console.log()` / `console.warn()` liberally during development
- Keep debug logging in code — do NOT strip after fixing issues
- Dev-only UI: gate with `import.meta.env.DEV` (Vite tree-shakes in production)

## i18n
- All user-visible strings through `$t('key')` — no hardcoded text in components
- Keys follow dot-notation: `section.subsection.label`
- Both `en.json` and `de.json` must be updated together
- Rust backend errors remain English (technical/log strings)

## MSP Protocol
- Strict request-response: one request at a time, wait for reply before next
- Scheduler owns the serial connection exclusively (dedicated thread)
- Priority-based polling: Attitude > Status > Analog > GPS > Secondaries
- Feature gating: `InavVersion` + `Feature` enum in `msp/features.rs`
- Min supported firmware: INAV 7.0.0

## File Organization
- `src/lib/components/` — Svelte UI components
- `src/lib/components/widgets/` — HUD widget components
- `src/lib/controllers/` — domain logic (no UI)
- `src/lib/adapters/` — data transformation layers
- `src/lib/helpers/` — pure functions
- `src/lib/stores/` — Svelte stores (reactive state)
- `src/lib/config/` — configuration data (map providers, widget registry)
- `src/lib/utils/` — generic utilities (geo, units)
- `src/lib/i18n/` — i18n setup + locale JSON files
- `src-tauri/src/` — Rust backend modules
- `docs/` — architecture decisions, roadmap, changelog

## Documentation
- Keep `docs/ROADMAP.md` updated when completing features
- Keep `docs/CHANGELOG.md` updated for notable changes
- ADR format in `docs/ARCHITECTURE.md` for significant decisions
- New docs: **propose first** with rationale, wait for user OK before creating
- Organize docs in subfolders by topic (e.g. `docs/dev/`, `docs/user/`)
- Separate user-facing docs from developer docs for future use

## What NOT to do
- Don't make unrelated changes in the current flow — if a refactor or improvement is clearly beneficial, **propose it as a separate step** and wait for approval
- Don't add docstrings/comments/type annotations to code you didn't change
- Don't add error handling for impossible scenarios
- Don't create abstractions for one-time operations
- Don't remove **general-purpose** debug logging (`eprintln!`, `console.log`) — temporary bug-hunting instrumentation may be removed after the issue is resolved
- Don't use `any` type in TypeScript — use proper types or `unknown`
