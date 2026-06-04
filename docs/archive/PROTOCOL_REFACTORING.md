# Protocol Refactoring & MAVLink Integration — Workstream Plan

> **ARCHIVED (2026-06-04)** — multi-protocol workstream shipped (architecture in ADR-010, progress in ROADMAP M6); Phase 5 generic commands not pursued. See the STATUS note below.
> _Archived = out of active focus (we don't look here for what's left to build), **not** frozen — later references and tweaks are still fine._

*Created: 2026-04-19*

> **STATUS (2026-06-04): largely shipped** — architecture captured in ADR-010, progress in ROADMAP
> M6. Phases 1–4 are live (ByteTransport trait + Serial/TCP/UDP/BLE, MAVLink basis, MAVLink
> telemetry, crash-safe raw logging). Deviations from this plan as written:
> - The MAVLink module shipped as **`mavlink_proto/`** (codec / handler / handshake / parser /
>   mission), not the `mavlink/` layout sketched here, and without a standalone `decoder.rs` /
>   `streams.rs`.
> - The **MSP raw log is a pre-parsed telemetry text log, not the MWP-v2 binary `.raw`** of
>   Decision #10 (`raw_logger.rs`); the MAVLink **tlog** (`tlog_logger.rs`) is as planned.
> - **Phase 5 (generic two-way MAVLink commands — arm/disarm/mode/param) was not pursued** —
>   Kite GC is receive-only for live telemetry.
> - The MAVLink **mission protocol** (the "Future" section) shipped separately — see
>   [MISSION_MULTIAUTOPILOT_PLAN.md](../active/MISSION_MULTIAUTOPILOT_PLAN.md).
>
> **Archive candidate** (workstream done; remaining ideas tracked in ROADMAP).

This document defines the complete architecture plan for multi-protocol support (MSP + MAVLink) including transport layer refactoring, crash-safe raw logging, and unified data pipeline.

---

## Decisions (Locked)

| # | Decision | Value |
|---|----------|-------|
| 1 | MAVLink Firmware Scope | ArduPilot + PX4 + INAV MAVLink (common dialect basis) |
| 2 | MAVLink Version | v1 + v2 (backwards-compatible) |
| 3 | Rust Crate | `mavlink` crate (generated message definitions, parser, serializer) |
| 4 | Transports | Serial + TCP + UDP (all existing transports) |
| 5 | Protocol Selection | Explicit UI dropdown (no auto-detect) |
| 6 | GCS System/Component IDs | sysid=255, compid=190 (MAV_COMP_ID_MISSIONPLANNER, industry standard) |
| 7 | MAVLink Mission System | Separate modular backend, implemented AFTER live telemetry |
| 8 | Scheduler Architecture | Separate modules: MspScheduler (poll) + MavlinkHandler (push) |
| 9 | Transport Trait | New `ByteTransport` trait extracted from Serial/TCP/UDP; protocol layers on top |
| 10 | MSP Raw Log Format | MWP v2 Binary Capture (.raw) — compatible with MWP replay tools |
| 11 | MAVLink Raw Log Format | Standard tlog (.tlog) — compatible with Mission Planner |
| 12 | Recording Strategy | Raw-first → DB import after disarm → archive/delete raw file |
| 13 | Unified Data Pipeline | Early unification into TelemetryRecord (DB) + TelemetryData (Frontend) |

---

## Architecture Overview

### Current State (MSP Only)

```
┌─────────────────────────────────────────────┐
│  Frontend (Svelte)                          │
│  telemetry.ts ← Tauri events               │
│  connection.ts ← Tauri commands             │
└────────────────┬────────────────────────────┘
                 │
┌────────────────┴────────────────────────────┐
│  Backend (Rust)                             │
│  commands/connection.rs → Handshake         │
│  scheduler/ → Priority poll loop            │
│  transport/ → Transport trait (msp_request) │
│  flightlog/ → DB recorder + raw CSV logger  │
└─────────────────────────────────────────────┘
```

### Target State (MSP + MAVLink)

```
┌─────────────────────────────────────────────┐
│  Frontend (Svelte)                          │
│  telemetry.ts ← SAME Tauri events          │
│  connection.ts ← protocol dropdown added    │
└────────────────┬────────────────────────────┘
                 │
┌────────────────┴────────────────────────────┐
│  Backend (Rust)                             │
│                                             │
│  commands/connection.rs                     │
│    ├─ MSP path → MspHandshake + MspScheduler│
│    └─ MAVLink path → MavHandshake + MavHandler│
│                                             │
│  transport/                                 │
│    ├─ byte_transport.rs  (ByteTransport trait)│
│    ├─ serial.rs          (impl ByteTransport)│
│    ├─ tcp.rs             (impl ByteTransport)│
│    ├─ udp.rs             (impl ByteTransport)│
│    ├─ ble.rs             (impl ByteTransport)│
│    └─ msp_transport.rs   (MSP framing layer)│
│                                             │
│  msp/                                       │
│    ├─ codec.rs           (existing)          │
│    ├─ scheduler.rs       (extracted from scheduler/)│
│    └─ ...                                    │
│                                             │
│  mavlink/                                   │
│    ├─ mod.rs             (module root)       │
│    ├─ handler.rs         (reader thread + heartbeat)│
│    ├─ decoder.rs         (message → telemetry mapping)│
│    ├─ commands.rs        (command/ACK state machine)│
│    └─ streams.rs         (SET_MESSAGE_INTERVAL config)│
│                                             │
│  flightlog/                                 │
│    ├─ raw_logger.rs      (refactored: MWP v2 binary)│
│    ├─ tlog_logger.rs     (new: MAVLink tlog) │
│    ├─ raw_importer.rs    (new: raw → DB after disarm)│
│    ├─ recorder.rs        (simplified: delegates to raw loggers)│
│    └─ ...                                    │
└─────────────────────────────────────────────┘
```

---

## Transport Layer Refactoring

### ByteTransport Trait

```rust
/// Low-level byte-stream transport — protocol agnostic.
pub trait ByteTransport: Send {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, TransportError>;
    fn write(&mut self, data: &[u8]) -> Result<usize, TransportError>;
    fn description(&self) -> String;
    fn close(&mut self) -> Result<(), TransportError>;
}
```

All existing transports (Serial, TCP, UDP, BLE) implement `ByteTransport`. The current `Transport` trait with `msp_request()` becomes `MspTransport` — a wrapper that adds MSP v1/v2 framing on top of any `ByteTransport`.

### Layer Stack

```
Application Layer:   MspScheduler / MavlinkHandler
                         │                │
Protocol Layer:     MspTransport    MavlinkCodec (mavlink crate)
                         │                │
Transport Layer:    ────── ByteTransport ──────
                    Serial │ TCP │ UDP │ BLE
```

---

## MSP Scheduler (Existing, Refactored)

No functional changes — only structural:
- Extract from `scheduler/mod.rs` into `msp/scheduler.rs`
- Use `MspTransport` wrapper instead of raw `Transport` trait
- Same priority-based poll loop, same telemetry slots, same command channel

---

## MAVLink Handler (New)

### Architecture

```
MavlinkHandler
├── Reader Thread (continuous)
│   ├─ Read bytes from ByteTransport
│   ├─ Parse MAVLink frames (mavlink crate)
│   ├─ Dispatch to decoder → Tauri events
│   └─ Route COMMAND_ACK to pending command waiters
│
├── Heartbeat Writer (1 Hz timer)
│   └─ Send GCS HEARTBEAT every 1 second
│
└── Command Channel (mpsc)
    ├─ SendCommand(COMMAND_LONG) → wait for ACK
    ├─ RequestStreams(SET_MESSAGE_INTERVAL)
    └─ Stop
```

### Push Telemetry vs MSP Polling

| Aspect | MSP | MAVLink |
|--------|-----|---------|
| Data flow | GCS polls FC | FC pushes to GCS |
| Rate control | GCS controls poll rate | GCS requests rate via SET_MESSAGE_INTERVAL |
| Keepalive | Implicit (constant polling) | Explicit GCS HEARTBEAT @1Hz required |
| Arm detection | Poll MSPV2_INAV_STATUS arming_flags | HEARTBEAT base_mode & MAV_MODE_FLAG_SAFETY_ARMED |
| Handshake | Sequential MSP requests | Wait for HEARTBEAT, send stream requests |

### MAVLink Message Scope (Phase 1)

**Receive:**

| Message | ID | Purpose | Widget Fields |
|---------|----|---------|---------------|
| HEARTBEAT | 0 | Arm state, flight mode, autopilot type | armingFlags, flightMode, fcVariant |
| SYS_STATUS | 1 | Battery, sensor health, CPU load | voltage, current, sensorStatus, cpuLoad |
| GPS_RAW_INT | 24 | GPS fix, satellites, HDOP | fixType, numSat, gpsHdop |
| ATTITUDE | 30 | Roll, pitch, yaw | roll, pitch, yaw |
| GLOBAL_POSITION_INT | 33 | Lat, lon, alt, heading, vario | lat, lon, altMsl, heading, vario |
| VFR_HUD | 74 | Airspeed, groundspeed, alt, climb, heading, throttle | airspeed, groundSpeed, altitude, vario |
| BATTERY_STATUS | 147 | Detailed battery info | mAhDrawn, cellCount |
| RC_CHANNELS | 65 | RC input + RSSI | rssi |
| HOME_POSITION | 242 | Home lat/lon/alt | homeDistance, homeBearing |
| STATUSTEXT | 253 | FC text messages | (log/display only) |

**Send:**

| Message | ID | Purpose |
|---------|----|---------|
| HEARTBEAT | 0 | GCS keepalive @1Hz |
| SET_MESSAGE_INTERVAL | 511 | Request stream rates |
| COMMAND_LONG | 76 | General commands (arm, disarm, etc.) |
| MISSION_* | various | Mission protocol (Phase 5) |

---

## Crash-Safe Raw Logging

### MSP Raw Log: MWP v2 Binary Capture Format

Compatible with MWP's `mwp-log-replay` tool.

```
File header: "v2\n" (3 bytes: 0x76 0x32 0x0A)

Repeated records:
┌──────────────────┬──────────────┬───────────┬─────────────────────┐
│ time_offset (f64)│ data_size(u16)│ dir (u8)  │ raw_bytes[data_size]│
│ 8 bytes LE       │ 2 bytes LE   │ 'i' / 'o' │ variable            │
└──────────────────┴──────────────┴───────────┴─────────────────────┘
```

- `time_offset`: Seconds since recording start (IEEE 754 double, LE)
- `data_size`: Size of raw MSP frame bytes
- `direction`: `0x69` ('i') = received from FC, `0x6F` ('o') = sent to FC
- `raw_bytes`: Complete MSP v1/v2 frame including header, length, CRC

### MAVLink Raw Log: Standard tlog Format

Compatible with Mission Planner, QGroundControl, MAVExplorer.

```
Repeated records:
┌──────────────────────┬─────────────────────────────┐
│ timestamp_usec (u64) │ raw_mavlink_frame (variable) │
│ 8 bytes LE           │ MAVLink v1 or v2 frame       │
└──────────────────────┴─────────────────────────────┘
```

### Recording Flow

```
ARM detected
  ├─ Open raw log file:
  │    MSP:     raw_logs/{timestamp}_{craft_name}.raw
  │    MAVLink: raw_logs/{timestamp}_{craft_name}.tlog
  ├─ Start appending every frame received/sent
  │
DISARM detected
  ├─ Close raw log file
  ├─ Parse raw log → extract telemetry snapshots
  ├─ Insert flight + telemetry_records into SQLite DB
  ├─ Async enrichment (weather, geocode) — same as today
  └─ Move raw log to archive/ folder (or delete per setting)
```

### Settings

| Setting | Default | Options |
|---------|---------|---------|
| Raw Log Archive | Keep | Keep (archive/) / Delete after import / Disabled |
| Archive Path | `{db_path}/raw_archive/` | User configurable |

---

## Implementation Phases

### Phase 1: Transport Refactoring

**Goal:** Extract `ByteTransport` trait without breaking existing MSP functionality.

**Files to create/modify:**
- `transport/byte_transport.rs` — New trait definition
- `transport/serial.rs` — Implement `ByteTransport` (extract from existing `Transport`)
- `transport/tcp.rs` — Implement `ByteTransport`
- `transport/udp.rs` — Implement `ByteTransport`
- `transport/ble.rs` — Implement `ByteTransport`
- `transport/msp_transport.rs` — MSP framing wrapper over `ByteTransport`
- `transport/mod.rs` — Re-export both traits

**Validation:** All existing MSP functionality must work unchanged after refactoring.

### Phase 2: MAVLink Basis

**Goal:** `mavlink` crate integrated, MavlinkHandler skeleton, protocol dropdown in UI.

**Files to create/modify:**
- `Cargo.toml` — Add `mavlink` dependency
- `mavlink/mod.rs` — Module root
- `mavlink/handler.rs` — Reader thread + heartbeat writer skeleton
- `mavlink/handshake.rs` — Wait for HEARTBEAT, extract FC info
- `commands/connection.rs` — Add MAVLink connect path
- Frontend: `connection.ts` — Add protocol type field
- Frontend: `Toolbar.svelte` — Protocol dropdown

### Phase 3: MAVLink Telemetry

**Goal:** All widgets fed by MAVLink data, identical to MSP experience.

**Files to create/modify:**
- `mavlink/decoder.rs` — Message → TelemetrySnapshot mapping
- `mavlink/streams.rs` — SET_MESSAGE_INTERVAL requests
- `mavlink/handler.rs` — Wire decoder to Tauri events
- Emit SAME Tauri event names as MSP (`telemetry-gps`, `telemetry-attitude`, etc.)

### Phase 4: Raw Logging (Crash-Safe)

**Goal:** Crash-safe recording for both protocols, DB import after disarm.

**Files to create/modify:**
- `flightlog/raw_logger.rs` — Rewrite: MWP v2 binary format for MSP
- `flightlog/tlog_logger.rs` — New: MAVLink tlog format
- `flightlog/raw_importer.rs` — New: Parse raw logs → DB after disarm
- `flightlog/recorder.rs` — Simplify: delegate to protocol-specific loggers
- Settings: raw log archive mode

### Phase 5: MAVLink Commands

**Goal:** Two-way command support for arming, mode changes, parameter access.

**Files to create/modify:**
- `mavlink/commands.rs` — COMMAND_LONG + COMMAND_ACK state machine
- `mavlink/params.rs` — PARAM_VALUE / PARAM_SET (optional)

### Future: MAVLink Mission Protocol

**Deferred** — separate workstream after live telemetry is stable.

- `mission/mavlink_codec.rs` — MISSION_COUNT/REQUEST_INT/ITEM_INT/ACK state machine
- `mission/mod.rs` — Abstract mission trait (MSP WP vs MAVLink Waypoints)
- Frontend: mission store abstraction

---

## Unified Data Pipeline

Both protocols converge at the **same** data interfaces:

```
MSP Frames ──→ MSP Decoder ──→ TelemetrySnapshot ──→ Tauri Events ──→ TelemetryData (Svelte)
                                       │
MAV Frames ──→ MAV Decoder ──→ TelemetrySnapshot ──→ Tauri Events ──→ TelemetryData (Svelte)
                                       │
                                  Raw Logger ──→ .raw / .tlog file
                                       │
                              (after disarm)
                                       │
                                  DB Importer ──→ TelemetryRecord (SQLite)
```

The frontend never knows which protocol is active — it receives identical Tauri events. The only protocol-specific UI is the connect dialog (protocol dropdown + transport settings).

---

## Module Structure Reference

```
src-tauri/src/
├── transport/
│   ├── mod.rs              # Re-exports
│   ├── byte_transport.rs   # ByteTransport trait (NEW)
│   ├── msp_transport.rs    # MSP framing over ByteTransport (NEW)
│   ├── serial.rs           # impl ByteTransport (REFACTORED)
│   ├── tcp.rs              # impl ByteTransport (REFACTORED)
│   ├── udp.rs              # impl ByteTransport (REFACTORED)
│   └── ble.rs              # impl ByteTransport (REFACTORED)
├── msp/
│   ├── mod.rs              # Re-exports
│   ├── codec.rs            # MSP v1/v2 encode/decode (existing)
│   ├── features.rs         # Version gating (existing)
│   ├── scheduler.rs        # Priority poll loop (MOVED from scheduler/)
│   └── constants.rs        # MSP codes (existing)
├── mavlink/
│   ├── mod.rs              # Module root (NEW)
│   ├── handler.rs          # Reader thread + heartbeat (NEW)
│   ├── handshake.rs        # HEARTBEAT wait + FC info (NEW)
│   ├── decoder.rs          # Message → telemetry (NEW)
│   ├── streams.rs          # Stream rate requests (NEW)
│   └── commands.rs         # COMMAND_LONG/ACK (NEW)
├── flightlog/
│   ├── mod.rs              # Re-exports
│   ├── recorder.rs         # Session lifecycle (SIMPLIFIED)
│   ├── raw_logger.rs       # MWP v2 binary format (REWRITTEN)
│   ├── tlog_logger.rs      # MAVLink tlog format (NEW)
│   ├── raw_importer.rs     # Raw → DB after disarm (NEW)
│   ├── db.rs               # SQLite operations (existing)
│   └── types.rs            # Shared types (existing)
├── commands/
│   ├── connection.rs       # Connect/disconnect (EXTENDED)
│   └── ...
└── state.rs                # AppState (EXTENDED: protocol type)
```

---

*This document will be updated as implementation progresses.*
