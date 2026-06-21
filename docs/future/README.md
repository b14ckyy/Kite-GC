# docs/future — Exploratory ideas (NOT on the roadmap)

This folder holds **thought-pieces and direction notes for ideas that are explicitly _not
planned_ yet** — usually big features that are gated on something external (commercial
viability, hardware, a partner, etc.).

**Rules of this folder**
- Nothing here is committed scope. It is **not** the ROADMAP and **not** an ADR.
- These are starting points so that *when* an idea becomes real, there is already a direction,
  a list of considerations, and the open questions written down.
- When an idea graduates to "we're doing this", it moves into the normal flow: a plan under
  `docs/active/`, ROADMAP entries, and ADR(s) for the actual decisions — and the note here gets a
  pointer to those (or is archived).

## Index
- [RADIO_SOURCE_RADAR.md](RADIO_SOURCE_RADAR.md) — track foreign vehicles from radio telemetry (CRSF/
  MAVLink/FrSkyX) as a third Radar source family. Was Radar Phase 5; **cancelled for initial release** —
  no use case for tracking extra radios beside the primary link (technically unblocked now the shared
  passive parser exists).
- [MULTI_OPERATOR_CENTRAL_ARCHIVE.md](MULTI_OPERATOR_CENTRAL_ARCHIVE.md) — central, multi-operator
  flight archive (sync local SQLite stores into a company-wide archive). Gated on the app going
  public and earning revenue.
- [UAV_GEOZONES_NFZ.md](UAV_GEOZONES_NFZ.md) — explicit drone no-fly / geozone maps (not the generic
  airspace overlay). Worldwide → multi-provider; normalize to ED-269/318. Gated on the geozone-data
  landscape maturing + the per-source licensing/commercial question.
- [WaypointDisable.md](WaypointDisable.md) — disable/enable a waypoint in a loaded mission without deleting
  it. **Cut** (not gated, actively rejected): no standard `.waypoints`/JSON representation → export
  round-trip incompatibility + risky codec complexity for a low-use feature. Kept for the design reasoning.
- [EXTERNAL_API.md](EXTERNAL_API.md) — external control API (read state / set waypoints / run commands)
  for AI/LLM-assisted mission planning: external tool *authors* intent → Kite *validates* against a real
  terrain/airspace/traffic envelope → FC *flies* (LLM out of the control loop). Lean MCP + a
  schema-agnostic intent/envelope validator. Gated on demand + the safety work; prompted by the INAV/URML
  outreach (#11651).
