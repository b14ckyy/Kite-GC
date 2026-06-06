# Radar Tracking — Core & Sources (Plan A)

> Status: **In progress** (2026-06-06). **Phase 0 (core) + Phase 1 (ADS-B online) + Phase 2 (ADS-B
> serial MAVLink receiver) shipped**; next: ADS-B TCP transport, then Phase 3 (FormationFlight).
> Concrete plan for the *foreign-vehicle tracking* subsystem ("Radar").
> This doc covers the **backend subsystem + data sources** (Phases 0–3). The user-facing **Advanced
> Panel + map visualization** is a separate plan — see `RADAR_TRACKING_PANEL_AND_MAP.md` (Plan B). An
> ADR will be written once the core architecture is locked.
>
> **Agreed framing decisions (2026-06-06):**
> - **Name:** *Radar* (internal module `radar/`, NavRail tab `radar`) — even though it isn't a real
>   radar; the term is established (FormationFlight née INAV Radar, MWP).
> - **Split:** two plan docs (this = Core & Sources; Plan B = Panel & Map).
> - **First real source:** ADS-B **online** (testable without hardware).
> - **Dedup: per *system*, not cross-system.** Within ADS-B, multiple feeds (several online providers
>   for coverage + a local receiver/MSP download) are merged into one list keyed by **vehicle ID
>   (ICAO)**. ADS-B / FormationFlight / Radio stay **separate** lists (matches the stacked-list UI). In
>   practice cross-system collisions don't happen: radio telemetry is rarely used (and not alongside
>   FormationFlight), and the user's own UAVs don't transmit ADS-B.
>
> **Refinements (2026-06-06 hints):**
> - **Heterogeneous data is first-class:** each system has its own fields; `TrackedVehicle` is an
>   optional-field superset, no lossy common schema (§4).
> - **No simulator for ADS-B** — online gives real data; ADS-B-receiver and FormationFlight **hardware are
>   on hand** (bench-testable, no flight). Phasing reordered by testability (§9).
> - **ADS-B receiver = MAVLink-ADSB over *both* Serial USB and WiFi/network** — one decode loop, two
>   transports (§7.1).
> - **Radio telemetry reuses a shared telemetry parser** (also planned for primary connections) with
>   two sinks — main pipeline + filtered radar feed; hence sequenced **last** (§7.3).

---

## 1. Goals & non-goals

**Goal:** track *other* vehicles on the map — beyond the single MSP/MAVLink-connected main UAV —
via several independent systems, in a background subsystem that **does not touch or compromise** the
existing telemetry path.

**In scope (this plan):**
- A standalone Rust `radar` subsystem: source management, normalized vehicle model, dedup/TTL, events.
- Three source families: **ADS-B**, **FormationFlight** (the ESP32 mesh radar, formerly INAV-Radar),
  **Radio telemetry** (CRSF /
  MAVLink-telemetry / FrSkyX).
- Frontend store that mirrors the consolidated vehicle state.

**Out of scope (→ Plan B):** the Advanced Panel UI, settings UI, and all map rendering.

**Non-goals:** collision avoidance / TCAS logic, recording foreign tracks to the flight DB, controlling
foreign vehicles. (Possible future work, explicitly deferred.)

---

## 2. Why a separate subsystem (the hard constraint)

The main link is **exclusive**: `AppState.protocol` holds exactly one `ActiveProtocol::Msp | Mavlink`
([state.rs](../../src-tauri/src/state.rs)), and the scheduler owns the serial port on a dedicated
thread ([scheduler/mod.rs](../../src-tauri/src/scheduler/mod.rs)). The radar subsystem therefore:

- lives in its **own** `AppState` field (`radar: Mutex<RadarManager>`), with its **own** threads and
  its **own** transports — never a second handle to the main link;
- emits its **own** Tauri event(s); the existing `telemetry-*` events are untouched;
- has **two** classes of source by where the bytes come from:
  1. **Manager-owned** sources on their own transport (online HTTP, network TCP/UDP, a dedicated
     FormationFlight serial receiver, a dedicated MAVLink ADS-B receiver). The `RadarManager` opens and
     owns these.
  2. **FC-shared** sources whose data rides the *main link* the scheduler already owns (ADS-B
     downloaded from the connected INAV UAV via MSP; radio telemetry relayed through the FC). These
     **must not** open a second handle — instead the scheduler gains an optional low-priority radar
     poll slot that feeds the radar pipeline (see §6).

This keeps the "scheduler owns the serial connection exclusively" rule intact.

---

## 3. Module layout (Rust)

```
src-tauri/src/radar/
  mod.rs            # RadarManager: owns sources, merge/TTL, emits `radar-vehicles`
  vehicle.rs        # TrackedVehicle, VehicleSystem, source enums, validity flags
  manager.rs        # source registry + lifecycle (start/stop/reconfigure), per-system stores
  source.rs         # RadarSource trait + SourceHandle + update channel types
  dedup.rs          # per-system merge (by vehicle id) + TTL expiry
  sources/
    sim.rs          # dev-only, optional: synthetic vehicles (NOT a required deliverable)
    adsb_online.rs  # Phase 1: HTTP pollers (OpenSky / adsb.fi / adsb.lol / airplanes.live)
    adsb_mavlink.rs # Phase 2: MAVLink ADSB_VEHICLE receiver — serial OR WiFi/network (one decode loop)
    adsb_msp.rs     # Phase 4: ADS-B list downloaded from the FC (scheduler-fed)
    formationflight.rs # Phase 3: FormationFlight (ESP32) receiver over MSP (own serial)
    radio_*.rs      # Phase 5: built on the shared telemetry parser's radar sink (CRSF / MAVLink#33 / FrSkyX)
commands/radar.rs   # tauri commands: radar_configure / radar_set_source_enabled / radar_snapshot ...
```

`AppState` gains `pub radar: Mutex<RadarManager>` (constructed in `AppState::new`, idle until a source
is enabled). New commands registered in [lib.rs](../../src-tauri/src/lib.rs).

---

## 4. Data model (`vehicle.rs`)

```rust
pub enum VehicleSystem { Adsb, FormationFlight, Radio }   // the three top-level groups (separate lists)

pub enum VehicleSource {                            // concrete feed within a system
    AdsbOnline(&'static str /* provider */),
    AdsbReceiver, AdsbMsp,
    FormationFlight,
    RadioCrsf, RadioMavlink, RadioFrsky,
}

pub struct TrackedVehicle {
    pub id: String,            // stable per system: ICAO hex (ADS-B) | peer id/MAC (radar) | sysid (radio)
    pub system: VehicleSystem,
    pub sources: Vec<VehicleSource>,   // which feeds currently report this id (after per-system merge)
    pub callsign: Option<String>,
    pub lat: f64, pub lon: f64,
    pub alt_m: Option<f64>,            // see alt_ref
    pub alt_ref: AltRef,               // BaroMsl | GeoMsl | Relative | Unknown
    pub heading_deg: Option<f64>,
    pub ground_speed_ms: Option<f64>,
    pub vertical_speed_ms: Option<f64>,
    pub category: Option<VehicleCategory>,  // light/large/rotorcraft/uav/... when known
    pub signal: Option<f64>,           // rssi/snr where available
    pub squawk: Option<u16>,           // ADS-B only
    pub last_seen_ms: i64,             // unix ms; drives TTL + UI "age"
    pub valid_pos: bool,               // has a usable lat/lon this update
}
```

Relative bearing / distance / relative-altitude to the user are **derived in the frontend** from the
existing user location ([userLocation.ts](../../src/lib/helpers/userLocation.ts)) + home — keeps the
backend location-agnostic and avoids per-frame recompute on the Rust side.

**Heterogeneous fields are expected, not a flaw.** The systems carry genuinely different data: ADS-B
(ICAO, squawk, FL, emitter category), FormationFlight (peer name/id, LoRa RSSI, relative position), radio
telemetry (whatever the link reports). `TrackedVehicle` is therefore a **superset with optional
fields** — each source fills only what it natively has; the rest stay `None`. No lossy "lowest common
denominator" coercion. The per-system lists (Plan B) can show **different columns per system**, so the
model never has to pretend FormationFlight has a squawk or ADS-B has a LoRa RSSI. If a source has native
fields with no model slot, they go in a small per-system `extra` map rather than bloating the struct.

---

## 5. Source abstraction & manager

```rust
pub trait RadarSource: Send {
    fn system(&self) -> VehicleSystem;
    fn source(&self) -> VehicleSource;
    /// Spawn the source's worker (thread or async task); it pushes batches into `tx`.
    fn start(self: Box<Self>, tx: mpsc::Sender<SourceUpdate>) -> SourceHandle;
}

pub struct SourceUpdate { pub source: VehicleSource, pub vehicles: Vec<TrackedVehicle> }
```

- Each source runs **independently** (HTTP pollers on a thread with their own interval; serial
  receivers in a read loop; the scheduler-fed ones push from the scheduler thread — see §6).
- The `RadarManager` runs a single **aggregator** loop: drains `SourceUpdate`s, applies **per-system
  dedup** (`dedup.rs`: group by `system`, key by `id`, last-write-wins per field with the freshest
  `last_seen_ms`; `sources` accumulates which feeds reported it), prunes entries older than the
  per-system TTL, and emits a consolidated snapshot.
- **Event:** `radar-vehicles` — a snapshot `{ adsb: [...], formationFlight: [...], radio: [...], stats }`.
  Emitted on change, throttled (e.g. ≤5 Hz) to avoid flooding the WebView. Same event name
  regardless of which sources are active (mirrors the project's protocol-agnostic event rule).
- TTL defaults (configurable): ADS-B 30–60 s, FormationFlight ~10 s, Radio ~8 s.

**Commands (`commands/radar.rs`):**
`radar_configure(config)` · `radar_set_source_enabled(source, bool, params)` ·
`radar_snapshot()` (pull current state on panel open) · `radar_clear()`. All return
`Result<_, String>`, errors/logs in English.

---

## 6. FC-shared sources via the scheduler (no second handle)

ADS-B-from-the-FC and FC-relayed radio telemetry ride the main link. Plan:

- Add an **optional radar poll slot** to the scheduler ([scheduler/mod.rs](../../src-tauri/src/scheduler/mod.rs)),
  lowest priority, e.g. 1–2 Hz, enabled only when an FC-shared radar source is on. It polls the
  relevant MSP code (e.g. the INAV ADS-B vehicle list), decodes to `TrackedVehicle`s, and pushes them
  into the radar manager's channel (a `Sender` handed to the scheduler at start, or via an
  `AppState`-held bridge).
- This reuses the existing `SchedulerHandle::msp_request` path and the priority-degradation logic —
  radar polling naturally yields to attitude/status/GPS when bandwidth is tight.
- Gating: only meaningful for the **INAV/MSP** main connection; a feature/version check decides whether
  the ADS-B list MSP code is available (extend [msp/features.rs](../../src-tauri/src/msp/features.rs)).

---

## 7. Source families — protocol notes

Protocol details are taken as **reference** from INAV firmware source and **MWPTools** — **no code is
copied**. Exact message codes/endpoints are confirmed during implementation.

### 7.1 ADS-B (system `Adsb`)
- **Online (Phase 1 — SHIPPED):** an async `reqwest` poller ([sources/adsb_online.rs](../../src-tauri/src/radar/sources/adsb_online.rs))
  polls all enabled providers each cycle (sequentially) within a **radius** (point query) of a **live
  query centre** and merges them by ICAO.
  - **Query centre = the connected UAV's position (valid fix) else the map viewport centre** — so the
    user can scan anywhere by panning when no UAV is connected. Updated live via `radar_set_center`
    (no pipeline restart); the frontend pushes it on telemetry / map-pan / connection change.
  - **Providers (verified):** **adsb.lol**, **adsb.one** (both *built-in* — fixed URL, toggle-only,
    not editable/removable), **adsb.fi** (a *custom* example row, editable/removable). Endpoints (the
    "re-api" / readsb family, `ac[]` JSON):
    - adsb.lol / adsb.one: `https://api.adsb.{lol,one}/v2/point/{lat}/{lon}/{dist}` (dist = NM).
    - adsb.fi: `https://opendata.adsb.fi/api/v3/lat/{lat}/lon/{lon}/dist/{dist}`.
    - **Rate limit ≈ 1 req/s per provider** — bursts (e.g. dev config-restarts) can get the IP
      temporarily blocked.
  - **URL template** with `{lat}`/`{lon}`/`{dist}` placeholders covers any provider path (built-in or
    custom), `{dist}` filled in NM.
  - **Params:** **radius** km dropdown (10/25/50/75/100, default 25, cap 100) + **poll interval**
    dropdown (2/5/10/30 s, default 5; ≥2 s for the 1 req/s limit).
  - **Per-provider status:** after each cycle the source emits `radar-adsb-status` (`[{name, count,
    ok}]`); the panel shows a green contact-count badge or a red ✕ on error per enabled provider.
  - **Response → `TrackedVehicle`:** `hex`→`id`, `flight`→callsign, `lat`/`lon`, `alt_baro` ft→m,
    `gs` kt→m/s, `track`→heading, `baro_rate` fpm→m/s, `squawk`, `category` (defensive — `alt_baro`
    may be `"ground"`).
  - **Other providers** (OpenSky bbox + auth; airplanes.live) can be added as custom rows.
- **Dedicated receiver (MAVLink-ADSB) — Phase 2, SHIPPED (serial).** ([sources/adsb_mavlink.rs](../../src-tauri/src/radar/sources/adsb_mavlink.rs))
  A dedicated thread opens the serial port, parses frames with the project's `MavParser` (v1+v2),
  decodes `ADSB_VEHICLE` (#246) → TrackedVehicle, pushes the working set every 1 s (merged with the
  online feeds by ICAO), reconnects on error, and emits the same per-provider `radar-adsb-status`.
  - **DTR/RTS gotcha (important):** many USB-serial ADS-B receivers (ADSBee, PicoADSB) only stream
    once the host **raises DTR + RTS** — terminals do this, a bare `serialport::open()` does not, so
    the port opened green but read **0 bytes**. Fixed via `SerialConnection::set_control_signals(true,
    true)`, called only by the radar receiver (the FC path is unchanged).
  - **TCP/WiFi (same MAVLink decode) — pending.** Confirmed: the **ADSBee** exposes a USB/UART/**TCP**
    sink (RP2040 + ESP32-S3, MAVLink1/2 #246); PicoADSB is serial. TCP transport added next.
  - Verified live: ~9 frames/s, contacts match Flightradar.
  - (Possible future: selectable per-source protocol parser — MAVLink / Beast / raw / GDL90 — if a
    device can't do MAVLink. Not needed so far.)
- **MSP from FC (later):** the connected INAV UAV's onboard ADS-B receiver list, via the scheduler
  radar slot (§6). Confirm the INAV MSP code for the ADS-B vehicle list.
- **Network feeder (optional, low priority):** a local dump1090 over TCP — JSON `aircraft.json` or
  SBS/BaseStation port 30003. Only if a non-MAVLink feeder is actually wanted; the user's hardware is
  MAVLink, so this is deferred/optional.
- **Dedup:** all the above merge into **one ADS-B list keyed by ICAO** (online + MAVLink receiver +
  MSP-from-FC).
- **No simulator needed for ADS-B** — online feeds give real data without hardware, and the receiver +
  FormationFlight hardware are on hand. The Phase-0 `sim` source is therefore **optional/dev-only** (see §9),
  not a required deliverable.

> **Reference — MWP "Radar View" (for the protocol/URLs only, not the UX):**
> - User guide: https://mwp-user-guide.pages.dev/mwp-Radar-View/ · source: https://codeberg.org/stronnag/mwptools
> - MWP encodes a source as an `adsbx://host?range=R&interval=T[&format=path/{}/{}/{}][&api-key=key:value]`
>   pseudo-URI; defaults are `adsbx://api.adsb.lol?range=40&interval=1200` and `adsbx://api.adsb.one?...`,
>   with adsb.fi via the explicit `format=api/v2/lat/{}/lon/{}/dist/{}`. Range NM (≤250), interval ms (≥1000).
> - **We do NOT copy MWP's file-based, manual config UX.** We take only the providers + endpoint shapes and
>   present a clean **Name / API-URI / API-Key** row table in the panel (Plan B §3.2), pre-filled with the
>   three free defaults. No code copied.

### 7.2 FormationFlight (system `FormationFlight`, Phase 3)
- Open-source ESP32 mesh-radar project — **formerly INAV-Radar / ESP32-INAV-Radar, now FormationFlight.**
  Peers exchange position (LoRa), a receiver module speaks **MSP**. A **dedicated receiver** connects to
  Kite on its **own serial port** (manager-owned), and we poll the peer table. Up to ~4–6 peers.
- Repos to validate the MSP messages against (both reportedly use the same MSP messages — confirm):
  - Current: https://github.com/FormationFlight/FormationFlight
  - Original: https://github.com/OlivierC-FR/ESP32-INAV-Radar
- Confirm which MSP message exposes the peer list and its layout (INAV has `MSP2_COMMON_SET_RADAR_POS` /
  `..._ITD` for *pushing* peers into the FC; the GCS read path from a FormationFlight receiver needs
  confirming from the repos + MWPTools). Peer id (MAC/index) → `id`. **No code copied.**

### 7.3 Radio telemetry (system `Radio`, **last** — gated on the shared telemetry parser)
- **Built on a shared telemetry parser.** A CRSF / MAVLink-telemetry / FrSkyX parser is already on the
  roadmap as an **option for primary connections** (an alternative telemetry input alongside MSP /
  MAVLink). That **same parser is reused here** with **two sinks**:
  1. **Main pipeline** — full telemetry → the telemetry store (primary-connection use).
  2. **Radar tracker** — a *filtered* view (other vehicles' id + position/velocity only) → the radar
     manager.
  So radio-telemetry radar is **deferred until that parser exists**, then wired with the second sink —
  no duplicate protocol code.
- Per protocol the radar sink extracts: **CRSF** GPS frame (lat/lon/groundspeed/heading/alt/sats);
  **MAVLink** `GLOBAL_POSITION_INT` (#33) from *other* system IDs (multi-vehicle), kept distinct from
  the main FC link; **FrSkyX / SmartPort** GPS sensor packets.
- Lowest priority family (the user notes radio telemetry is rarely used in practice, and not alongside
  FormationFlight). Sequenced last for this reason **and** the parser dependency.

---

## 8. Settings (persisted in the existing `settings` store)

New `radar` block (frontend `settings` + mirrored into the backend via `radar_configure`). The
**master switch + per-system enables surface in Main Settings → Data → "Telemetry"** (renamed from
"Telemetry Rates"); the **detailed source lists are edited in the Radar panel** (Plan B §2–3) but
persist in the same block:

```ts
radar: {
  enabled: boolean;                          // master — off hides the whole panel/feature
  adsb: {
    enabled: boolean;                        // system toggle (Settings)
    online: { name: string; url: string; apiKey?: string; enabled?: boolean }[];  // panel-edited rows
    hard:   { transport: 'serial'|'network'|'bluetooth'; params: {...}; enabled?: boolean }[];
    mspFromFc: boolean;                      // ADS-B list via the scheduler radar slot
    radiusKm: 10 | 25 | 50 | 75 | 100;       // dropdown, default 25, hard cap 100 km
    pollSec: number; ttlSec: number;
  };
  formationFlight: {                               // UI label "FormationFlight"
    enabled: boolean;
    hard: { transport: 'serial'; params: {...}; enabled?: boolean }[];  // serial only
    ttlSec: number;
  };
  radio: {                                   // UI label "Radio Telemetry" (built last)
    enabled: boolean;
    hard: { transport: 'serial'|'bluetooth'; params: {...}; enabled?: boolean }[];
    ttlSec: number;
  };
}
```

All radar settings default **off** — zero impact until explicitly enabled. (`hard.params` shape per
transport — port/baud, host/port, BLE id — defined when those rows are built.)

---

## 9. Phasing

Reordered by **testability** — the user has ADS-B-receiver and FormationFlight hardware on hand, and online
ADS-B needs none; radio telemetry is last because it depends on the shared telemetry parser (§7.3).

- **Phase 0 — Core skeleton.** `radar` module (manager, vehicle model, source trait, dedup/TTL),
  `AppState.radar`, commands, `radar-vehicles` event, frontend store. The first real source (ADS-B
  online, Phase 1) is the pipeline driver — **no required sim source**; a trivial `sim` may exist as a
  dev-only toggle but isn't a deliverable. → write the ADR at the end of this phase.
- **Phase 1 — ADS-B online. ✅ SHIPPED.** Async pollers, multi-provider merge by ICAO, radius +
  poll-interval dropdowns, live query centre (viewport / UAV), built-in vs custom providers,
  per-provider status badges. adsb.lol / adsb.one / adsb.fi verified.
- **Phase 2 — ADS-B MAVLink receiver. ✅ SHIPPED (serial; TCP next).** Serial `ADSB_VEHICLE` (#246)
  reader, merged with online by ICAO, DTR/RTS fix, port-dropdown + per-source status in the panel.
  TCP/WiFi (same decode) is the remaining transport (ADSBee has it).
- **Phase 3 — FormationFlight.** Own serial; decode the peer table. Hardware on hand and well
  documented → bench test (module on a craft + USB to PC, no flight needed) confirms we read peers
  correctly.
- **Phase 4 — ADS-B from FC (MSP).** Scheduler radar slot (§6); INAV ADS-B list decode; INAV/version
  gate. Optional non-MAVLink network feeder (dump1090) only if wanted.
- **Phase 5 — Radio telemetry.** After the shared telemetry parser lands: wire its second (radar) sink;
  CRSF / MAVLink-#33 / FrSkyX.

Each phase is independently shippable (a source can be enabled/disabled in isolation).

---

## 10. Risks / open questions

- **Exact protocol codes/layouts** (INAV ADS-B list MSP code; FormationFlight peer-read mechanism;
  per-provider JSON shapes) — confirmed during implementation from INAV source + MWPTools.
- **Online API limits / keys** — OpenSky anonymous limits; some providers need a key. The settings
  schema carries optional `apiKey`/`url` per provider.
- **Throughput** — `radar-vehicles` snapshots must be throttled; consider diff events if lists get
  large (hundreds of ADS-B contacts in dense airspace) — measure in Phase 1.
- **Scheduler budget** — the radar MSP slot must stay lowest priority so it never starves attitude/GPS.
- **Offline/field use** — online ADS-B needs connectivity; receivers/MSP/radar work offline.
