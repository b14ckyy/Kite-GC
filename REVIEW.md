# Review guidelines for Kite Ground Control

Instructions for automated code review (Qodo `REVIEW.md`) and for human reviewers alike. The same
file lives on every core branch (`master`, `development`, `release/**`) and is kept identical.

## Project context

Kite Ground Control is a desktop/mobile ground-control station for INAV, ArduPilot and PX4 aircraft:
Tauri 2 with a Rust backend (`src-tauri/`) and a Svelte 5 / SvelteKit / TypeScript frontend (`src/`).
It is a GPL-3.0-or-later desktop app that talks to the user's own aircraft over serial, Bluetooth, TCP
or UDP — not a networked service.

## Review depth

- A PR touching `src-tauri/src/msp`, `mavlink_proto`, `scheduler`, `transport`, `flightlog` or `mission`
  deserves the deepest pass — these talk to live aircraft and to the flight database.
- Docs-only PRs (`docs/user/**`), locale-only PRs (`src/lib/i18n/locales/**`) and pure tooling PRs
  (`tools/**`, `.github/**`) need a light pass.

## Out of scope

- `src/lib/i18n/locales/*.json` — translations; text that is not English is expected there
- `tools/simulators/**` — bench tooling, not shipped
- `docs/user/**` — reviewed for accuracy against the code, not for style
- Merge-up PRs (`merge/*` source branches) and snapshot PRs (`chore/snapshot-*`) carry only
  already-reviewed commits

## What to look for

Focus on real defects:

- wrong protocol framing or field scaling; unit mix-ups (m vs cm, deg vs 1e-7 deg, V vs 0.01 V)
- off-by-one in buffers, lock-order and deadlock risks
- panics or `unwrap` on data the aircraft sent — everything from the FC is untrusted input and may be
  truncated or out of range
- state that wrongly survives a disconnect/reconnect, or is lost across it
- Svelte `$effect` blocks that synchronously write the state they read (self-write loop)
- flight database: any edit to an existing migration step, a schema change without a new
  `PRAGMA user_version` step, or a write path that can leave a `.ktmp` session or the main DB
  inconsistent on a crash

Protocol facts to review against:

- MSP is strict request→response: one request in flight, the scheduler owns the serial connection.
- MAVLink messages are addressed per link and system id.
- The unified GPS fix scale is `0 none / 1 2D / 2 3D / 3 DGPS-RTK` on every protocol and decoder.
- Frontend events keep the same name regardless of protocol (MSP/MAVLink).

## Do not flag

- `console.log` / `console.warn` / `eprintln!` / `log::debug!` lines — debug logging is kept on purpose
- missing error handling for impossible states
- missing docstrings or type annotations on code the PR did not change
- the absence of unit tests for UI code
- "secure error handling", input sanitisation against a malicious FC, or secrets management — this is a
  desktop app talking to the user's own aircraft
- `Co-Authored-By` trailers in commits

## Rules

Report a violation only when the diff shows it.

### Svelte / TypeScript

- Svelte 5 runes only: `$state`, `$derived`, `$effect`, `$props()`, `$bindable()`. No `export let`,
  no `$:` reactive statements, no `on:click` — events are `onclick={handler}`.
- No TypeScript `any`. A `// @ts-expect-error` needs a one-line rationale.
- Every user-visible string goes through `$t('key')`; keys are added to BOTH
  `src/lib/i18n/locales/en.json` and `de.json` (fr/bg/zh may lag). No hard-coded UI text in templates.
- `src/routes/+page.svelte` is a thin orchestrator: no new inline UI blocks, utility functions or large
  CSS there — extract a component under `src/lib/components/`.
- Numeric inputs use the project's `NumberStepper` component, 0/1 switches the `Toggle` component,
  enums a `<select>`.
- CSS: never a bare `backdrop-filter: blur(...)`. Map elements use `var(--glass-blur, blur(6px))`,
  panel shells and their popups `var(--panel-blur, blur(10px))`. Dark-theme palette only
  (accent `#37a8db`, body `#3d3f3e`, panels `#2e2e2e`, borders `#272727`, muted `#949494`).

### Rust backend (`src-tauri/`)

- Tauri commands return `Result<T, String>`; internal errors via `anyhow` or custom types; error and
  log strings in English.
- Logging goes through the `log` facade: `warn` = recoverable problem or tester-relevant default-level
  diagnostic, `info` = milestone, `debug` = verbose detail. `eprintln!` is dev-only. Do not demand that
  existing `eprintln!` / debug lines be removed.
- Dev-only code is gated with `#[cfg(debug_assertions)]` and has a no-op stub for release.
- Database migrations: never edit an earlier migration; a schema change is a new `user_version` step
  appended to the chain.
- One feature = one module folder (`msp/`, `mavlink_proto/`, `flightlog/`, `mission/`, `transport/`,
  `scheduler/`).
- Minimum INAV firmware is 7.0.0; new MSP commands are gated through `msp/features.rs`
  (`InavVersion` + `Feature`).

### Protocol-agnostic layers

- The multi-vehicle / fleet layer (vehicle registry, recorders, group flights) must not assume MAVLink;
  INAV over MSP gets the same hooks later. Flag MAVLink-only types leaking into those modules.
- Mixed firmware families on one link are not supported; code must not try to handle it.

### Project conventions

- Every new source file carries the SPDX header `GPL-3.0-or-later` and the author's own copyright line.
- A fix commit / PR body states in one line whether it is `Affects: <released version> and earlier` or
  `Regression: introduced by <sha>, never released`.
- Base branch: features and refactors target `development`; a fix for a released version targets the
  OLDEST affected `release/<x.y>.x` branch and is merged up afterwards — never cherry-picked. PRs
  against `master` are snapshots or docs-only.
- A `release/**` branch takes user-facing bug fixes and docs only — no CI, tooling, test-only or
  refactor changes.
- Public user docs live under `docs/user/` (MkDocs Material) and must match the code exactly; a feature
  PR that changes user-visible behaviour touches the matching page or says why not.
- Code, comments, commit messages and docs are English.
