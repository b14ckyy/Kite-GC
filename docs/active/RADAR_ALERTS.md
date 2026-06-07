# Radar — Conflict Alerts (Plan C)

> Status: **C0 + C1 shipped** (2026-06-07) — core logic, banner, audio, map highlight, settings. See
> **ADR-035**. Smart proximity/conflict alerts for foreign vehicles vs the connected UAV, on top of the
> [radar subsystem](RADAR_TRACKING_CORE.md) + [map](RADAR_TRACKING_PANEL_AND_MAP.md). ADS-B-first (the only
> system with reliable position+velocity today); FormationFlight/Radio inherit it automatically once they
> feed the same `TrackedVehicle`s.

## 1. Scope & decisions (2026-06-07)
- **Protected point = the connected UAV only** (valid GPS fix). No UAV/fix ⇒ alerts off (no GCS/area
  alerting in this plan).
- **Two alert stages:**
  - **Stage 1 — Warn zone (caution):** a contact is **inside a horizontal warn radius** (+ within the
    vertical relevance band) **AND the distance is decreasing** (range-rate beyond a small deadband).
    Cheap, always available, no velocity vector needed.
  - **Stage 2 — Min-approach (warning):** predicted **closest point of approach (CPA)** from the current
    course + vertical speed of **both the contact and the UAV**, falling under the miss-distance
    thresholds within a look-ahead window. Needs a contact velocity.
- **UAV course-stability gate (decided):** the UAV's horizontal course is used in the CPA **only if it
  has stayed within ±20° over the last 10 s** (steady straight flight, e.g. long-range cruise). While
  manoeuvring/loitering the heading is unreliable ⇒ treat the UAV as a **non-translating point**
  (horizontal velocity = 0); its vertical speed (vario) is still used. (Stage 1 always still applies.)
- **Vertical relevance band:** contacts more than **±2000 m** from the UAV's altitude never alert (an
  airliner 10 km overhead is irrelevant even if it passes over). This is the **same value** as Stage 1's
  vertical threshold — one number gates both stages.
- **Outputs:** map highlight (2D + 3D) · **alert banner along the top of the map** · audio. (No panel
  list / FPV-HUD alerting in this plan — can be added later.)
- **Settings (now):** only **Stage 1 on/off** and **Stage 2 on/off** toggles. The numeric parameters are
  **not** user-editable yet, but the code keeps them in one overridable config object (§5) so per-user
  tuning can be added later without refactoring. All parameters are documented in §4 for the future user
  guide.

## 2. Conflict model (math)
Compute in a local **ENU frame centred on the UAV** (metres: east `e`, north `n`, up `u`). Reuse the
existing geo helpers; everything is plain arithmetic + one quadratic minimum (no matrices).

**Inputs per contact** (already in `TrackedVehicle` + enrichment): position, `heading_deg`,
`ground_speed_ms`, `vertical_speed_ms`. **UAV** from telemetry: position, course, ground speed, vario.

**Velocities** (ENU, m/s): `v = (speed·sin(hdg), speed·cos(hdg), verticalSpeed)`.
- `v_uav`: horizontal part only if the course passed the stability gate (§3), else `(0, 0, vario)`.
- `v_c`: contact velocity.

**Relative state:** `r = p_c − p_uav` (ENU offset of the contact), `v_rel = v_c − v_uav`.

**Stage 1 (range-rate):** horizontal sep `d_h = hypot(r.e, r.n)`, vertical sep `d_v = |r.u|`. Range-rate
`ḋ = Δd_h/Δt` (computed per update from the previous horizontal distance — inherently relative). Caution
when `d_h ≤ R_warn` **and** `d_v ≤ H_warn` **and** `ḋ < −ḋ_min` (closing faster than a small deadband).

**Stage 2 (CPA):** only evaluated for contacts already within the **arming range** `R_arm` (beyond that the
prediction is meaningless — see §4 rationale).
```
t_cpa = −(r · v_rel) / (v_rel · v_rel)        # seconds; only meaningful if v_rel·v_rel > 0
```
- If `t_cpa ≤ 0` (already diverging) or `v_rel ≈ 0` ⇒ no Stage-2 conflict (Stage 1 may still fire).
- Clamp `t_cpa` to the look-ahead window `[0, T_la]`.
- CPA offset `r_cpa = r + v_rel · t_cpa`; miss `d_h_cpa = hypot(r_cpa.e, r_cpa.n)`,
  `d_v_cpa = |r_cpa.u|`.
- **Warning** when `t_cpa ≤ T_la` **and** `d_h_cpa ≤ R_cpa` **and** `d_v_cpa ≤ H_cpa`.

**Jitter protection (decided):** the CPA prediction is noisy far out, and a small miss-cylinder flips
on/off near the boundary. So Stage 2 is governed by **persistence + hysteresis**, *not* by enlarging the
cylinder:
- **Confirm-in:** the Stage-2 condition must hold for ~**3 s** (≈ 3 consecutive updates) before the
  warning is raised — a single noisy frame raises nothing.
- **Exit margin:** the warning is held until the miss distance exceeds the threshold by **×1.3**
  (> 1300 m / > 325 m) and stays out for a few seconds — no chattering at the edge.

This applies to Stage 1 too (widened `R_warn` + hold). **Stage 1 additionally requires the contact to
still be *approaching* to stay latched** — once `ḋ` turns positive past `recedeDeadband` (it has flown
past / is leaving), the caution clears after the hold even if still inside the radius.

**Range-rate sampling:** `ḋ` is recomputed only when the contact reports a *new* position (`lastSeenMs`
changes) and **held between** those bursty updates — otherwise the unchanged distance between ADS-B
samples reads as "not closing" and Stage 1 flickers off every other frame.

## 3. Stability gate (UAV course)
- Keep a short ring buffer of `(t, heading)` for the UAV (last ~10 s).
- Stable ⇔ the angular spread over the window ≤ **20°** (handle 0/360 wrap). Stable ⇒ use UAV horizontal
  velocity in §2; unstable ⇒ horizontal velocity 0 (vario still used).
- Also require a minimum UAV ground speed (`V_uav_min`) for "course" to mean anything — below it (loiter /
  hover) treat the UAV as a non-translating point regardless of the spread.

## 4. Parameters & defaults (documented for the user guide)
Single source of truth: an `AlertConfig` object in the controller (§5). Defaults below; all in SI
(metres, seconds, m/s) internally, shown to the user in their display units.

| Parameter | Symbol | Default | Meaning |
|-----------|--------|---------|---------|
| Warn radius (horizontal) | `R_warn` | **5000 m** | Stage 1 caution if a contact is inside this and closing. |
| Warn vertical band | `H_warn` | **2000 m** | Stage 1 vertical limit; also the global relevance cutoff (contacts further off in altitude never alert). |
| Closing-rate deadband | `ḋ_min` | **10 m/s** | Stage 1 only fires if the contact closes faster than this (noise floor / ignores parallel & slow-drift traffic). |
| Recede deadband | `recedeDeadband` | **1 m/s** | Stage 1 *clears* once the contact recedes faster than this — a contact that has flown past stops alerting even while still inside the radius. |
| CPA miss radius (horizontal) | `R_cpa` | **1000 m** | Stage 2 warning if the predicted closest horizontal pass is under this. |
| CPA miss height (vertical) | `H_cpa` | **250 m** | Stage 2 vertical miss limit at CPA. |
| Look-ahead | `T_la` | **45 s** | How far ahead the CPA is predicted. |
| Arming range | `R_arm` | **10000 m** | Stage 2 is only computed for contacts already within this range. |
| UAV course-gate window | — | **10 s** | Rolling window for the heading-stability test. |
| UAV course-gate spread | — | **20°** | Max heading spread over the window to count as "steady straight flight". |
| UAV min ground speed | `V_uav_min` | **5 m/s** | Below this the UAV is treated as a non-translating point. |
| Stage-2 confirm time | — | **3 s** | The CPA condition must persist this long before warning. |
| Exit hysteresis factor | — | **1.3×** | Alert clears only once separation exceeds the threshold by this factor (+ a short hold). |

**Rationale for `T_la` = 45 s + `R_arm` = 10 km** (worth keeping): below 10 000 ft (3048 m) civil traffic
is speed-limited to **250 kt ≈ 463 km/h ≈ 129 m/s**. A 45 s look-ahead therefore reaches a head-on
closure of at most ~**5.7 km** — close enough that the 1000 m safety cylinder triggers reliably when the
geometry actually converges, while Stage 1 (pure distance, 5 km) gives at least ~35 s of caution lead-time
before the closest approach. The 10 km arming range only ever matters for a supersonic jet co-altitude
with the UAV (≈ 30 000 ft), which is effectively impossible for this use case — so it costs nothing and
bounds the maths.

## 5. Architecture (frontend)
Pure frontend — all inputs are already in the browser; outputs are UI/audio (ADR-009 *controllers/*).
- **`controllers/radarAlerts.ts`** — the brain. Subscribes to `radarVehicles` + `telemetry`; maintains the
  UAV heading ring buffer (§3) + per-contact previous horizontal distance (for `ḋ`) + per-contact
  persistence/hysteresis state; evaluates Stages 1/2 each radar update; produces an **alerts store**:
  `{ level: 'caution' | 'warning', vehicleId, dH, dV, tCpa?, bearing }[]` plus the current worst level.
- **`AlertConfig`** — one exported constant holding every numeric parameter from §4 (the single source of
  truth). The controller reads its thresholds **only** from this object, merged with optional overrides, so
  adding user-tunable parameters later = feed overrides from settings into the merge (no logic change).
- **Settings (now):** `RadarAlertSettings { stage1Enabled, stage2Enabled, soundEnabled, voiceEnabled }`
  (default all on) — four switches in the ADS-B tab's "Alerts" group; the numeric params stay in
  `AlertConfig`. A derived `radarAlertLevels` store (vehicleId → level) feeds the map highlights.
- Consumers (shipped):
  - **Banner** — `RadarAlertBanner.svelte`, top of the map. Two stacked banners list **all** affected
    contacts: **warning** (red, white text + black outline, pulsing) with an **evade heading** ⟂ to the
    contact's track away from the CPA + the minimal list (callsign · speed · CPA dist); **caution**
    (yellow, black text) with the detailed list (callsign · bearing · type word · speed · rel-alt) +
    "hold position / increase distance". Click a row → selects the contact.
  - **Map** — the contact's **ground circle = `R_cpa`** (coupled, so visual ≡ the "never enter" zone)
    pulses red (warning) / yellow (caution) in 3D via a `CallbackProperty` material; `requestRenderMode`
    is switched to continuous **only while an alert is active**, then back to on-demand. 2D draws a pulsing
    CSS ring around the contact icon. Both read `radarAlertLevels`; the alert level is part of the 3D
    `groundSig` so the material is rebuilt only on a real state change (flicker-safe).
  - **Audio** — `controllers/alertAudio.ts`: a synthesised tone (Web Audio) + a spoken callout (Web Speech,
    localised to the UI language with an English fallback when no voice is installed). **Stage 1 chimes
    once per contact entering the zone** (no loop); **Stage 2 repeats** every 8 s while active and
    **suppresses Stage-1 sounds**. Separate sound / voice switches.
  - **Debug Monitor (dev only)** — the in-app `DebugPanel` is now multi-tab (MSP | **Alerts**). The Alerts
    tab is the per-contact live readout (`d_h`, `d_v`, `ḋ`, `t_cpa`, miss, stage flags, course-gate). A
    global **GPS-inject** row (checkbox + lat/lon/MSL, ⌖ = fill from map centre) overrides the UAV position
    so alerts can be tested over busy airspace from the desk.

## 6. Phasing
- **C0 — Core logic ✅ (shipped):** `radarAlerts` controller + `AlertConfig` + alerts store; Stage 1 +
  Stage 2 math; stability gate; persistence/hysteresis; Debug Monitor "Alerts" tab + GPS-inject.
- **C1 — Banner + audio + map highlight ✅ (shipped):** `RadarAlertBanner` (stacked warning/caution),
  tone + localised voice callouts, 3D pulsing collision circle + 2D rings, the four settings switches,
  i18n (en/de/fr). (Audio landed here alongside the banner rather than as a separate C2.)
- **C3 — Tuning (open):** finalise thresholds from real flights; expose numeric params as user settings
  (the `AlertConfig` override path is already there); GPS-inject movement sim (course/speed/vario) for
  vertical/Stage-2 testing; optional FPV-HUD cue.

## 7. Open / later
- **GPS-inject movement sim** (course/speed/vario) — static injection can't exercise the vertical CPA well.
- **No-velocity contacts:** Stage 1 only (CPA needs a velocity) — confirmed acceptable.
- **Later:** full multi-contact prioritisation; protect GCS/area too; FormationFlight/Radio once feeding;
  user-tunable numeric parameters (C3); pre-recorded callout audio for engines without TTS (Linux WebKitGTK).
