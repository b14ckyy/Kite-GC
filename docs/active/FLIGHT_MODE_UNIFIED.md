# Unified Flight-Mode Pipeline — Protocol-Agnostic Mode Model

> **STATUS: IMPLEMENTED (DB v12) — decision recorded in ADR-044.** The flight-mode path is now
> protocol-agnostic: protocol adapters classify raw mode data into a canonical model; the pipeline,
> widget, track-coloring and recording/replay consume only that model. Future protocols (CRSF /
> Smartport / Betaflight) become an adapter + a few registry entries — no pipeline or widget surgery.
> Kept as the detailed implementation reference (code comments point here); DATA_PIPELINE.md has the
> shipped data-flow summary.

## Problem

Today a single `u32` (`flight_mode_flags` / `activeFlightModeFlags`) carries **two different
semantics**, discriminated downstream by `fcVariant`:

- **INAV/MSP** — a **bitmask** of active mode boxes (normalized to our `FLIGHT_MODE` bits in
  `scheduler/telemetry.rs`); the frontend `classifyFlightMode` priority-selects a primary + extracts
  modifier chips (ALT/HDG/…).
- **ArduPilot/MAVLink** — a **single raw `custom_mode` enum**; the frontend `classifyArduPilotMode`
  looks it up in a per-vehicle table.

So the widget + track-coloring are **protocol-aware** (`classifyMode(flags, fcVariant)`), and raw
protocol values flow through the unified pipeline — which contradicts the adapter architecture and
will force pipeline+widget changes again when CRSF/Smartport land. INAV's rich **primary + modifiers**
stacking (Horizon+Alt, Acro+Tune) must be preserved; ArduPilot's flat single mode must stay correct.

## Principle

> **Classify in the input adapter. Carry canonical through the pipeline. Present via one output registry.**

Raw bits/enums die in the adapter. From there only a canonical `FlightModeState` flows. The widget and
track-coloring never see protocol details again.

## Canonical contract

Rust (backend, serialized to the frontend + recorder):

```rust
// flightmode/mod.rs (new)
#[derive(Clone, Serialize)]
pub struct FlightModeState {
    pub primary: String,        // canonical id, e.g. "poshold", "rth", "ardu_fbwa"
    pub modifiers: Vec<String>, // canonical modifier ids, e.g. ["althold","headfree"]; empty for Ardu/CRSF
}
```

TS (frontend store mirror):

```ts
interface FlightModeState { primary: string; modifiers: string[] }
```

**No display label, no color on the wire** — those stay frontend (i18n + theme). The backend emits only
canonical **ids**; ids are **strings** (a new protocol can introduce `crsf_*` with no central number
allocation — confirmed preference).

### The anti-info-loss trick: category drives color, id keeps the exact label

Each id maps (in the frontend registry) to an exact **label** and a shared **category**. The **category**
is the common semantic axis (drives badge color + track-coloring); the **label** stays exact per system.
→ ArduPlane `auto` and INAV `mission` are different ids/labels ("Auto" vs "Mission") but share
category `mission` → **same track color, no info lost.**

## Vocabulary (reuses today's ids; only regroups color → category)

Categories (color buckets) — colors reused from the current palette:

| category | color | members (examples) |
|---|---|---|
| `manual` | `#808080` | INAV manual; Ardu manual |
| `acro` | `#c0c0c0` | INAV acro (fallback); Ardu acro/qacro |
| `stabilized` | `#59aa29` | INAV angle/horizon; Ardu stabilize/fbwa/training |
| `althold` | `#e8c820` | INAV NAV_ALTHOLD (modifier); Ardu althold/fbwb/qhover |
| `poshold` | `#00bcd4` | INAV poshold; Ardu loiter/qloiter/poshold/drift |
| `cruise` | `#ff8c00` | INAV course-hold; Ardu cruise/circle |
| `mission` | `#37a8db` | INAV NAV_WP; Ardu auto |
| `guided` | `#2980b9` | Ardu guided/guided_nogps |
| `rth` | `#9b59b6` | INAV NAV_RTH; Ardu rtl/smartrtl/qrtl/autortl |
| `launch` | `#e91e9c` | INAV NAV_LAUNCH; Ardu takeoff |
| `land` | `#e67e22` | Ardu land/qland; INAV autoland (modifier) |
| `failsafe` | `#e60000` | INAV failsafe / failsafe_rth |
| `autotune` | `#e8c820` | INAV AUTO_TUNE (modifier); Ardu autotune |
| `other` | `#808080` | thermal, systemid, brake, throw, … + unknown fallback |

IDs are taken **1:1 from the existing tables** — INAV from `trackColors.ts` `MODES` (primary) +
the widget modifier list; ArduPilot from `ARDU_PLANE_MODES`/`ARDU_COPTER_MODES`. Identical
cross-protocol ids (acro, manual, guided, circle, land, autotune) collapse to **one** registry entry;
everything else keeps its own. Only the *selection logic* moves — the ids/labels already exist.

INAV modifier ids (the chips today): `althold`, `headinghold`, `headfree`, `soaring`, `autotune`,
`flaperon`, `autoland`.

## Backend design

**New `flightmode` module** (`src-tauri/src/flightmode/mod.rs`):
- `classify_inav(flags: u32) -> FlightModeState` — ports the `MODES` priority list (primary) + the
  modifier extraction from `trackColors.ts` into Rust. Input is the already-normalized INAV bitmask.
- `classify_ardupilot(custom_mode: u32, variant: &str) -> FlightModeState` — ports
  `ARDU_PLANE_MODES`/`ARDU_COPTER_MODES` (number → id), no modifiers.
- (later) `classify_crsf(...)`, etc.

**Wiring:**
- **MSP** (`scheduler/telemetry.rs` / `poll_slot`): after `decode_status` yields the bitmask, call
  `classify_inav` and emit **`telemetry-flightmode`** (`FlightModeState`) + feed the recorder.
- **MAVLink** (`handler.rs` HEARTBEAT arm): call `classify_ardupilot(custom_mode, fc_variant)`, emit
  `telemetry-flightmode` + feed the recorder.
- The raw `flight_mode_flags` stays in `StatusData` (recorder forensic column only — **not** used for
  display anymore).

**Recorder + DB:** snapshot gains `mode_primary: Option<String>` + `mode_modifiers: Vec<String>`,
set via a new `on_flightmode(&FlightModeState)`. New `telemetry_records` columns
`mode_primary TEXT`, `mode_modifiers TEXT` (comma-separated ids, empty when none). Raw
`active_flight_mode_flags` kept as forensic.

## Frontend design

- **Store** (`telemetry.ts`): add `flightMode: FlightModeState` from the `telemetry-flightmode` event;
  `activeFlightModeFlags` (raw) retained only where still needed (live-track legacy — see below) or
  removed.
- **Output registry** (`helpers/flightModeRegistry.ts`, new): `MODE_REGISTRY: Record<id, {i18nKey,
  category}>` + `CATEGORY_COLOR: Record<category, string>` + helpers `modeLabel(id)`, `modeColor(id)`,
  `modeCategory(id)`. **The single presentation source.**
- **Widget** (`FlightModeWidget.svelte`): render `flightMode.primary` as the badge (color =
  category), `flightMode.modifiers` as chips — uniform, no `isArduPilot`, no `FLIGHT_MODE` bits.
  `inMission` becomes `modeCategory(primary) === 'mission'`.
- **Track-coloring** (`trackColors.ts`): `segmentTrackByFlightMode` / `trackPointColorizer` /
  `getUsedFlightModes` segment by **`record.mode_primary`** → `modeCategory` → `CATEGORY_COLOR`.
  `classifyMode`/`classifyFlightMode`/`classifyArduPilotMode` + the per-protocol tables are **deleted**.
- **Live track** (`stores/liveTrack.ts` + `+page` `appendLivePoint`): store `mode_primary` (string) per
  point instead of `mode_flags` (number); Map/Map3D live-trail coloring uses the registry.
- **Replay** reads `mode_primary`/`mode_modifiers` straight from the DB record → no re-classification,
  fully agnostic. `telemetryAdapter.ts` maps the new columns through.
- **`+page:1678`** WP-mode gate (`& NAV_WP`) → `modeCategory(primary) === 'mission'`.

## DB migration + backfill

Incremental `PRAGMA user_version` step (never modify earlier migrations): `ALTER TABLE
telemetry_records ADD COLUMN mode_primary TEXT; … ADD COLUMN mode_modifiers TEXT;` plus the same on the
temp-session DDL (`TELEMETRY_RECORDS_DDL_FULL`) so temp + main stay identical.

**Backfill:** the migration can re-derive `mode_primary` for existing rows from `active_flight_mode_flags`
+ the flight's `fc_variant` using the new Rust classifiers (one-time, best-effort; on any error leave
NULL). Given we are in **early alpha** (fresh DB expected at release) this backfill is *optional
nice-to-have* — NULL `mode_primary` simply renders as an `other`/unknown category in replay. The
**mechanism** (versioned chain + additive columns) is the robust post-release extension path.

## Consumer migration checklist (from the audit)

- `telemetry.ts` (store + `telemetry-status` listener) — add `telemetry-flightmode`.
- `FlightModeWidget.svelte` — registry-based; drop `isArduPilot`/`FLIGHT_MODE` usage.
- `trackColors.ts` — segment by `mode_primary`+category; delete `classify*` + protocol tables.
- `Map.svelte` (`segmentTrackByFlightMode`, `classifyFlightMode` trail @633, `updateTrail` @909).
- `Map3D.svelte` (`segmentTrackByFlightMode`, `trackPointColorizer`, `classifyFlightMode` @2214).
- `LogPlayer.svelte` (`getUsedFlightModes` legend).
- `telemetryAdapter.ts` (@65 map the new columns).
- `flightlogTypes.ts` (record type: add `mode_primary`/`mode_modifiers`).
- `liveTrack.ts` + `+page` `appendLivePoint` (@266) — per-point `mode_primary`.
- `+page` @1678 WP-mode gate → category check.

## Build order (one pass)

1. Backend `flightmode` module + classifiers ( port INAV `MODES`/modifiers + ArduPilot tables).
2. Emit `telemetry-flightmode` from MSP + MAVLink; recorder `on_flightmode` + DB columns + migration.
3. Frontend registry + store field.
4. Widget → registry.
5. Track-coloring + live-track + Map/Map3D + LogPlayer → registry/`mode_primary`.
6. Delete the old `classify*` + protocol tables; `npm run check` clean.
7. Docs: finalize the **DATA_PIPELINE.md** "Flight Mode" section to the implemented detail, add a
   CHANGELOG entry and an ADR for the canonical model.

## Out of scope / notes

- **Betaflight** (also stacked modes) is a *future* adapter — the model already supports it
  (primary + modifiers); only a new input adapter + ids needed. No further complexity expected
  (INAV is the main stacked-mode system).
- Arming stays separate (`arming_flags`), unchanged.
- Raw `flight_mode_flags` is retained purely as a forensic DB column.
