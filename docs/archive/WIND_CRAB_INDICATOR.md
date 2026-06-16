# Wind / Crab Indicator + Flight-Path Marker

> **ARCHIVED (2026-06-16).** Phase 1 (heading / course-over-ground / crab cues) **shipped** — see
> As-built below. Phase 2 (FC **wind estimate** → wind arrow + wind-derived flight-path marker) is
> **parked**: it needs `MSP2_INAV_WIND`, still unmerged in INAV (likely INAV 10), and is not required
> before the Kite release. **3D versions of the map markers are deferred** until explicitly requested
> (the turn arc would need a 3D spline). Revisit Phase 2 only if the firmware ships the wind message and
> there is demand. Orig. GitHub issue #3.

## The idea

Three related cues around the difference between where the nose points and where the aircraft actually
moves:

- **Crab indicator** — crab angle = **heading** (nose) − **ground track / COG** (actual motion). The
  visible "flying sideways" in wind. Built from heading + COG (Phase 1).
- **Direction markers on the map** — nose lines for **HDG** and **COG** + a **turn-radius curve** from
  the COG rate of change. Built (Phase 1).
- **Wind indicator + flight-path marker (FPM)** — wind arrow + the velocity vector on the PFD. Needs the
  FC wind estimate (Phase 2, parked).

## Directional data — validated (2026-06-16)

Two canonical channels, code-verified end-to-end:

| Channel | INAV / MSP | ArduPilot / MAVLink | Blackbox |
|---|---|---|---|
| **Heading (FC fused/AHRS)** | `MSP_ATTITUDE.yaw` | `ATTITUDE.yaw` | `heading` / attitude field |
| **COG (course over ground)** | `MSP_RAW_GPS.cog` | `GLOBAL_POSITION_INT` velocity `atan2(v_y,v_x)` (GPS_RAW_INT.cog fallback) | `gps_ground_course` |
| **Ground speed** (gate for COG validity) | ✅ | ✅ | ✅ |

**Decision: one heading source = the FC fused heading** (AHRS/EKF attitude yaw); one COG source = the GPS
course. We display what the FC reports as-is.

**INAV without a compass (web-validated):** the wind estimator is real/always-on (since v6.0), but INAV's
compass-free heading is COG-referenced — so **crab ≈ 0 without a magnetometer**; ArduPilot / with-mag
shows the real crab. That coincidence is the honest result, not a bug.

## As-built — Phase 1 (shipped)

### Pipeline cleanup (no schema migration, no new fields)

The two channels existed in the DB but were populated inconsistently. Made consistent across **all paths**:

- **Canonical mapping:** DB `yaw` / `telem.yaw` = FC fused heading (icon / compass / AHI); DB `heading` /
  `telem.course` = course over ground (radar CPA, track). Column names are historical; documented at source.
- **MAVLink** (`mavlink_proto/handler.rs`): `course` ← COG from the fused velocity `atan2(v_y, v_x)`
  (held < 0.5 m/s); `GLOBAL_POSITION_INT.hdg` (= heading) no longer leaks into `course`.
- **Blackbox** (`flightlog/blackbox.rs`): DB `heading` ← `gps_ground_course`; DB `yaw` ← `heading`/attitude.
- **Replay** (`adapters/telemetryAdapter.ts`): de-conflated (`yaw ← r.yaw`). **2D + 3D replay marker
  fixes** (`Map.svelte`, `Map3D.svelte`): the model/FPV/camera now orient on the FC heading (`td.yaw`),
  not `point.heading` (= COG) — so replay shows the real crab instead of riding the track "on rails".
- Pre-1.0: old MAVLink/Blackbox rows are inconsistent → re-import test logs.

### Compass widget (`CompassWidget.svelte`)

Heading unchanged (rose + white centre number under the fixed pointer). Added a **COG track bug** (amber
inward triangle on the rose rim at `course − yaw` — the gap to the pointer is the crab) and a **smaller
amber COG degree readout** above the heading number. Both hidden below 1.5 m/s (COG noise).

### Map direction lines (`Map.svelte`)

From the aircraft: **HDG** (solid blue) and **COG** (dashed amber) lines, each with a dark casing for
contrast, in a dedicated pane **above the flight track but below the UAV model**. Length = **15 s of
travel** (velocity-vector, geographic → scales with zoom). A **turn-radius arc** (thin white + casing,
under the H/B lines) shows the predicted ground path: curvature from the turn rate, length 15 s **capped
at a 180° sweep** (no spiral). Everything runs through the existing follow **smoother** (position,
heading, course, **and** an eased turn rate) so the lines/arc track the UAV stably without whipping. The
turn rate is low-passed and computed on the **source time base** (wall clock live, log timestamp in
replay → playback-speed-independent). Arc hysteresis: shows above **5 °/s**, stays while ≥ **3 °/s** and
for **2 s** after (rides out fluctuations). COG line hidden below 1.5 m/s.

### Settings

**Direction indicators** toggle in Settings → Data → Telemetry (under Airspeed), **default ON**
(`settings.directionLines`); gates the map nose lines + turn arc.

## Phase 2 — wind arrow + FPM (parked, firmware-gated)

Needs the FC wind estimate. **Blocker:** INAV PR [iNavFlight/inav#11611](https://github.com/iNavFlight/inav/pull/11611)
adds **`MSP2_INAV_WIND`** (`0x2231`) — unmerged as of 2026-06-13 (likely INAV 10). MAVLink already has
`WIND` / `WIND_COV`.

`MSP2_INAV_WIND` payload: `[0–1]` uint16 `windSpeed` cm/s; `[2–3]` uint16 `windAngle` deg (wind-**FROM**,
0 = N). When built: protocol-agnostic "wind" event (MSP + MAVLink), low-priority ~1 Hz poll (traffic-wary),
gate on "wind valid" + INAV version; FPM on the PFD; replay needs a recorded wind column (decide then).

## Deferred

- **3D map markers** (HDG/COG lines + turn arc) — only if explicitly requested; the turn arc needs a 3D spline.
- **Dedicated signed crab readout** — covered by the compass dual (heading + COG) readout; add a separate
  numeric `HDG − COG` only if wanted.
- Slip/skid "ball" (issue #2) — **rejected** (no accelerometer path without extra traffic).
