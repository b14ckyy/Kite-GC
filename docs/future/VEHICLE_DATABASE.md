# Vehicle Database — Feature Sketch (deferred)

> STATUS: future / not-planned · 2026-06-24. A larger dedicated feature, parked until there's time.
> This is a short thought-piece, not a committed plan. Propose a full plan in `docs/active/` before
> implementing.

## Idea
A persistent **vehicle registry**: the user keeps a record per aircraft, links flights to it, and gets
**cumulative stats + record values** per vehicle over time.

## Concept
- **A vehicle entity** holding base data the user can fill in: name, **airframe** (type/size), **motor(s)**,
  **FC** (board/firmware), ESC, props, battery setup, weight, notes — i.e. anything worth recording.
- **Selectable in post-flight**, or **auto-linked by craft name** on INAV (the FC reports `craft_name`;
  ArduPilot has no equivalent → manual select / map a `SYSID`?). One vehicle ↔ many flights.
- Each flight log carries a **vehicle id**; the registry aggregates across linked flights:
  - cumulative **flight time / count / distance / energy (mAh/Wh)**, per-vehicle,
  - **record values** (top speed, max altitude, longest/ farthest flight, max climb, …),
  - maybe maintenance counters (motor hours, packs cycled) later.

## How it fits the existing model
- Parallels the **battery DB** ([[project_battery_management]] / `docs/archive/BATTERY_MANAGEMENT.md`):
  a DB table + manager UI + a soft-link from flights, with `.kbatt`-style export/import.
- Reuses the flight-log DB + the post-flight linking flow (same place the battery serial is set today).
- Craft-name auto-link mirrors how flights already capture `craft_name`; the record/aggregate math mirrors
  the battery aggregate queries.

## Open questions (for the real plan)
- Schema: a `vehicle` table (id, fields…) + `flight.vehicle_id` FK; migration is a 1.0 concern only.
- ArduPilot/PX4 identity (no craft name) — manual select, or key off a parameter / SYSID?
- How records interact with replay-only / imported logs (blackbox / tlog) that may lack a vehicle.
- Overlap with the battery DB (share a "gear" manager shell?).
- Export/import format for sharing a vehicle profile.

## Scope note
Bigger than a quick-note item — needs its own `docs/active/` plan + an ADR for the schema before any
code. Listed in `docs/QUICK_NOTES.md`.
