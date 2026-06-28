# RC Control over MAVLink — ArduPilot + PX4 (cross-platform plan)

> ARCHIVED (2026-06-28) — feature complete. ArduPilot SITL-verified; PX4 validation is user-side and
> not kept open.

> Status: **ArduPilot SHIPPED** (2026-06-21, SITL-verified) · **PX4 IMPLEMENTED, UNTESTED** (2026-06-21,
> no PX4 SITL/hardware available here — needs validation). Extends the shipped INAV/MSP RC control to
> MAVLink flight stacks. The **shared layer** (HID backend, profiles, engine, engage gate, deadman) is
> reused as-is — see the archived `docs/archive/MSP_RC_CONTROL.md`. This doc covers only what differs: the
> **per-platform send adapter** and the platform gating. Doc stays in `active/` until PX4 is validated.
>
> **Implemented (ArduPilot, SITL-verified):** §2 platform gating (dropdown behind "Device", locked on
> connect, persisted offline choice), §3 `RC_CHANNELS_OVERRIDE` adapter (Primary/Secondary, no forced
> release on disengage), §4 platform-aware `rcLayout` + Primary/Secondary group labels. Plus: seed from
> the FC's `RC_CHANNELS` broadcast, an **adaptive read timeout + catch-up pacing** in the MAVLink handler
> so the rate actually hits 10–25 Hz (the read-driven loop otherwise quantized it ~⅓ low), an
> **armed-disengage confirmation** (protocol-agnostic), and the RC tab hidden on passive-telemetry links.
>
> **Implemented (PX4, UNTESTED):** §5 `MANUAL_CONTROL` (#69) adapter — a dedicated 4-axis manual mapping
> model (roll/pitch/throttle/yaw → y/x/z/r, z=0 = mid) + up to 6 aux axes + buttons 1–32 (FC maps the
> action), its own editor/monitor (`ManualConfig`/`ManualStates`), `rcManual` store + live evaluator,
> reuses engage/deadman/rate/armed-confirm. Modes/arm stay on the control panel (`DO_SET_MODE`). A
> `COM_RC_IN_MODE` reminder is shown. **Needs a PX4 SITL/hardware pass before it can be called shipped.**
>
> **Deferred:** per-platform last-profile auto-load (§6), `SYSID_MYGCS` / `COM_RC_IN_MODE` *mismatch*
> warnings (currently a static hint), GCS-side expo/curves (we leave expo to the FC).

Related: **ADR-054** (RC control — the decision record this plan + `docs/archive/MSP_RC_CONTROL.md` detail),
ADR-010 (schedulers own the link), ADR-052 (`VEHICLE_CONTROL.md` — the MAVLink command path, mode
switching, arming), `docs/archive/MSP_RC_CONTROL.md` (the INAV/MSP RC pipeline + shared layer).

---

## 1. The platform divide

RC injection is **not** one mechanism across stacks:

| Stack | Primitive | Model | Closeness to our MSP work |
|---|---|---|---|
| **INAV** | `MSP_SET_RAW_RC` + `MSP2_INAV_SET_AUX_RC` | per-channel µs, RAW stream + latched AUX | (shipped) |
| **ArduPilot** | `RC_CHANNELS_OVERRIDE` (#70) | **per-channel µs** (one message, CH1–16/18) | **very close** — firmware-gated copy |
| **PX4** | `MANUAL_CONTROL` (#69) | **normalised 4 axes + buttons** (no per-channel) | **different** — own mapping mode |

So ArduPilot reuses almost the whole pipeline with a different encoder; PX4 needs its own reduced
mapping. The HID → `rcEngine` → channel-µs front of the pipeline stays identical.

---

## 2. Platform gating + selection

We must know the target stack to pick the channel layout + send adapter.

- **Connected:** derive the platform from the FC (the handshake / `fcVariant` already distinguishes
  INAV / ArduPilot / PX4) and **lock** it for the session.
- **Offline (config without an FC):** a **platform dropdown placed behind the "Device" selector**
  (INAV · ArduPilot · PX4), so the user can build/verify a profile for a chosen stack. On connect the
  detected platform wins and the dropdown locks.
- The platform drives `rcLayout` (the channel split, §4) and which adapter streams.

---

## 3. ArduPilot — `RC_CHANNELS_OVERRIDE` (#70)

A near-copy of the MSP pipeline, **firmware-gated**: same engage gate, seed-on-connect, deadman, link
probe, channel mapping/profiles — only the wire encoder + the channel semantics change.

**Message:** one frame, `chan1_raw … chan18_raw` (uint16 µs) + `target_system/component`. ArduPilot's
limit is **`NUM_RC_CHANNELS = 16`** (verify against source) — so we use **CH1–16**.

**Per-channel value semantics (verify against the MAVLink XML + ArduPilot `RC_Channels` at impl time):**
- **CH1–8 (Primary):** `0` = **release** the channel back to the real RX; `UINT16_MAX (65535)` =
  **ignore** (leave as-is); otherwise the µs override.
- **CH9–16 (Secondary):** `0` = **ignore** (the extension default — does *not* release!); `65534` =
  **release**; `65535` = ignore.

→ This different release encoding at the 1–8 / 9–16 boundary is exactly why we mirror INAV's two-group
UI here as **Primary CH1–8 / Secondary CH9–16** (§4). It's one message on the wire, but the adapter
encodes "skip / release / set" per group correctly.

**No override bitmask** (unlike INAV) — "which channels" is expressed by the 0/65534/65535 values in the
message itself. Our "send CH1–CHmax, gaps = skip" model maps over, but **`skip` ≠ `release`**: a gap
must be the *ignore* value for its band, never `0` on CH1–8 (which would release it).

**Deadman is FC-side too:** param **`RC_OVERRIDE_TIME`** (default ~3 s; `-1` = never). No fresh override
within the window → ArduPilot reverts to the real RX / RC failsafe. We keep our own deadman as the
front-line stop and rely on the FC timeout as backstop.

**GCS gating:** ArduPilot typically only accepts overrides/commands from **`SYSID_MYGCS`**; `RC_OPTIONS`
has bits affecting override behaviour. The panel should surface a mismatch.

**No "override mode" gate** (no `BOXMSPRCOVERRIDE` equivalent) — sending overrides takes effect
immediately (subject to arming/failsafe). That makes our **explicit manual engage gate even more
important** here. Stages collapse to two: (1) monitoring/offline, (2) engaged → streaming overrides.

**Modes:** via the existing `DO_SET_MODE` / vehicle-control panel (ADR-052), or by mapping a flight-mode
channel into the override frame.

---

## 4. Channel split per platform

| Platform | Primary (sticks) | Secondary (aux) | Max | Transport |
|---|---|---|---|---|
| INAV 9.1+ | CH1–16 RAW_RC | CH17–32 AUX_RC | 32 | two MSP messages |
| INAV 8.0–9.0 | CH1–16 RAW_RC | — | 16 | one MSP message |
| **ArduPilot** | **CH1–8** | **CH9–16** | **16** | one `RC_CHANNELS_OVERRIDE` |
| PX4 | 4 axes (x/y/z/r) | buttons / aux | — | `MANUAL_CONTROL` |

`rcLayout.ts` becomes platform-aware (today it's INAV-only): the split + max come from the locked
platform, and both the config editor and the live monitor group by it.

---

## 5. PX4 — `MANUAL_CONTROL` (#69) — implemented, untested

Fundamentally different from the channel platforms — **not** per-channel PWM, so PX4 uses a **dedicated
manual mapping model**, not the channel grid. (Verified against PX4 docs + `mavlink_receiver.cpp`.)

**Wire fields** (`mavlink` crate confirmed): `x` (pitch), `y` (roll), `z` (thrust), `r` (yaw), all
normalised **−1000…1000**; `buttons` + `buttons2` (32 button bits); `enabled_extensions` + `aux1…aux6`
continuous extension axes. `target` = FC sysid. `INT16_MAX` on an axis = "invalid".

- **Throttle (`z`)**: PX4 maps stick **[−1,1] → throttle [0,1]** for **both** multicopter and fixed-wing,
  so **`z` = 0 is mid** (hover/centre), −1000 = 0 %, +1000 = full. We send `z = 0` at stick centre.
- **Activation:** PX4 ignores `MANUAL_CONTROL` unless **`COM_RC_IN_MODE`** allows a MAVLink/joystick
  source (0 = RC only and 4 = disabled both block it; 1/2/3/5–8 allow it). We show a static reminder; a
  live *mismatch* warning is deferred (we don't read the param yet).
- **Buttons:** sent as the raw bitfield; **the FC maps each button → action per vehicle** (PX4/QGC
  joystick config). Simpler for us and more flexible for the user — nothing button-related to maintain GCS
  side. Modes/arm still go through the control panel (`DO_SET_MODE` / `COMPONENT_ARM_DISARM`, ADR-052).
- **No seed:** PX4 doesn't echo `MANUAL_CONTROL` and manual axes are live (no latched state) → engage
  starts streaming immediately (a throttle-position jump is possible, as in QGC; accepted).
- **Failsafe/deadman:** PX4 has a short manual-setpoint timeout → `COM_RC_LOSS_T` failsafe action. Our
  heartbeat deadman + the no-forced-release-on-disengage grace concept apply the same as ArduPilot.
- `RC_CHANNELS_OVERRIDE` is also accepted by recent PX4 (it has `handle_message_rc_channels_override`),
  but it routes through PX4's RC-calibration / `RC_MAP_*` path (fragile, FC-config-dependent), so
  `MANUAL_CONTROL` is the canonical, chosen path.

**Implementation:** a `ManualMap` (4 sticks + ≤6 aux + buttons 1–32) in `stores/rcManual.ts` with a live
evaluator (`manualOutput`, reusing `rcEngine.makeFrame` for input resolution), a dedicated editor
(`ManualConfig.svelte`) + monitor (`ManualStates.svelte`), the `rc_stream_set_manual` command, and the
MAVLink handler sending `MANUAL_CONTROL` when `fc_variant == "PX4"`. Engage/deadman/rate/armed-confirm
reused. **Status: compiles + type-checks; NOT yet flown against PX4 SITL/hardware — validation pending.**

---

## 6. Channel mapping + profiles (reused)

Unchanged from the MSP work — the 8 mapping methods, A/B/H inputs, Learn, per-channel name, the engine
and the profile files all stay. Additions:

- A profile can be **universal** (works on any stack, within that stack's channel limit) or
  **platform-specific** — the user chooses by how they map it.
- **Per-platform last-loaded profile:** remember the last active profile *per platform* and auto-load it
  when that platform is selected/detected (so switching INAV ↔ ArduPilot picks the right config).
  `settings.rcControl` grows a small `lastProfileByPlatform` map.

---

## 7. Safety

- No `BOXMSPRCOVERRIDE`-style gate on MAVLink → the **manual engage** (long-press, default off, never on
  connect) is the primary guard; seed-on-connect still avoids a jump.
- **Disengage = no forced release (grace window).** We deliberately do *not* send a release frame
  (CH1–8 `0` / CH9–16 `65534`) on disengage. With the GCS as the sole RC source (no physical RX) an
  explicit release fires ArduPilot's RC failsafe *instantly*; stopping the stream instead lets the FC hold
  the last override for `RC_OVERRIDE_TIME` (~3 s, default) as a re-engage window before its own failsafe.
  The `override_channels` encoder therefore only ever emits override-or-ignore, never the release sentinel.
- **Armed-disengage confirmation** (protocol-agnostic, also for INAV/PX4): releasing control while the
  vehicle is armed (`armingFlags & 0x04`) prompts a confirm dialog — disengaging hands flying back to the
  FC, which with no RX means a failsafe (RTL/Land/**disarm**, per the FC's `FS_*` config).
- **Arming is NOT blocked over the override band.** `RC_CHANNELS_OVERRIDE` behaves like INAV's RAW_RC
  (fails safe via `RC_OVERRIDE_TIME`), not like latching AUX_RC — so the AUX-latch arming block from the
  MSP plan does not apply here (there is no latch to get stuck on). Recommend not mapping Arm to an
  override channel anyway; use the dedicated control panel's Arm/Disarm + `DO_SET_MODE`.
- `SYSID_MYGCS`: ArduPilot only accepts overrides from this GCS sysid (we send 255). A mismatch silently
  drops overrides — surfacing a warning is **deferred**.
- Deadman front-line (our heartbeat) + FC backstop (`RC_OVERRIDE_TIME`).

---

## 8. Phasing

1. [x] **Platform gating** — platform dropdown (offline, persisted) + connect-lock + platform-aware
   `rcLayout` + Primary/Secondary group labels (`stores/rcPlatform.ts`, `stores/rcLayout.ts`).
2. [x] **ArduPilot adapter** — `RC_CHANNELS_OVERRIDE` (Primary/Secondary ignore semantics) streamed from
   the shared `RcTxState` in the MAVLink handler; reuses engage/seed/deadman. Seeds from the FC's
   `RC_CHANNELS` broadcast. Adaptive read timeout + catch-up pacing for accurate 10–25 Hz. No forced
   release on disengage (grace window — see §7). Firmware-gated. SITL-verified.
3. [ ] **Per-platform profile auto-load** (`lastProfileByPlatform`). Deferred.
4. [~] **PX4** — `MANUAL_CONTROL` full manual mode (4 sticks + aux1–6 + buttons 1–32). **Implemented,
   UNTESTED** — needs a PX4 SITL/hardware pass (no PX4 SITL available to the author).

---

## 9. Open questions

- Exact `RC_CHANNELS_OVERRIDE` release magic-numbers for CH9–16 — confirm against the current MAVLink
  XML + ArduPilot source (the 0 / 65534 / 65535 split above).
- Does ArduPilot need the override stream gated behind a parameter we should read/show?
- PX4 scope (full manual vs deferred).
- Where the MAVLink RC stream lives relative to the existing MAVLink message loop (own cadence vs
  interleave), mirroring the MSP scheduler's RC slot.
