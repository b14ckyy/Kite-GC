# docs/reference — Living reference docs (not plans)

Stable **reference** material that describes how the system works *as built* — not feature plans with
open work (those live in `docs/active/`) and not completed plans (`docs/archive/`). These are kept current
as the code evolves and are the canonical place to look up data shapes, DB layout and protocol details.

## Index
- [DATA_PIPELINE.md](DATA_PIPELINE.md) — how telemetry flows source → store → widget, live + replay; the
  protocol adapters (MSP / MAVLink / passive telemetry), the unified event names, stores and the
  recording/replay split.
- [FLIGHTLOG_DATABASE.md](FLIGHTLOG_DATABASE.md) — the flight-logging SQLite model (tables, columns,
  `user_version` migration chain) targeted for replay, mission context and fault analysis.
- [PROTOCOL_FLIGHT_MODES.md](PROTOCOL_FLIGHT_MODES.md) — how each autopilot stack reports flight modes
  (INAV bitmask vs ArduPilot/PX4 enums); reference when adding a new protocol.
