# Radio Telemetry (passive monitoring) — Plan

**Status:** **Planning (2026-06-16).** Interface + architecture first, then research, then a FrSky
capture-to-file for validation, then the unified-pipeline adapter. **No code until this plan is OK'd.**
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
- **Phase C — FrSky → unified pipeline adapter.** Map decoded FrSky sensors onto the existing telemetry
  events so widgets/map work live. Define which sensors map to which fields.
- **Phase D+ — more protocols.** CRSF, LTM, MAVLink-passive (decoder reuse, TX disabled). Then, later:
  `.csv` raw recording, and a decision on DB recording for telemetry-mode flights.

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

## Open questions (to settle during research/validation, not blocking the plan)

- Does EdgeTX/ETHOS emit raw `0x7E` S.Port frames, a decoded plain-text variant, or something else? →
  the capture answers this.
- Do the BLE modules present a profile our `ble.rs` already supports, or do we need a new one?
- Per-transport quirks (BLE chunking vs. clean serial framing) — handle in the detector/decoder if needed.
