# Archived feature plans

Completed feature/implementation plans live here instead of being deleted. ADRs
(`../ARCHITECTURE.md`) record the *architectural* decisions; these docs preserve the
*detailed* feature-level reasoning — parameters chosen, alternatives rejected, step-by-step
build order — that is useful when revisiting or extending a feature later.

A plan is moved here once its planned scope is fully shipped. Docs with open/deferred items
stay in `../` (the active `docs/dev/` folder).

## Contents

- **PatternGenerator.md** — Survey pattern generator (six shapes). Implemented; see ADR-024/025.
  Recovered from git history (it had been deleted on 2026-05-29).
- **COLORED_TRACK_PLAN.md** — Colored flight tracks + flight-mode widget + UAV nav-state coloring.
  All steps S1–S10 shipped (S9 mode-label i18n intentionally dropped — mode names stay English).
- **ARDUPILOT_IMPORT_PLAN.md** — ArduPilot DataFlash `.bin` import (decode → DB → replay).
  Phases 1–3 shipped. (`.tlog` import is separate, tracked in ROADMAP / PROTOCOL_REFACTORING.)
- **BATTERY_MANAGEMENT.md** — Battery library + manager (schema v10, serial soft-link, `.kbatt`
  export/import, End-Flight capture, lifetime baseline + consolidation). Phase A + B shipped.
  Phase C (telemetry wear metrics) cut from scope; multi-battery per flight stays a future item.
