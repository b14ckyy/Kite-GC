# Contributing

Thanks for your interest in improving Kite Ground Control! Bug reports, fixes, features, translations and
documentation are all welcome.

## Getting started

1. Read **[Building from source](building.md)** and get a dev build running with `just dev`.
2. For anything non-trivial, **open an issue first** to discuss the approach — it saves rework.
3. Branch off `development`, make your change, run the checks, and open a pull request **against
   `development`** (see the branching model below).

## Branching model

Kite uses two long-lived branches. Which one you target matters:

| Branch | What it is | Push to it? |
| --- | --- | --- |
| **`development`** | The integration trunk — all new work lands here, and it is kept buildable. **This is what you branch from and what your PR targets.** | Only through a reviewed PR |
| **`master`** | The release line. It carries what is released or about to be, and takes fixes for the current release only. Releases are cut from here as tags. | Only through a reviewed PR |
| **`feat/<name>`** | Short-lived working branches, one per feature or fix, cut from `development` and deleted after the merge. Anything may be broken here. | Freely — it's yours |

```
feat/my-feature ──▶ development ──▶ master ──▶ tag (release)
                         ▲              │
                         └──────────────┘
                        fixes flow back down
```

**Nobody commits to `master` or `development` directly** — not even the maintainer. Every change
arrives as a pull request, so there is always a diff, a CI run and a place to comment.

**Where your branch lives** depends on your access: maintainers create `feat/*` branches in the main
repository, everyone else forks and opens the PR from the fork. The workflow is otherwise identical.

**Fixes for an already-released version** go on `master` and are merged down into `development`
afterwards, so nothing is lost. If a released version needs a patch after the trunk has moved on, the
branch is cut from that version's **tag**, not from `master`.

**Documentation is the one exception.** A change to these pages that touches no code — a correction, a
clarification, a missing note — targets `master` directly, because the published site must always
describe the released app. Documentation *for a new or changed feature* is not covered by this: it
belongs in the same branch as the feature and reaches `master` together with it.

## Before you open a PR

Run the static checks — the project leans on them heavily:

```bash
just check    # svelte-check + TypeScript + cargo check
```

CI runs the same checks (plus clippy) on Linux, Windows and macOS for every push to `development` /
`master` and every PR targeting them. **PRs should be green before review.**

For a **bug-fix PR**, please state in the description whether the bug exists in the **released
version** or was **introduced since** (and by what, if you know). Only the first kind belongs in the
release notes, so this one line saves the archaeology at release time.

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

Kite ships in English, German, French, Chinese and Bulgarian. For contributions:

- **`en.json` is the source of truth and is required** — every new or changed UI string must have its
  English key.
- **Other locales (`de.json`, `fr.json`, `zh.json`, `bg.json`) are optional but very welcome.** Keeping
  them in sync is appreciated; an AI assistant makes this quick and is the recommended way to fill in
  translations. Chinese and Bulgarian came in exactly that way, as community contributions.
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
