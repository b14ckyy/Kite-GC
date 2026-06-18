# Telemetry Forwarding & Conversion ("Relay") — Plan

> ARCHIVED (2026-06-18) — Relay shipped (P1–P3: LTM/MAVLink/CRSF/SmartPort × Serial/BLE/TCP/UDP, ADR-051).
> Remaining work is **validation against real GCS/trackers** (and a few polish items) — tracked in
> `ROADMAP.md`, not active build work.

**Status:** P1+P2+P3 BUILT (2026-06-18) — LTM/MAVLink/CRSF/SmartPort encoders × Serial/BLE/TCP/UDP
outputs. TCP+LTM verified live against mwptools; the rest pending hardware/GCS validation. Not committed.

## Goal

Tap the **live** inbound telemetry already flowing through Kite, re-encode it into a chosen wire protocol
(**LTM / MAVLink / CRSF / SmartPort**), and emit it out a **second** link (serial / BLE / TCP / UDP). This
turns Kite into a telemetry transcoder/relay for **antenna trackers** (U360GTS, …), **mobile monitoring
apps**, **other GCS**, or **online streaming**. Forwarding is unrestricted — we relay everything we can
represent in the target protocol; the tracker is just one consumer.

## Scope

**In scope**
- **Live only** — tapped from the active connection's decoded telemetry. **No replay** (no Cesium-clock /
  playback forwarding; that complexity is explicitly out).
- **Any inbound source** — MSP, MAVLink, *and* the passive Telemetry mode (LTM/CRSF/SmartPort in). All
  decoders already produce the same unified events, so one tap covers every input protocol.
- **Always decode → unified state → re-encode.** Even when input protocol == output protocol (e.g.
  SmartPort in → SmartPort out) we go through the unified pipeline. **No raw byte passthrough** (see
  rationale below).
- **Multiple relays** (design for N), each with its own protocol + output transport, runnable
  concurrently.
- **Rate = paced, not per-event.** Emission is triggered by a single **pacer field — the attitude
  update** (always present, typically the highest rate; falls back to GPS for attitude-less sources). On
  each pacer tick we emit **one complete frame set** from the cache. This decouples the output rate from
  the input's framing/republish cadence: per-event emission would inflate the output badly for chunked
  inputs (e.g. the passive SmartPort decoder re-publishes *all* cached fields at 10 Hz, and a naive
  per-event mapping also rebuilt the LTM S-frame 3× per cycle). No rate configuration. Periodic-only
  frames (MAVLink heartbeat) will additionally run on a small timer later.

**Out of scope**
- Replay/playback forwarding.
- Bidirectional relays (we only push telemetry out; no command path back).
- Re-deriving fields the unified model doesn't carry.

### Why not raw passthrough?

Tapping the raw byte stream *before* decode and forwarding it verbatim was considered and rejected:
- It needs a **second tap point** (raw bytes) on top of the unified-event tap we already have for every
  input protocol — exactly the "two query points" we want to avoid.
- It **cannot convert** between protocols (the whole point of the feature).
- A single re-encode path builds any output protocol from one normalized state.

Minor accepted trade-off: fields present in the source protocol but absent from our unified model are
dropped.

## Architecture

```
 all decoders (MSP scheduler / MAVLink / passive)
        │  unified per-type updates (Attitude / GPS / Status / Analog / Airspeed / Nav …)
        ▼
   Backend telemetry tap  ──▶ emits to frontend (as today)  +  feeds the RelayHub  ← NEW
        │
        ▼
   RelayHub  (app state)
   ├─ latest-values cache (assembles multi-field frames; source for periodic frames)
   └─ fan-out to N active Relays:
       ├─ Relay 1: encoder/ltm  → output/serial  (COM5)
       ├─ Relay 2: encoder/mav  → output/tcp     (server :5760)
       └─ Relay 3: …            → output/udp | output/ble
```

- The **tap** is a backend **event listener**: the `RelayHub` registers Rust-side listeners (Tauri
  `Listener` trait) for the same `telemetry-*` events the decoders already emit, deserializes the payload
  into the unified structs and updates the cache. This needs **zero changes to the producers** (just
  `Deserialize` on the data structs) and fully decouples the relay from MSP/MAVLink/passive. The
  serialize→deserialize round-trip is negligible at ≤5 Hz. No feedback loop: relays write protocol bytes
  to an output transport, not Tauri events.
- The **latest-values cache** holds the most recent value per field, updated on every inbound event.
  Emission is **paced on the attitude update** (fallback GPS): each tick the encoder builds a full frame
  set from the cache. Later, MAVLink heartbeat-style frames run additionally on a small timer.
- **Relays are independent.** Each owns a protocol encoder + an output transport + its own stats.

### Proposed module layout (Rust)

```
src-tauri/src/telemetry_forward/
  mod.rs           — RelayHub (app state), Tauri commands, on_<type> tap entry points
  relay.rs         — one Relay: config + encoder + output transport + stats + status
  cache.rs         — latest-values telemetry cache
  encoders/
    mod.rs         — Encoder trait (feed unified update → Vec<u8> frames)
    ltm.rs         — P1
    mavlink.rs     — P3 (reuse mavlink_proto where possible)
    crsf.rs        — P3
    smartport.rs   — P3
  output/
    mod.rs         — OutputSink trait (write bytes; status; reconnect)
    serial.rs      — P1 (reuse transport::serial write)
    ble.rs         — P2 (reuse transport::ble write)
    tcp.rs         — P2 (small server: clients connect to Kite, broadcast frames)
    udp.rs         — P2 (send to configured host:port; optional broadcast)
```

Frontend: a new `RelayPanel.svelte` (dropdown under the connect bar) + a `relay` store + config in the
existing settings store + a `relayController.ts`.

## Output transports

| Kind   | Model                                                          | Status |
|--------|---------------------------------------------------------------|--------|
| Serial | write to a 2nd COM port (covers HC-05 / BT-SPP, e.g. U360GTS) | P1 built |
| TCP    | **server** — clients (Android app / GCS) connect *to* Kite (`0.0.0.0:<port>`) | P2 built |
| UDP    | send to a configured `host:port` (broadcast-capable)          | P2 built |
| BLE    | Kite as BLE central, write to a device characteristic (profile-based `connect_ble`); shown in the combined device list | P2 built |

The relay output **must be a different device** than the primary connection.

## UI — dropdown panel under the connect bar

Toggle button at the **far right of the connection bar, always visible** (`▼ Relay`). Opens an
Info-Panel-style panel that unfolds downward:

```
 toolbar: …[Connect]                                         [▼ Relay]
 ┌ Panel ──────────────────────────────────────────────────────────────┐
 │ LINK   MSP · RX 880 B/s · 42 msg/s · TX 12 B/s                       │
 │ ── Relays ──────────────────────────────────────────────────────────│
 │ ◉ [LTM ▼]     [COM5 ▼] [115200 ▼]        ● active · 3.1 kB/s         │
 │ ◉ [MAVLink ▼] [TCP :5760 ▼]              ○ waiting (no client)       │
 │ [ + Relay ]                                                          │
 └──────────────────────────────────────────────────────────────────────┘
```

- **Top: LINK diagnostics** for the primary connection — RX/TX B/s, msgs/s, active protocol. Operator-grade
  subset of the Debug Monitor stats (needs an always-available slim stats event, since the Debug Monitor
  ones are dev-gated). Shown whenever connected.
- **Below: Relays.** Each row ≈ a copy of the connect bar: protocol dropdown + **combined device dropdown**
  (serial ports + BLE devices + TCP/UDP targets in one list) + baud (only when a serial port is selected) +
  an **enable toggle** (◉) + a status dot.
- **No Start button.** Relays auto-connect with the primary path (push telemetry needs no handshake). The
  enable toggle keeps a config but pauses it.
- Status dot states: `active` / `waiting` (e.g. TCP no client) / `device missing` / `error`.

> The combined serial+BLE device dropdown is built here first. Porting it to the **main connect bar** is a
> separate follow-up refactor (the main bar also carries TCP/UDP + baud and is central/working — not
> touched mid-feature).

## Persistence & auto-connect

- Relay configs are **always fully persisted** between sessions (frontend settings store).
- On primary connect, the frontend hands the saved configs to the backend `RelayHub`, which **auto-starts
  each enabled relay whose output device is currently available**. Unavailable → status `device missing`,
  periodic retry.
- Config shape (per relay):
  ```
  { id, enabled, protocol: 'ltm'|'mavlink'|'crsf'|'smartport',
    output: { kind:'serial'|'ble'|'tcp'|'udp', port?, baud?, bleDeviceId?, host?, udpPort? } }
  ```
- Serial identity stored by path; reconnect when the path reappears.

## LTM encoder — field mapping (Phase 1)

Inverse of `passive_telemetry/decoders/ltm.rs`. Frame = `$T <type> <payload little-endian> <crc>`, where
**crc = XOR of the payload bytes only**. One A+G+S set is emitted per pacer tick (attitude update), each
sub-frame only if its source data is present in the cache.

**G-frame** (`'G'`, 14 bytes) — GPS / position:
| field        | bytes | unified source                          |
|--------------|-------|-----------------------------------------|
| latitude     | i32   | `GpsData.lat` × 1e7                      |
| longitude    | i32   | `GpsData.lon` × 1e7                      |
| ground speed | u8    | `GpsData.ground_speed` (m/s)            |
| altitude     | i32   | `GpsData.alt_msl` × 100 (cm)            |
| sats × fix   | u8    | `num_sat << 2 \| (fix_type & 0b11)`     |

**A-frame** (`'A'`, 6 bytes) — attitude:
| field   | bytes | unified source        |
|---------|-------|-----------------------|
| pitch   | i16   | `AttitudeData.pitch`  |
| roll    | i16   | `AttitudeData.roll`   |
| heading | i16   | `AttitudeData.yaw`    |

**S-frame** (`'S'`, 7 bytes) — status:
| field          | bytes | unified source                                         |
|----------------|-------|--------------------------------------------------------|
| vbat (mV)      | u16   | `AnalogData.voltage` × 1000                             |
| consumed (mAh) | u16   | `AnalogData.mah_drawn`                                  |
| rssi           | u8    | `AnalogData.rssi` (scaled to 0–255)                    |
| airspeed (m/s) | u8    | `AirspeedData.airspeed`                                 |
| status         | u8    | `flightmode << 2 \| failsafe << 1 \| armed`            |

**Detail to nail in P1:** the LTM `status` flight-mode field is an enum (0–21). Our unified flight mode is a
bitfield. We need the **inverse** of `classify_ltm_mode` — a mapping unified-mode → LTM enum for the common
modes (manual, acro, angle, horizon, althold, poshold, RTH, waypoint, cruise, …). `armed` from
`arming_flags & 0x04`; `failsafe` from the failsafe mode bit.

## Phases

- **P1 — LTM over serial (the testable core).**
  - Backend telemetry tap → `RelayHub` + latest-values cache.
  - `encoders/ltm.rs` (G/A/S) + `output/serial.rs` + a single relay.
  - Persistence + auto-connect + the dropdown panel with LINK stats + one relay row.
  - **Test target: U360GTS via HC-05 virtual COM.**
- **P2 — transports (built).** BLE / TCP-server / UDP output; multiple relay rows ("+ Relay"); combined
  Serial+BLE device dropdown.
- **P3 — encoders (built).** MAVLink (`encoders/mavlink.rs`, reuses `mavlink_proto::codec::serialize_v2`;
  HEARTBEAT @1 Hz + ATTITUDE / GPS_RAW_INT / GLOBAL_POSITION_INT / VFR_HUD / SYS_STATUS), CRSF
  (`encoders/crsf.rs`, big-endian, attitude in rad×10000, CRC8-DVB-S2; GPS/ATTITUDE/BATTERY/VARIO/
  FLIGHT_MODE), SmartPort (`encoders/smartport.rs`, 0x7E-framed sensor frames, decimal-packed modes).
  All are the inverse of the matching `passive_telemetry` decoders.
  - **Untested vs. real GCS:** MAVLink HEARTBEAT advertises a generic type + armed flag only (no vehicle
    type / custom flight mode in the unified model); CRSF/SmartPort mode strings/packing are best-effort
    inverses. Validate against Mission Planner / QGC / mwptools / an OpenTX sensor screen.

## Implementation notes (as built)

- **Port guard.** TCP listen ports must be unique (local bind); UDP unique by `host:port` (same port to
  different hosts is fine). The frontend auto-bumps to the next free port so a duplicate can't be
  configured. The backend `configure()` **diff-reconciles**: unchanged relays are reused (no rebind →
  editing one relay never drops another's TCP clients); removed/changed relays are dropped **first** +
  a 120 ms pause before rebuild, so a freed TCP listen port can rebind cleanly.
- **MAVLink pitch is negated** — MAVLink pitch+ = nose-up, our (INAV) pitch+ = nose-down. Only MAVLink;
  LTM/CRSF/SmartPort are INAV-native and expect our convention.
- **LTM carries heading, not COG** (no COG field); a consumer like mwptools computes COG from positions.
- **Relay BLE scan** runs only while the panel is open **and** the primary link is connected via a
  non-BLE transport — never fighting the main connection's own BLE scan / a BLE primary link.

## Open points

- Validate the encoders against real consumers: MAVLink vs Mission Planner / QGC, CRSF vs a handset,
  SmartPort vs an OpenTX/Ethos sensor screen; LTM/U360GTS field validation (tracker not on hand yet).
- MAVLink HEARTBEAT advertises a **generic** vehicle type + armed flag only (`custom_mode = 0`) — no
  vehicle type / named flight mode in the unified model. Map these if a real type/mode source appears.
- CRSF / SmartPort flight-mode strings/packing are best-effort inverses — refine if a consumer misreads.
- Slim always-available LINK-stats event (the panel currently reuses the dev-gated Debug Monitor stats).
- Practical max concurrent BLE relays (find empirically; serial/TCP/UDP unbounded).
- **Option B — DONE (2026-06-18).** The passive decoders (frsky/crsf/ltm) now re-emit a unified event
  only when a **fresh frame** updated that type since the last `publish()` (per-type `fresh_*` flags set in
  `apply()`, gated + cleared in `publish()`) — not all cached state every 100 ms. So passive-sourced relays
  (and widgets/recorder) now run at the real frame rate (~3 Hz) instead of the fixed 10 Hz, with no
  per-value change-detection (a fresh frame with an unchanged value still emits — correct for a static
  craft on the ground). MSP/MAVLink already emitted at their real poll rate and are unaffected.

Relates to `docs/archive/RADIO_TELEMETRY.md` (decoders = the inverse direction), the MAVLink TX work, and
the panel framework (`docs/active/PANEL_FRAMEWORK.md`).
