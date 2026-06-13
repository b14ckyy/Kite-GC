# Wind / Crab Indicator + Flight-Path Marker (exploratory)

> **STATUS: NOT PLANNED / GATED ON FIRMWARE** — captured for direction only. Gated on INAV exposing
> the wind estimate over MSP. Nothing here is committed scope. Revisit once the firmware change merges
> and a release target is known. (GitHub issue #3, noted 2026-06-13.)

## The idea

Use the FC's wind estimate to add three related PFD/map cues:

- **Wind indicator** — wind direction + speed (e.g. an arrow on the map / a readout on the PFD).
- **Crab indicator** — the crab angle, i.e. the difference between **heading** (where the nose points)
  and **ground track** (where the aircraft actually moves). The visible "flying sideways" in wind.
- **Flight-path marker (FPM)** on the PFD — the velocity vector, marking where the aircraft is actually
  going. Splitting heading from track needs the wind estimate.

## The blocker — firmware

Needs INAV to send the wind estimate over MSP. That is added by:

- **INAV PR [iNavFlight/inav#11611](https://github.com/iNavFlight/inav/pull/11611)** (cherry-picked
  from #11508) — adds **`MSP2_INAV_WIND`** (`0x2231`).

As of **2026-06-13 the PR is NOT merged.** Unknown whether it lands in **INAV 9.1** or slips to
**INAV 10**. Until it ships there is no data path, so this stays parked.

### Message payload — `MSP2_INAV_WIND` (`0x2231`)

| Bytes | Type   | Field       | Units / notes                         |
|-------|--------|-------------|---------------------------------------|
| 0–1   | uint16 | `windSpeed` | cm/s                                  |
| 2–3   | uint16 | `windAngle` | degrees 0–359, wind-**FROM**, 0 = N   |

Wind is reported **FROM** (meteorological convention) — convert as needed for an arrow that points the
way the wind blows **to**.

## Considerations when it becomes real

- **Traffic cost.** This is a **new MSP poll** — the project is already wary of poll traffic. Likely a
  **low-priority secondary** (wind changes slowly; a few Hz, or even ~1 Hz, is plenty). Do not put it
  in the high-rate group.
- **Gating.** Both cues gate cleanly on **"wind valid"** → simply stay hidden until the FC sends a
  valid estimate, so no half-broken indicator on FCs/firmware without it. Feature-gate on the INAV
  version that introduces `MSP2_INAV_WIND` (`msp/features.rs`).
- **Crab geometry.** Crab angle ≈ difference between heading and GPS ground course; the wind vector
  lets us show *why* (and compute the FPM offset) rather than just the raw heading−course delta.
- **Where it renders.** PFD (FPV HUD) for the FPM/crab; the 2D/3D map for the wind arrow. Reuses the
  existing PFD work — see the FPV HUD in the 3D rework.
- **MAVLink parity.** MAVLink already exposes wind (`WIND`/`WIND_COV`) — if/when this is built, wire
  both protocols to the same internal "wind" event so the indicator is protocol-agnostic (project rule).
- **Replay.** Showing wind/crab on replay would need the values recorded → a DB column. Decide then
  whether it is live-only (no schema change) or recorded; default to live-only unless there is demand.

## Related

- Slip/skid "ball" (issue #2) was **rejected** — no accelerometer data path without extra traffic.
  Wind/crab is the analogous PFD-coordination idea that *is* feasible once the firmware sends the data.
