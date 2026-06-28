# Multi-Battery (ArduPilot / PX4) — Feature Plan

> ARCHIVED (2026-06-28) — feature complete (ADR-059), user-verified via QuadPlane DataFlash replay.
> Live multi-monitor validation is user-side; not kept open.

> STATUS: **shipped-local** · 2026-06-26 (verified by the user via QuadPlane DataFlash replay; live
> path implemented, not SITL-tested). See **ADR-059**. Schema **v18** (`battery_records`). Decisions
> taken: Schema **B** (dedicated table); AUTO = highest draw + hardcoded 1 A margin + 5 s hysteresis;
> safety override = configurable alert % (Settings → Alerts) → lowest-% pack; widget alert state; two
> dockable widgets; **data-gated** (multi UI only when >1 instance); top bar = primary, no estimate
> (native % or voltage). ArduPilot/PX4 can run **up to 10 independent battery monitors** simultaneously
> (e.g. a big Li-Ion for the fixed-wing forward motor + a small LiPo for the VTOL hover lift). Kite used
> to collapse all of them into **one** battery (ignoring the instance id). This adds per-instance
> handling (live + import + logging + replay) and overhauls the battery widget. INAV stays
> single-battery and degrades gracefully.

## Research findings (verified)

ArduPilot battery monitors are **independent** and **always reported in parallel** — there is no
"active battery" concept and no automatic total. Each monitor is identified by an **instance id (0..9)**;
its physical role (forward vs VTOL) is user configuration, **not** in the telemetry (the
`battery_function` enum exists but ArduPilot rarely sets it usefully). Identify by instance id only.

- **MAVLink (live):** one `BATTERY_STATUS` message **per configured monitor**, distinguished by the
  `id` field. Per instance: `current_battery` (cA), `current_consumed` (mAh), `battery_remaining` (%),
  `temperature`, `voltages[]` (pack/cell mV; total packed into `voltages[0]`, overflow into `[1]`,
  `65535` = unused), `charge_state`, `fault_bitmask`. `SYS_STATUS` carries **only the primary** battery
  → that's why Kite shows just one today.
- **DataFlash (import):** one `BAT` record per monitor per sample, with field **`Instance`** (older
  logs: `Inst`). Fields: `Instance, Volt, VoltR, Curr, CurrTot (mAh), EnrgTot (Wh), Temp, Res, RemPct`.
  Per-cell voltages (if present) are in separate `BCL` messages. `POWR` is board power status, **not** a
  pack. Verified on the user's QuadPlane logs: `BAT` logged at exactly 2× the `POWR` rate →
  `instances = {0, 1}` (two packs).
- ArduPilot reports only **configured** monitors (`BATTx_MONITOR ≠ 0`); instance ids are usually but
  not guaranteed contiguous — never hardcode 0/1, drive everything off the observed id set.

## Data model

A new per-instance battery list flows alongside the existing single-battery fields. **The existing
`telemetry-analog` path and the top-bar battery indicator stay exactly as they are** (primary monitor) —
this feature is additive.

- **Backend struct** `BatteryInstanceData { id: u8, voltage, current, mah_drawn, percentage, cell_count,
  temperature: Option }`. New event **`telemetry-batteries`** carrying `Vec<BatteryInstanceData>`.
  - **MAVLink live:** accumulate the latest `BATTERY_STATUS` per `id` in the handler, emit the array.
    `SYS_STATUS` keeps feeding the primary `telemetry-analog` (unchanged).
  - **INAV:** single battery → emit a one-element array (= the existing analog). Widget logic below
    degrades to "one instance, no toggle".
- **Frontend store:** `telemetry.batteries: BatteryInstance[]` (id-keyed). Existing scalar
  `voltage/current/mAhDrawn/…` remain = primary (top bar, compatibility).

## Widget overhaul (the core of this feature)

The widget is small — we will **not** cram all packs in at once. It shows **one** instance with a
selector. Selection state is **per widget instance**, persisted by widget id.

**Selection precedence (highest wins):**
1. **Manual pin** — user toggled to a specific instance; it stays.
2. **Safety override** — if any instance's `%` is below the **configurable alert threshold**
   (Settings → Alerts), show the instance with the **lowest %** (most urgent).
3. **AUTO default** — show the instance with the **highest current draw**.

**AUTO switching detail:**
- A **hardcoded** current margin (the new candidate must lead the shown one by > margin) **plus** a
  **5 s dwell hysteresis** (must lead continuously for 5 s) before switching — prevents flapping during
  transition where both packs briefly draw high.
- EMA/timers kept in plain (non-reactive) widget vars; the `$effect` must not read the `$state` it
  writes (the main-thread-freeze pattern — same discipline as the speed widget's ACC bar).

**Manual control:** a toggle button cycles **AUTO → instance 0 → instance 1 → … → AUTO**. With a single
instance the toggle is disabled and AUTO is implicit.

**Alert state (bonus):** when the *shown* instance is below the alert threshold, the widget enters a
visual **alert state** (e.g. red border/emphasis) — a general low-battery cue, not just colour on the %.

**Header:** shows which pack is displayed (instance number, later optionally a user name) + an AUTO
indicator.

## Two dockable battery widgets (tester request)

Register a second widget entry (`battery2`) in the widget registry pointing at the same `BatteryWidget`
component; each instance keeps its **own** pin/AUTO state (keyed by widget id in settings). Settings get
two toggles ("Battery 1" / "Battery 2"). Use case: pin Battery 1 → forward pack, Battery 2 → VTOL pack;
or leave both on AUTO.

## Logging & replay (schema — NEEDS SIGN-OFF)

The widget's AUTO logic needs the **per-instance time series** on replay, so logging must capture all
instances. `telemetry_records` is one row per timestamp with a single battery. Options:

- **A — fixed second-battery columns** (`voltage2`, `current2_a`, `mah_drawn2`, `battery_pct2`,
  `battery2_instance`). Simple; covers the dominant 1–2 pack case; caps at 2 (more would be dropped).
- **B — dedicated `battery_records(flight_id, timestamp_ms, instance, voltage, current_a, mah_drawn,
  pct, temp)` table** (RECOMMENDED). Clean N-instance model, matches the wire reality. Primary stays
  denormalised in `telemetry_records` (top bar / existing replay unchanged); the full per-instance
  detail lives here. Replay loads it grouped by instance.

Recommendation: **Option B** — future-proof (up to 10), no awkward column sprawl, and it leaves the
existing single-battery replay path untouched. Schema bump (additive `CREATE TABLE`, idempotent +
self-heal, mirrors v16/v17). Import: track a `BatState` **per instance** (HashMap keyed by `Instance`)
and emit one `battery_records` row per instance at each sample. **Confirm A vs B before implementation.**

## Phasing

- **P1 — live + widget (no schema):** MAVLink `telemetry-batteries`, store, widget overhaul (AUTO/pin/
  toggle/alert), two dockable widgets. INAV degrades to single. Immediate value, no DB risk.
- **P2 — logging + replay:** schema (Option B), ArduPilot import per `Instance`, replay feeds the
  per-instance series so AUTO works on recorded flights too.

## Out of scope / later

- Per-instance **naming** (like the Battery Manager serials) — show instance numbers for now.
- Battery-Manager **multi-pack link** (multiple serials per flight) — separate, larger change.
- Cell-level voltages (`BCL`) — total pack voltage only for now.
- PX4 specifics beyond the standard `BATTERY_STATUS` instance model.

## Verification

- `npm run check` (0/0) + `npm run build`; `cargo check` + `cargo test --no-run`.
- Live (ArduPilot SITL or the user's craft): both packs visible; AUTO follows current draw with the 5 s
  hysteresis; manual toggle pins; alert state triggers below threshold.
- Replay of the user's QuadPlane log: forward pack shown in cruise, VTOL pack in hover (after P2).
- INAV: single battery, toggle disabled, no regression in the top bar.

## Notes

The temporary import diagnostics (`[BAT-FIELDS]`, `[BAT-INST]`) used during research are removed when
P2 lands. The `BAT` instance field name is **`Instance`** (fallback `Inst`).
