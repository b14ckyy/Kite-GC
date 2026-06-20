# RC Control (INAV) — GCS Joystick / HID Steering over MSP

> Status: **PLAN** (2026-06-20) — not started. Large, safety-critical feature built incrementally.
> Scope of this doc: **INAV only** (ArduPilot RC-override is a separate, later question). The MSP
> version gates (`Feature::MspRc` 8.0+, `Feature::AuxRc` 9.1+) already exist in `msp/features.rs`.
>
> **First delivery = Phases 1 + 2 only** (HID foundation + mapping/config). The actual RC streaming
> (Phases 3–5) is designed here but built later, step by step. UI details (compact + advanced panel)
> are sketched but finalised when we get there.

Related: ADR-010 (multi-protocol schedulers — the scheduler owns the serial link exclusively),
ADR-029 (panel framework + control library), VEHICLE_CONTROL.md (MAVLink command path; §11 deferred
HID/joystick RC), STICK_OVERLAY.md (`GimbalStick.svelte` built store-free as a live-HUD reuse hook).

Reference studied: **mwptools** (`src/mwp/mwp-handle_serial.vala`, `mwp-hid-config.vala`,
`mwp-sticks.vala`, `src/samples/hidex/`). We reuse its *concepts* (fire-and-forget flag byte,
half/full-duplex timing, per-axis mapping model) but **not** its architecture — MWP runs SDL in a
separate UDP server process to dodge GTK/SDL main-loop conflicts; we have no such constraint and read
HID directly in a Rust backend thread.

---

## 1. Why this is hard

RC-over-MSP is not "send sticks to the FC". The behaviour depends on **how the FC's receiver is
configured**, on a **per-channel override bitmask** in the FC, on **firmware version**, and on a
**failsafe contract** that must never be violated. Getting any of these wrong can mean an unflyable or
runaway aircraft. AUX_RC (the author's own INAV PR) exists precisely to make the *safe, simple* subset
of this tractable. The plan therefore leads with the safe primitive and adds full stick streaming only
behind explicit, configuration-aware gating.

---

## 2. The two MSP primitives

### 2.1 `MSP_SET_RAW_RC` (cmd 200) — streamed full RC frame · INAV **8.0+**
- Carries channel values as `u16` (µs, ~1000–2000, centre 1500), **always starting at channel 1**.
- **Channel order = MSP/RX standard AETR**: `[Roll, Pitch, Throttle, Yaw, AUX1…AUX28]`
  *(verify against INAV source at implementation time — order follows the rx channel map).*
- **Fire-and-forget (8.0+):** in the **MSPv2 header the flag byte is set to `1`** for SET_RAW_RC. INAV
  recognises this and **sends no reply** → zero downlink cost. This is why we **block RC over MSP for
  INAV < 8.0** (older firmware always replies → wastes the scarce downlink). MWP does exactly this
  (`serial-device.vala`: `flag = (cmd == SET_RAW_RC) ? 1 : 0`).
- **Re-send rate:** default **10 Hz**, **min 5 Hz**, **max 25 Hz**. Above 25 Hz buys nothing (no RC
  link is faster). **Below 5 Hz → the FC acts** (see §4 failsafe).
- **Has a failsafe contract** (§4) — this is the *safe* streaming primitive.

### 2.2 `MSP2_INAV_SET_AUX_RC` (cmd 0x2230) — latched AUX channels · INAV **9.1+**
- Sets **individual AUX channels, range 13–32 only**. **Channels 1–12 are blocked by design**
  (the four sticks + the low AUX band that carries arm/mode are off-limits to this path).
- **Latch semantics:** a value, once set, **persists in the FC** until the next AUX_RC update. It is
  *not* a streamed frame.
- **No failsafe, no auto-fallback.** If the GCS disconnects, the channel simply **holds its last
  value**. → **Arming via AUX_RC is possible (CH13+) but strongly discouraged** — there is no failsafe
  net under it.
- **Independent of the SET_RAW_RC override** — works in *all* control modes (incl. alongside an MSP-RC
  radio), because it never touches the sticks and so can't collide with stick streaming.
- This is the **safe, simple entry point** (Phase 3): flip persistent switches/modes, nothing more.

---

## 3. The three control modes (configuration-driven)

The override scope is **not** a user preference — it is dictated by the FC's RC configuration. We
auto-detect a best guess and have the operator **explicitly confirm** (safety-critical).

| Mode | FC / radio situation | Sticks via `SET_RAW_RC` | AUX via `AUX_RC` | Arming |
|---|---|---|---|---|
| **A — Override** | Normal RX present + `BOXMSPRCOVERRIDE` mode active (toggled via RC switch *or* AUX_RC) | ✅ overrides physical sticks, **subject to the FC override bitmask** | ✅ always | only what override + config permit |
| **B — MSP-RC radio** | A radio (e.g. **mLRS**) already feeds the FC over MSP-RC | ❌ **blocked** — our SET_RAW_RC would collide with the radio's | ✅ AUX only | via AUX_RC only (discouraged) |
| **C — Pure MSP-RC** | `rx_type = MSP`, **no** external radio (direct serial / internet link) | ✅ full takeover | ✅ | technically possible |

**"AUX always works"** is the through-line: AUX_RC is usable in every mode because it never collides
with stick streaming.

**Detection signals (best guess → explicit confirm):**
- Is `BOXMSPRCOVERRIDE` present in the mode ranges? → Mode A is available.
- RX type = MSP (`MSP_RX_CONFIG`)? → Mode B or C.
- Live radio link present (RSSI / LQ ≠ 0)? → distinguishes **B** (radio present) from **C** (none).

---

## 4. Channel override rules (the fiddly part)

1. **SET_RAW_RC always starts at channel 1.** To control only ch5+ch6 we **must** send ch1…ch6.
   Lower channels we don't want to touch are sent as **`0` = skip** (the FC keeps the RX value for a
   zero channel).
2. **FC override bitmask.** When MSP overrides a standard RC link, a **bitmask in the FC settings**
   independently defines **which channels MSP is even allowed to override**. Our send mask and the
   FC mask both apply — a channel passes only if neither blocks it. The GCS should **read/display**
   the FC bitmask so the operator understands why a channel won't move.
3. **AUX_RC is channel 13–32 only** (§2.2) and ignores the bitmask question (it's a different path).
4. Net send model per channel: `value = mapped_stick` if we own it, else `0` (skip). The advanced
   panel surfaces, per channel, {owned-by-GCS / skipped / blocked-by-FC-mask / driven-by-which-mode}.

---

## 5. Failsafe / deadman contract (mode-independent)

**The streaming pipeline is a deadman.** If anything in the chain breaks — HID controller unplugged,
no fresh axis frame within the watchdog window, frontend/backend disconnect, app close — we **stop
sending SET_RAW_RC**. We **never** hold or repeat the last stick value.

What the FC does once streaming stops (<5 Hz) then depends on its config:
- **MSP is the only configured receiver** → **Failsafe → RTH** (the safe outcome for Mode C).
- **A normal serial RC link also exists** → **fallback to the standard RC link** (Mode A/B).

The watchdog lives in the **Rust backend (gilrs) thread**, independent of the webview — so a frozen,
unfocused or crashed UI cannot keep stale sticks streaming. AUX_RC, being latched, has *no* such net;
that is exactly why we keep arming/critical switches off it.

---

## 6. HID architecture — Rust backend thread, native per-OS backends

**Decision: read HID in a dedicated Rust backend thread** (not the Web Gamepad API), using **native
per-OS raw backends** (not a gamepad-abstraction library).

Why a backend thread (not the webview):
- **Timing & safety:** the Web Gamepad API polls in the webview's `requestAnimationFrame`, which the
  OS **throttles/pauses when the window loses focus or is minimised** → RC would freeze → unintended
  failsafe. A backend thread streams on a deterministic timer regardless of UI focus.
- **One owner:** read → map → stream all live in one place (the backend), next to the scheduler that
  owns the serial link.

Why native backends (not `gilrs` / a gamepad lib): gilrs (and any *gamepad* abstraction) forces an
**Xbox layout** — on Windows via Windows.Gaming.Input's *Gamepad* projection. A HOTAS / RC transmitter
has extra axes (throttle sliders, mini-sticks) and hats that don't fit that model → they get
**misclassified as buttons**, and hats collapse to fixed switch-buttons. Confirmed on a VelocityOne
Flightstick. This is exactly why mwptools reads SDL's *raw* joystick API, not the gamecontroller layer.

We instead read the **raw device topology** directly, per OS:
- **Windows** → `Windows.Gaming.Input::RawGameController` (`AxisCount`/`ButtonCount`/`SwitchCount` +
  `GetCurrentReading`). Raw axes/buttons/switches, no projection. The `windows` crate is already in the
  tree (pulled by btleplug); we just enable the `Gaming_Input` features. *(chosen over SDL2, which would
  need cmake — absent here — or a vendored `SDL2.dll`.)*
- **Linux** → `evdev` (pure Rust): raw `ABS_*` axes, `BTN_*` keys, `ABS_HAT*` axis pairs → hats.
- **Other** (macOS): null backend (no devices) so it still builds.

Both backends yield the same `HidDevice` / `HidSnapshot { axes, buttons, hats }` shape, so the frontend
+ later mapping are platform-agnostic. *(The Linux backend compiles only on Linux and is verified on the
test machine, not the Windows dev host.)*

Flow:
```
native backend poll (~50 Hz, backend thread)   [hid/windows.rs · hid/linux.rs]
  → raw axes / buttons / hats → hid-input event → frontend (calibration view; later: stick HUD)
  ── later phases ──
  → apply per-axis mapping (channel, invert, deadband, expo)
  → watchdog (fresh-frame deadman, §5)
  → SchedulerCommand::RcStream { channels }   // new scheduler command
  → scheduler emits SET_RAW_RC @ 10–25 Hz (fire-and-forget flag byte)
```

**Scheduler integration:** RC streaming becomes a **new `SchedulerCommand` variant** — the scheduler
thread already owns the serial port exclusively (`scheduler/mod.rs`), so RC must flow through it, not a
parallel serial path. Streaming cadence is interleaved with polling (MWP's half-duplex idea), or the
scheduler runs an independent RC tick (full-duplex) — decided in Phase 4.

---

## 7. Mapping model + profiles (Phase 2)

Per-axis / button / hat mapping. Concept borrowed from MWP, trimmed:

- **Axis:** `{ channel: 1..32, source: 'axis', invert, deadband, expo }` (min/max/trim later if needed).
- **Button / hat:** `{ channel, source, … }` — latch mode (momentary / toggle / multi-position) TBD.
- **Live calibration:** the collapsible raw-input monitor streams raw values so the operator can wiggle
  a stick to identify the control before assigning it (MWP does this; the only sane UX).
- Channel labels come from the FC: read `MSP_BOXNAMES` / `MSP_MODE_RANGES` so each AUX channel shows
  **which mode box it drives** ("CH5 → ANGLE", "CH7 → RTH"). (Needs MSP — after the local part.)

**Profiles (shipped).** Mappings live in **shareable profile files**, NOT in settings/localStorage:
`Documents/KiteGC/HID-Profiles/<name>.json` (`hid/profiles.rs` + `stores/rcProfiles.ts`). A profile is
`{ name, deviceUuid?, deviceName?, channels }` and is **never auto-linked** to a device or FC — the user
picks the active profile and the matching FC config themselves. The panel's config side has a profile
dropdown + **Save** (overwrite, confirm) / **New** (name prompt) / **Delete** (confirm; keeps the
working config loaded). `settings.rcControl` holds only `{ enabled, selectedUuid, activeProfile }`.

**Shared centre deadband (shipped).** A small ±0.05 (2.5% of full travel) scaled centre deadband is
applied to every raw axis in `hid/mod.rs` (both backends) so a controller's resting centre offset
(seen up to ~0.04 on a gamepad) can't leak a stray command. Per-channel deadband/expo come on top in
the mapping layer.

---

## 8. UI — two-stage panel (sketch, finalised later)

Reuse the panel framework (ADR-029), same two-stage pattern as Vehicle Control / Mission.

- **Compact panel (control surface):**
  - Live **stick inputs** — reuse `GimbalStick.svelte` (built store-free for exactly this).
  - **Channel states** — current value per active channel.
  - **What each channel controls** — pulled from the FC via Box IDs / mode ranges.
- **Advanced panel (expand → right-hand split):**
  - Full HID mapping/config (axes, buttons, deadband, expo, invert).
  - Mode selection + the FC override bitmask view (§4).
  - Failsafe/rate settings (re-send rate 5–25 Hz).

Exact layout deferred — agreed to settle when we build it.

---

## 9. Version gating summary

| Capability | Min INAV | Feature flag |
|---|---|---|
| AUX_RC latched switches (CH13–32) | **9.1** | `Feature::AuxRc` |
| SET_RAW_RC stick streaming (fire-and-forget) | **8.0** | `Feature::MspRc` |
| RC over MSP at all | **8.0** | (block < 8.0 — pre-8.0 replies waste downlink) |

---

## 10. Phasing

1. **HID foundation** *(shipped)* — native per-OS backend (RawGameController / evdev), device
   enumeration, live axis/button/hat stream + shared centre deadband. Verified on Windows.
2a. **Profiles + raw monitor relocation** *(shipped)* — shareable profile files (§7), Save/New/Delete,
   collapsible raw-input monitor on the config side.
2b. **Channel mapping** *(now)* — assign axis/button/hat → channel 1..32 with method/behaviour, write
   into the active profile; live channel-value view. (The complex part.)
3. **AUX_RC path (9.1+)** *(later)* — latched switches CH13–32. The safe, simple first real control.
4. **SET_RAW_RC streaming (8.0+)** *(later)* — codec flag byte, `SchedulerCommand::RcStream`, deadman
   watchdog, Modes A/C, send-mask + zero-skip.
5. **Mode detection + takeover UI** *(later)* — config-driven mode (§3) with explicit confirm, live
   stick HUD, arming policy.

---

## 11. Open questions (revisit before Phase 3+)

- **Arming policy** — decided per mode once takeover UX exists. Default leaning: **block arming via the
  GCS RC path** unless Mode C (pure MSP-RC) and behind an explicit Hold-to-Confirm gesture. AUX_RC
  arming stays disabled (no failsafe net).
- **Half- vs full-duplex** streaming cadence (interleave with polling vs independent RC tick).
- **Exact SET_RAW_RC channel order** vs the rx channel map — confirm against INAV firmware source.
- **ArduPilot RC override** — entirely separate mechanism (`RC_CHANNELS_OVERRIDE`); its own future plan.
- Button latch / multi-position semantics — final model in Phase 2/3.
