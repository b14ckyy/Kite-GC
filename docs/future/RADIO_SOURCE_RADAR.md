# Radio-Source Radar — track foreign vehicles from radio telemetry (NOT planned)

> Exploratory note. Was **Phase 5** of the Radar subsystem ([../archive/RADAR_TRACKING_CORE.md]);
> **cancelled for the initial release** (2026-06-21). No current use case for tracking additional radios
> beside the primary FC link.

## Idea
Feed the **Radar** subsystem a third source family (after ADS-B and FormationFlight): foreign vehicles
seen on **radio telemetry** links — CRSF / MAVLink (`GLOBAL_POSITION_INT`) / FrSkyX / LTM — relayed
through the FC or a secondary listen-only receiver, each surfaced as a `Radio*` `VehicleSystem` in its own
stacked list (never raising conflict alerts; monitoring / pilot-to-pilot only, per ADR-035).

## Why it's deferred
- **No real use case yet.** In practice you don't track several other radios next to your own primary
  link; the radar value is in ADS-B (manned traffic) and the FormationFlight mesh.
- It would add per-protocol radar sinks + dedup across yet another source family for little benefit.

## What's already in place (so resuming is cheap)
- The **shared passive-telemetry parser** now exists (CRSF / SmartPort / LTM / MAVLink-passive decode +
  DB + AP passthrough), so the long-standing blocker ("deferred until a shared telemetry parser exists")
  is **resolved** — a radio radar sink could hang off that parser's output.
- The radar data model already reserves the group: `VehicleSystem::Radio` + `RadioCrsf / RadioMavlink /
  RadioFrsky` source kinds, separate stacked list, `id` = sysid.

## Resume trigger
A concrete user need to see *other* radio-telemetry aircraft on the map alongside the primary vehicle.
Then: a `radio_*` radar source built on the passive parser's sink → graduates back to a `docs/active/`
plan + ROADMAP entry.
