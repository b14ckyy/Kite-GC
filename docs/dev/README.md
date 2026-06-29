# Kite GC — development docs

Open, working development docs for **Kite Ground Control**, kept in the repository so plans, references
and conventions are visible to contributors. The polished, published developer guide lives on the
documentation site under **For developers** (`docs/user/for-developers/`); this folder is the **living
working area** (browse it on GitHub).

## Layout

- **[`ROADMAP.md`](ROADMAP.md)** — high-level roadmap (what's shipped, what's planned).
- **[`AI-ASSIST-RULES.md`](AI-ASSIST-RULES.md)** — the conventions & guardrails used here for
  **AI-assisted development**, provided as a reusable template.
- **`active/`** — feature plans with open work. Propose a plan here before building anything substantial.
- **`future/`** — exploratory ideas, not yet planned (often gated on something external).
- **`archive/`** — completed plans, moved here when shipped (kept for their detailed reasoning).
- **`reference/`** — technical reference (data pipeline, flight-log DB schema, …).

## Proposing & tracking a plan

Anyone — maintainers or contributors — can propose and track work in the open:

1. **Open an issue** to discuss the idea and agree on scope.
2. **Add a short plan doc** in `active/` — goal, approach, key decisions, and what's in / out of scope.
   Keep it focused; it's a working doc, not a spec to perfect.
3. **Build it**, linking the PR(s) back to the plan.
4. **When it ships**, move the doc to `archive/` with a one-line `> ARCHIVED (date) — …` banner so the
   reasoning is preserved without cluttering the active list.

A completed plan is never deleted — it's archived.
