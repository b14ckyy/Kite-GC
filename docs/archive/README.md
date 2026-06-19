# Archived feature plans

Completed feature/implementation plans live here instead of being deleted. ADRs
(`../ARCHITECTURE.md`) record the *architectural* decisions; these docs preserve the
*detailed* feature-level reasoning — parameters chosen, alternatives rejected, step-by-step
build order — that is useful when revisiting or extending a feature later.

A plan is moved here once its planned scope is fully shipped. Docs with open/deferred items
stay in `../active/`.

**Archived ≠ frozen.** Moving a doc here just means it's **out of the active focus** — we don't
look here for "what's left to build". Later references, fixes and tweaks to an archived feature
are still perfectly fine; the doc simply isn't on the active work surface anymore.

## Contents

- **PatternGenerator.md** — Survey pattern generator (six shapes). Implemented; see ADR-024/025.
  Recovered from git history (it had been deleted on 2026-05-29).
- **COLORED_TRACK_PLAN.md** — Colored flight tracks + flight-mode widget + UAV nav-state coloring.
  All steps S1–S10 shipped (S9 mode-label i18n intentionally dropped — mode names stay English).
- **Map3DRework.md** — CesiumJS 3D view overhaul. All phases shipped: track outline + ground shadow +
  altitude curtain (Phase 1, incl. live trail), mission path in 3D (Phase 2), camera incl. heading-follow
  jitter fix + FPV cockpit view (Phase 3), jagged-track / terrain-derived-geoid altitude rework (Phase 4).
- **ARDUPILOT_IMPORT_PLAN.md** — ArduPilot DataFlash `.bin` import (decode → DB → replay).
  Phases 1–3 shipped. (`.tlog` import is separate, tracked in ROADMAP / PROTOCOL_REFACTORING.)
- **BATTERY_MANAGEMENT.md** — Battery library + manager (schema v10, serial soft-link, `.kbatt`
  export/import, End-Flight capture, lifetime baseline + consolidation). Phase A + B shipped.
  Phase C (telemetry wear metrics) cut from scope; multi-battery per flight stays a future item.
- **PROTOCOL_REFACTORING.md** — Multi-protocol (MSP + MAVLink) workstream. Phases 1–4 shipped
  (architecture in ADR-010); Phase 5 generic two-way MAVLink commands not pursued (receive-only).
- **UI_SCALING.md** — Global UI scale 100/125/150 % on the chrome layer. Shipped (archived 2026-06-04).
- **MISSION_LIBRARY_AND_DB.md** — Reusable mission library + DB + flight linking. Backend + logic +
  UI shipped; awaiting simulator/field testing.
- **MISSION_LIBRARY_UI.md** — UI surface for the mission library (Manager / editor save / logbook
  link). Shipped; awaiting simulator/field testing.
- **M5_TEST_CHECKLIST.md** — Manual verification checklist for M5 (recording + logbook). Kept for
  reference (M5 shipped).
- **RADIO_TELEMETRY.md** — Passive (listen-only) radio telemetry: SmartPort/CRSF/LTM decode → unified
  pipeline + DB, ArduPilot passthrough, fresh-frame rate fix. Shipped. **Open (resume on trigger):**
  MSP-over-SmartPort uplink (blocked in ETHOS — core dev to ship a one-line-change custom build; probe
  mechanism armed in the dev build), native-CRSF validation, armed-flight DB verify, RC Link widget.
- **TELEMETRY_FORWARDING.md** — Telemetry Relay (forwarding/conversion, ADR-051): re-encode live telemetry
  into LTM/MAVLink/CRSF/SmartPort and emit out Serial/BLE/TCP/UDP. P1–P3 shipped (TCP+LTM verified vs
  mwptools). **Open (resume on trigger):** validate the encoders against real GCS/trackers; MAVLink real
  vehicle-type/mode in HEARTBEAT.
- **ARDUPILOT_MISSION_LIBRARY.md** — ArduPilot/PX4 mission-library DB parity with INAV (ADR-050): save/
  dedup/preview/export + flight-link over the format-agnostic `missions` table. Phase 1 shipped. **Open
  (deferred):** Phase 2 provenance / fc-file-db sync chips for the AP store. (The broader ArduPilot
  WP-type/vehicle-class work stays active in `../active/ARDUPILOT_WAYPOINT_ARCHITECTURE.md`.)
