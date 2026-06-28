# Vehicle Control — GCS Command & Guided Steering

> ARCHIVED (2026-06-28) — V1 complete. ArduPilot SITL-verified (Copter / Plane / QuadPlane); PX4
> on-hardware validation is user-side. INAV guided / HID RC remain deferred (§11).

> Status: **V1 shipped** (2026-06-19) — MAVLink command panel (ArduPilot + PX4) on the `control`
> nav-rail tab (shown only while connected via MAVLink). **SITL-verified on ArduPilot Copter / Plane /
> QuadPlane** (modes, arm/disarm, takeoff, RTL, Guided + Fly-Here, change alt/speed, set home, abort
> landing, mission start/restart/set-WP, VTOL transition, RC-presence lock, safety gestures). The
> **PX4 path is firmware-aware in code but untested on real hardware** (test-pilot pending). INAV
> guided (WP #255) and joystick/HID RC control remain **deferred** (see §11). Recorded as ADR-052.
>
> **Firmware/vehicle learnings baked into V1:**
> - **Mission Start** is firmware-specific — ArduPilot `MISSION_START`, PX4 enters **Mission mode**.
> - **Land** is firmware/vehicle-aware — Copter/PX4 `NAV_LAND`, QuadPlane → **QLAND**, plain ArduPlane
>   fixed-wing has no land-now command so the button is **hidden** (landing = AUTO sequence / RTL).
> - **Set Loiter Radius** param name differs — ArduPilot `WP_LOITER_RAD`, PX4 `NAV_LOITER_RAD`.
> - **Set Heading** is **Copter-only** (`CONDITION_YAW`). The fixed-wing `GUIDED_CHANGE_HEADING` path
>   was cut: ArduPlane's guided heading slew is a sticky override that bypasses waypoint nav and is not
>   cleared by `DO_REPOSITION` (even rejects descent) — an ArduPilot bug worth reporting upstream.
> - **VTOL transition** (QuadPlane class) — PX4 via `DO_VTOL_TRANSITION` (MC=3/FW=4), ArduPlane by mode
>   (forward → Guided, hover → QLOITER; its command path is AUTO-only).
> - **`Change Alt`** is a `DO_REPOSITION` → Guided-only; hidden outside Guided.
> - **QuadPlane detection** (reports `MAV_TYPE_FIXED_WING`) is resolved by re-probing `Q_ENABLE` until
>   the FC answers (the single connect-time request could be lost → mis-detected as a plain plane).
> - Set WP / Mission Restart use `DO_SET_MISSION_CURRENT` (needs reasonably recent PX4 firmware).
> - **Advanced controls → FUTURE / on-request** (2026-06-21): set-servo, gripper/payload, gimbal/ROI on
>   the right-hand `advanced` split panel — not in the initial release; build on demand. (HID/joystick RC
>   control already shipped as its own RC-control feature — see `MAVLINK_RC_CONTROL.md`.)

Related: ADR-010 (multi-protocol schedulers), ADR-029 (panel framework + control library),
ADR-043 (MAVLink stream rates / GCS-requested), ADR-044 (unified flight-mode model),
ADR-046 (catalog-driven editor + shared popup framework), ADR-039 (home/launch reference).

---

## 1. Goal

Let the operator command the vehicle **directly from the GCS, without a transmitter** — arm,
take off, switch into autonomous modes, fly to a clicked point, change speed/altitude, run/pause
the mission, land, RTL. This is the GCS-as-controller capability that Mission Planner and
QGroundControl have. Our bar: do it **better than MP's flat button grid** — compact, safe,
fast, and consistent with the rest of Kite GC's panel framework.

Two distinct capabilities share this surface; only the first is in V1:

1. **Discrete commands** (V1): mode, arm, takeoff/land/RTL, mission control, Guided "fly here",
   speed/altitude. One-shot `COMMAND_LONG`/`SET_MODE`/`SET_POSITION_TARGET` with ACK feedback.
2. **Continuous RC / joystick control** (later): a HID gamepad streamed as `MANUAL_CONTROL`
   (Ardu/PX4) or `MSP_SET_RAW_RC` (INAV) at ~20–50 Hz. Separate layer, same panel home.

---

## 2. Scope

**V1 (this plan):**
- MAVLink only — **ArduPilot + PX4**.
- Curated mode switching (GCS-safe modes only; stick-required modes hard-locked, see §5).
- Arm / Disarm (force flag), Takeoff (with target altitude), Land, RTL.
- Guided **toggle** + map-click "Fly Here" popup (vehicle-aware fields), with Speed/Altitude.
- Mission start / pause-continue / set-current.
- Visible `COMMAND_ACK` feedback per action.
- Safety layer: slide-to-confirm Arm, `HoldToConfirm` (2 s fill) for all other commands.
- Compact panel, usable in parallel with the map.

**Deferred (future phases, §9):**
- INAV guided steering (WP #255 reposition + WP #0 live-home) — verified to exist, see §6.4.
- Joystick / HID continuous RC control (all FCs).
- Secondary commands: servo/relay, gimbal/ROI, reboot, calibration.
- Larger split panel (only if telemetry feedback / logs justify it).

**Non-goals:** switching into stick-flown modes without an RC link; replacing the mission editor.

---

## 3. Command landscape (V1)

| Action | Mechanism | Notes |
|---|---|---|
| Set mode | `SET_MODE` (or `MAV_CMD_DO_SET_MODE`) with `custom_mode` | Per-firmware/vehicle encoding — §5 tables. |
| Arm / Disarm | `MAV_CMD_COMPONENT_ARM_DISARM` | p1 = 1/0, p2 = 21196 (force). |
| Takeoff | `MAV_CMD_NAV_TAKEOFF` | Copter: arm → GUIDED → takeoff(alt). Plane/VTOL differ. |
| Land | `MAV_CMD_NAV_LAND` | Or land mode. |
| RTL | `MAV_CMD_NAV_RETURN_TO_LAUNCH` | Or RTL mode. |
| Fly here (Guided) | `MAV_CMD_DO_REPOSITION` (192) / `SET_POSITION_TARGET_GLOBAL_INT` (86) | Reposition param4 = yaw; planes loiter (radius). |
| Change speed | `MAV_CMD_DO_CHANGE_SPEED` (178) | Airspeed/groundspeed by type. |
| Mission start | `MAV_CMD_MISSION_START` (300) | |
| Pause / continue | `MAV_CMD_DO_PAUSE_CONTINUE` (193) | |
| Set current WP | `MISSION_SET_CURRENT` / `MAV_CMD_DO_SET_MISSION_CURRENT` | |

MAVLink commands are **not** strict request→response like MSP: we fire `COMMAND_LONG` and the FC
replies asynchronously with `COMMAND_ACK` (`ACCEPTED` / `DENIED` / `TEMPORARILY_REJECTED` /
`UNSUPPORTED`). The control module correlates the ACK by command id and surfaces the result.
The GCS heartbeat (needed for ArduPilot GCS-failsafe / command acceptance) is already sent by our
MAVLink stack — out of scope here.

---

## 4. Guided interaction design

Validated against ArduPilot/PX4 behaviour (Copter docs: GUIDED is fully automatic — fly to a
point and wait there until a new point or a mode change; the target shows with a heading line).

### 4.1 The Guided toggle (firmware-aware backend, identical UX)

A single toggle button drives both the FC mode and the map interaction mode:

- **ON** → send the firmware's reposition-ready mode **once**, then arm the map-click interaction:
  - ArduPilot → set mode **GUIDED**
  - PX4 → set mode **HOLD** (Auto/Loiter) — PX4 has no "GUIDED"; `DO_REPOSITION` works from Hold
  - INAV (future) → ensure **Position Hold** (WP #255 is only honoured there, §6.4)
- **OFF** → send **nothing** to the FC; only disable the map interaction. The vehicle stays in its
  current mode. This keeps the FC stable — no mode churn on toggle-off.
- The user selecting **any other mode** auto-toggles Guided **OFF** (interaction follows the FC).

### 4.2 Map-click "Fly Here" popup

In Guided-ON, a map click opens a popup (styled like the WP-editor popup, ADR-046) instead of
firing immediately — the popup **is** the confirmation step:
- Coordinates (read-only or fine-tunable), plus vehicle-aware fields (§4.3).
- A **"Fly Here"** button sends `DO_REPOSITION` / `SET_POSITION_TARGET_GLOBAL_INT`.
- **Last-used values persist** for the next click (alt, speed, heading).

### 4.3 Vehicle-aware popup fields

Because the panel only exists while connected, the vehicle type is always known (`mavTypeToClass`,
`missionArdupilot.ts`):

| Vehicle | Fields | Rationale |
|---|---|---|
| **Copter / VTOL (multirotor)** | Altitude + Heading (yaw) | Can hover and hold heading. `DO_REPOSITION` p4 = yaw. |
| **Plane (fixed-wing)** | Altitude + Loiter radius/direction | Cannot hover — GUIDED loiters around the point. No meaningful heading. |
| **Rover** | Position + Speed | No altitude. |
| **INAV (future)** | Altitude + Heading | WP #255 honours alt (`!=0`) + p1 heading (1–359). |

---

## 5. Flight-mode model (curated + hard-locked)

### 5.1 Two-tier curation

Not every mode may be commanded from the GCS — switching into a stick-flown mode without an RC
link is an instant crash. Modes are classed:

- **GCS-safe (autonomous, stable without stick input)** — offered prominently.
- **Stick-required** — hidden by default; reachable only behind an explicit "All modes" reveal with
  a warning, and only when the RC hard-lock (§5.2) is satisfied.

### 5.2 RC-presence hard-lock (double safety)

We already read RC channels (INAV `MSP_RC`; ArduPilot/PX4 `RC_CHANNELS`/rssi). **No live RC input →
no transmitter connected → stick-required modes are globally locked**, regardless of the reveal.
They unlock only once a real RC source exists — either a physical TX seen on the RC stream, or
(future) our own HID→MAVLink/MSP RC layer actively streaming. (Open detail: pick a robust
"RX present" signal — channel activity vs. rssi vs. SYS_STATUS sensor-health bit.)

### 5.3 Mode tables

**ArduCopter** (`Mode::Number`, used as `custom_mode`) — verified against ArduPilot source:

| # | Mode | Class |
|---|---|---|
| 3 | AUTO | safe |
| 4 | GUIDED | safe (Guided toggle) |
| 5 | LOITER | safe |
| 6 | RTL | safe |
| 7 | CIRCLE | safe |
| 9 | LAND | safe |
| 16 | POSHOLD | safe* (holds, but accepts stick override) |
| 17 | BRAKE | safe |
| 21 | SMART_RTL | safe |
| 0 / 1 / 2 / 13 | STABILIZE / ACRO / ALT_HOLD / SPORT | stick-required (locked) |

**ArduPlane** (`Mode::Number`) — key values; **verify full list against firmware headers at
implementation**:

| # | Mode | Class |
|---|---|---|
| 10 | AUTO | safe |
| 11 | RTL | safe |
| 12 | LOITER | safe |
| 15 | GUIDED | safe (Guided toggle) |
| 7 | CRUISE | safe* |
| 1 | CIRCLE | safe |
| 0 / 4 / 5 | MANUAL / ACRO / FBWA | stick-required (locked) |
| 17–21 | QSTABILIZE / QHOVER / QLOITER / QLAND / QRTL | VTOL — Q-autonomous safe, Q-manual locked |

**ArduRover** (`Mode::Number`) — key values; verify at implementation:

| # | Mode | Class |
|---|---|---|
| 4 | HOLD | safe |
| 5 | LOITER | safe |
| 10 | AUTO | safe |
| 11 | RTL | safe |
| 12 | SMART_RTL | safe |
| 15 | GUIDED | safe (Guided toggle) |
| 0 / 1 | MANUAL / ACRO | stick-required (locked) |

**PX4** — `custom_mode` is packed: bits 16–23 = `main_mode`, bits 24–31 = `sub_mode`
(low 16 bits reserved). Verified from `px4_custom_mode.h`:

| custom_mode | main | sub | Mode | Class |
|---|---|---|---|---|
| AUTO+LOITER | 4 | 3 | Hold (Guided toggle target) | safe |
| AUTO+MISSION | 4 | 4 | Mission | safe |
| AUTO+RTL | 4 | 5 | Return | safe |
| AUTO+TAKEOFF | 4 | 2 | Takeoff | safe |
| AUTO+LAND | 4 | 6 | Land | safe |
| MANUAL | 1 | 0 | Manual | stick-required (locked) |
| ACRO | 5 | 0 | Acro | stick-required (locked) |
| STABILIZED | 7 | 0 | Stabilized | stick-required (locked) |
| ALTCTL / POSCTL | 2 / 3 | 0 | Altitude / Position | stick-required (locked) |
| OFFBOARD | 6 | 0 | Offboard | special (not exposed in V1) |

Encoding helper: `custom_mode = (sub_mode << 24) | (main_mode << 16)`. `SET_MODE.base_mode` must
include `MAV_MODE_FLAG_CUSTOM_MODE_ENABLED` (bit 0).

---

## 6. Per-firmware notes

### 6.1 ArduPilot
Guided target reachable after arm; takeoff in GUIDED. GCS-failsafe expects our heartbeat (already
sent). `DO_REPOSITION` p4 = yaw heading; planes use loiter radius (p3, sign = direction).

### 6.2 PX4
No "GUIDED" mode — Hold is the reposition-ready state. `DO_REPOSITION` historically executed from
Hold; PX4 accurately reports `MAV_TYPE`, so vehicle-aware fields are reliable. Reject behaviour:
PX4's feasibility checker may `DENY` commands the airframe can't do — surface the ACK reason.

### 6.3 INAV (future)
Has a guided equivalent — confirmed in firmware (`navigation.c::setWaypoint`):
- **WP #255** → directly `setDesiredPosition()`. Only honoured when **armed**, **GCS-assisted nav
  enabled** (`isGCSValid()`), action = WAYPOINT. lat/lon → XY; `alt != 0` → also altitude;
  `p1` in 1–359 → heading. This is "reposition the Position-Hold target" — structurally identical
  to `DO_REPOSITION`.
- **WP #0** → live-sets the home position (armed + GCS-assisted).
- Caveat (analogous to MSP-RC-override): INAV needs **GCS-assisted nav** enabled, else WP #255 is
  ignored — the panel must surface this prerequisite.

This is why the panel is firmware-spanning value and why joystick + INAV guided belong in the same
panel home long-term.

### 6.4 Joystick / HID (future)
- PX4 → `MANUAL_CONTROL` (#69), normalized ±1000. Clean, intended path (what QGC uses).
- ArduPilot → `RC_CHANNELS_OVERRIDE` (#70) in µs, or `MANUAL_CONTROL`; mode-dependent, watch
  failsafe on stream stop.
- INAV → `MSP_SET_RAW_RC` (200). Confirmed in firmware (`fc_msp.c`, `settings.yaml`): requires MSP
  as primary RX **or** the **"MSP RC Override" flight mode + a channel mask**. FC-config
  prerequisite, not a casual override.
- When this layer streams, it counts as an "RC source" and may unlock the stick-required modes.

---

## 7. Panel form factor

- **Compact panel** (PanelShell `compact`, ADR-029), usable beside the map. Contents:
  mode control · Arm/Disarm · Takeoff/Land/RTL · Guided toggle · (Speed/Alt live where relevant).
- Guided "fly here" needs **no large panel** — it lives in the map popup.
- **Mode control is the open UI question.** A dropdown is 3 clicks (open → pick → set) — too slow.
  V1 ships a **dropdown placeholder**; once the real curated mode set is visible we redesign toward
  **direct one-tap mode buttons** for the common safe modes (e.g. RTL / LOITER / AUTO / LAND /
  GUIDED). Final layout decided then.
- A larger split panel is deferred until telemetry feedback / command log justifies it.

---

## 8. Safety layer

Commanding a physical vehicle is outward-facing and partly irreversible — confirmation gestures
are mandatory, modelled on QGC (which uses slide-to-confirm for critical actions):

- **Arm** → slide-to-confirm slider (deliberate, familiar from QGC).
- **All other commands** → **`HoldToConfirm`**: a 2 s long-press button that fills left→right while
  held; release early cancels. Touch- and touchpad-friendly (better than a slider on a laptop
  trackpad). **Reusable component in the control library** (ADR-029) — not panel-local.
- **Disarm / emergency** → must stay fast; long-press (2 s) is acceptable and prevents accidental
  in-flight disarm. (Revisit if a faster ground-disarm path is wanted.)
- Every action shows its **`COMMAND_ACK` result** (accepted / denied + reason).
- Stick-required modes obey the RC hard-lock (§5.2) on top of the gesture.

---

## 9. Architecture

### 9.1 Rust `control/` module (new, alongside `mission/`)
- Encodes `COMMAND_LONG` / `SET_MODE` / `SET_POSITION_TARGET_GLOBAL_INT` for the connected target
  (`target_system`/`target_component` from telemetry).
- **ACK correlation**: subscribes to `COMMAND_ACK`, matches by command id, resolves the pending
  Tauri call. Timeout → `TEMPORARILY_REJECTED`-style error.
- Mode tables (Ardu Copter/Plane/Rover + PX4 main/sub) live here as data, classed safe/stick.
- Tauri commands return `Result<T, String>`; event/log strings English (CLAUDE.md). Reuses the
  existing MAVLink handler; respects the scheduler's exclusive port ownership (ADR-010).

### 9.2 Frontend
- **`MavCommandPanel.svelte`** — PanelShell `compact`, ardu/px4 profile via `mavTypeToClass`.
  Mode control (curated), action buttons (gesture-wrapped), Guided toggle.
- **`HoldToConfirm` + arm slider** → shared **control library** components (ADR-029), reusable.
- **Guided map interaction** → a store flag (like the WP-edit interaction) flips map click handling;
  click opens the Fly-Here popup; reuses the WP-editor popup shell (ADR-046).
- **`controllers/vehicleControl.ts`** — domain logic (no UI): mode resolution, command dispatch,
  ACK surfacing, RC-presence lock state. Guided-toggle ↔ active-mode sync.
- Persist last Fly-Here values (alt/speed/heading) per session.

### 9.3 Data we already have
Vehicle class (`missionArdupilot.ts`), active mode (unified flight-mode model, ADR-044), RC channels
(telemetry), home/launch reference (ADR-039), target ids (MAVLink handler).

---

## 10. Open questions
1. **RC-present signal** — channel activity vs. rssi vs. SYS_STATUS sensor-health bit (§5.2).
2. **Mode UI** — dropdown → direct buttons; finalize after seeing the real curated set (§7).
3. **Plane/Rover mode enum numbers** — confirm full lists against firmware headers at implementation
   (Copter + PX4 already verified).
4. **Emergency disarm** — is 2 s hold acceptable, or do we want a distinct fast path on the ground?
5. **Takeoff altitude reference** — relative-to-home (MP convention); confirm against our 3D
   altitude handling.

---

## 11. Future phases
- **P-INAV**: INAV guided (WP #255 reposition + WP #0 live-home) into the same panel.
- **P-Joystick**: HID gamepad → continuous RC (`MANUAL_CONTROL` / override / `MSP_SET_RAW_RC`),
  unlocks stick-required modes, RC source for the hard-lock.
- **P-Secondary**: servo/relay, gimbal/ROI, reboot, calibration; possible split panel.
