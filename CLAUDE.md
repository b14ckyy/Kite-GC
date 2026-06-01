# CLAUDE.md — Kite Ground Control (INAV GCS)

Project instructions for Claude Code. Loaded automatically every session.

## Communication
- The user writes in **German** — **respond in German**. All code, comments, commit
  messages and docs in **English**.
- The user does **not** author code (C/embedded background — reads code, doesn't write it).
  Claude writes all code based on instructions.

## Git & workflow
- **NEVER commit or push unless the user explicitly says to** (after testing). Always wait
  for confirmation before any git operation.
- **Dual-commit**: commit docs **before** code, as **two separate commits** (docs commit
  first, then the code commit).
- End every commit message with:
  `Co-Authored-By: Claude Opus 4.8`
- Commit directly on the default branch (`master`) — no feature branch unless asked.
- Multi-line commit messages: pipe a **bash heredoc** to `git commit -F -`. (The PowerShell
  `@'...'@` here-string does **not** work in the Bash tool — it injects stray `@` lines.)

## Pre-delivery self-review (every time, before presenting work)
1. `npm run check` (svelte-check) passes with **0 errors**; run `npm run build` for
   substantial changes. `cargo check` after significant Rust changes.
2. All new user-visible strings use `$t()` with keys in **both** `en.json` + `de.json`.
3. No TypeScript `any` (use `// @ts-expect-error` + rationale if `$t()` types are too strict).
4. No hardcoded UI text left in templates.

## Tech stack (locked)
- **Tauri 2.0** — Rust backend + **Svelte 5 / SvelteKit / TypeScript** frontend
- Maps: **Leaflet** (2D), **CesiumJS** (3D)
- Backend: Rust, `rusqlite` (bundled), `serialport`
- i18n: `svelte-i18n` (ICU Message Format) — `en.json`, `de.json`
- Run: `npx tauri dev` / `cargo tauri dev`

## Svelte 5 — mandatory
- **Runes only**: `$state`, `$derived`, `$effect`, `$props()`, `$bindable()`
- **NEVER** legacy Svelte 4: no `export let`, no `$:`, no `on:click`
- Events: `onclick={handler}`. Props: `let { a, b } = $props()`

## Frontend architecture (ADR-009)
- `+page.svelte` = **thin orchestrator only** — no inline UI blocks, utility functions or
  heavy CSS. Extract a component at ~100 lines or a distinct responsibility.
- `controllers/` domain logic (no UI) · `adapters/` data transforms · `helpers/` pure
  functions · `stores/` shared reactive state · `config/` data (map providers, widget
  registry) · `utils/` generic (geo, units) · `i18n/` setup + locale JSON
- Components are self-contained via `$props()` — avoid direct store access unless needed.

## CSS & theming (INAV Configurator dark theme)
- Accent `#37a8db` · body `#3d3f3e` · panels `#2e2e2e` · borders `#272727` · muted
  `#949494` · success `#59aa29` · error `#d40000`
- Font `'Segoe UI', Tahoma, sans-serif`; `color-scheme: dark` on the root element
- CSS variables where practical; widget content sized in `vmin` (no fixed px);
  glassmorphism panels (`backdrop-filter: blur()` + semi-transparent backgrounds)

## Rust backend
- `eprintln!()` liberally — keep debug prints in code, do **not** remove after fixing
- One feature = one module folder (`msp/`, `flightlog/`, `mission/`, `transport/`, `scheduler/`)
- DB migrations: incremental `PRAGMA user_version` chain — **never modify earlier migrations**
- Dev-only code: `#[cfg(debug_assertions)]` with zero-cost no-op stubs for release
- Tauri commands return `Result<T, String>`; `anyhow`/custom errors internally; emit events
  with the **same names regardless of protocol** (MSP/MAVLink); error/log strings stay English

## Frontend debug logging
- `console.log()` / `console.warn()` liberally during dev; keep it in code; gate dev-only UI
  with `import.meta.env.DEV` (Vite tree-shakes it out of production)

## i18n
- Every user-visible string via `$t('section.subsection.label')` — never inline text
- Update `en.json` **and** `de.json` together; prefer simple keys; when params are needed use
  named placeholders (`{name}`) passed as an object
- Rust backend / log strings remain English

## MSP protocol
- Strict request→response (one request at a time; wait for the reply before the next)
- The scheduler owns the serial connection exclusively (dedicated thread)
- Priority polling: Attitude > Status > Analog > GPS > Secondaries
- Feature gating via `InavVersion` + `Feature` (`msp/features.rs`); min firmware **INAV 7.0.0**

## Documentation
- All docs currently live under `docs/dev/` (everything is dev-facing for now). Keep
  `docs/dev/ROADMAP.md` and `docs/dev/CHANGELOG.md` current; ADRs in `docs/dev/ARCHITECTURE.md`.
- New docs: **propose first** with rationale and wait for OK; keep developer docs under
  `docs/dev/` and create `docs/user/` only once genuinely user-facing docs appear.
- **Never delete a completed feature plan** — when a plan doc's scope is fully shipped, **move
  it to `docs/dev/archive/`** (add a one-line `> ARCHIVED (date) — ...` banner). ADRs capture
  architecture; archived plans preserve the detailed feature-level reasoning for later reuse.

## What NOT to do
- No unrelated changes mid-flow — propose a beneficial refactor as a **separate step** and
  wait for approval
- Don't add docstrings/comments/type annotations to code you didn't change
- No error handling for impossible scenarios; no abstractions for one-time operations
- Don't remove **general-purpose** debug logging (`eprintln!`, `console.log`); temporary
  bug-hunting instrumentation may be removed once the issue is resolved
- No `any` in TypeScript; never deliver code without the final i18n + type-check pass
