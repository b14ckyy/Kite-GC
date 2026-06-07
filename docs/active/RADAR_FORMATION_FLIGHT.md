# Radar — FormationFlight (INAV-Radar / ESP32) integration

> Status: **Plan / proposed** (2026-06-07). Adds the `formationFlight` radar system: peers shared by an
> ESP32 INAV-Radar module (LoRa / ESP-NOW) over a serial MSP link. Monitoring / pilot-to-pilot only —
> **never raises conflict alerts** (ADR-035). Builds on the [radar subsystem](RADAR_TRACKING_CORE.md) +
> [map](RADAR_TRACKING_PANEL_AND_MAP.md); contacts use the existing `TrackedVehicle` model
> (`system='formationFlight'`) and flow through the same aggregator.

## 1. What it is
INAV-Radar (a.k.a. ESP32-Radar / FormationFlight: OlivierC-FR/ESP32-INAV-Radar, mistyk/inavradar-ESP32):
each aircraft carries an ESP32 + radio that **broadcasts its own position** and **relays peers**. The ESP
module talks **MSP to a flight controller** — it polls the FC for its own telemetry, broadcasts it, and
pushes received peers back into the FC (for the OSD) via `MSP2_COMMON_SET_RADAR_POS`.

Kite joins the formation as a **ground node**: it presents itself to the module as an FC, advertises the
**GCS location**, and reads the peers the module relays.

## 2. Key decision — Kite emulates an *armed INAV FC* (MSP slave)
The module is the MSP **master** (it sends requests); the FC is the **slave** (responds). So Kite's
FormationFlight source is an **MSP slave / FC emulator** on a dedicated serial port.

Crucially, the module decides whether it may transmit from the FC variant string:
```cpp
// ESP32-INAV-Radar: silent/listen-only when the host is a GCS
if (... || curr.host == HOST_GCS) { sys.lora_no_tx = 1; }
```
- Reporting **`FC_VARIANT="INAV"` + armed + a valid GPS fix** ⇒ the module runs as a **full TX node** →
  it broadcasts our position *and* relays peers to us. **This is what we do** (same as MWPTools).
- Reporting `"GCS"` ⇒ the module's historically-buggy listen-only path (the listener was never
  initialised without the TX scheduler). We **avoid** it for now; can be offered later as an option.

## 3. Protocol (MSP over serial, 115200)
**Requests Kite must answer** (MSPv1 unless noted) — exact payloads pinned against the module's MSP lib at
implementation:

| Msg | Code | Kite returns |
|-----|------|--------------|
| `MSP_API_VERSION` | 1 | MSP protocol + a plausible API version |
| `MSP_FC_VARIANT` | 2 | `"INAV"` (4 chars) — makes the module a full TX node |
| `MSP_FC_VERSION` | 3 | e.g. 8.0.0 |
| `MSP_NAME` | 10 | our node name (configurable; e.g. "KITE-GCS") |
| `MSP_RAW_GPS` | 106 | fix=3, numSat≈12, **GCS lat/lon** (°·1e7), alt (m), speed 0, course 0 |
| `MSP_ATTITUDE` | 108 | level (heading 0) |
| `MSP_ANALOG` | 110 | dummy battery (so the module's display/LQ logic is happy) |
| status / boxids | (per lib) | report **armed** (the module's `getActiveModes()` source) |

**Peers pushed to us** — `MSP2_COMMON_SET_RADAR_POS` (**0x100B**), 19 bytes packed:
```
id:u8 · state:u8 · lat:i32(°·1e7) · lon:i32 · alt:i32(cm) · heading:u16(°) · speed:u16(cm/s) · lq:u8(0–4)
```
`state`: 0 undefined · 1 armed · 2 lost. Also optional `MSP2_COMMON_SET_RADAR_ITD` (0x100C, display
extras) and `MSP2_COMMON_GET_RADAR_GPS` (0x100F). Kite **ACKs** every command (empty reply, same code) so
the module's request/response loop doesn't stall.

## 4. Position source = GCS location
Reuse the existing resolved GCS location (`userGeoLocation` — OS geolocation, **arming-location fallback**
on a live connection). Kite advertises that as a **stationary ground node** with a synthetic 3D fix +
armed so the module always transmits — even on the bench with no UAV. (No UAV-position option for now:
the on-board aircraft carries its own radar node, so advertising the UAV here would duplicate it.)

## 5. Data model (→ `TrackedVehicle`, `system='formationFlight'`)
Per peer from `SET_RADAR_POS`: `lat/lon` (°), `altM = alt/100`, `headingDeg`, `groundSpeedMs = speed/100`,
`signal = lq` (0–4), `validPos` from `state` (drop `lost`). Id = the slot (`ff-<id>`); `callsign` from
`SET_RADAR_ITD`/`NAME` when available. Flows through the existing aggregator (per-system merge + TTL) and
the existing panel/map rendering — FormationFlight already has its own list group + map visibility toggle.

## 6. Architecture (Rust, isolated)
- New source `radar/sources/formation_flight.rs` implementing `RadarSource` (serial, like the local ADS-B
  MAVLink receiver). It owns its serial port, runs the **MSP-slave loop** (decode request → encode
  response; decode `SET_RADAR_POS` → push a `SourceUpdate`). Emits `system='formationFlight'`.
- **Fully isolated** from the main scheduler/FC link — a FormationFlight fault must never disturb the
  safety-critical connection.
- **MSP frame codec:** extract the low-level MSPv1/v2 frame encode/decode (XOR / CRC8-DVB-S2) into a small
  shared helper (one source of truth for framing); write the **slave responder** fresh here (the main
  pipeline's logic is master-role and not reusable as-is). If extraction is messy, copy just the codec.

## 7. Config / settings
Extend `radar.formationFlight` from `{ enabled }` to also hold the serial **port + baud** (default 115200)
and the advertised **node name**. Panel: a source row under the FormationFlight tab (port picker + baud),
mirroring the local ADS-B receiver UI.

## 8. Phasing
- **F1 — Link + FC emulation + peer parse:** serial source, MSP slave answering the request set as an
  armed INAV FC at the GCS location, parse `SET_RADAR_POS` → contacts. Validate on the bench with two
  **ESP32-C3** nodes over ESP-NOW (no LoRa hardware needed).
- **F2 — Polish:** `SET_RADAR_ITD` name/extras, node-name setting, panel source row, status/LQ display.
- **F3 — later:** optional `"GCS"` listen-only mode (no phantom node) once the FC-emulation path is solid;
  dedup if a peer also appears via ADS-B.

## 9. Open / to confirm at implementation
- Exact `getActiveModes()` query set (MSP_STATUS / MSP_BOXIDS / MSP2_INAV_STATUS) the module's MSP lib uses.
- Whether the module gates TX on armed-only vs fix-only (we report both to be safe).
- `MSP_NAME` length / encoding the module expects.
