# AI assistant rules

Ready-to-use rules for an **AI coding assistant** working on **Kite Ground Control**. Written for any
agent (primarily **Claude Code**, but also Cursor, Copilot-style agents, …) — save it under whatever
filename your tool auto-loads (`CLAUDE.md`, `AGENTS.md`, `.cursorrules`, …).

**Stack:** Tauri 2.0 desktop app — Rust backend + Svelte 5 (runes) / SvelteKit / TypeScript frontend;
SQLite via `rusqlite`; i18n via `svelte-i18n`.

> Keep **personal/local** preferences (your language, machine paths, private doc locations, commit
> attribution) in a separate local file. This file is the shared, project-wide ruleset.
>
> For *how the app works* (architecture, data pipeline, protocols, DB schema) read the developer docs
> under `docs/user/for-developers/` and `docs/dev/reference/` — **don't** put that knowledge here.

## Operating rules
- **Never** run `git commit`/`push` — or any hard-to-reverse / outward-facing action — unless explicitly
  asked. Wait for confirmation.
- Commit on the default branch unless a branch is requested. For multi-line messages, pipe a heredoc to
  `git commit -F -` (don't rely on shell here-strings that inject stray characters).
- **Stay in scope.** Do only what was asked; propose unrelated refactors as a separate step and wait for
  approval.
- Don't add comments / docstrings / type annotations to code you didn't change.
- Match the surrounding code's style, naming and comment density.
- A change isn't done until it passes the **quality gate** below.

## Quality gate — run before presenting any change
- `npm run check` passes with **0 errors** (also `npm run build` for substantial changes; `cargo check`
  after Rust changes).
- No TypeScript `any` (only `// @ts-expect-error` + a reason if a library's types force it).
- Every user-visible string uses `$t()` with an `en.json` key; no hardcoded UI text left in templates.

## Frontend (Svelte 5 + TypeScript)
- **Runes only:** `$state`, `$derived`, `$effect`, `$props()`, `$bindable()`. **Never** legacy Svelte 4
  (`export let`, `$:`, `on:click`) — use `onclick={…}` and `let { a } = $props()`.
- Keep `+page.svelte` a **thin orchestrator**; extract a component at ~100 lines or a distinct
  responsibility.
- Respect the layering: `controllers/` (domain logic, no UI) · `adapters/` (data transforms) ·
  `helpers/` (pure functions) · `stores/` (shared state) · `config/` (data tables) · `utils/` (generic) ·
  `components/` (self-contained via `$props()`).
- **Reuse the shared UI framework** (buttons, panels, toggles, the numeric stepper) and the theme tokens
  — never hand-roll buttons or inputs. Missing a capability? Extend the shared component.

## Backend (Rust)
- One feature = one module folder. Tauri commands return `Result<T, String>` (use `anyhow`/custom errors
  internally). Emit events with the **same names regardless of protocol**. Log/error strings stay English.
- DB changes are **incremental `PRAGMA user_version` migrations** — never modify an existing migration.
- **Logging by level:** `warn!` = recoverable problem or a diagnostic that must show at the default level;
  `info!` = milestones; `debug!` = verbose opt-in detail. `eprintln!` is dev-only/temporary (not captured
  in the release log) — remove or downgrade once the issue is resolved. Apply incrementally; flag missing
  convention-logging and get sign-off before adding it.
- Gate dev-only code with `#[cfg(debug_assertions)]` (zero-cost no-op stubs in release).

## i18n
- `en.json` is the source of truth and is **required** for every new/changed string. Other locales are
  optional but welcome (AI-assisted top-ups encouraged); missing keys fall back gracefully. Use named
  placeholders (`{name}`).

## Debugging & diagnostics
- **The level-based file log is the most important diagnostic.** Backend diagnostics go through the `log`
  facade and are captured to a user-gated log file (Settings → Diagnostics), so a tester or user can send
  it when something goes wrong. Treat it as a first-class feature, not an afterthought.
- **Proactively add sensible, missing log output** while you work: when a feature has tester-relevant
  state, milestones or failure paths that aren't logged, add them at the right level (`warn!` for problems
  / default-visible diagnostics, `info!` for milestones, `debug!` for verbose detail). Apply incrementally
  — don't mass-overhaul, but don't leave a new failure path silent either.
- `console.log` / `console.warn` are **dev-time only** — keep them and gate dev-only UI with
  `import.meta.env.DEV`, but they are **not** in the user's file log, so prefer the `log` facade for
  anything a user might need to report.
- **In-app Debug Monitor** (`DebugPanel.svelte`) — a multi-tab live diagnostics surface toggled from the
  status bar, available in dev builds **and** in release builds started with `--debug` (so testers can use
  it too, gated by the runtime `isDebugMode` flag). Existing tabs: MSP, MAVLink, Telemetry, Alerts, RC and
  3D Performance, plus tools like a BLE GATT explorer and dev GPS injection.
  - **When a feature has internal numbers or state worth inspecting against live data** — computed
    geometry, alert/CPA math, scheduler timing, derived state, link stats — **add a new tab (or extend an
    existing one)** rather than relying only on `console`, so both you and the user can watch it live.
  - Keep each tab read-only/observational unless a dev-only injection (like the GPS one) is the point;
    load values when the tab opens and don't add overhead while it's closed.

## Documentation
- Working dev docs live in `docs/dev/` (roadmap, `active/` / `future/` / `archive/` plans, `reference/`);
  published docs in `docs/user/` (the MkDocs site, including the *For developers* guide).
- Propose new docs before creating them. **Never delete a completed plan** — move it to `archive/` with an
  `ARCHIVED (date)` banner. Code-verify every documented fact; never guess.

## Never
Commit/push unasked · change things outside the task · introduce `any` · leave hardcoded UI text · add
error handling for impossible cases · build abstractions for one-time use · remove general-purpose debug
logging · deliver without passing the quality gate.
