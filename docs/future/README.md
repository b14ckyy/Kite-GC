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
- [MULTI_OPERATOR_CENTRAL_ARCHIVE.md](MULTI_OPERATOR_CENTRAL_ARCHIVE.md) — central, multi-operator
  flight archive (sync local SQLite stores into a company-wide archive). Gated on the app going
  public and earning revenue.
- [UAV_GEOZONES_NFZ.md](UAV_GEOZONES_NFZ.md) — explicit drone no-fly / geozone maps (not the generic
  airspace overlay). Worldwide → multi-provider; normalize to ED-269/318. Gated on the geozone-data
  landscape maturing + the per-source licensing/commercial question.
