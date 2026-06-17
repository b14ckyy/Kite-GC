# Radio Telemetry (passive monitoring) — Plan

**Status:** FrSky / S.Port path **shipped through Phase D** (decode + DB recording; pending armed-flight
verify). **CRSF (Phase E) built but WIP/TEMPORARY (2026-06-17)** — detector + decoder + unified-pipeline
adapter done; framing/CRC + ATTITUDE confirmed, but **only against mLRS** (non-native CRSF, transcodes
INAV MSP → CRSF; emits no OK/!ERR sentinels so arming is untestable). Remaining scalings + flight-mode +
arming need a **native CRSF rig** to validate. Test BT module is unusable (BLE kink #6).
**Created:** 2026-06-16

---

## Goal

Receive and decode **passive telemetry** streams forwarded by the **transmitter/ground side** (EdgeTX
or ETHOS radios, ELRS backpacks, DIY bridges) — *not* tapped off the aircraft. One ground-control
"Telemetry" mode that **autonomously detects** which protocol is on the wire and decodes it into the
**same telemetry events** the MSP/MAVLink paths already emit, so the existing widgets/map light up
without per-protocol frontend code.

Supported (eventually): **FrSkyX / SmartPort**, **CRSF**, **LTM**, **MAVLink-passive** (push-only).
We **start with FrSkyX over BLE**.

**Not in scope (for now):** DB recording of telemetry-mode flights. A `.csv` raw recording is
**reserved for a later phase**. Sending anything outbound on any telemetry transport (it is strictly
listen-only — no heartbeats, no waypoint upload, nothing).

---

## Confirmed decisions

- **D1 — Third connection mode.** The `MSP | MAVLink` switch gains a third entry **`Telemetry`**. It is a
  single entry for *all* passive protocols; the specific protocol is **auto-detected**, not user-picked.
- **D2 — Detector = reference table.** A signature table maps incoming framing → protocol. Each incoming
  chunk is checked against the registered matchers; the first confident match **locks** the decoder for
  the session. **FrSkyX registered first**; the rest grow incrementally. (Native SmartPort polling is
  *probably never needed* on the ground side — revisit only if the data says otherwise.)
- **D3 — Baud is neutral / manual.** Protocol baud rates only matter receiver↔FC. On the ground it is
  **BLE/BT (baud-less)** or **USB (always 57600 or 115200, picked manually)**. No baud↔protocol coupling,
  no auto-baud scan. The existing baud dropdown stays for serial; ignored for BLE.
- **D4 — Transport: BLE-first**, serial + TCP/UDP later. Both test radios (EdgeTX + ETHOS) have **BLE
  telemetry-forwarding modules**, so BLE is the first wired path. Future: CRSF over ELRS backpack →
  TCP/UDP; radio → BLE; DIY → BT-SPP / USB / RF-module COM. All ride the existing `ByteTransport`.
- **D5 — Dev-only for now.** The `Telemetry` switch entry is gated behind `import.meta.env.DEV` until the
  pipeline is real (nothing feeds widgets/DB yet).
- **D6 — MAVLink-passive reuses the existing MAVLink decoder**, with **all TX suppressed** (no heartbeat,
  no stream-rate config, no param/mission/command writes). Wire format is identical — it is effectively
  "Full Telemetry" mode that never transmits. The current active MAVLink switch entry is unchanged.
- **D7 — Unified output.** Every decoder emits the **same Tauri telemetry events** as MSP/MAVLink
  (`AttitudeData`, `GpsData`, `AnalogData`, `StatusData`, …) so the frontend is protocol-agnostic.
- **D8 — Validation capture before the adapter.** Before wiring FrSky → events, we **capture the raw
  stream to file on connect** (everything, from the first byte) for offline analysis — to confirm what
  the radio actually emits and that it is authentic. The adapter is built only after that review.

---

## Architecture

### Connection layer
- `ProtocolType` gains `'telemetry'` (frontend). The switch renders a 3rd segment, **DEV-gated**.
- `connect()` (backend) gets a third branch `connect_passive_telemetry()`: opens the chosen
  `ByteTransport` (BLE first), **no handshake**, synthesizes a minimal `FcInfo` (e.g.
  `fc_variant: "Telemetry"`), starts the passive handler thread.
- New `ActiveProtocol::PassiveTelemetry(handle)` in `state.rs`; `disconnect()` stops it and drops the
  transport (mirrors the MSP/MAVLink arms).

### Passive handler (new module `src-tauri/src/passive_telemetry/`)
Push-stream handler modeled on the MAVLink handler (owns the `ByteTransport`, `Stop` command, returns the
transport on stop). **Strictly listen-only — it never calls `write_bytes`.**

```
passive_telemetry/
  mod.rs        // handle + start() + ActiveProtocol wiring
  handler.rs    // reader loop: read chunks → detector → (capture | dispatch)
  detector.rs   // signature reference table + classify(); locks a protocol per session
  capture.rs    // raw-stream capture to file (validation phase)
  decoders/
    frsky.rs    // FIRST — FrSkyX / SmartPort-derived
    crsf.rs     // later
    ltm.rs      // later
    mavlink.rs  // later — thin reuse of mavlink_proto parser, TX disabled
```

### Detector (reference table)
A table of `{ protocol, matcher(&[u8]) -> Confidence }`. Candidate framing signatures (to be **confirmed
empirically** in research/validation — FrSkyX plain-text format is the big unknown):

| Protocol | Likely signature | Note |
|---|---|---|
| FrSkyX / SmartPort | `0x7E` framing + `0x10` data-frame header, byte-stuffing `0x7D` | or a "decoded plain-text" variant from EdgeTX/ETHOS — TBD |
| CRSF | sync `0xC8`, `len`, `type`, CRC8/DVB-S2 | 420 kBaud on wire, but ground side is reframed |
| LTM | `$T` = `0x24 0x54` + frame-type char (`A/G/S/O/N/X`) | fixed per-frame lengths |
| MAVLink | magic `0xFE` (v1) / `0xFD` (v2) + length/CRC | reuse `MavParser` to confirm |

`classify()` accumulates bytes until one matcher is confident, then locks it. Until locked (and always,
in the validation phase) the raw bytes also go to the capture sink.

---

## Phases

- **Phase A — Interface + skeleton.** 3rd `Telemetry` switch entry (DEV), `ActiveProtocol::PassiveTelemetry`,
  passive handler thread (listen-only), detector reference table with **FrSky registered**, BLE transport
  path. No decoding output yet beyond detection state.
- **Phase B — FrSky validation (capture-to-file).** On connect, capture the full raw stream to file (see
  below). Debug Monitor tab shows: detected protocol, byte rate, live framing/hex tail. **Goal: hand the
  capture files back for analysis** to confirm format + authenticity. *(EdgeTX **and** ETHOS, both via BLE.)*
- **Phase C ✅ — FrSky → unified pipeline adapter (shipped).** `decoders/frsky.rs` maps S.Port appIDs onto
  the existing telemetry events (attitude/gps/altitude/analog/airspeed/status/flightmode). MODES (flight
  mode + armed, decimal-column-packed → normalized bitmask → `classify_inav`) and GNSS (sats + fix)
  decoded. INAV 7/8/9 via dispatch-by-appID. Bench-validated.
- **Phase D ~ — DB recording (built; pending armed-flight test).** The passive path now creates a
  `FlightRecorder` (when flight logging is enabled) and feeds it the decoded telemetry; arm/disarm is
  driven by the FrSky MODES armed bit (`arming_flags & 0x04`), reusing the existing recorder verbatim.
  No raw byte log on this path (FrSky has no MSP raw stream).
- **Phase E ~ — CRSF (Crossfire / ELRS) → unified pipeline.** Mirrors the FrSky path. **E1 ✅** detector
  CRC-validates CRSF frames (no false positives next to SmartPort; sync `0xC8`/`0xEA`/`0xEE`). **E2 ✅**
  raw `.bin`/`.jsonl` capture **+ decoded `radiotelem_<ts>.crsf.txt` dump**; Debug Monitor shows a live
  CRSF-frame counter. **E3 ~ partial** — bench capture confirmed framing/CRC (sync `0xEA`) + the ATTITUDE
  scaling exactly; GPS/battery/airspeed/baro and the flight-mode (0x21) string→mode map are source-derived
  but **not yet empirically validated** (the bench rig sent no fix/battery/baro/0x21). **E4 ✅ built** —
  `decoders/crsf.rs` accumulates state + `publish()`es unified events + feeds the recorder; pending a real
  armed INAV+CRSF flight to confirm the remaining scalings. See the CRSF section below for the frame map.
- **Phase F+ — remaining protocols.** LTM, MAVLink-passive (decoder reuse, TX disabled); later a `.csv`
  raw recording; link-quality field for the RC Link widget; ArduPilot FrSky-passthrough (0x5000) decoder.

---

## Validation capture (Phase B) — file format

Written **on connect** (capture everything from the first byte), stopped on disconnect, into the app
data/log dir (`radiotelem/` subfolder), timestamped:

- **`radiotelem_<ts>.bin`** — the exact concatenated byte stream, losslessly re-parseable.
- **`radiotelem_<ts>.jsonl`** — one record per transport read / BLE notification:
  `{ "t_ms": <ms since connect>, "len": <n>, "hex": "7e 10 ..." }`. Preserves **chunk boundaries +
  timing** (critical for BLE notification framing analysis).

(Format chosen so it is trivial for me to re-parse and reconstruct framing. This is a dev capture, **not**
the future `.csv` telemetry recording.)

---

## Research checklist (after this plan is OK'd, before coding)

1. **U360GTS telemetry parsers** as an architecture baseline for push-message decoding:
   <https://github.com/raul-ortega/u360gts/tree/master/src/main/telemetry> (old project, stale docs, but
   the message architecture is a good reference). FrSkyX is expected to be ~"decoded plain-text SmartPort".
2. **EdgeTX telemetry-out** wire format over BLE / "Telemetry Mirror" / serial — what bytes actually leave
   the radio.
3. **ETHOS telemetry-out** wire format over BLE — likely differs from EdgeTX; capture both.
4. Confirm BLE GATT profile of each radio's telemetry module (NUS / custom) against our existing
   `transport/ble.rs` profile support.

---

## BLE transport discovery (ETHOS X20RS)

The ETHOS X20RS exposes **no known serial profile**. A Web-Bluetooth scan showed only standard DIS
(`0x180A`) + GAP (`0x1800`) services — no Notify characteristic — but Web Bluetooth under-reports
services (only lists ones explicitly requested), and the FrSky docs confirm the X20 BT module has a
**Telemetry mode**. ETHOS firmware is closed-source (the `FrSkyRC/ETHOS-Feedback-Community` repo has only
translations + PDF manuals, no GATT UUIDs), so the streaming characteristic must be found by inspection.

**Built: a listen-only BLE auto-discovery path** (`transport/ble.rs::connect_ble_listen`), used by the
Telemetry mode for the `Ble` transport. It connects to *any* device (no profile match needed), dumps the
full GATT table to the Debug Monitor (`ble-gatt-services`), subscribes to **every Notify/Indicate
characteristic**, and routes their bytes into the capture/sniffer pipeline (per-characteristic activity
emitted as `ble-gatt-char-data`). This both reveals what the radio exposes and captures the stream once
the radio's BLE telemetry mode is active. **To test: set the radio's Bluetooth mode to Telemetry, then
connect** — the streaming service/characteristic should appear and start delivering bytes.

## Validated: FrSky S.Port over BLE (ETHOS X20RS, INAV 9.x)

First real capture (241 KB, 21 922 frames) — framing fully reverse-engineered:

- **Transport (BLE):** vendor service **`0xFFF0`**, characteristic **`0xFFF6`** (WriteNR + **Notify**) carries
  the stream; `0xFFF3` (WriteNR) is the unused uplink. Listen-only = subscribe to `0xFFF6`. See the BLE
  discovery note above (Web-BLE under-reporting hid `0xFFF0`).
- **Frame:** raw FrSky **S.Port**, `0x7E`-delimited (frames separated by `7E 7E`), with **`0x7D`
  byte-stuffing** (`7D xx → xx XOR 0x20`). Unstuffed frame = **9 bytes**:
  `<physID> 0x10 <appID:2 LE> <value:4 LE> <crc>`. Type byte is always `0x10` (data frame).
- **physID:** `0x00` = the flight controller, `0x98` = the receiver (RSSI/RxBt on `0xF101`/`0xF104`).
- **CRC:** the trailing byte is **not** the standard S.Port checksum (no common variant matched). The
  `0x7E` framing + constant `0x10` type is reliable on its own; CRC validation is deferred (corrupt-frame
  filtering only).
- GPS (`0x0800` lat/lon, `0x0820` alt, `0x0840` course) decoded to a real location — capture is authentic.

## INAV S.Port appID map + version coverage (7.0 vs 8.0/9.0)

Standard FrSky fields are **stable across INAV 7/8/9** (same appID + encoding): `0x0100` Alt, `0x0110`
Vario, `0x0200` Current, `0x0210` VFAS, `0x0300` Cells, `0x0830` GPS-speed, `0x0840` Heading, `0x0820`
GPS-alt, `0x0800` lat/lon, `0x0430` Pitch, `0x0440` Roll, `0x0450` FPV, `0x0460` Azimuth, `0x0420`
Home-dist, `0x0700/0710/0720` Acc, `0x0A00` Airspeed, `0x0910` A4, `0xF102/0xF103` ADC1/2.

**The INAV-specific status fields moved** (verified against `telemetry/smartport.c` at tags 7.0.0 / 8.0.0 / 9.0.0):

| Data | INAV ≤ 7.x | INAV ≥ 8.0 |
|---|---|---|
| Flight modes | `0x0400` (T1) | `0x0470` (MODES) |
| GNSS state | `0x0410` (T2) | `0x0480` (GNSS) |

In 8.0+ the old IDs are renamed `LEGACY_MODES`/`LEGACY_GNSS`, kept only as `#define`s and **not
transmitted** (removal slated for INAV 10). 7.x transmits `0x0400`/`0x0410`; 8.0+ transmits
`0x0470`/`0x0480`. The bit-packing of the modes/GNSS payload also differs between the old and new fields.

**Coverage strategy — dispatch by appID, no version sniffing:** the legacy (`0x0400`/`0x0410`) and new
(`0x0470`/`0x0480`) IDs are disjoint, so the appID itself disambiguates the firmware era. We implement
**both** decoders and route by appID; whichever the FC emits is decoded with its matching layout. This
auto-covers 7.x ↔ 8.0+ (and future FCs/Betaflight) without detecting a version. The same principle applies
to CRSF (INAV also reworked CRSF custom frames across versions) — decode by frame/sub-type, not version.

## ArduPilot — separate decoder (FrSky passthrough / "Yaapu")

ArduPilot exposes only **minimal native FrSky fields**; almost everything (attitude, GPS, battery,
**text status messages**, AP flight modes) is packed into the **DIY appID range `0x5000–0x52FF`** using the
ArduPilot **FrSky passthrough** ("Yaapu") protocol — bit-packed, MAVLink-derived messages, a completely
different decoding from INAV's per-sensor appIDs. This is handled as its **own decoder**, documented and
kept strictly separate from the INAV/standard-FrSky path (selected by detecting `0x5000`-range frames).
Scope for a later phase.

## CRSF (Crossfire / ELRS) — frame map + decoder plan (Phase E)

> **⚠ WIP / TEMPORARY (2026-06-17).** The CRSF path is built but **not validated against a native CRSF
> system**. All current test data came from **mLRS**, which is **not native CRSF** — it *emulates* CRSF
> telemetry by transcoding INAV's **MSP-parsed** data. The mLRS author doesn't run INAV and has relied on
> external testing, so that emulation isn't 100 % trustworthy. Critically, **mLRS emits no `OK`/`!ERR`
> disarmed sentinels — only the active mode string** — so there is **no arming edge to test**, and our
> `armed = string ∉ {OK, WAIT, !ERR}` logic (see below) can't be confirmed with mLRS. **A real CRSF-based
> system (ELRS/Crossfire on native INAV) is required** to validate arming, the flight-mode strings, and
> the GPS/battery/airspeed/baro scalings before this is considered done.

Research validated against the official **TBS CRSF spec** (<https://github.com/crsf-wg/crsf>) and INAV
`telemetry/crsf.c` (master). Decode **by frame type**, no version sniffing (same principle as S.Port).

### Framing
`[sync] [len] [type] [payload] [crc8]`

- **sync / device address** — telemetry originating at the FC uses `0xC8`
  (`CRSF_ADDRESS_FLIGHT_CONTROLLER` / `CRSF_TELEMETRY_SYNC_BYTE`). A radio-forwarded stream may re-wrap
  with `0xEA` (radio TX) or `0xEE` (TX module) — **confirm from the capture** (as we did for S.Port).
- **len** — counts `type + payload + crc` (i.e. `payload_len + 2`). Full on-wire frame = `len + 2` bytes.
- **crc8** — **CRC8 / DVB-S2, poly `0xD5`**, computed over `type + payload` (everything between `len` and
  `crc`, exclusive). Identical algorithm to MSP v2 → **reuse `MspCodec::crc8_dvb_s2`** (`msp/codec.rs`).

### INAV CRSF frames → unified telemetry events

| Type | Frame | Layout (big-endian) | Conversion → unified event |
|---|---|---|---|
| `0x02` | GPS | `lat:i32, lon:i32, gspeed:u16, hdg:u16, alt:u16, sats:u8` | lat/lon ÷ 1e7; ground speed ÷ 36 → m/s (frame is km/h×10); course ÷ 100 (frame is centideg); alt − 1000 → m; **no fix type in frame** → derive (sats ≥ 4 ⇒ 3D) → `GpsData` |
| `0x07` | Vario | `vspeed:i16` | ÷ 100 → m/s → `AltitudeData.vario` |
| `0x08` | Battery | `volt:u16, curr:u16, cap:u24, remain:u8` | INAV sends `getBatteryVoltage()/10` & `getAmperage()/10` (its getters are centi-units) → **÷ 10 ⇒ V / A**; cap = mAh; remain = % → `AnalogData` |
| `0x09` | Baro alt | `alt_packed:u16` | packed: high range in m, low range decimetres − 10000 offset → `AltitudeData.altitude` *(confirm packing from capture)* |
| `0x0A` | Airspeed | `aspd:u16` | ÷ 36 → m/s (frame is km/h×10) → `AirspeedData` |
| `0x1E` | Attitude | `pitch:i16, roll:i16, yaw:i16` | radians × 10000 → ÷ 10000 → rad → deg; yaw `rem_euclid(360)`; **order pitch, roll, yaw** → `AttitudeData` |
| `0x21` | Flight mode | null-terminated **ASCII string** | see below → `StatusData` + `telemetry-flightmode` |
| `0x0C`/`0x0D`/`0x29`/`0xF0` | RPM / Temp / Device-info / MSP-over-tlm | — | **ignored for now** (`0x29` device name is a possible future identity source) |

### Flight mode is a STRING — the key difference from S.Port

S.Port packs modes into a decimal-column bitmask (`decode_modes`); **CRSF sends one ASCII mode string**
per frame (`crsfFrameFlightMode`). INAV's strings: armed → `ACRO` (default), `ANGL`, `HOR`, `ANGH`,
`AH`, `HOLD`, `LOTR`, `CRUZ`, `CRSH`, `WP`, `RTH`, `WRTH`, `MANU`, `TURT`, `HRST`, `GEO`, `!FS!`
(failsafe); disarmed → **`OK`**, **`WAIT`** (no GPS fix/home), **`!ERR`** (arming disabled).

- **Armed** = the string is **not** in `{ "OK", "WAIT", "!ERR" }`. ⚠ **Untested** — mLRS (our only test
  source so far) never sends these disarmed sentinels, only the active mode, so this can't be exercised
  until a native CRSF rig is available. On such a non-conformant source the heuristic would read *always
  armed*; revisit once we can capture a real arm/disarm edge.
- A small **string → mode mapper** sets the matching `F_*` bit (e.g. `ANGL`→`F_ANGLE`, `RTH`/`WRTH`→
  `F_NAV_RTH`, `HOLD`/`LOTR`→`F_NAV_POSHOLD`, `CRUZ`/`CRSH`→`F_NAV_COURSE_HOLD`, `WP`→`F_NAV_WP`,
  `AH`→`F_NAV_ALTHOLD`, `MANU`→`F_MANUAL`, `!FS!`→`F_FAILSAFE`, `ACRO`→none) then reuses
  `classify_inav` for the same `FlightModeState` shape. CRSF carries only the **dominant** mode (not a
  full bitmask), so the widget shows a single mode — expected.

### Detector hardening (E1)

Replace the crude `sync + len-range` heuristic with **full-frame CRC validation**: for each candidate
start (`sync ∈ {0xC8, 0xEA, 0xEE}`, `len ∈ 2..=62`, full frame present in the window), recompute
`crc8_dvb_s2(type..payload)` and only count it as a hit when the CRC matches. A CRC-valid CRSF frame is
essentially false-positive-free, so CRSF locks cleanly alongside FrSky's `0x7E` counting (FrSky frames
won't pass CRSF CRC, and CRSF frames rarely contain `0x7E`).

### Validation workflow (E2/E3)

The `.bin` + `.jsonl` capture is protocol-agnostic and already runs. Add a **decoded CRSF frame dump**
(one line per frame: `type`, hex payload, decoded fields) during the validation phase + a Debug Monitor
view, so scalings (especially **battery units** and **baro packing**) can be confirmed against the real
stream before wiring the adapter — exactly the FrSky procedure.

### Open questions (settle from the capture)

- Does the radio forward **raw CRSF frames** (sync `0xC8`) or re-wrap/transcode them? Which BLE
  characteristic carries them (`connect_ble_listen` subscribes to all Notify chars, so it's covered)?
- Baro-altitude (`0x09`) packing thresholds — confirm the m/dm boundary empirically.
- Battery scaling — INAV getter units vs. the CRSF nominal 0.1 V/A; confirm `÷10` yields real volts/amps.

## Logbook / DB integration (Phase D)

How telemetry-mode flights appear in the logbook (passive telemetry carries no FC identity):

- **`source` = `live`** — it's a real-time recording (not a blackbox file), so it stays eligible for
  linking. The *protocol* lives in `flight.protocol`, set by the handler once detected:
  `Telemetry (SmartPort | CRSF | LTM | MAVLink)`. Shown as a **Protocol** row in the flight detail.
- **Firmware = N/A** — no handshake, so `fc_variant`/`fc_version` are empty; the detail shows `N/A`
  (named protocols still show e.g. `INAV 9.1.0` = variant + version).
- **Platform = Generic (`255`)** — SmartPort/CRSF carry no vehicle type, so the map shows the generic
  arrow (not a defaulted multirotor) and the UAV-type reads "Generic" (user can override). Also fixed a
  pre-existing platform-enum mismatch in `uavIcons.ts` (was `4=boat,5=other`; canonical is
  `4=rover,5=boat,6=other`, matching INAV `flyingPlatformType_e` + the logbook dropdown).
- **Auto-link to a blackbox:** the existing matcher is craft-name + start-time ±60 s. Telemetry logs have
  **no craft name**, so a duration fallback was added: when the live flight's `craft_name` is empty, match
  on a **near-identical duration** (±10 s, covering the arm/disarm grace) within the ±60 s window. Exact
  craft-name matches still win. Manual linking remains available.

## Link-quality fields (for a future RC Link widget)

Confirmed on the bench (physID `0x98` = receiver): **RSSI = `0xF101`**, **Link Quality / VFR = `0xF010`**,
**RxBt = `0xF104`**. `0xF010` is injected by the receiver/radio, not in INAV's `smartport.c`. These feed a
planned protocol-agnostic **RC Link widget** (RSSI + LQ, also MSP link-stats / CRSF). Needs a unified
`link_quality` field on the live telemetry pipeline (the DB already has one for replay).

## BLE link — internal kinks (lessons learned)

Practical gotchas for connecting to radio BLE telemetry modules (keep for future transports):

1. **Web-Bluetooth scanners under-report services.** They only list services the page explicitly
   requested, so a generic web scan of the X20RS showed *only* DIS (`0x180A`) + GAP (`0x1800`) and hid the
   real vendor service. **Always do a native full GATT enumeration** (btleplug/BlueZ/CoreBluetooth) — that
   sees everything.
2. **Don't require a known profile.** The telemetry service is vendor-specific (`0xFFF0` on the X20RS) and
   varies per radio. Connect to any device, enumerate, and **subscribe to whatever characteristic has the
   Notify property** (`connect_ble_listen`). Hard-coding a profile would have missed it.
3. **The streaming service only appears in the radio's "Telemetry" BT mode.** In other BT modes (trainer,
   etc.) the vendor service may be absent — set the radio's Bluetooth mode to Telemetry first.
4. **BLE is just a byte pipe** — decode the payload (S.Port/CRSF/…) independently of the link; chunk
   boundaries from notifications don't align with protocol frames (the `.jsonl` capture preserves them).
5. **Don't subscribe to standard SIG services.** `connect_ble_listen` now skips Generic Access (0x1800),
   Generic Attribute (0x1801) and Device Info (0x180A). Subscribing to Generic Attribute's **Service
   Changed (0x2A05, Indicate)** makes WinRT demand an authenticated link → spurious pairing/PIN prompt,
   even though the vendor telemetry characteristic needs none. Telemetry always lives on a vendor service.
6. **Some BT modules mandate bonding at the link layer (can't be worked around).** A retro-fitted older
   FrSky BT module (Radiomaster/EdgeTX, CRSF) demanded a passkey **on connect** — identically on Windows
   *and* Android, i.e. peripheral-driven (SMP Security Request / authenticated characteristic), so kink #5
   doesn't help and btleplug (no pairing API) can't satisfy it cleanly. Combined with a ~5 s pairing
   window, a bond it never persists, and data tearing off after 1–2 min, the module is effectively
   unusable for a stable link. Lesson: such a link can't be opened "without pairing" in software — use a
   module that doesn't force bonding (e.g. the X20RS, 0xFFF0/0xFFF6) or a wired/ELRS-backpack transport.

## Open questions (to settle during research/validation, not blocking the plan)

- Does EdgeTX/ETHOS emit raw `0x7E` S.Port frames, a decoded plain-text variant, or something else? →
  the capture answers this.
- Do the BLE modules present a profile our `ble.rs` already supports, or do we need a new one?
- Per-transport quirks (BLE chunking vs. clean serial framing) — handle in the detector/decoder if needed.
