# Radar — FormationFlight (INAV-Radar / ESP32) integration

> ARCHIVED (2026-06-23) — F1 fully shipped (ADR-036); no open work (the "open" note below is just a
> serial-reset gotcha, not a backlog item). Kept for the detailed feature-level rationale.

> Status: **F1 shipped** (2026-06-07) — link + FC emulation + peer parse + 2D/3D rendering, bench-validated
> with two ESP32-C3 over ESP-NOW. See **ADR-036**. Adds the `formationFlight` radar system: peers shared by
> an ESP32 INAV-Radar/FormationFlight module (LoRa / ESP-NOW) over a serial MSP link. Monitoring /
> pilot-to-pilot only — **never raises conflict alerts** (ADR-035). Builds on the
> [radar subsystem](RADAR_TRACKING_CORE.md) + [map](RADAR_TRACKING_PANEL_AND_MAP.md); contacts use the
> existing `TrackedVehicle` model (`system='formationFlight'`) and flow through the same aggregator.

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

> Verified against the FormationFlight firmware (`src/lib/MSP`): it requests `MSP_NAME / MSP_FC_VARIANT /
> MSP_FC_VERSION / MSP_ANALOG / MSP_RAW_GPS / active-modes`. Kite answers those; everything else (incl. the
> sets below) gets an **empty ACK** (MSP requires a reply unless a no-reply flag is set, which the module
> does not). Kite answers `MSP_API_VERSION / MSP_ATTITUDE` too if asked. **TX is gated on the host type
> (FC_VARIANT), not on armed** — so reporting `"INAV"` is what makes the module transmit.

**Pushed to us (fire-and-forget commands):**
- `MSP2_COMMON_SET_RADAR_POS` (**0x100B**), 19 bytes packed — the peer data:
  ```
  id:u8 · state:u8 · lat:i32(°·1e7) · lon:i32 · alt:i32(cm) · heading:u16(°) · speed:u16(cm/s) · lq:u8(0–4)
  ```
  `state`: **disarmed(0) · armed(1)** (the firmware's `msp_radar_pos_t` only documents these). **No peer
  name/callsign is ever sent over MSP.**
- `MSP2_COMMON_SET_RADAR_ITD` (0x100C) — `{ type:u8, char msg[20] }`: a **module status / RSSI string**
  (e.g. "booting…"), *not* a peer name. We ACK it (display in FF settings is a later nicety).
- `MSP2_SENSOR_GPS` — the module can act as a GPS source for the FC; irrelevant to Kite, ACKed + ignored.

## 4. Position source = GCS location
Reuse the resolved GCS location (`userGeoLocation` — OS geolocation; the arming-location fallback is a
later refinement). Kite advertises it as a **stationary ground node** with a synthetic 3D fix, pushed live
to the backend (`radar_set_node_pos`) so it follows without restarting the source. The module then
transmits — even on the bench with no UAV. (No UAV-position option: the on-board aircraft carries its own
radar node, so advertising the UAV here would duplicate it.)

## 5. Data model + rendering (→ `TrackedVehicle`, `system='formationFlight'`)
Per peer from `SET_RADAR_POS`: `lat/lon` (°), `altM = alt/100`, `headingDeg`, `groundSpeedMs = speed/100`,
`signal = lq` (0–4). **Id → letter A..F** (`id 0→A`, matching the FF OSD) in `callsign` — there is no name
over MSP. Armed/disarmed → `extra.ffState`; **`lost` is a TIMEOUT we apply ourselves** (no
`SET_RADAR_POS` for >12 s → red, kept 5 min at the last position), *not* a firmware state. Flows through
the existing aggregator + the same panel/map; FF has its own list group + map visibility toggle.

**Rendering (FF-specific, not the ADS-B altitude scale):**
- **State colour:** armed = dark blue, disarmed = grey, lost = grey + red outline (2D) / red tint (3D).
- **2D:** a slim **paper-plane** silhouette (own SVG), 20 % larger than ADS-B, heading-rotated, with the
  letter as a bigger badge label (translucent grey background).
- **3D:** `ff-uav.glb` (placeholder paper-plane), tinted by state, **just the model + a thin SOLID
  drop-line** in the state colour — no ground circle / heading arrow (those stay ADS-B-only; the solid vs
  dashed line distinguishes the two).
- **List:** link quality as 0–4 pips (the only signal/freshness cue we get).

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
- **F1 — Link + FC emulation + peer parse + rendering ✅ (shipped):** serial MSP-slave source, FC
  emulation, live GCS node position + node name, `SET_RADAR_POS` parse, full 2D/3D state-coloured
  rendering, LQ list, lost-timeout retention, geoid fix. Bench-validated with two ESP32-C3 over ESP-NOW.
- **F3 — later:** `SET_RADAR_ITD` status string shown in FF settings; optional `"GCS"` listen-only mode
  (no phantom node) once the FC-emulation path is solid; dedup if a peer also appears via ADS-B; a real
  paper-plane `ff-uav.glb` (placeholder copies `uav-plane.glb`).

## 9. Gotchas / lessons (F1)
- **Reopening the serial port resets the ESP32** (USB auto-reset DTR/RTS). So the FF source is kept open
  across radar reconfigures and only restarted on a **port/baud** change (`reconcile_ff`); the node name +
  GCS position are **live** (shared `Arc`s) so changing them never cycles the port. The source also does
  **not** assert DTR/RTS.
- **`MSP_NAME` is clamped to ≤15 printable-ASCII bytes** — the module stores it in `char name[16]` and uses
  it as a C-string; a name filling all 16 bytes (no NUL) overflows it and crashes the firmware.
- **No peer name over MSP** ⇒ letters A..F (matches the OSD). Confirm the letter matches the OSD; offset if
  the firmware's slot ids start at 1.
- **`lost` is our timeout, not a firmware state.** The state byte only gives armed/disarmed.
- **Geoid:** a radar-only scene (no UAV/track) never computed the geoid offset, so contacts sank under the
  terrain — now computed at the GCS reference (`computeGeoidOnce`).
