# Contributing

Thanks for your interest in improving Kite Ground Control! Bug reports, fixes, features, translations and
documentation are all welcome.

## Getting started

1. Read **[Building from source](building.md)** and get a dev build running with `just dev`.
2. For anything non-trivial, **open an issue first** to discuss the approach — it saves rework.
3. Branch off `development`, make your change, run the checks, and open a pull request **against
   `development`** (see the branching model below). A fix that must reach the released version goes
   against its maintenance branch instead.

## Branching model

Kite uses three kinds of long-lived branches. Which one you target matters:

| Branch | What it is | Push to it? |
| --- | --- | --- |
| **`development`** | The integration trunk — all new work lands here, and it is kept buildable. **This is what you branch from and what your PR targets.** | Only through a reviewed PR |
| **`master`** | A smoke-tested snapshot of `development`, taken by the maintainer when the trunk is in good shape. The repository's front page (README) is read from here, and every release line is cut from it. Nothing is developed on `master` and nothing is released from it directly. | Only the snapshot merge |
| **`release/<major.minor>.x`** | A release line. Cut from `master` at the feature freeze of a feature release, it carries the release candidates, the final release and every patch of that line — and the documentation of that version. Long-lived: it stays open as long as the line is maintained. | Only through a reviewed PR |
| **`feat/<name>`** | Short-lived working branches, one per feature or fix, cut from `development` and deleted after the merge. Anything may be broken here. | Freely — it's yours |

```
feat/my-feature ──▶ development ──▶ master ──▶ release/1.1.x ──▶ v1.1.0-rc1 … v1.1.0 … v1.1.2
                         ▲          (snapshot)  (feature freeze)              (tags)
                         │                            ▲
                         └──────── merge-up ───────────┴──── release/1.0.x
                                        fixes flow upwards
```

**Nobody commits to `master`, `development` or a release branch directly** — not even the
maintainer. Every change arrives as a pull request, so there is always a diff, a CI run and a place to
comment.

**Where your branch lives** depends on your access: maintainers create `feat/*` branches in the main
repository, everyone else forks and opens the PR from the fork. The workflow is otherwise identical.

**Fixes for an already-released version** go on the **oldest** release branch that has the bug
(`release/1.0.x` for a bug that exists in 1.0) and are merged **upwards** afterwards — into the newer
release line, if there is one, and into `development` — so nothing is lost and nothing is fixed twice.
Base the PR on the release branch, not on `development`, and say so in the description.

Release lines are maintained for a while: a feature release receives patches until the **second**
feature release after it has shipped (1.0.x until 1.2.0 — see [Release support](../release-support.md)),
so two lines take fixes side by side. That is why the regression marker below matters: it tells us at
a glance which lines a fix belongs to.

**Documentation follows the same branches.** Each release branch is the source of that version's pages
on this site — the version dropdown maps one entry to one release line, and **Dev** to `development`.
A change to these pages that touches no code — a correction, a clarification, a missing note — therefore
targets the **release branch of the version it describes** and reaches the newer lines with the same
merge-up as a code fix. Documentation *for a new or changed feature* belongs in the same branch as the
feature and reaches a release line together with it. The changelog is kept once for all released
versions: the newest release line renders it, the older versions' Changelog pages lead there.

## Before you open a PR

Run the static checks — the project leans on them heavily:

```bash
just check    # svelte-check + TypeScript + cargo check
```

CI runs the same checks (plus clippy) on Linux, Windows and macOS for every push to `development`,
`master` and the release branches, and for every PR targeting them. **PRs should be green before review.**

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

**Finding the key for a string you see on screen:** `python tools/i18n-key-paths.py` writes a pseudo-locale
`src/lib/i18n/locales/xx.json` in which every string is replaced by its own key path (`sensors.gyro`,
`rcLink.noLink`, …). Register it locally in `src/lib/i18n/index.ts` as described in the script's header,
start the dev app and switch the language to it — the interface then shows the key at every position
instead of the text. The file is git-ignored; take the `index.ts` registration out again before you commit.
Contributed by teodoryantcheff.

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

For questions and discussion there is the **[Kite Discord](https://discord.gg/3FM7EWhkg9)** (English): `#help` for
user questions, `#ideas` for feature ideas before they become issues, `#development` for
contributors — PRs, translations, architecture.
