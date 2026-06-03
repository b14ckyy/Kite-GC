# Battery Management — reusable battery packs linked to the flight log

**Status:** Phase A + B implemented (2026-06-03) — **awaiting hardware/simulator testing**. Mirrors
the mission library pattern ([`MISSION_LIBRARY_AND_DB.md`](MISSION_LIBRARY_AND_DB.md)): packs are
first-class DB entities, flights link the pack that was flown (soft link by serial), and the wear
data is derived from the linked flight logs + a persistent baseline. The Battery Manager is a
view-toggle inside the Flight Logbook panel. See **Implementation notes** at the end for the deltas
agreed during the build. Deferred to later slices: dedicated battery file export/import,
disarm/import serial-entry capture, and the Phase C per-flight telemetry metrics.

The goal is a complete battery-management system so (especially commercial) operators can track
how their packs perform and wear over their lifetime.

---

## Where batteries are managed (decision)

The **Battery Manager is a view toggle inside the Flight Logbook panel** — exactly the relationship
the **Mission Manager** has with the Mission Planner panel. No new panel chrome; the logbook's list
area switches between **flights** and **batteries**, and the detail area shows a flight or a pack.

- A **"Battery Manager" button** sits in the list toolbar, **right next to Refresh**. It toggles the
  list between the flight tree and the battery tree.
- View state (open + selected battery) lives in a **`stores/batteryManager.ts`** store so it survives
  close/reopen and drives the panel width (same approach as `stores/missionManager.ts`).
- The existing **top-right grouping/sort `<select>`** is reused: in the battery view it offers the
  battery grouping modes instead of the flight sort modes.

---

## Step 0 — Flight Logbook design unification (separate, visual-only first)

Before adding the battery view, unify the logbook's controls with the rest of the app (the Mission
Manager is the reference). Purely visual, **its own commit**, no behaviour change.

- The logbook's `.cache-clear-btn` is currently **`font-size: 9px; padding: 1px 6px`** — far smaller
  than the same class elsewhere (Mission Manager: **`11px; 4px 10px`**). Raise it to the app standard.
- Align `.setting-select` (the sort dropdown), the search input, and the small icon buttons
  (✎ / ✕ / ✓) to the shared sizing.
- (Future, non-blocking: extract a shared button style/class so this can't drift again — out of scope
  for Step 0.)

---

## Data model

### New table `battery_packs`

Identity = **serial number** (user-defined, the physical pack's serial), so the same pack is one row
and every flight that used it links to it. (Unlike missions, whose identity is a content hash; a
battery has a real-world identifier.)

| Column | Purpose |
|---|---|
| `id` PK | |
| `serial` TEXT UNIQUE NOT NULL | identity — the physical pack serial (user-defined) |
| `label` TEXT | friendly name / nickname |
| `manufacturer` TEXT, `model` TEXT | |
| `chemistry` TEXT | `lipo` \| `liion` \| `life` \| `lihv` |
| `cell_count` INTEGER | series count (S) |
| `capacity_mah` INTEGER | nominal capacity |
| `c_rating_discharge` INTEGER, `c_rating_charge` INTEGER | |
| `connector` TEXT | XT60 / XT90 / … (inventory) |
| `in_service_date` TEXT | when the pack entered service |
| `status` TEXT NOT NULL DEFAULT `'active'` | `active` \| `storage` \| `retired` \| `damaged` |
| `notes` TEXT | |
| `created_at` TEXT NOT NULL DEFAULT `(datetime('now'))` | |
| `base_flight_seconds`, `base_mah`, `base_cycles`, `base_charges` | **persistent consumption baseline** — never auto-updated; only ever *added to* by the manual additive editor (and optionally by the flight-deletion "transfer" option). Lifetime = baseline + Σ(linked flights). (No `base_wh`: Wh-used was dropped — see Implementation notes.) |

The first block (serial … notes) is **mutable identity/spec** (overwritten on edit). The `base_*`
block is the **consumption baseline** with different write semantics (additive only) — see below.

### `flights` addition — **soft link by serial (not a hard FK)**

- `battery_serial TEXT` (nullable) — the **serial of the pack flown**, stored on the flight itself.
  **One battery per flight.** Multi-battery (swaps / parallel packs) is deferred to a later
  `flight_batteries` join table.
- **The link is resolved at read time** by matching `flights.battery_serial` against
  `battery_packs.serial` — there is **no foreign key**. Consequences (all desirable):
  - A flight may carry a serial that has **no matching pack row** → it is shown as *"battery not in
    library"* (the "not in DB" marker is **derived**, not stored, so it self-updates).
  - **Deleting a pack** simply deletes its row; flights keep their `battery_serial` and just fall back
    to the "not in library" state. No NULLing, no orphan handling.
  - **Importing a pack** with that serial makes all those flights resolve again automatically — their
    history becomes traceable against the (re)imported library.
  - **Manual linking** can therefore accept a serial that isn't in the DB yet (see flows).
- Lifetime sum is keyed on the serial: `Σ flights WHERE battery_serial = pack.serial`.

### Migration (v9 → v10)

`CURRENT_SCHEMA_VERSION = 10`; idempotent **self-healing** `ensure_v10_schema` (CREATE TABLE IF NOT
EXISTS `battery_packs` + a `column_exists` guard for `flights.battery_serial`), called unconditionally
in `migrate()` like `ensure_v8_schema` / `ensure_v9_schema`. Existing DBs gain the table/column on next
open, no data loss.

---

## Field model (four layers)

The table above is **layer A (identity, manual)**. The other three are mostly **derived from the
linked flight logs**; they are computed (and partly cached), not hand-entered.

**B — Lifetime = persistent baseline + dynamic sum of linked flights:**
- `cycles` = cumulative discharged mAh ÷ nominal capacity (**equivalent full cycles**, the industry
  measure — *not* a raw flight count). Flight count is shown separately.
- total flight time ("age"), flight count, lifetime mAh, lifetime Wh, first/last used.
- **The displayed lifetime is always recomputed on view open** as `baseline + Σ(linked flights)`. The
  pack carries a **persistent consumption baseline** (`base_flight_seconds`, `base_mah`, `base_wh`,
  `base_cycles`, `base_charges`) that **never changes automatically** — see *Lifetime computation & data
  integrity* below.

**C — Per-flight (derived from telemetry — `telemetry_records.voltage` / `current_a` / `mah_drawn`):**
- mAh drawn (already `flights.battery_used_mah`), **Wh** (∫ V·I dt — more meaningful than mAh because
  voltage matters), start/rest voltage, landing (end) voltage, min voltage / sag floor, max current,
  max power, average current, peak C-rate actually pulled. Per cell = `vbat ÷ cell_count` (estimate).
- Stored as columns on `flights` (e.g. `batt_wh`, `batt_v_start`, `batt_v_end`, `batt_v_min`,
  `batt_i_max`, `batt_ir_mohm`), computed once at finalize / import / link — not recomputed per render.

**D — Health / wear (the commercial core, derived over the pack's history):**
- **Internal resistance per cell (mΩ)** estimated via ΔV/ΔI under load — *the* wear marker; rising IR
  = aging. Trend over cycles.
- **Capacity fade / State-of-Health %** — usable mAh to a fixed cutoff trending down.
- Sag-at-reference-load trend.

### Caveats (clarify early)
1. **No per-cell voltages.** INAV reports only pack `vbat` + current, not per-cell. Per-cell values are
   `vbat ÷ cell_count` estimates; true cell spread needs a smart / per-cell sensor → optional later.
2. **No battery identifier in telemetry.** The FC does not say which pack is installed → the link is by
   **serial number** (entered by the operator; see flows). At connect, `cell_count` + the configured
   capacity *could* be read via MSP to prefill/match a new pack (nice-to-have).

---

## Lifetime computation & data integrity

The wear figures shown in the Battery Manager are **never stored as a running total**; they are
**recomputed on every view open** as:

```
lifetime metric = pack.base_<metric>  +  Σ over flights WHERE battery_serial = pack.serial ( per-flight metric )
```

This makes the linked flights the single source of truth for the dynamic part, and the `base_*`
columns the persistent part that survives independently of any flight.

**Write semantics — two categories:**
- **Identity / spec** (serial, capacity, manufacturer, chemistry, cell count, C-ratings, …): the editor
  **overwrites** these normally.
- **Consumption baseline** (`base_*`): the manual editor is **purely additive** — you enter *deltas*
  (e.g. "+40 cycles, +3.2 h, +12000 mAh" from a bench charger or another logger) and the pack stores
  `base_<metric> += delta`. You **cannot** blow away accumulated history by typing an absolute value.
  *(A signed/negative delta to correct an over-count is an open option — Phase D.)*

**Flight deletion when a battery is linked** — solves the "delete a flight → its contribution silently
vanishes from the pack's lifetime" problem. Deleting such a flight shows a **warning popup** listing the
exact battery statistics that flight contributed, with three choices:
1. **Cancel** — keep the flight.
2. **Delete** — the flight (and its contribution) is removed; the lifetime sum drops accordingly.
3. **Transfer to baseline & delete** — `base_* += this flight's contribution`, then delete the flight, so
   the lifetime total is preserved. **Use only if the flight will not be re-imported later** (re-import
   would re-add the same contribution → double count). Beyond that, **correct tracking is the operator's
   responsibility.**

> Dependency: option 3's exact per-flight contribution comes from the **Phase C** per-flight battery
> columns (`batt_wh`, sag, IR, …). In **Phase B** the transferable contribution is limited to what
> already exists — `battery_used_mah`, duration, and the derived equivalent cycles.

---

## Export / import

### Dedicated battery export (NOT bundled into `.kflight`)
Batteries are exported through their **own dedicated export** (one or more packs to a battery file) —
they are **not** bundled into `.kflight`. Before writing, the export **consolidates the linked flights'
wear into the pack's persistent baseline** (`base_* += Σ(linked flights)`) **after a confirmation**, so
the exported record is **fully self-contained** and portable into a database that does not have those
flights.

- The consolidation is written into the **exported copy** so it stands alone; the source DB keeps its
  dynamic model (auditable) intact — the export is **non-destructive to the source** (decided). The source
  pack's `base_*` is not touched, so no "already counted" flag on flights is needed and there is no
  double-count risk.
- The confirmation tells the operator exactly how many linked flights / how much wear will be baked in.

### `.kflight` (flights)
`.kflight` export bundles the **linked WP mission** with the flights (fixes today's mission-link loss on
export) — **batteries are not included** here (they have their own export). This mission bundling is
scheduled **later**, not part of the first battery slice.

### Import dedup (rough; details TBD)
On importing a battery whose **serial already exists** in the DB, **ask the operator which record to
keep** (existing vs. imported). Detailed dedup rules (field-level merge, baseline reconciliation) are a
**separate later point**; for now the goal is that exported batteries carry their full state and import
cleanly, with a duplicate-serial prompt.

---

## Link flows (serial-number driven)

The link is **the serial written onto the flight** (`flights.battery_serial`); resolution to a pack is by
serial match at read time. Entering a serial that **isn't** in the library is allowed — the flight keeps
it and shows *"battery not in library"* until a pack with that serial is created/imported. Optionally,
an unknown serial offers a **"Create new pack"** shortcut.

- **Phase B — manual link in the flight detail** (like the mission Link/Unlink): a Battery row showing
  the serial (+ label if the pack is in the library), a **Link** control (enter serial → set
  `flights.battery_serial`; offer create-pack if unknown), and **Unlink** (clear `battery_serial`).
- **Phase C — on disarm** (recording enabled): the flight summary is shown → enter the serial there.
  *(Future: barcode / RFID scanner to auto-fill the serial.)*
- **Phase C — on log import** of non-recorded flights: the operator enters the serial.

---

## Battery Manager UI (within the logbook panel)

### List view (toggle from flights)
- **Grouping** via the reused top-right `<select>`, three modes:
  - **Cell Count → Capacity** (2-level tree)
  - **Capacity → Cell Count** (2-level tree)
  - **Flat** (no grouping)
- **Groups are always ordered large → small.** An **optional Asc/Desc toggle** orders the *leaf packs*
  within a group (not the groups).
- Leaf row shows: label/serial, chemistry, `cell_count`S, capacity, status badge; quick lifetime hint
  (cycles / flights) once Phase B stats are in.

### Detail view (a pack selected — reuses the logbook wide/detail width)
- **Editable identity** (label, serial, manufacturer/model, chemistry, cell count, capacity, C-ratings,
  connector, in-service date, status, notes).
- **Lifetime stats** — Phase B can already show flight count, total flight time, summed
  `battery_used_mah`, and a cycle estimate (from `capacity_mah`); Wh / sag / IR / SoH follow in Phase C.
- **Linked flights** — `SELECT … WHERE battery_serial = ?` (paginated), rows clickable → open the flight
  (reuse the `requestOpenFlightId` jump the Mission Manager already uses).
- **Actions:** New battery, Export (dedicated, consolidates → confirm), Delete (warn: *N flights reference
  this serial and will show "battery not in library" until a pack with this serial exists again*; the
  flights keep their serial — no NULLing).
- **Empty state:** frame + greyed actions, like the Mission Manager.
- **Add usage** (additive baseline editor): enter consumption deltas (cycles / hours / mAh / Wh /
  charges) that get **added** to the pack's `base_*` (never overwrite). Phase D for the full editor;
  the `base_*` columns and the additive write path land in Phase B so the model is correct from the start.

### New battery
A creation form (Manager "New" button, and the unknown-serial popup during linking). Serial is required
(identity); the rest is optional and editable later. Default label suggestion if none.

---

## Phasing

- **Phase A — Logbook design unification** (Step 0 above). Visual only, own commit.
- **Phase B — backbone + manager:** schema v10 (`battery_packs` incl. the `base_*` baseline +
  `flights.battery_serial` **soft link**) · Rust CRUD (`battery_save`/upsert-by-serial, `get`, `list`,
  `find_by_serial`, `update`, `delete`, `list_flights_for_serial`, **additive `add_usage`**) + commands +
  store wrappers · `stores/batteryManager.ts` · `BatteryManager.svelte` (toggle, grouped/flat list, sort
  reuse, detail, New/Delete) · manual serial link/unlink in the flight detail (serial written on the
  flight; unknown serials allowed) · **lifetime = baseline + Σ(flights with this serial)** from existing
  columns · **flight-deletion warning + "transfer to baseline"** · **dedicated battery export**
  (consolidate-into-baseline + confirm) and import (duplicate-serial prompt).
- **Phase C — metrics + capture flows:** per-flight derived columns (Wh, sag, min-V, IR estimate) from
  telemetry at finalize/import/link (these sharpen the deletion-transfer + lifetime sum) · lifetime
  aggregation (cycles, Wh, IR/SoH trends) · disarm flight summary serial entry · import serial entry.
- **Phase D — manual usage + health dashboard:** full additive usage editor (signed corrections) ·
  health charts (IR trend, capacity fade, SoH) per pack.

---

## Open points / deferred (non-blocking)
- **Multi-battery per flight** (swaps / parallel) → later `flight_batteries` join table.
- **Per-cell telemetry** (smart/per-cell sensors) → optional later; until then `vbat ÷ cell_count`.
- **MSP prefill** of cell count / capacity at connect when creating a pack → nice-to-have.
- **Barcode / RFID** serial capture at disarm → future input method.
- **ArduPilot** packs: the model is firmware-agnostic; `chemistry` + serial identity carry over.

---

## Implementation notes (Phase A + B, as built)

Refinements agreed during the build, on top of the design above:

- **No Wh-used metric.** Wh-used per pack is linear to mAh and unreliable from telemetry, so it was
  dropped from the lifetime (no `base_wh`). Instead **energy is a computed spec**: nominal V × cells ×
  (mAh ÷ 1000), shown informationally (e.g. LiPo 6S 5000 mAh → 111 Wh).
- **Chemistry voltages (per cell), internal table** → drive the computed **nominal voltage**, **voltage
  range**, and **energy**: LiPo 3.7 / 3.2 / 4.2 · Li-Ion 3.6 / 2.5 / 4.2 · LiFe 3.3 / 2.5 / 3.65 ·
  LiHV 3.8 / 3.2 / 4.35 (nominal / min / max).
- **Connector** is a dropdown (XT30/60/90, XT60H, EC3/5/8, AS150/AS150U, MR30/60, MT60, Deans (T),
  Other) to avoid spelling drift.
- **Charges** is a **manual-only** lifecycle figure (added via the additive usage editor; no automatic
  source — a flight is not a charge).
- **Create / edit / add-usage are modal popups**; numeric fields use the shared `NumberStepper`.
- **List ordering:** the ▲/▼ button sets the **group ordering** (both levels) in the grouped views; the
  **leaf packs are always serial-ascending**. The **flat** view instead exposes a sort-field dropdown
  (serial / cell count / capacity) + the ▲/▼ direction.
- **Status special groups:** **Storage** (second-to-last) and **Retired & Damaged** (last) are pulled
  out of the normal grouping into trailing collapsible groups in every mode (incl. flat). All groups
  start **collapsed** (consistent with the Flight Logbook tree).
- **List row** shows the serial first, then the label (both white).
- **No manual Refresh button:** the list auto-reloads on **disarm** (`flight-recording-ended`) and on
  **disconnect** (covers a just-recorded live flight); otherwise it loads on open / import.

**Still deferred (next slices):** dedicated battery file **export** (consolidate-into-baseline +
confirm) and **import** (duplicate-serial prompt); **disarm flight-summary** & **log-import**
serial-entry capture; **Phase C** per-flight telemetry metrics (Wh, sag, internal resistance) and the
flight-deletion "transfer to baseline" dialog.
