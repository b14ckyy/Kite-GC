# Contributing

Thanks for your interest in improving Kite Ground Control! Bug reports, fixes, features, translations and
documentation are all welcome.

## Getting started

1. Read **[Building from source](building.md)** and get a dev build running with `just dev`.
2. For anything non-trivial, **open an issue first** to discuss the approach — it saves rework.
3. Branch, make your change, run the checks, and open a pull request against `master`.

## Before you open a PR

Run the static checks — the project leans on them heavily:

```bash
just check    # svelte-check + TypeScript + cargo check
```

CI runs the same checks (plus clippy) on every push and PR. PRs should be green before review.

## Coding conventions

**Frontend (Svelte 5 / TypeScript)**

- **Runes only** — `$state`, `$derived`, `$effect`, `$props()`, `$bindable()`. No legacy Svelte 4
  (`export let`, `$:`, `on:click`); use `onclick={…}` and `let { a } = $props()`.
- **No `any`** in TypeScript.
- **All user-visible text goes through i18n** — `$t('section.key')`. **`en.json` is mandatory**; other
  locales are optional (see [Internationalisation](#internationalisation) below). Never hard-code UI text.
- **Reuse the shared UI framework** — the `Button`, panel, toggle and stepper components and the theme
  tokens. Don't roll your own buttons/inputs. See **[UI framework & theme](ui-framework.md)**.
- Keep page components thin; extract substantial UI into components.

**Backend (Rust)**

- One feature per module folder; Tauri commands return `Result<T, String>`.
- Database changes are **incremental migrations** (`PRAGMA user_version`) — never modify an existing
  migration.
- Route diagnostics through the `log` facade at the right level; user-facing/error strings stay English.

**Comments & scope**

- Keep changes focused; propose unrelated refactors separately.
- Match the surrounding code's style and comment density.

## Internationalisation

Kite ships in English, German and French. For contributions:

- **`en.json` is the source of truth and is required** — every new or changed UI string must have its
  English key.
- **Other locales (`de.json`, `fr.json`) are optional but very welcome.** Keeping them in sync is
  appreciated; an AI assistant makes this quick and is the recommended way to fill in translations.
- Use **named placeholders** (`{name}`) for parameters, passed as an object.

Missing non-English keys fall back gracefully, so an English-only PR is fine — a maintainer (or you, with
AI help) can top up the other languages afterwards.

## Licensing & contributor terms

Kite Ground Control is licensed under **[GPL-3.0-or-later](https://www.gnu.org/licenses/gpl-3.0.html)**.

- Every source file carries an SPDX header:
  ```
  // SPDX-License-Identifier: GPL-3.0-or-later
  // Copyright (C) 2026 Marc Hoffmann (b14ckyy)
  ```
  Add it to any new source file you create.
- By submitting a contribution you agree it is licensed under the project's GPL-3.0-or-later terms.

!!! note
    The project may later adopt a formal Contributor License Agreement (CLA) to keep relicensing/
    distribution options open. If that happens it will be documented here and in the repository; for now,
    contributions are accepted under the GPL-3.0-or-later license above.

## Reporting bugs & ideas

Use the **[GitHub issue tracker](https://github.com/b14ckyy/Kite-GC/issues)**. For bug reports, the
in-app diagnostics log (Settings → Diagnostics) and your OS / autopilot / firmware versions help a lot —
see **[Reporting a problem](../troubleshooting/reporting-issues.md)**.
