# Kite Ground Control — Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed
- **WP popup delete left a stale selection (INAV mission editor).** Deleting a waypoint via its map
  popup only cleared the primary index, not the multi-select set — so the waypoint that shifted into the
  freed index stayed highlighted and a subsequent click started a multi-selection. The popup delete now
  fully clears the selection, matching the panel's delete button.
- **Airspace Manager panel didn't refresh on connect.** An already-open panel didn't show the geozone /
  geofence sections until a tab switch, because the FC capability loads asynchronously after connect; the
  panel now re-initialises on connect/disconnect and when that capability resolves.
- **Mission EEPROM save.** "Save to EEPROM" persisted nothing (a later "Load from EEPROM" returned the
  old mission) — the `MSP_WP_MISSION_LOAD`/`SAVE` command codes were swapped (18/19), so the app actually
  sent *load* on save (overwriting the upload with the stored mission) and *save* on load. Corrected to
  INAV's codes (LOAD=18, SAVE=19); EEPROM save/load now round-trip.

### Changed
- **Geozones reboot the FC after saving + are locked while armed.** INAV recomputes the internal geozone
  structures only at boot, so "Save to FC" now writes + EEPROM + **reboots** (the link drops, then the
  reconnect handshake re-reads). Geozone editing is disabled while the craft is armed.
- **Airspace Manager panel reworked.** Dropped the two-column split: the panel is single-column with a
  **Nearby** view (default) and a **Settings/editor** view, switched by a header button (no nearby list
  while editing). When the OpenAIP overlay is disabled in settings but a geozone-capable INAV FC is
  connected, the panel still appears and shows **only** the geozone editor + its overlay toggles.
- **Default UDP connection port is now 14550** (the MAVLink convention); switching the transport flips
  between TCP `5761` and UDP `14550` without overwriting a custom port (e.g. SITL `5762`). TCP unchanged.

### Added
- **ArduPilot / PX4 Geofence — full editor.** Third Airspace-Manager safety subsystem (after Autoland and
  Geozones), the MAVLink counterpart to the INAV geozone editor. On connecting to an ArduPilot/PX4 FC,
  Kite downloads the on-board geofence (inclusion/exclusion × polygon/circle + the return point, via
  `MAV_MISSION_TYPE_FENCE`) and shows it on both maps — **blue inclusion / amber exclusion**, with the 3D
  view extruding each zone to the global vertical limit (`FENCE_ALT_MAX` / `GF_MAX_VER_DIST`). Zones can be
  **created and edited** on the map (drag handles, edge-midpoint insert, waypoint-style popup for exact
  coordinates + radius) — the same UX as the geozone editor. The Airspace Manager panel gains a **Fence**
  section (add inclusion/exclusion × polygon/circle, per-zone type toggle + radius) plus the global
  enforcement params shown with friendly labels, units and spec-based value bounds: the fence master
  switch is a toggle and the **breach action** is a named dropdown (options chosen automatically by
  autopilot + vehicle type — Copter/Plane/Rover/PX4 — while the raw firmware code is what gets written).
  **Save to FC** uploads the geometry + writes the changed params (no reboot needed). A new **Geofence**
  layer toggle (2D + 3D, capability-gated) sits under Geozones. See `docs/active/GEOFENCE.md`.
- **INAV Geozones — map display (read-only).** On connecting to a geozone-capable INAV FC (**≥8.0**),
  Kite now downloads the on-board geozone config (all zones + their vertices, via `MSP2_INAV_GEOZONE` /
  `..._VERTEX`) and shows it on both maps. Inclusive (Flight-Zone) zones are drawn **blue**, Exclusive
  (No-Flight-Zone) **amber**, and the **fence action** drives the line/fill style (scheme adopted from
  MWPTools): `None` → dashed thin, `Avoid` → solid thin, `Pos-Hold`/`RTH` → solid thick; a translucent
  area fill for every exclusive zone with a real action and for inclusive RTH zones. The **3D** view
  extrudes each zone between its min/max altitude (circle → cylinder, polygon → hull; AGL terrain-
  anchored, AMSL geoid-referenced, "no upper limit" capped) and mirrors the same line/fill scheme via
  boundary polylines. A new **Geozones** layer toggle (2D + 3D, default on, only shown when a capable FC
  is connected) sits under Airspaces in the Airspace Manager panel, which also lists the configured
  zones (collapsible rows: number · shape · vertex count / radius, with type/altitudes/action on
  expand). Geozones are always shown while the Mission Planner is in edit mode.
- **INAV Geozones — editor + mission safety check.** Geozones can now be **created and edited** and
  written back to the FC. Add a circle or polygon (sized to the current map zoom — a circle = 2 tiles
  radius, a polygon = a ~2×3-tile trapezoid); toggle **Edit on map** to drag the labelled vertex /
  centre / radius handles, click an edge midpoint to insert a vertex, or open a waypoint-style popup for
  exact coordinates (+ radius) and per-vertex delete. The panel edits each zone's type (Inclusive ↔
  Exclusive), fence action, lower/upper altitude (10 m steps; 0 upper = no limit), radius and AGL/AMSL
  reference (which auto-converts the altitudes via the terrain elevation). **Save to FC** writes the
  whole set as a batch + EEPROM, gated by **sanity checks** matching INAV's (≤63 zones, ≤126 vertices
  total, circle = radius > 0, polygon ≥ 3 vertices + non-self-intersecting, upper > lower; polygon
  winding auto-corrected to CCW on save). A **mission safety check** (in mission edit mode) flags —
  without ever blocking — when the planned path would cross a No-Flight-Zone (red) or leave an inclusion
  zone (amber, only enforced when launch/home is inside one, mirroring INAV), or when launch/home sits
  inside an NFZ (arming may be blocked). It is altitude-aware (waypoint height vs each zone's band) and
  draws the offending legs **red** on both the 2D and 3D maps. See `docs/active/GEOZONES.md`.
- **INAV Safe Home Manager + autoland config.** On connecting to INAV, Kite now downloads
  all safehomes (any version) plus — on **INAV ≥7.1** — the fixed-wing autoland approach config and the
  approach-relevant `nav_fw_land_*` settings. A new **Safe Home Manager** (house button in the INAV
  mission panel, ≥7.1) edits the 8 safehome slots (enable, lat/lon, a **+** button that drops the point
  at the map centre) and their per-site approach (approach/land alt, two headings with an **Excl.**
  exclusive-direction toggle, turn direction, sea-level ref), plus the global approach params (approach
  length, glide/flare alt+pitch, pitch-to-throttle modifier). Flare fields show only when a rangefinder
  is present; REL↔MSL altitude toggling auto-converts via the terrain elevation at the point. **Save to
  FC** writes the whole config as one batch + EEPROM. Versions >9.1.x show a "not validated" hint. The
  **2D map** shows enabled safehomes (draggable) with a green `max_distance` ring (disarmed-only) + a
  yellow `loiter_radius` ring and the full approach pattern (downwind/base/final, INAV-configurator
  style). The **3D map** mirrors this: teardrop markers, both rings (loiter raised to the approach
  altitude) and the approach drawn as a real terrain-relative **descent** (downwind level → base −33 % →
  final to the ground). Each slot has a **Clear** button that resets it to unset; empty (0,0) safehomes
  aren't drawn and only pre-fill a default approach altitude in the editor (set slots show their loaded
  values as-is). See `docs/archive/AUTOLAND_SAFEHOME.md`.
- **Automatic `blackbox_decode` download.** INAV Blackbox imports need the external `blackbox_decode`
  tool (kept external so it can track new INAV versions). When it's missing, Kite now offers a one-click
  download from the latest [iNavFlight/blackbox-tools](https://github.com/iNavFlight/blackbox-tools)
  release instead of just erroring — Windows (`.zip`) and Linux/macOS (`.tar.zst`), installed into a
  writable app-data `bin/` dir. Settings → Flight Logbook shows the installed decoder version
  (`blackbox_decode --version`, read-only) with a Download/Update button, so it's clear when an update is
  needed for a newer INAV log format. Android is not supported (no Blackbox import there).
- **Runtime debug mode for release builds (`--debug`, ADR-056).** Starting a shipped release build with
  `--debug` exposes the full in-app Debug Monitor (incl. the MSP/MAVLink stat tabs + DEV playground) and
  raises the diagnostic file log to Debug — no separate debug binary needed. The stat trackers are now
  compiled into every build and gated at runtime (one atomic load when off; ~44 kB larger executable);
  `tauri dev` is unchanged. Amends ADR-008 (compile-out → runtime gate).
- **RC profiles split by platform.** The profile dropdown now only shows profiles matching the active
  platform group — INAV/ArduPilot share the channel-method model, PX4 is a separate manual (MANUAL_CONTROL)
  model, and the two aren't interchangeable. Profiles carry an explicit `kind` (inferred for older files);
  switching platform deselects an incompatible active profile instead of showing it.
- **Telemetry Relay: Serial and BLE split into separate output categories** again (Serial / BLE / TCP /
  UDP) instead of one combined "Device" picker. The BLE discovery listener now runs whenever the relay
  panel is open (pure event subscription, collision-free), and the panel runs its own serial+BLE scan when
  connected via a non-BLE transport — so BLE devices appear without first selecting BLE in the main connect
  bar. (A BLE *primary* link still can't be scanned in parallel — single adapter.)
- **RC input methods: axis methods now carry an "Axis" prefix** (Axis Passthrough / Axis Analog Adjust /
  Axis Dual Source), mirroring the "Button …" methods, so the selected method reads unambiguously when the
  dropdown is closed.
- **Diagnostic file logging (ADR-055).** The backend now writes a real log file so connection problems
  leave a trace. Until now a logger was **never installed**, so every `log::` call was a silent no-op — a
  failed connect only showed a UI toast. A custom `log::Log` logger (no new dependency) writes a rotating
  TXT to `<AppData>/kite-gc/kite-gc.log` (portable → `data/`; the previous run is kept as `*.prev`).
  Verbosity is set in **Settings → Diagnostics** (OFF / Errors / Warnings / Debug; default Warnings) and
  applied at runtime; an **Open Folder** button reveals the file. To diagnose a connection issue: set
  Debug, reproduce, send the log. Connection-flow logging (transport open, MAVLink handshake, FC-stack
  identification, success/failure) is now captured.
- **MAVLink handshake — autopilot-component lock (PX4 robustness).** The handshake locked onto the *first*
  non-GCS HEARTBEAT, which on multi-component systems (PX4 with a gimbal/companion reporting
  `autopilot=INVALID` from a non-autopilot component) could mis-identify the FC and skip the PX4 path. It
  now waits for an actual autopilot heartbeat (`autopilot≠INVALID` or component 1), logging what it sees.
- **RC Control — PX4 over MAVLink (`MANUAL_CONTROL`, implemented — UNTESTED).** PX4's native joystick path
  (#69) instead of channel override: a dedicated **manual mapping** model — assign a HID axis to each of
  roll/pitch/throttle/yaw (→ y/x/z/r, normalised −1000…1000, **z=0 = mid throttle**), up to **6 aux axes**,
  and HID buttons → **MANUAL_CONTROL button numbers 1–32** (the FC maps each to an action per vehicle, so
  there's nothing button-related to maintain GCS-side). Its own compact editor + monitor replace the
  channel grid when the platform is PX4; the `rcManual` store evaluates the setpoint live. Engage gate,
  deadman, rate and the armed-disengage confirmation are reused; modes/arm stay on the control panel
  (`DO_SET_MODE`). A `COM_RC_IN_MODE` reminder is shown (PX4 ignores manual control unless it allows a
  MAVLink source). **No PX4 SITL/hardware was available to test this — it compiles + type-checks but needs
  validation before it's considered shipped.** See `docs/active/MAVLINK_RC_CONTROL.md` §5.
- **RC Control — ArduPilot over MAVLink (SITL-verified).** The same HID/mapping/profile UI now steers
  ArduPilot via `RC_CHANNELS_OVERRIDE` (#70) — a firmware-gated copy of the MSP pipeline. A **platform
  dropdown** behind "Device" (INAV · ArduPilot · PX4) drives the layout/adapter; it's derived from the FC
  and **locked** on connect, and the offline choice is persisted for config without an FC. ArduPilot uses
  **Primary CH1–8 / Secondary CH9–16** (one override frame; the channel-group labels switch to
  Primary/Secondary accordingly). Engage **seeds** from the FC's `RC_CHANNELS` broadcast (no extra
  request) so there's no jump; the MAVLink handler streams the override at the selected **10/15/20/25 Hz**
  with an **adaptive read timeout + catch-up pacing** (the read-driven loop otherwise ran the rate ~⅓
  low). On disengage we send **no forced release** — the FC holds the last override for `RC_OVERRIDE_TIME`
  (~3 s) as a re-engage grace window instead of failsafing instantly (no physical RX = GCS is the sole RC
  source). A new **armed-disengage confirmation** (protocol-agnostic) warns before handing control back
  while armed. The RC tab is hidden on passive-telemetry links (no uplink). _Deferred:_ per-platform
  profile auto-load, `SYSID_MYGCS` mismatch warning, PX4 `MANUAL_CONTROL`. See
  `docs/active/MAVLINK_RC_CONTROL.md`.
- **RC Control — live send pipeline (Phase 4).** Kite can now actually steer INAV over MSP. Channel split
  is **CH1–16 RAW_RC / CH17–32 AUX_RC** (keeping the mode switches incl. `MSP RC OVERRIDE` on the
  releasable RAW band). Three stages: **monitoring** (not engaged — preview only), **AUX-only** (engaged:
  AUX_RC streams independent of override, so the GCS can flip the override switch itself and drive AUX
  modes) and **full control** (engaged + override active: RAW_RC takes over the sticks). Engaging is a
  manual long-press toggle (default off, never auto on connect). On connect we read the FC channels once
  (`MSP_RC`) and **seed** every non-passthrough channel so the takeover never jumps. Streaming runs at
  highest scheduler priority: RAW_RC fire-and-forget (no-reply flag, zero downlink) at a selectable
  **10/15/20/25 Hz** (default 10), trimmed to the highest controlled channel; AUX_RC on-change only,
  sending the minimal channel run and re-sending until the FC ACK (works on an uplink-only link). A
  **deadman** (frontend heartbeat → scheduler) stops the stream on any chain break, and a 2 s **link-speed
  probe** after takeover warns if the link can't sustain the chosen RC rate.
- **RC Control — safety locks + FC validation (Phase 3b).** Reads the FC mode ranges (`MSP_MODE_RANGES`)
  and shows, under each channel, which flight-mode box it triggers (alarm-coloured only on AUX, since
  only AUX_RC channels latch on link loss). A safety evaluation over the AUX channels you control:
  ARM / RTH / FAILSAFE → **blocks** RC output (a latched critical switch couldn't be cleared on GCS
  loss); CRUISE / WP / POSHOLD / ALTHOLD → **warns** (overridable only by MANUAL/RTH), escalating to a
  block when no MANUAL mode is configured. Receiver-type hints (MSP vs SERIAL/NONE) and an
  override-bitmask check with a one-click **"set override bitmask"** for your RAW_RC channels — applied
  at runtime only (no EEPROM save, reverts on reboot). Also: Button-Toggle now toggles on release with a
  0.5 s abort (a long hold doesn't toggle, freeing the long-press for a second function on the button).
- **RC Control — MSP message builder, FC reads + RAW/AUX split (Phase 3a).** Groundwork for streaming
  RC to INAV over MSP: tested byte encoders (`msp/rc_encode.rs`) for `MSP_SET_RAW_RC` (trimmed u16-LE)
  and `MSP2_INAV_SET_AUX_RC` (2/4/16-bit packed, `0`=skip), an FC-config read (`rc_read_fc_config`:
  `receiver_type` + `msp_override_channels`) surfaced in a new dev **MSP-RC** debug-monitor tab, and the
  transport split — CH1–12 (MSP-RC) vs CH13–32 (MSP-AUX) on INAV 9.1+, a single ≤16-channel block on
  8.0–9.0 — which now groups both the channel config editor and the live monitor. Also: an optional
  **hold-to-toggle** (0.5–2 s) on the Button-Toggle method (anti-accidental, e.g. for an arming switch)
  and the Button-Step range (originally capped at 15 for a 4-bit AUX channel; later raised to 25 once AUX
  moved to 16-bit). No live streaming yet.
- **RC Control — channel mapping (Phase 2b, local).** A channel-centric mapping editor: each RC channel
  (1..32) is driven by one of **8 input methods** — passthrough, analog-adjust (axis sets rate),
  dual-source (two triggers add/subtract), button hold, button toggle (2–6 positions), button step
  (discrete 3–25), button adjust (constant-rate) and **button set** (up to 6 buttons, each press latches
  the channel to its own fixed µs value). Inputs are referenced as **A/B/H** labels (axes /
  buttons / hat directions) and assigned via dropdown or **Learn** (binds the input you move/press the
  most). Per-channel **name** (shown in the live channel monitor), invert, deadband and rate as
  applicable. Everything is normalised −1..+1 internally with a live µs preview, computed by a pure,
  reusable helper layer (`helpers/rcMethods.ts`) + engine (`stores/rcEngine.ts`) and saved into the
  active profile. No MSP yet (streaming is a later phase).
- **RC Control — profile system + config layout (Phase 2a, local).** Shareable RC config **profiles**
  stored as files under `Documents/KiteGC/HID-Profiles/<name>.json` (not localStorage), with a profile
  dropdown + **Save** (overwrite, confirm) / **New** (name prompt) / **Delete** (confirm; keeps the
  working config loaded). Profiles are never auto-linked to a device or FC — the user manages the
  matching FC settings. The raw-input monitor moved to a collapsible section on the panel's
  configuration side (default collapsed; it's only a wiring check); the channel mapping comes next.
  Backend `hid/profiles.rs`, store `stores/rcProfiles.ts`. Also adds a small shared ±0.05 (2.5%) scaled
  centre deadband to every raw axis (both backends) so a controller's resting offset can't leak a
  stray command.
- **RC Control — HID/joystick input foundation (Phase 1, local).** Groundwork for GCS RC steering of
  INAV over MSP (`docs/archive/MSP_RC_CONTROL.md`). A new opt-in **RC** nav-rail panel (Settings → Data →
  "RC Control") with a live device picker + calibration view (bipolar axis bars, hat indicators,
  numbered buttons) — works offline, no FC needed. Input is read by a dedicated backend thread using
  **native per-OS raw backends** (not a gamepad library, which misclassifies HOTAS/RC-transmitter axes
  as buttons): **Windows.Gaming.Input `RawGameController`** on Windows, **evdev** on Linux. Verified on
  Windows (VelocityOne Flightstick — all axes/buttons/hats incl. a rotary trim axis). No channel mapping
  or MSP yet (next phases). Backend `src-tauri/src/hid/`, store `stores/hid.ts`, panel `RcControlPanel`.
- **Toolbar arming + battery indicators.** Flanking the sensor-health bar: a prominent **arming traffic
  light** (green = ready & disarmed, amber = armed, red = not ready) and a **battery indicator** (glyph +
  %). INAV lists the blocking `ARMING_DISABLED_*` reasons on hover (read straight from the armingFlags
  bitfield — no extra query); ArduPilot/PX4 derive "not ready" from the SYS_STATUS PREARM_CHECK bit and
  show the latest "PreArm: …" reasons (tracked independently of the toast filter). Battery uses the
  FC-native percentage (INAV capacity / MAVLink remaining) when present, else a per-cell voltage estimate;
  voltage / cell count / mAh on hover.
- **Map & terrain cache controls.** The tile-cache size selector gains **2 GB / 5 GB** tiers (default
  stays 200 MB), and a new **Terrain Cache** row shows the on-disk Copernicus DEM size + tile count with a
  **Clear** button (the terrain cache is uncapped, so it's size-only — no limit selector).
- **Vehicle control — GCS command & guided steering (V1, MAVLink).** A new `control` nav-rail panel
  (shown only while connected via MAVLink) to command an ArduPilot/PX4 vehicle directly from the GCS,
  without a transmitter — done better than Mission Planner's flat button grid. **SITL-verified on
  ArduPilot Copter/Plane/QuadPlane; PX4 path is firmware-aware but untested on real hardware.** Features:
  curated **mode switching** (firmware/vehicle mode tables; stick-flown modes hidden behind a reveal and
  **hard-locked unless an RC link is present**), **Arm** (slide-to-confirm) / **Disarm** + all other
  actions via a reusable **`HoldToConfirm`** (1 s) gesture, **Takeoff** (alt), **Land**, **RTL**,
  **Abort Landing** (go-around), a **Guided toggle** + map-click **"Fly Here"** popup
  (`DO_REPOSITION`, vehicle-aware fields), active-flight adjustments (**Change Alt** — Guided-only,
  **Change Speed**, **Set Loiter Radius**, **Set Home**, **Set Heading** — Copter-only), **Mission**
  start/restart/set-current, and **VTOL transition** for QuadPlane. Async `COMMAND_ACK` feedback per
  action. Firmware divergence handled throughout (Mission Start, Land, loiter-radius param, transition,
  PX4 packed custom_mode). Robust **QuadPlane detection** (re-probes `Q_ENABLE`). Backend `control.rs`
  (COMMAND_LONG/INT + ACK correlation, PARAM_SET) + `controllers/vehicleControl.ts`. INAV guided + HID/
  joystick RC + advanced outputs (servo/gripper/gimbal) are deferred. See `docs/active/VEHICLE_CONTROL.md`.
- **Map auto-framing: frame mission on load + go-to-UAV on connect.** Loading a mission onto the map
  (file / FC download / standalone library / INAV multi-mission switch) now frames the whole mission in
  the viewport, in 2D **and** 3D — free pan/look only, never over a replay (a replay-linked mission keeps
  the track centred). Connecting a UAV jumps once to the craft at a sensible zoom (2D ~16 / 3D ~600 m),
  deferred to the first 3D fix. Home/launch is included when applicable; via a small `mapCamera` signal bus.
- **Per-waypoint hover tooltip lists every parameter (INAV + ArduPilot/PX4).** In view mode, hovering a
  waypoint now shows its full parameter set — the same data as the panel's detail footer, both fed by one
  shared `missionWpDetails` helper so they can't drift (they used to: the footer listed all params while
  the tooltip showed only name + altitude).
- **ArduPilot/PX4 edit-mode reference labels.** Each location waypoint now carries a permanent black
  reference label in edit mode (altitude + frame, plus the command's key params), mirroring the INAV
  edit labels.
- **Stick / gimbal overlay (replay).** Two animated Mode-2 transmitter gimbals beside the replay player,
  à la Blackbox Explorer. Replay-only — built from the log-imported RC columns (live RC is only ~1 Hz, not
  worth the bandwidth). **INAV blackbox** shows `rcCommand` as the blue primary **and**, when the log has
  it, raw `rcData` as a dimmed orange dot behind — so you can actually see the FC's expo / self-level / nav
  override of the raw stick. **ArduPilot `.bin`** shows RCIN (blue). Glass panels flush with the player bar
  (measured height). Reusable `GimbalStick` component + pure `stickInput` adapter (per-firmware channel
  order: INAV `[R,P,Y,T]` vs ArduPilot AETR; µs vs centred-rcCommand scaling).
- **PX4 mission planning.** The PX4 path on the shared MAVLink mission editor is now complete. PX4 speaks
  the same mission protocol and MAV_CMD ids as ArduPilot, so the planner reuses the whole pipeline — only
  the **command catalog** differs: PX4 gets its verified supported subset (no `JUMP_TAG`, no extra loiter
  variants, no relay/parachute/fence/condition commands), resolved firmware-aware. Unlike ArduPilot, PX4
  has **one mission interpreter for all airframes**, so there is **no vehicle-type selector** — the full
  PX4 catalog (including VTOL commands) is always offered and a soft-warning flags a VTOL command on a
  non-VTOL airframe (or any PX4-unsupported command) once connected. The PX4 airframe class is read
  straight from `MAV_TYPE` (PX4 reports it accurately). Critically, the **home-slot convention is now
  firmware-aware**: ArduPilot reserves mission item 0 for home, PX4 does not — upload/download no longer
  inject/drop a home placeholder for PX4 (which would otherwise corrupt the mission).
- **ArduPilot/PX4 takeoff waypoint is positionable when planning offline.** The takeoff marker used to
  be pinned to the mission centroid (no real coordinates) and recomputed on every edit, so it often sat
  in the way and could not be moved. Offline (no UAV connected) it is now **freely draggable** and keeps
  its position — stored coordinates win, the editor exposes its lat/lon fields, and converting an existing
  waypoint to Takeoff keeps its location instead of snapping to centre. With a UAV connected it still
  anchors on the FC home and is locked (the real takeoff point). The position is also the correct target
  for PX4 / ArduPlane takeoff. 2D and 3D stay in sync.
- **Flight recording: live RC-link metrics + QuadPlane vehicle type (DB schema v14).** Recorded flights
  now capture the full RC link for replay — `link_quality` (LQ) is populated live (was Blackbox-only) and
  two new `telemetry_records` columns, `link_snr` (dB) and `link_rssi_dbm`, store SNR + raw uplink RSSI
  from the unified link-stats pipeline (CRSF `0x14`, INAV 9.1 `MSP2_INAV_GET_LINK_STATS`, SmartPort LQ).
  The RC Link widget therefore shows the real values on replay, not just Blackbox LQ. A detected ArduPilot
  **QuadPlane** (`Q_ENABLE`) is now recorded as platform type **VTOL** so the flight-detail vehicle type
  is correct (it reports `MAV_TYPE_FIXED_WING` on the wire). Additive, idempotent migration.
- **ArduPilot mission: vehicle-class selector + catalog-driven waypoint representation (ADR-046).** The
  mission firmware picker is now a 3-way `SegmentedToggle` (INAV / ArduPilot / PX4); the ArduPilot toolbar
  adds a **vehicle-type dropdown** (Plane / Copter / QuadPlane / Rover / Boat / Sub) — both locked while
  connected, the class persisted for offline planning, so Copter/QuadPlane-specific commands are now
  reachable. **QuadPlane is auto-detected** from the `Q_ENABLE` parameter (a QuadPlane reports
  `MAV_TYPE_FIXED_WING`, so MAV_TYPE alone can't tell — ArduPilot issue #7137); the MAV_TYPE VTOL range is
  kept as a secondary signal (PX4 VTOL / manually-set MAV_TYPE). Waypoint **icons are now catalog-driven**
  and cover the whole command set: VTOL takeoff/land with a "V" badge, spline "S" / payload "P", ROI eye,
  Set-Home house (no more generic "?"). **Soft-warnings** flag mission items whose command isn't valid for
  the active vehicle class (⚠ in the list + a footer count, never blocking).
- **Jump / Jump-Tag map representation (ArduPilot + INAV).** The jump connector line now covers both
  `DO_JUMP` (→ item number) and `DO_JUMP_TAG` (→ the matching Jump Tag), with a readable **repeat-count
  badge (↺N)** pinned to the source waypoint instead of only a hover tooltip. A `JUMP_TAG` added while
  editing a waypoint is inserted **before** it and groups with that waypoint (its jump target), matching
  how the FC resumes. INAV gets the same ↺N badge plus a **User-Action indicator** — a 4-slot dot row
  (UA1–4 from `p3` bits 1–4, active slots bright) on the waypoint teardrop.
- **PX4 flight-mode decoding (MAVLink).** PX4 packs its flight mode (`main_mode` + `sub_mode`) into the
  HEARTBEAT `custom_mode` completely differently from ArduPilot's flat per-vehicle table, so PX4 modes
  were previously misclassified (the ArduPilot table was applied to all MAVLink variants). A new
  `classify_px4()` plus a `classify_mavlink()` dispatcher now route PX4 vs ArduPilot by `fc_variant`, for
  both the live link and tlog import; modes use the QGC names (Altitude, Position, Hold, Return, Offboard,
  VTOL Takeoff, …).
- **RC Link widget — protocol-agnostic link-quality readout.** A new adaptive HUD widget shows whatever
  the active link reports and hides the rest: LQ big when available (else RSSI %), with RSSI (dBm
  preferred) and SNR as secondary lines, colour-coded by quality. Backed by a unified `LinkStatsData`
  (`rssi_percent` normalized at the source, plus optional `rssi_dbm` / `lq` / `snr_db`) on a new
  `telemetry-linkstats` event, so every protocol fills only what it can. Sources: **CRSF** LinkStatistics
  `0x14` (uplink RSSI dBm + LQ + SNR), **SmartPort** (RSSI `0xF101` + LQ from `0xF010`/"VFR", decoded as
  `100 − loss`; the FC's unconfigured-channel RSSI-0 is ignored to stop a 0/100 flicker), **MAVLink**
  `RC_CHANNELS` RSSI, **LTM** S-frame RSSI, and **INAV** `MSPV2_INAV_ANALOG` RSSI plus — on **INAV 9.1+** —
  real dBm/LQ/SNR from `MSP2_INAV_GET_LINK_STATS` (`0x2103`, own 1 Hz poll, gated by `Feature::LinkStats`).
  Replay maps the recorded `link_quality` / `rssi`.
- **Telemetry Relay — forwarding & conversion (`TELEMETRY_FORWARDING.md`, ADR-051).** Re-encodes the live
  inbound telemetry (MSP / MAVLink / passive) into a chosen wire protocol and sends it out a second link —
  for antenna trackers, monitoring apps or other GCS. Protocols: **LTM / MAVLink / CRSF / SmartPort**
  (each the inverse of the matching passive decoder); outputs: **Serial / BLE / TCP-server / UDP**. UI is a
  dropdown under the connection bar (`⇅ Relay`): a basic LINK diagnostic (RX/TX B/s, msgs/s, protocol) plus
  N relay rows (protocol · combined Serial+BLE device / TCP / UDP · enable toggle · live status). Configs
  persist and **auto-connect** with the primary link (push telemetry needs no handshake); emission is paced
  on the attitude update so the output rate follows the real data rate. TCP listen ports / UDP targets are
  kept unique (auto-bump). Verified live: TCP + LTM against mwptools from another PC.
- **Connection status box (left of the Disconnect button).** Shows the active protocol — primary
  (MSP / MAVLink / SmartPort / CRSF / LTM) plus an optional secondary tunneled inside it (ArduPilot
  passthrough → MAVLink) — and a data-flow dot: green while valid data arrives, red after >5 s without,
  grey until the first data. For passive telemetry the staleness tracks **fresh FC-origin frames** (not
  the receiver/TX housekeeping such as RSSI that keeps flowing after an FC-link loss), so it goes red on a
  real link loss **without disconnecting**; MSP/MAVLink use the telemetry heartbeat directly.
- **Flight-path marker (velocity vector) on the AHI widget + 3D FPV HUD.** Shows where the aircraft is
  actually going vs. where the nose points — derived purely from existing telemetry (no wind estimate):
  lateral = COG − heading (crab), vertical = flight-path angle `atan2(vario, ground speed)` vs. pitch.
  Body-frame / pilot's view (upright; the horizon turns around it). Yellow on the AHI, green & conformal
  (FOV-scaled) on the FPV HUD. Live + replay; hidden below 1.5 m/s and eased per update. Shared geometry
  in `utils/flightPath.ts`.
- **Heading / course / crab direction cues (`WIND_CRAB_INDICATOR.md`, archived).** The compass widget now
  shows a **course-over-ground track bug** + amber COG readout alongside the heading (the gap = crab), and
  the 2D map draws **HDG / COG nose lines** plus a **predicted turn-radius arc** at the aircraft (velocity-
  vector length = 15 s of travel, the arc capped at a 180° sweep). All run through the position smoother
  (incl. an eased turn rate) so they track stably; the turn rate is computed on the source time base so
  replay is playback-speed-independent. New **Direction indicators** toggle (Settings → Data → Telemetry,
  default on). The wind-arrow / flight-path-marker idea is parked pending INAV's `MSP2_INAV_WIND`.
- **RF link / radio-shadow analysis in the Terrain Analyzer (Feature 4 / `RF_LINK_ANALYSIS.md`).**
  For a mission or a flown track, the profile chart now shows **where terrain degrades or blocks the
  radio link** from the launch point as a background green→red "rainbow" loss field. Three toggleable
  methods — **LOS occlusion**, **Fresnel/knife-edge diffraction**, and **two-ray ground reflection**
  (the lobing pattern that shows up on long-range RSSI plots) — combined per sample, with a per-band
  selector (**5.8 / 2.4 / 0.9 / 0.433 GHz**). Geometric blockage is fundamental (red under any method;
  two-ray only adds where there's clear line-of-sight). A **clutter/vegetation offset** (default 10 m)
  accounts for forest/buildings the bare DEM misses. Adds a **line-of-sight clearance line** (AGL view)
  and, in Track mode, overlays the **logged RSSI** so predicted vs. measured can be compared directly.
  Radial 1°-from-home sampling keeps it cheap. It is an honest *risk indicator*, not an exact RSSI
  predictor (near-field, antenna pattern/attitude and per-pixel canopy are out of scope). The RSSI
  overlay uses a **robust, fixed scale** (2nd–98th percentile so dropout spikes don't squash it) with
  **auto-detected unit** (dBm / percent / raw) and its own **show/hide toggle** (disabled in Waypoint
  mode or without RSSI). Tuned for smooth pan/zoom on long flights: the loss field renders as 5 px
  colour bands (not one gradient stop per pixel) and the clearance recompute is decoupled from the
  zoom window.
- **ArduPilot/PX4 mission library — full parity with INAV (ADR-050).** ArduPilot missions can now be
  **saved to the library, deduplicated, retrieved, previewed and linked to flights** exactly like INAV
  missions — the ArduPilot editor gained a "Save to library" button and the "Mission Manager", and the
  Manager itself is now format-aware: each mission shows a format tag (INAV / ArduPilot / PX4), a
  **format filter** appears next to Import once two or more formats are present (listing only the formats
  actually in the library), the preview renders ArduPilot missions, and `.waypoints` files can be imported
  into the library. Loading a mission of the other format auto-switches the editor (keep-in-memory or
  discard, since INAV and ArduPilot keep separate missions) and is blocked while a different FC is
  connected. Live/imported ArduPilot flights link their mission the same way INAV flights do. Built on the
  already format-agnostic mission DB — **no backend change** — and structured so **PX4 rides the same path**
  (it appears automatically once a PX4 mission is saved; no further code needed).
- **Import recorded raw logs into the logbook (ADR-049).** A new "Import Raw Log" action parses a
  recorded `.rawmsp` (MSP) or `.tlog` (MAVLink) file into the logbook. A continuous log (one file across
  many arm/disarm cycles) is **split into individual flights** at arm/disarm — applying the same 5 s
  re-arm grace as live recording, but **filling** the short grace gap (the raw log has that data, unlike
  a live recording). Parsed flights are stored as normal **live** flights (with a duplicate check, so
  re-importing one that's already recorded is skipped). Lets you recover flights from raw logs taken with
  the DB disabled, or from logs shared by others.
- **Raw MSP serial logging, mwptools-compatible (ADR-049).** The "Save Raw Data Logs" option now records
  the actual MSP serial traffic — both directions, with timing — to a `.rawmsp` file, using **mwptools'
  own raw "v2" format**. That means the logs replay directly in mwp's tools ("Replay mwp RAW log" /
  `mwp-log-replay`) and any existing MSP decoder. It's the MSP counterpart to MAVLink's `.tlog`: each
  protocol now logs in its community's de-facto raw format. (Replaces the previous decoded telemetry-CSV
  "raw" log — a lossless raw capture is more useful and a human-readable decode can be derived from it
  later.)
- **Flight times shown in the flight's local time, not UTC (ADR-048).** Log titles, the logbook list and
  the replay clock now read the wall-clock time **of the place the flight happened** — so a single-area
  pilot sees the correct local time, and a cross-timezone operator no longer sees flights apparently at
  3 a.m. Under the hood every flight now stores a true-UTC instant **plus** the location's UTC offset:
  live flights take the ground-station PC's own offset (it sits at the field); imports resolve the offset
  from the start coordinates (timezone-from-coordinates lookup + DST). This also fixed INAV Blackbox
  imports, whose stored time wasn't true UTC — their 3D real-lighting sun was previously off by the
  offset. Flights with no known offset (older rows / no GPS) keep showing **UTC** with a small marker.
- **ArduPilot missions render in 3D — one shared renderer with INAV (ADR-047).** The 3D map drew only
  INAV missions; ArduPilot now renders there too, through a single model-driven renderer: per-platform
  adapters resolve each mission model (INAV `WpAction`/`alt_mode`, ArduPilot `MavCmd`/altitude-frame +
  takeoff anchoring) into one protocol-neutral 3D model, then one draw path renders both identically —
  markers, flight-path lines, jump/RTH connectors, the active-WP glow and ground drop-lines, sharing the
  same geoid/terrain compensation and the ADR-045 icon specs.
- **FC system messages on screen (MAVLink STATUSTEXT).** ArduPilot's status messages (mode changes,
  prearm failures, errors, …) appear in a single compact **banner** at the top edge — one line per
  message, newest at the bottom, the field scrolling to the latest (≈5 lines visible), colour-coded by
  severity (info blue / warning amber / error red) with a matching audio cue (gentle for info,
  discreetly alarming for warnings/errors). The banner fades out 60 s after the last message; repeated
  identical lines are de-duplicated and info cues are rate-limited so a boot-time flood doesn't
  machine-gun the speaker. A **System Messages** setting (DATA tab → Alerts: Off / Errors / Warnings /
  All) controls verbosity by `MAV_SEVERITY` — protocol-neutral, so it can gate INAV messages later too.
- **Real per-sensor health on the header bar, MAVLink included.** ArduPilot now feeds the sensor bar
  from the standard `SYS_STATUS` present/health bitmasks (previously a hard-coded stub that left
  mag/baro dark), mapped to the same model INAV uses. The bar is **adaptive** — it shows only the
  sensors actually present, so it adapts per airframe — and gained **rangefinder + pitot** tiles (data
  INAV already provided but never displayed). GPS keeps its amber "fix below 3D" nuance.
- **EKF estimator indicator (ArduPilot).** A new header tile shows the active core (**EKF2 / EKF3**,
  read once from the `AHRS_EKF_TYPE` parameter on connect) and its health as a green/amber/red light
  from `EKF_STATUS_REPORT` — green when all variances are nominal, amber when degraded, red on a GPS
  glitch / unhealthy filter. Hidden for INAV (which doesn't report EKF status).
- **ArduPilot active-waypoint tracking.** The Flight Mode widget shows `WP N/X` during an ArduPilot Auto
  mission (current item from `MISSION_CURRENT`, total from the loaded mission), and the active waypoint
  on the 2D map gets the same pulsing glow INAV already has — via the shared marker helper, so the two
  planners look identical.
- **Full ArduPilot mission editor — catalog-driven, ~40 commands (ADR-046).** The ArduPilot mission
  planner went from a flat ~11-command editor to a declarative command catalog (`arduCommandCatalog.ts`,
  modelled on QGroundControl): a **categorized, vehicle-filtered** command picker, a generic param editor
  (number field / enum dropdown) that shows **only the params each command uses**, per-param **(ⓘ)
  tooltips**, and an **Advanced** expander for rare fields. Non-location commands (DO_/CONDITION_) appear
  as **numbered, indented modifiers** under their waypoint — the INAV editing model, over ArduPilot's flat
  sequence. Each command shows a friendly name + its canonical `MAV_CMD` name (so Mission Planner / QGC
  users recognise it). Commands carrying data in the coordinate fields (e.g. `DO_DIGICAM_CONTROL`) are
  modelled honestly (params 1..7) instead of showing a bogus map position. The curated **modern** set is
  offered in the picker; legacy commands still round-trip (download / `.waypoints`) rendered raw; the full
  per-command rationale is in `docs/active/ARDUPILOT_COMMAND_COVERAGE.md`.

### Changed
- **Passive "Telemetry" connection mode is now available in release builds.** The listen-only,
  auto-detect protocol was gated to dev builds; the pipeline is complete, so it now ships.
- **3D mission overlay re-renders much faster.** The terrain-altitude resolution is cached by an input
  signature, so re-opening 3D (or a redundant render trigger) no longer re-samples terrain — fixing a
  15–20 s stall on repeated switches; per-waypoint elevation lookups are also batched into one IPC call.
- **Full-precision heading / course / attitude (decimal degrees).** `yaw` (FC fused heading) and `heading`
  (course over ground) are now carried and stored as `f64` (0.1° resolution) instead of rounded whole
  degrees — across MSP / MAVLink / FrSky live, Blackbox / ArduPilot / raw import, the DB and replay.
  Widgets still round for display (e.g. the compass), while the map direction lines, FPV HUD and
  flight-path marker use the full precision. No schema migration (SQLite stores the values as `REAL`
  via column affinity); **re-import existing INAV logs** to pick up the precise heading.
- **One "Import" button in the logbook instead of three.** The separate Blackbox / Raw Log / .kflight
  import buttons are now a single **Import** action: the file picker accepts every supported log type
  (INAV Blackbox `.txt`/`.bbl`/`.bfl`, ArduPilot `.bin`, raw `.rawmsp`/`.tlog`, `.kflight`) and the right
  importer is chosen per file by extension. Drag-and-drop uses the same dispatch (now incl. raw logs).
  Keeps the toolbar clean as more formats are added (e.g. radio CSV later).
- **3D real-time lighting clock stays in true UTC (ADR-048).** The sun position is physics (absolute
  instant + globe location), so it's automatically correct for the flight location — the new local-time
  display is a presentation layer only and does not shift the lighting clock.
- **Mission WP-editor popups are one shared framework now (ADR-046).** The INAV and ArduPilot map layers
  had separate copies of the popup-building + event-wiring code; the lifecycle + HTML/event primitives now
  live in one `missionEditorPopup` module. Its lifecycle carries a **content-signature redraw guard** —
  the popup DOM is rewritten only when the content actually changes, so live telemetry/home redraws no
  longer close an open dropdown mid-edit (fixed for both planners).
- **Mission waypoint markers now share one icon schema across INAV + ArduPilot (ADR-045).** The two
  planners used hand-copied, independently-drifting icon sets (e.g. loiter had become cyan on ArduPilot
  but orange on INAV). Shapes (teardrop, loiter ring, house, …) and a canonical colour palette now live
  in one `missionIconPrimitives` module that both mappers consume; each keeps only its own
  type→primitive mapping + platform-specific markers (ArduPilot takeoff, INAV fly-by-home). A shared
  marker changes in one place. ArduPilot loiter is back to the INAV orange + shows the hold value
  (∞/seconds/turns) and ROI is back to the INAV purple. The shared spec also carries the 2D/3D billboard
  format, so ArduPilot mission markers can render in 3D (previously INAV-only).
- **Flight mode is now protocol-agnostic end-to-end (ADR-044).** Mode classification moved into the
  protocol adapters: each emits a canonical `FlightModeState { primary, modifiers[] }` (string ids), and
  a single frontend registry maps id → label + **category** (category drives the colour, the label stays
  exact). The widget, 2D/3D track-coloring and replay no longer branch on the firmware — INAV keeps its
  rich **primary + modifier** stacking (Horizon+Alt, Acro+Tune), ArduPilot shows its real per-vehicle
  modes, and a future protocol (CRSF/Smartport/Betaflight) is just an adapter + a few registry ids.
  Recorded flights store the canonical mode (`telemetry_records` gains `mode_primary` / `mode_modifiers`,
  DB v12); replay reads it directly. Old (pre-v12) rows render as the neutral "other" category.
- **First-start defaults now show off more out of the box** (fresh installs only — existing settings
  are untouched): flight **recording + logbook on**, map provider **ESRI Hybrid**, **night mode Auto**,
  3D **real lighting + log-replay sun time on**, **nav panel open**, **GCS marker continuous**, the
  **Radar and Airspace panels enabled**, the right widget dock = **Flight Mode + Battery** (Battery
  moved out of the bottom dock, Raw Telemetry off by default). ADS-B built-ins are now **adsb.lol +
  adsb.fi** (adsb.one demoted to an off-by-default custom provider — currently unreachable).
- **UAV Info features** are laid out in a fixed **2-column** grid so the panel no longer widens with
  the feature count.
- **MAVLink stream set audited (ADR-043).** Requesting `EKF_STATUS_REPORT` for the new EKF tile
  prompted a pass over what we ask for vs. what the handler consumes: `NAV_CONTROLLER_OUTPUT` was
  requested but never read → dropped; `RC_CHANNELS` was disabled in ballast while the handler read RSSI
  from it (so MAVLink RSSI was always 0) → enabled at 1 Hz; `GPS_RAW_INT` dropped from position-rate to
  1 Hz (it only contributes fix/sats/HDOP now — position comes from the fused `GLOBAL_POSITION_INT`);
  and `VFR_HUD` is now gated on the airspeed module (its only consumer). Net less bandwidth, yet RSSI
  and EKF now work. Also: `AHRS3` (182) is no longer touched — obsolete in modern ArduPilot, it logged
  a "No ap_message" warning on connect.
- **Flight-mode track colour — PosHold/Loiter is more distinct.** The `poshold` category (INAV PosHold,
  ArduPilot Loiter) moved from cyan `#00bcd4` to turquoise `#1abc9c` so it no longer reads almost
  identical to the `mission` blue (ArduPilot Auto) on the track/badge.

### Fixed
- **Debug Monitor — frozen actual-rate for fire-and-forget sends + message-name/layout polish.** The
  per-code "Actual Hz" only counted responses (and rolled the window over inside `on_response`), so a
  no-reply send like `MSP_SET_RAW_RC` showed a stale frozen rate; it now measures `max(request,response)`
  over a time-based window, so the RC send cadence reads correctly and a quiet code decays to 0 instead of
  freezing. Added the missing human-readable names (MSP: MSP_RC, MSP_SET_RAW_RC, MSP_MODE_RANGES,
  MSP2_COMMON_(SET_)SETTING, MSP2_INAV_GET_LINK_STATS, MSP2_INAV_SET_AUX_RC; MAVLink: MANUAL_CONTROL,
  RC_CHANNELS_OVERRIDE, COMMAND_INT) so fewer rows show raw hex. The Name column now truncates with an
  ellipsis (full name on hover) instead of widening the table. New **Latency** column (request→response
  round-trip in ms per code) to inspect real MSP transaction times.
  zero-initialised reading (all axes 0.0 → −1.0) until a controller delivers its first report, so RC
  channels showed garbage extremes until something was moved. The HID backend now suppresses readings
  with a zero timestamp (waits for the first real one) and reuses controller objects across rescans;
  the panel shows "waiting for first input" until then. (Linux/evdev reads the kernel's cached state and
  was unaffected.)
- **Near-0,0 GPS glitch leaked into the map (ArduPilot).** The FC briefly reports a position right next to
  0°,0° before it has a real origin/fix (e.g. `N 0.00002 / W 0.00001`, 0 sats) — which slipped past the
  exact-0,0 check into the pre-arm track (a black line from null island) and the camera jump.
  `isValidGpsCoordinate` now rejects the whole ~111 m null-island neighbourhood (requiring BOTH components
  ~0, so real equator/prime-meridian flights are unaffected), fixing it for the camera + track in 2D & 3D.
- **Live link RX/TX rate stayed at 0 in release builds.** The Relay panel read its rate from the dev-only
  Debug Monitor events; a lightweight always-on link meter (`link-stats`, compiled in every build) now
  feeds it for both MSP and MAVLink.
- **Camera snapped to 0,0 before a solid GPS fix.** The go-to-UAV jump fired on `fixType ≥ 3`, which the
  FC can briefly report with near-0,0 coordinates; it now also requires ≥ 6 satellites (2D and 3D).
- **Home widget showed a bogus ~5800 km distance to 0,0.** Before a fix the launch auto-place falls back
  to the map centre (≈ 0,0), which was mirrored into Home with `set:true`; an invalid / 0,0 launch is no
  longer mirrored, so Home stays empty until a real reference (FC home on arm, a valid manual placement,
  or replay) exists.
- **ArduPilot 3D mission altitudes sank into the terrain when loaded offline.** The REL-altitude base used
  `homePosition.alt` whenever a home was set — but offline that is the stale `'manual'` home (the INAV
  launch mirror, ≈ sea level / wrong region), so REL waypoints anchored at sea level and dropped below
  ground. The base now trusts only the authoritative FC home (`source 'fc'`); otherwise it samples the
  terrain under the takeoff waypoint (a positioned takeoff is now ArduPilot's launch reference) or the WP
  centroid.
- **HOME widget showed a bogus distance with no connection/log.** The GPS-injection store fired once on
  app start with `active: false` and bumped `telemetry.lastUpdate`, making an idle store look "live" (lat/
  lon 0,0) → HOME computed a distance to it. The clear branch now only runs when an injection was actually
  active, so idle widgets stay blank until a real connection/replay.
- **Mission load left a stale launch point / framed the wrong area.** A previously-loaded mission's launch
  point stuck across loads (resurrected via the launch→home `'manual'` mirror), so loading a mission in a
  new region drew a long launch line to the old spot and zoomed the map out continent-wide. Each load now
  resets the launch to that mission's own home/first-WP, and the map fit ignores the INAV launch / non-FC
  home for the wrong system.
- **ArduPilot WP edit popup.** The Advanced section collapsed on every +/- value step (its open state was
  DOM-only and lost on the popup rebuild — now persisted per section), and long param labels (Acceptance /
  Pass Radius) wrapped (wider, no-wrap label column scoped to the ArduPilot popup).
- **Spurious scrollbar in the mission WP detail panel.** The per-waypoint parameter panel capped its
  height (`max-height: 180px; overflow-y: auto`), so an ArduPilot WP's full param set (Lat/Lon/Alt + Hold/
  Acceptance/Pass Radius/Yaw) tripped an inner scrollbar. Removed the cap — the PanelShell already pins the
  footer and scrolls the column only as a last resort, so all params show cleanly (INAV + ArduPilot/PX4).
- **North-crossing heading "spin" in INAV blackbox replay.** A per-value ">360 → ÷10" scaling heuristic
  left decidegree `yaw` values in the 0–36° band (`attitude[2]` ≤ 360 decidegrees) undivided, so the
  displayed heading swung wildly — the UAV model on the map and the compass appeared to spin over 1–3 s —
  whenever a turn passed through north, in either direction. The FC heading (`attitude[2]`, decidegrees)
  is now **always ÷10** like roll/pitch; course-over-ground (`gps_ground_course`, degrees) is unchanged.
  Re-import affected INAV logs.
- **Flight-path marker frozen on the AHI widget.** The marker's smoothing ran in a `requestAnimationFrame`
  loop that read a reactive value which went stale, pinning the marker at the dial edge regardless of the
  real crab. Reworked to ease in a reactive `$effect` (untracked self-read) so it tracks live again —
  consistent with the FPV HUD.
- **Heading vs. course-over-ground now consistent across all paths.** Unified the two channels (FC fused
  heading vs. GPS course) for live MSP, live MAVLink and Blackbox import, and de-conflated replay so the
  UAV model/icon — in **2D and 3D, live and replay** — points along the real heading (showing crab in
  wind) instead of riding the ground track "on rails". MAVLink now derives COG from the fused velocity
  rather than mis-using `GLOBAL_POSITION_INT.hdg` (which is the vehicle heading).
- **MAVLink GPS — satellites/fix no longer flash, HDOP shows.** `GLOBAL_POSITION_INT` was emitting a
  hard-coded `fix=2, sats=0` that fought `GPS_RAW_INT`'s real values (flashing 0↔N); fix type and sat
  count are now owned solely by `GPS_RAW_INT` (cached, reused by `GLOBAL_POSITION_INT`), and HDOP is
  emitted from `GPS_RAW_INT.eph` via the same stats event INAV uses (was always "–").
- **3D mission overlay no longer flickers.** The lines/markers rebuilt on every (sub-metre-jittering,
  ~0.2 Hz) HOME because the home handler re-broadcast `launchPoint` unconditionally; and the overlay
  lines z-fought the terrain. Fixed by deduping HOME, skipping identical redraws (quantised model
  signature), and rendering the overlay lines as depth-test-free primitives (ADR-047).
- **3D live track is coloured from the start.** A GPS frame arriving before the first flight-mode update
  baked a grey "unknown-mode" leading segment into the (immutable) live track; track points are now
  recorded only once the flight mode is known.

### Added
- **MAVLink telemetry stream rates — GCS-requested, with MSP parity (ADR-043).** ArduPilot is
  push-based: it streams per its `SRn_*` params as soon as it sees a GCS heartbeat, which floods slow
  RC links (ELRS/CRSF/mLRS) with messages no widget uses. Kite now requests an explicit rate set on
  connect via `SET_MESSAGE_INTERVAL`, driven by the **same two knobs as MSP** — Attitude rate +
  GPS Position rate — with everything else at 1 Hz and the high-rate "ballast" (RAW_IMU, pressures,
  IMU2/3, vibration, AHRS, …) disabled. Cuts the default ~2 KB/s firehose to ~500–650 B/s so it fits a
  real radio link, and the two knobs double as the bandwidth control. A new **Full MAVLink Telemetry**
  setting (default off) leaves streaming entirely to the FC's params (resetting any prior reduction to
  default) — for high-bandwidth links + capturing everything in the `.tlog`.
- **MAVLink Debug Monitor tab.** The in-app Debug Monitor gains a **MAVLink** tab alongside MSP/Alerts:
  every message ID seen in the session with its direction (RX/TX), count, measured rate and
  "last seen" age, plus aggregate MSG/s + throughput — the push-side counterpart to the MSP poll view.
- **MAVLink home from the FC (`HOME_POSITION`).** The map now shows the aircraft's stored home from
  `HOME_POSITION` (the authoritative locked green "H"), consistent across reconnects, instead of only
  guessing from the GPS fix at arm.
- **Full connection path is remembered.** Transport, host, TCP/UDP port and BLE device are now
  persisted alongside port/baud/protocol and restored on the next launch — no re-entering the whole
  connection each time.
- **Custom window titlebar — reclaims the native title-bar height (Linux especially).** The app now
  runs borderless (`decorations: false`) with its own titlebar: the window controls (minimize /
  maximize-restore / close) are folded into the existing toolbar, the toolbar doubles as the drag
  region (double-click maximizes), and thin edge/corner grips re-add window resizing (the GTK resize
  border is lost when decorations are off). On Linux/GNOME the tall native header bar is gone, so the
  toolbar *is* the titlebar — one bar instead of two. The window-state plugin is configured to persist
  everything **except** the decorations flag, so a previously-saved state can no longer restore the
  native bar over the config.
- **Live recording — crash-safe temp session store + complete capture (ADR-040).** A live flight is
  now recorded into a **separate per-session SQLite file** (`sessions/active_<ts>.ktmp`) instead of
  straight into the production database, so an app crash leaves a recoverable session file rather than
  a half-written, non-finalized flight. The recorder also now captures the fields it was dropping
  despite already polling them: **active waypoint + nav state** (so a live-recorded mission shows
  active-WP tracking on replay, matching live), **GPS HDOP/EPH/EPV**, and **packed sensor-health** — no
  schema change (the columns already existed).
- **Deferred commit + End-Flight dialog as the commit gate (ADR-041).** The temp session is committed
  into the main DB **only** on an explicit **Save** or when a new flight is armed after a **5 s grace**
  — nothing is written while the summary dialog is open. So **Discard Recording** simply drops the temp
  file, and an **accidental disarm re-armed within 5 s stays one continuous log** (INAV-style grace)
  instead of splitting into two flights. The End-Flight summary is now **modal** (a click next to it or
  Escape no longer closes it — it can't be dismissed by accident), gains a **Discard Recording** button
  (with confirmation) and drops the confusing **Skip**. The FC-synced flown mission is captured at
  disarm and linked when the flight is committed.
- **Live-recording recovery — orphan scan + continue-on-reconnect (ADR-042).** If the app crashed or
  was closed mid-flight, an unfinished recording is now detected on the next start and a prompt offers
  **Discard**, **Save Incomplete** (reconstruct + save the flight up to the last sample), or
  **Continue on Reconnect**. The latter waits for the next connection and — on the first polled status
  — **resumes the same log if the aircraft is still armed** (so a mid-air disconnect stitches back into
  one continuous flight, even if you reconnect over a **different protocol**), or finalizes it into the
  End-Flight dialog if disarmed. Clicking **Disconnect while the UAV is armed** now **asks first**
  (Stay Connected / Discard / Save Incomplete / Continue on Reconnect) instead of disconnecting
  immediately — handy when you only mean to change the COM port or switch to a telemetry link. A
  **USB unplug / BLE drop** is now detected (a fatal transport error, distinct from a response
  timeout) and shows the recovery prompt + cleans up the connection, so a reconnect works without
  having to manually disconnect first; a telemetry/OTA stall still just keeps polling (only time gaps
  in the log). _Save-trigger tuning (flush cadence) is a follow-up._
- **Home / Launch reference unified + recovered from the FC (ADR-039).** The orange draggable **"L"
  launch** point and the green **"H" home** marker are now one source-tagged reference per map
  (`homePosition.source` = `fc` | `manual` | `replay`): a connected FC home shows a **locked green "H"**
  pinned to home (the "L" hides); with no FC home the **draggable "L" is the home** and the Home widget
  follows it. Home is now **recovered on connect** via a one-shot `MSP_WP` #0 (INAV's RTH home) — so a
  mid-flight connect or an app restart no longer leaves Home unset — and it only re-homes on a **genuine
  disarmed→armed edge** (a reconnect while already armed keeps the marker put). The 3D "L" shows only
  during an active connection; on disconnect a locked home degrades to a manual reference (kept in place).
- **3D active-waypoint pulse.** The FC's current target waypoint now pulses with a green glow on the 3D
  globe, centred on the pin head — the same active-WP cue as the 2D map (driven by the same trusted
  `activeWpNumber`), rendered continuously only while a pulse is on screen.
- **About dialog + automatic build stamp.** An **About** button in the Settings panel header opens a
  dialog with the logo (placeholder), name, **version + short git commit + build date**, the GPL-3.0-or-later
  licence/copyright, a source-repo link, and a curated third-party-licence list. The commit id (with a
  `-dirty` marker when the tree has uncommitted changes) is injected at build time via Vite `define` — no
  manual version bump per build, so a tester's bug report can be tied to an exact commit.
- **Airspace Manager — aeronautical data overlay (P1, ADR-038).** A new nav-rail panel (under Radar) +
  backend subsystem over **OpenAIP**: **airspaces** (class-coloured polygons), **obstacles** (wind turbines,
  masts, towers — with a wind-turbine icon), **airports** (by type) and **RC/model airfields** on the 2D
  map. Enabled in Settings → Data (provider + your free OpenAIP API key). The panel (advanced two-column)
  has per-layer 2D/3D visibility toggles + a cache readout/clear on the left and a grouped, distance-sorted
  **nearby list** (Obstacles · Airspaces · Airfields, click to centre) on the right. **Zoom-density
  management** shows only big/important features when zoomed out and fills in detail (obstacles, RC fields,
  small airfields, minor airspaces) as you zoom in. _3D rendering + alerts are P2._
- **Airspace Manager — 3D + refinements (P2).** The globe now renders the aero layers, and the panel
  settings persist:
  - **Obstacle columns (3D).** Slim vertical columns on the **real terrain** (sampled per obstacle →
    geoid-independent, no float/sink), perspective-correct (not sprites). OpenAIP often omits the AGL
    height *and* mis-types turbines as generic obstacles, so we now read the **OSM tags** to detect wind
    turbines (proper icon + operator) and, where the height is missing, draw a **typed estimated** column
    (taller for turbines) rendered **visibly distinct** (translucent + yellow) so it never poses as
    surveyed data.
  - **Airspace volumes (3D).** Extruded floor→ceiling hulls for the airspaces **relevant to the UAV**
    (inside, or a boundary within range; country-sized FIR/UIR + upper-air/CTAs are skipped). The
    boundary section **facing the UAV** is given a pattern as an **approach reference** (perpendicular
    "zone" test + outward-sidedness so only the truly-facing wall lights up). Volumes are non-pickable
    (raw primitives) so they never block clicks or the camera.
  - **Airport markers (3D).** Each airport in the airfield range gets the **same type-coloured badge as
    2D** (disc + star, "H" for heliports) as a ground-clamped billboard + name label. _Runway geometry
    was evaluated but dropped — OpenAIP has no runway threshold coordinates, so a projected runway just
    cuts through the airport point (wrong for multi-runway fields)._
  - **2D click → all airspaces here.** Clicking the map lists **every** airspace stacked at the point
    (overlap-aware), dropping unclassified "free" airspace as clutter when a classified/critical one
    covers the same spot; a second click toggles the popup shut. FIR/UIR are no longer drawn.
  - **Typed 2D markers.** Subtle black obstacle silhouettes (white outline, no disc) and airport icons
    colour-coded by type (international = red, airport = blue, airfield = green, heliport = "H").
  - **Persistence + panel polish.** Per-layer 2D/3D visibility, the obstacle/airfield **render ranges**
    (1/2/5/10/15/25 km, used for both the 3D cull and the nearby list) and the **Compact** view now
    persist (in settings). The nearby list is capped to the nearest 10 per group within range; the
    confusing cache readout was removed.
- **ADS-B online query bounded to what the free-look camera looks at.** In the 3D free-look camera the
  online ADS-B fetch now centres on the screen-centre ground point, with that point's offset from the
  camera nadir (and the query radius) capped at 150 km — projected 150 km along the look direction when
  the view is above the horizon. The configured download radius is the floor (straight-down → exactly
  that). UAV-locked cameras (follow/orbit/fpv) and 2D keep the UAV/reference centre + configured radius.
  Cuts dense-area contact counts from ~600 to ~150 and stops loading aircraft 1000+ km away.
- **3D camera controls — touchpad-friendlier.** Tilt is now on **right-drag** (instead of the middle
  button), with the wheel/pinch for zoom and left-drag to rotate (middle + Ctrl+Left kept as extras);
  tilt no longer needs a middle mouse button. Zoom-out in the free camera is capped (~8000 km) so the
  view can't drift into the full-globe "space" regime where the controls change and widgets cover the
  globe. The follow/orbit chase cameras can now zoom out 3× further (up to 1500 m from the UAV).
- **Connection bar reworked onto the control framework.** Protocol pick is now a segmented toggle
  (MSP | MAVLink), Connect is the shared `<Button>` (blue → orange while connecting → red to
  disconnect), and all fields share one 28 px style so they align. The serial/BLE device dropdowns
  have a fixed width with ellipsis, so a long device name no longer stretches the bar across the
  window. The `SegmentedToggle` now content-sizes its segments (long labels like "MAVLink" are never
  truncated) and gained a `full` mode that fills a tab bar's width evenly (used by the Settings tabs).
- **Live connection discovery (ADR-037).** The connection bar now discovers devices on its own — no more
  manual refresh. **Serial:** ports are polled every second; a freshly plugged adapter is auto-selected and
  an unplugged one disappears (the last-used port is still restored on launch). **BLE:** selecting the BLE
  transport starts a continuous scan that fills the device list in real time (live RSSI), via a streamed
  backend scan session. Both pause while connecting/connected; the now-obsolete refresh buttons were removed.
  Radar / FormationFlight port pickers stay on manual refresh on purpose.
- **GCS location marker on the map (2D + 3D).** A satellite-dish marker shows where your ground station
  is — also used as the radar distance reference and the FormationFlight node position. A new *GCS marker*
  setting (**Off / Manual / Continuous**): *Manual* places it once via OS geolocation and lets you drag it
  or right-click **"Set GCS here"** (a **Reset** button snaps it back); *Continuous* follows the OS location
  live (ignoring sub-20 m jitter). Click the marker to check its GPS accuracy circle. It reuses *Your
  Location* (no second detector), so in Manual it snaps to the UAV's launch position when you connect.
- **FormationFlight / INAV-Radar support (ADR-036).** Kite joins an ESP32 INAV-Radar / FormationFlight
  formation as a **ground node**: connect the module over USB-serial (Radar panel → FormationFlight tab →
  port + baud + node name) and Kite emulates an INAV flight controller at your GCS location so the module
  shares it and relays the other aircraft. Peers appear as **paper-plane** contacts, coloured by state
  (armed = blue, disarmed = grey, lost = red, kept 5 min at the last position), labelled by letter (A–F,
  matching the OSD), with a link-quality readout in the list. Monitoring only — never raises a conflict
  alert. 2D + 3D.
- **3D contacts no longer sink under the terrain in radar-only scenes.** The geoid offset is now computed
  at the GCS location even with no UAV connected, so ADS-B / FormationFlight contacts sit at the right
  height.
- **ADS-B conflict alerts (ADR-035).** Smart two-stage collision avoidance for the connected UAV against
  foreign traffic. **Stage 1 — Caution:** a yellow advisory when a contact is inside a 5 km / ±2000 m
  zone and closing (it clears again once the contact has flown past). **Stage 2 — Collision warning:** a
  red alarm from the predicted 3D closest point of approach (course + climb/sink of *both* aircraft),
  with an **evade heading** at right angles to the intruder's track. Outputs: a **banner** at the top of
  the map listing every affected contact; **audio** (a tone plus a spoken "Traffic" / "Collision" callout
  in your language, English fallback); and a **map highlight** — the contact's 1 km ground circle (= the
  collision radius) pulses red/yellow in 3D, with pulsing rings around the 2D icons. New **Alerts** group
  in the Radar panel's ADS-B tab: Stage 1, Stage 2, sound and voice switches.
- **Debug Monitor is now multi-tab (MSP · Alerts) with GPS injection (dev).** The in-app dev monitor gained
  an *Alerts* tab showing the live conflict maths per contact, and a global *GPS inject* row (set the UAV
  position from the map centre) for testing alerts over busy airspace without flying.
- **3D FPV (cockpit) camera view + conformal HUD (ADR-034).** A fourth 3D camera mode (Free → Follow →
  Orbit → **FPV**) drops the eye onto the aircraft: the model is hidden and the camera takes its exact
  position (raised 0.5 m) and attitude. The flight track dims to 40% so it doesn't fill the view, and
  the scroll wheel changes the **lens FOV** (30–120°) rather than dollying. A minimal projected-style
  **HUD** overlays a *conformal* artificial horizon (pitch ladder + horizon that stay aligned with the
  real terrain horizon as you zoom), plus a compact bank scale, heading tape, and speed/altitude
  readouts in your display units.
- **Camera mode is remembered across 2D↔3D switches.** The 2D follow mode and the 3D camera mode
  (including FPV) are each restored when you toggle back to that view.
- **3D real-time lighting + day/night dimming (ADR-033).** New *Real Daytime and Lighting (3D)* setting
  lights the Cesium globe with the real sun position. *Log Replay Time (3D)* (gated on the former) drives
  the sky clock from the replayed flight's actual date/time, reconstructed from the flight start time +
  the relative telemetry timestamp — so the sun matches the real flight conditions and moves with the
  scrubber. A DEV-only time-of-day slider (top-right) lets you preview the lighting across 00:00–23:59.
- **Night Mode (2D & 3D)** — Off / Auto / On. Darkens only the map imagery (telemetry, markers, sky and
  sun stay bright) to Cesium's night brightness (×0.3). *Auto* fades smoothly at sunset based on your
  physical location + system time; combined with real lighting it always takes the *darker* of the two
  (never stacked, soft terminator preserved). *On* forces a flat night ground while keeping the real sky.
- **Your Location** setting (under Night Mode) with a one-click *Detect* button. The location is found via
  OS geolocation (on start + on demand) or the first valid UAV GPS fix per connection, persisted across
  sessions, and used only for Night-Mode *Auto* — never tied to the camera/view.
- **Log player shows the time-of-day** (flight start + elapsed) at the current playback position.

### Fixed
- **ArduPilot mission upload works (was always cancelled).** Uploading a mission failed with
  `MAV_MISSION_OPERATION_CANCELLED` after ~5 s. Two causes: the MAVLink handler-loop read timeout was 1 s
  (so each item reply lagged — now 50 ms, across TCP/serial/UDP), and decisively, we only answered the
  FC's `MISSION_REQUEST_INT` while ArduPilot (SITL) requests items with the deprecated `MISSION_REQUEST`
  (float) variant — we now answer both. Upload completes in under a second.
- **ArduPilot mission round-trip is now faithful.** The mission codec mapped commands/frames through a
  small hand-maintained whitelist, so any waypoint type Kite had no editor for was silently rewritten to
  a plain waypoint on upload. Commands and frames now map by their numeric id across the full MAVLink
  dialect (`FromPrimitive`), so every type the FC sends survives download→upload unchanged (shown
  generically until it gets a dedicated editor).
- **ArduPilot home slot (mission item 0) handled correctly.** ArduPilot reserves mission slot 0 for
  home, not a waypoint. Download dropped that slot so the planner shows only real waypoints; upload
  re-injects a home placeholder at slot 0 (the operator's waypoints follow at 1..). Fixes the off-by-one
  where the first list entry sat on home.
- **ArduPilot takeoff waypoint no longer sits at 0/0.** `NAV_TAKEOFF` has no horizontal position (the
  vehicle launches from where it is); its marker is now anchored on the FC home (fallback: mission-area
  centroid; otherwise hidden), the leg out of it is dashed to show the position isn't fixed, and a jump
  to/from it draws to the same anchor — matching Mission Planner.
- **Mission planner auto-switches to ArduPilot on connect again.** The connect-time autopilot detection
  still compared against `"ArduPilot"`, but the variant became vehicle-specific (`ArduPlane`/`ArduCopter`/…),
  so the planner stayed in INAV mode for ArduPilot FCs. It now matches the whole ArduPilot family.
- **No stray "Track mission?" prompt during a system switch.** Connecting an ArduPilot FC with an INAV
  mission still loaded raised the live track-mission prompt behind the Clear-or-Disconnect dialog; it is
  now suppressed while a system switch is pending (that dialog owns the loaded mission).
- **MAVLink track no longer sawtooths (2D + 3D, live and replay).** ArduPilot sends both
  `GLOBAL_POSITION_INT` (the fused EKF position) and `GPS_RAW_INT` (the raw, noisier, lagging receiver
  position); both were emitted as position, so the track flip-flopped between two slightly-offset
  solutions every frame. `GLOBAL_POSITION_INT` is now the single position authority — `GPS_RAW_INT`
  only contributes satellite count + fix type and reuses the fused fix — so map smoothing and the FPV
  view track cleanly again.
- **Downloading a mission no longer freezes the app.** The mission download/upload commands ran
  synchronously on the main thread, so the blocking microprotocol handshake locked the whole UI for the
  duration — painful for a large mission over a slow SiK link. They now run off the main thread
  (`ardu_mission_download`/`ardu_mission_upload` and the MSP `mission_download`/`mission_fc_info`).
- **Flight-mode badge with no FC connected is now "N/A".** The "Unknown" placeholder overflowed the
  fixed-size widget card; shortened to keep it inside the bounds.
- **ArduPilot flight mode now shows correctly.** Two MAVLink-side bugs made the mode widget show an
  INAV mode (e.g. "HORIZON"/"ACRO") for an ArduPilot vehicle: the connection reported a generic
  `fc_variant` of `"ArduPilot"` (so the frontend fell back to the Copter mode table even for a plane),
  and the handler squeezed the raw `custom_mode` into INAV box-flag bits instead of passing it through.
  Now the variant is vehicle-specific (`ArduPlane`/`ArduCopter`/`ArduRover`/`ArduSub`) and the raw mode
  number is forwarded, so the per-vehicle mode table resolves the real mode (Manual/FBW-A/Auto/RTL/…).
  Applies live and in replay of MAVLink-recorded flights.
- **Toolbar port field no longer has the number spinner.** The TCP/UDP port input dropped the native
  up/down arrows (clutter in the toolbar) — it is now a plain numeric field.
- **PFD bank scale now reads as two distinct marks.** The fixed zero-bank reference and the live roll
  pointer pointed the same way (both ▲), so the attitude display read as two identical pointers. The
  fixed **roll index** now points **down** at the live **roll pointer** (sky-pointer convention — they
  converge when wings are level), in both the AHI widget and the FPV HUD. In the AHI widget both marks
  are white (the green index was barely visible); in the green FPV HUD the pointer is bright/white and
  the index is HUD-green.
- **Panel view state now persists across switching panels.** The Radar panel kept reverting to the
  advanced view (instead of remembering Compact) and the Settings panel forgot which tab (Interface /
  Data) was open — both now persist (a `panelState` store), so panel operation is consistent.
- **Selecting a logbook entry while connected no longer loads it onto the map.** It used to load the
  linked mission + set home/launch from the flight in the background — which fought the live FC state
  and caused the FC↔map desync. While connected, a logbook selection now shows **details only**
  (nothing on the map); load a flight's mission via its linked-mission chip → Mission Manager.
- **Swapped video scales to the window.** With video as the background (swapped with the map) it was
  shown at its native resolution instead of filling the area; it now scales to full width/height with
  `object-fit: contain` (letterbox/pillarbox bars when the aspect ratio differs).
- **Live-recorded altitude now replays at the correct height.** A live recording replayed ~one terrain
  height too low (e.g. **−84 m AGL at takeoff** on a ~84 m-MSL field) — the 3D track sat underground —
  while the relative-altitude widget was fine. The recorder stored the **baro relative-to-home** altitude
  in `alt_m`, but `alt_m` is the **GPS-MSL** column the replay map uses as the track height (the adapter
  maps `alt_m → altMsl`). It now stores GPS MSL there (matching the live track + Blackbox import); the
  relative reading stays in `baro_alt_m` for the widget. _Existing live recordings keep the old wrong
  `alt_m` — only flights recorded after this fix replay correctly._
- **Live flight mode + waypoint tracking now decode correctly (MSP active modes).** A flown mission showed
  **"Cruise"** instead of **"Mission"** and the active waypoint never pulsed on a live connection (replay was
  fine — it reads the logged flight-mode flags directly). Root cause: the box-id→flight-mode table used the
  INAV **`boxId_e` enum ordinals**, but `MSP_BOXIDS` returns the **`permanentId`** field (different for most
  boxes — e.g. NAV WP is enum 19 but permanentId **28**, TURN ASSIST is enum 26 but permanentId **35**). The
  table is now keyed by the authoritative `permanentId` values from INAV `fc/fc_msp_box.c` (stable across
  releases), so all NAV/stabilization modes decode correctly.
- **Whole UI froze when toggling 2D follow mode with live telemetry.** In follow mode the rAF loop recentred
  the map every frame via `setView`, whose synchronous `moveend` ran `saveMapState` → `settings.patch` —
  i.e. an `$effect` wrote the settings store mid-flush, tripping Svelte's `effect_update_depth_exceeded` and
  killing every subsequent render (imperative Leaflet handlers kept working, so the freeze looked partial).
  Programmatic follow recentres no longer persist map state (they were never user-initiated), which also
  removes a 60 Hz localStorage-write + airspace-redraw storm.
- **3D live track: full height, no flicker, no FPV overshoot.** The live 3D trail is now driven by the
  `liveTrack` store (the full flown history since arm) instead of being built incrementally only from the
  moment 3D was first opened — so the whole track shows at the correct geoid height (the pre-3D portion no
  longer "comes out of the ground"). The growing segment uses a `CallbackProperty` (no per-point
  remove/re-add → no 1 Hz flicker), and the drawn tip trails the smoothed UAV by one point so the coloured
  line never shoots ahead of the model in the FPV view.
- **3D mission waypoints no longer flicker / sit underground with live telemetry.** The geoid helper
  re-rendered the whole mission on **every telemetry frame** (it reports success immediately once the
  offset is known), so `renderMission3D` repeatedly removed the waypoints, awaited terrain, and re-added
  them — a flicker that worsened once the new aero terrain-sampling shared the same pipeline. It now
  re-places the mission only when the geoid offset was *just* derived.
- **Mission launch/home defaults to WP1.** Loading a mission (DB / file / FC) with no launch point now
  derives one — live UAV HOME → the mission's embedded home → the first waypoint — so REL waypoint
  altitudes resolve before any edit. An existing (e.g. user-placed) launch point is kept.
- **Waypoint editing UX.** Entering edit mode auto-switches to the 2D map (waypoints can't be edited in
  3D), and while editing, map clicks no longer pop the airspace info list (they only de-select waypoints).
- **3D radar no longer stutters when many contacts load.** Contact entities (the glb model + ground
  geometry) are now pooled and reused as aircraft enter/leave the view instead of being destroyed and
  rebuilt — building a Cesium model per contact was a main-thread "Scripting" stall (160–250 ms) that
  recurred whenever fresh aircraft appeared (e.g. panning the free-look camera). The pool pays the model
  setup once (per model class) and then reuses it, so only the very first fill of a dense area has any
  cost. Also added a skip-guard so an unchanged contact's model isn't re-touched every poll.
- **Portable mode no longer writes anything to system paths (Windows).** The window-state plugin saved
  its `.window-state.json` into `%APPDATA%\com.kitegc.app\` — the one thing that escaped the portable
  `data/` folder (it cannot be redirected on Windows). It is now disabled when a `.portable` marker is
  present, so a portable build keeps **all** state (settings, IndexedDB tile cache, SQLite DBs, raw logs,
  terrain cache) under `data/`. Trade-off: portable builds don't persist window geometry (ADR-030).
- **3D lighting toggle now refreshes immediately.** Turning *Real Daytime and Lighting* on/off updated
  the globe only on the next camera move (`requestRenderMode` needs an explicit `requestRender()` after
  an appearance-only state change).
- **Terrain profile chart — readable labels in compact layouts.** The altitude (Y) axis labels could
  overlap on big elevation ranges in the compact-wide view / on small screens; the axis tick counts now
  scale with the available plot size. Waypoint numbers (dense survey patterns especially) are lifted
  into a staggered band at the top of the plot with a thin dashed connector down to each dot (like the
  3D-map markers); labels that still can't fit without overlap are dropped (the dot stays).
- **3D replay model now sits at the correct height from the first frame.** On loading a Blackbox
  replay the model was placed a few metres above/below ground until the first position update — the
  `playbackPoint` effect positioned it before the (async) geoid offset finished computing. It is now
  re-snapped onto the first track point once the offset is ready (INAV + ArduPilot).

### Fixed — replay performance: terrain-HUD freeze on log switch (ADR-032)
- **The whole app no longer stutters / stops loading map tiles during replay after switching logs
  with the player open.** Root cause: the **Live-AGL** terrain HUD accumulates the flown path and
  samples the terrain of each new segment; loading a *different* log while the player stays open made
  the next segment span the two sites — a single `terrain_profile` request across thousands of km
  (hundreds of thousands of DEM samples) that parked the backend and starved the main thread's map-tile
  callbacks (terrain kept meshing in worker threads; imagery, loaded on the main thread, froze). Fix:
  the HUD now **resets on a discontinuity** — time running backwards (scrub / new flight) OR a
  position jump > 1000 m (log switch / big seek).
- **Live-AGL history is now bounded** — it accumulates to 5 km then trims to the most recent 1.5 km,
  so the per-tick profile fold stays flat instead of growing for the whole replay (the compaction runs
  only once every few km of travel). The full-track Terrain-Analysis panel is unaffected.
- **DEM tile fetches now time out** (connect 8 s / total 25 s) so a stalled download can't hang the
  terrain provider's load lock indefinitely.

### Fixed — 3D map: no more dark-blue full-tile reload at the over-zoom threshold
- Crossing into a sparse over-zoom region (ESRI) no longer blanks the **entire** globe to dark blue for
  1–2 s. The placeholder-detection refresh used to re-apply the imagery provider (`layers.removeAll()`
  — a full-globe teardown), which on a moving 3D replay could storm into a stutter. The 1–2 blank tiles
  that slip through before the hash is confirmed are self-correcting on the next camera move instead.

### Added — 2D↔3D view continuity (ADR-031)
- **The 3D map is kept in RAM** after the first open — toggling 2D↔3D is now instant (no Cesium
  viewer / terrain re-init). While hidden its render loop is paused (no GPU cost); entities keep
  updating from the stores, so re-show is current.
- **The view follows you across the switch.** 3D→2D re-centres the 2D map on the spot the 3D camera
  looks at; 2D→3D points the camera at the 2D centre — applied **synchronously, no fly-to sweep**.
  Each view keeps its **own zoom** (zoom is not transferred). Switching back to 3D restores the
  exact camera you left (drift-free) unless you moved the 2D map; follow/orbit re-anchor on the UAV.

### Added — mission launch/home stored in the library (DB schema v11)
- **The mission's launch/home point is now persisted to the DB** (`missions.home_lat/home_lon`,
  migration v10→v11) — previously it was only written to the `.mission` file export (`<mwp>` meta),
  so a library mission lost its launch reference and **REL-altitude waypoints rendered at the wrong
  height** in the 3D preview. Saved on every library save; restored to the launch point when a
  mission is loaded in the planner.
- **Replay derives the launch reference** from the actual flown **start point** (fallback: the
  mission's saved home, then its first waypoint) — so a flight log with a linked REL mission shows
  the waypoints at the correct elevation immediately, even for missions saved before this change.

### Fixed — 3D elevation offset + camera
- **3D mission preview now gets the geoid offset without a live link or replay.** The MSL→ellipsoid
  geoid offset was only derived when a UAV (live/replay) was drawn; a mission-only preview placed
  every waypoint off by the local undulation (tens of m). It's now derived from the first drawn
  feature (UAV fix **or** waypoint) via a single-flight, awaitable computation — so a flight log
  with a **linked mission** no longer races: the track and the mission share the one offset instead
  of the mission drawing at 0.
- **3D camera zoom no longer creeps in** one step per 2D↔3D round-trip (terrain-vs-ellipsoid range
  mismatch) — the exact camera matrix is replayed when the 2D map wasn't moved.
- **2D map no longer re-centres on the replay trail on every switch** — `fitBounds` now runs only on
  the first load of a track from the DB (the "already framed" key is module-scoped, surviving the
  2D map's remount).

### Added — 3D UAV models + attitude + motion smoothing
- **3D UAV models on the Cesium map** — procedural low-poly glTF models replace the flat position
  point during live + replay and show the craft's full attitude. Per platform: **quad** (multirotor),
  **tricopter** (Y-frame), **fixed-wing** (airplane), **VTOL** quadplane, and a generic extruded
  **arrow** for the rest (helicopter/rover/boat/other). Aviation nav-light colours (port red /
  starboard green) on the rotor rings / wing details make an inverted or banked attitude readable;
  cyan nose arrow. Hand-written `.glb` generators in `scripts/gen-uav-*.mjs` (no build deps); assets
  in `static/models/`. Lightly tinted by flight-mode colour (MIX) so the mode still reads.
- **Full 3D attitude** (heading + pitch + roll) on the model, taken from the same unified
  `TelemetryData` the AHI widget uses (consistent across INAV/ArduPilot, live + replay). Built from
  explicit body axes in the local ENU frame, so it stays correct at all attitudes (inverted, high
  bank) — not a small-angle Euler approximation.
- **Adaptive 3D motion smoothing** — interpolates position and attitude *separately*, re-basing only
  on real data changes with a median-of-recent-intervals transition time. Tracks the true data rate
  (2–10 Hz GPS vs 10 Hz attitude), rejects single aliased/dropped samples, and holds (no
  extrapolation) across gaps/packet loss. The follow/orbit camera is driven from the smoothed state.
- **New "VTOL" platform type** (manual override in the flight-detail dropdown — INAV does not parse
  it) with its own quadplane 3D model and en/de/fr labels.

### Added — replay model override + the 2D map renders the same 3D models
- **Replay model override** — a dropdown in the replay control (bottom-right, opposite the track
  colouring) forces a specific model (Quad / Tricopter / Plane / VTOL / Generic) or **Auto** (from
  the flight's platform type). Live-switchable; the marker model swaps instantly.
- **The 2D map now renders the same glTF models top-down**, replacing the flat SVG silhouettes — a
  dependency-free canvas renderer with a per-pixel **z-buffer** (correct occlusion for
  interpenetrating parts, e.g. a tilted multirotor's arms vs the body), flat shading from a side
  light, a soft drop shadow, and full **attitude** (heading/pitch/roll → roll/bank read on 2D too).
  Size scales with zoom **and the UI-scaling setting**; orientation is smoothed at 60 fps in the
  follow loop. Single source of truth: the **same `.glb` assets** as 3D — loaded by a small parser
  (`uavMesh`), selected via a shared helper (`uavModels`, also used by 3D + the dropdown), drawn by
  `uavTopDown`. The old `createUavIcon` / SVG silhouette path was removed.
- Models given a **near-white base** so the marker tint reads clearly on both maps.

### Added
- **Experimental French locale (`fr`)** — selectable in Settings → Language (Français). Full key
  parity with `en` (UAV/FPV terms kept English). _Not on the mandatory dual-update list_ — new
  `en` keys fall back to English via `fallbackLocale` until `fr` is updated.

### Fixed
- **3D follow/orbit camera zoom drift** — the camera slowly zoomed in/out depending on the craft's
  flight direction (its radial motion was baked into the auto-zoom). `lockRange` is now measured
  against the previous frame's target, not the moved one — mouse-wheel zoom still sticks.
- **Marker colour = nav state, consistently** (2D + 3D, live + replay) — restored the deliberate
  split (the **track** shows flight mode, the **marker** the navigation state: Idle/RTH/PosHold/
  Landing/Emergency/Landed — see `COLORED_TRACK_PLAN`). 3D-live previously fed flight-mode flags into
  the nav-state lookup and 3D-replay used the flight-mode colour; both now use `nav_state`.
- German locale: added the missing `survey.clockwise` / `survey.counterClockwise` (CW/CCW) keys
  (they previously fell back to the key/English).

### Changed (internal cleanup)
- Modernized `catch (e: any)` → `catch (e)` (unknown) across the mission panels + `+page` (no more
  `any` in catch clauses).
- Silenced the intentional MAVLink deprecation warnings with `#[allow(deprecated)]` + rationale
  (we deliberately still route legacy `MISSION_REQUEST`/`MISSION_ITEM` for FC compatibility, and
  wire command 201 = `MAV_CMD_DO_SET_ROI`).
- Refreshed the DEVLOG project tree to a maintenance-friendly folder-level overview (the old
  per-file tree had gone stale after the component-subfolder + panel-framework reorg).

## [0.5.0] - 2026-06-04

### Fixed
- **Window size/position now persists** across launches (`tauri-plugin-window-state`) — the app
  no longer always reopens at the default 1280×800. Saves on close, restores on next launch.
  See ADR-030.

### Added — Mission stats, type-specific UAV symbols, FlightDetail polish
- **Mission stats** in the INAV editor footer: total leg distance, total climb/descent and an
  estimated flight time (`computeMissionStats()` — carry-forward per-WP cruise speed + hold times,
  counting only the active part up to the first Land/RTH). The time shows `~` when an assumed cruise
  speed is used (WPs at default speed) and `≥` when a PosHold-∞ makes it unbounded; unit-aware.
- **Type-specific UAV symbols on the 2D map** (multirotor / airplane / helicopter via
  `uavShapeForPlatform()`), for both live and replay markers; the replay marker uses the flight's
  `platform_type` (live FC type only while connected). Icons enlarged for visibility. **3D** keeps
  the plain coloured position point for now — a proper 3D model will replace it later.
- **Platform type is now editable** in the flight detail (dropdown under Craft Name, INAV mixer
  enum) and persisted (`flightlog_update_platform_type`) — the reliable way to set the replay
  symbol regardless of import guesses, and it fixes existing entries in place.
- **Import also parses the platform type** as a best-effort default (was hardcoded to multirotor):
  ArduPilot `.bin` maps the MSG vehicle banner (Plane → airplane, Copter → multirotor, Rover →
  rover, Sub/Blimp → other); INAV Blackbox has no explicit platform header, so it's inferred
  heuristically from the logged field set (single `motor[0]` + `servo[...]` → fixed-wing, ≥3 motors
  → multirotor). The import value is just a default — correct it via the dropdown if wrong.
- **FlightDetail** mission/battery link affordances migrated to the shared `Button` `compact`
  variant (jump chips + link/unlink/save controls; new `link` chain icon in the registry),
  replacing the ad-hoc inline chips for a consistent panel-framework look.
- **Logbook group aggregate stats** — each tree-group header (both levels) now shows the group's
  total flight time + distance next to the flight count (unit-aware), computed in `buildFlightTree()`.

### Added — Reusable panel framework + per-panel migration
- **`PanelShell` + control library** (`Button`, `Toggle`, `SegmentedToggle`, flat-SVG icon
  registry) now back every nav-rail panel — one shell with `info` / `compact` / `advanced` /
  `fullscreen` / `wide-compact` variants, standardised field widths (380 px main / 500 px detail),
  a 200 px content-field minimum (whole panel scrolls when too short), no in-panel close button
  (closed via the rail ✕). See `docs/active/PANEL_FRAMEWORK.md`.
- **All panels migrated** onto the framework (built in parallel behind duplicate "v2" rail
  buttons for side-by-side review): UAV Info (`info`), Flight Logbook (`info`/`compact`/`advanced`)
  + Battery Manager (own shell, 1:2), Mission planner (INAV/ArduPilot) + Mission Manager, Terrain
  Analyzer (`fullscreen`/`wide-compact`, converted in place), Video, and Settings (reorganised into
  Interface / Data tabs via a slide toggle, grouped subsections, tiny hints dropped except Cesium).
- Battery delete dialog now also offers **Retire / Mark Damaged**; mission/terrain/video adopt the
  shared `Button`/`Toggle`/`SegmentedToggle` controls; transient status lines auto-clear after 10 s.
- Fix: a restored **Flight Logbook** tab now loads its entries on app start (no tab-switch needed).

### Changed — NavRail (consistent behaviour + flat icons)
- **Reordered** the rail to match the typical workflow: UAV Info · Mission · Terrain · Logbook ·
  Camera · Settings (the dev-only DEV playground stays last).
- **Consistent button behaviour:** the Terrain Analysis button no longer toggles its overlay
  closed on re-click — like every other nav-rail button it only opens/selects. Closing is done
  by closing the whole rail (the hamburger ✕) or selecting another tab.
- **Flat, high-contrast SVG icons** replacing the mixed glyph/emoji set: UAV Info = microchip
  (FC; neutral across UAV types), Settings = 6-tooth gear (sharp teeth), Logbook = solid spiral
  notebook (knocked-out text lines), Mission = filled map marker, Terrain = two solid peaks,
  Video = solid movie camera. Icons use `currentColor` (follow inactive/hover/active states) and
  fill ~90 % of the button.
- **Active state** uses a dark translucent fill (`rgba(0,0,0,0.5)` + blur) so the accent border +
  icon stay readable over bright maps; inactive icons nudged a touch brighter.

### Added — Global UI Scaling
- **UI scale setting (100 / 125 / 150 %)** in Settings → Language, persisted as `uiScale`. Scales
  the whole chrome — toolbar, nav rail, panels, widget docks, dialogs, status bar — via CSS `zoom`
  on a `.ui-scale` wrapper (sized `/scale` so it still fills the viewport). See `docs/archive/UI_SCALING.md`.
- **The map stays at native resolution:** the single Leaflet/Cesium instance is hoisted into an
  unzoomed `.layer-map` (no re-mount), so tiles stay crisp and pointer/clicks stay pixel-accurate.
  Map overlays are scaled individually instead — **WP markers, parameter labels, the WP editor popup,
  Leaflet hover tooltips, and the right-click context menu** all follow `--ui-scale`.
- **Side panels** now bound to the scaled container (vertical overflow scrolls instead of being clipped);
  the **mission WP list** keeps a ≥5-row minimum height (panel scrolls when the detail + buttons don't fit).
- Chosen over a `rem` refactor (258 px font-sizes, 0 rem) — `zoom` reflows everything together, no
  per-component rework. Native `title=` panel tooltips are **not** scalable (rendered outside the DOM by
  WebView2) → a custom tooltip/assistance system is on the roadmap.

### Changed — Mission editing
- Outside edit mode, a waypoint can now be **deselected**: tap empty map, or tap the already-selected WP
  again (marker or list row). Previously a selection was sticky until another WP was picked.
- Selecting a WP in edit mode now **centres it in the visible area** (biased clear of the mission panel /
  player) instead of letting Leaflet's popup auto-pan dump it at the edge.

### Added — Battery Management (battery library, Phase A + B)
- **Pilot fields** (DB schema **v9**): per-flight **Pilot name** + **Pilot ID**, manually editable
  in the flight detail (inline edit, saved together). Forward-looking anchor for a future
  operator/login system. Self-healing migration (existing DBs gain the columns on next open).
- **Flight Logbook design unification:** the logbook control buttons now match the app style
  (11px, accent-blue hover; destructive = red), the sort select / search input align in height,
  and the toolbar wraps when needed.
- **Battery Manager** — a **view-toggle inside the Flight Logbook** (🔋 button → battery list;
  ← Back returns to flights), styled like the logbook (wide list, widest when a pack is selected).
  - **DB schema v10:** `battery_packs` (identity = serial) + a **soft `flights.battery_serial`
    link** resolved at read time (no FK; a serial with no pack shows "not in library"; deleting a
    pack just leaves flights pointing at a missing serial; re-importing re-resolves them).
  - **Pack detail:** editable identity (label, maker/model, chemistry, cells, capacity, C-ratings,
    connector dropdown, in-service date, status, notes), **computed** nominal voltage / voltage
    range / energy (Wh) from chemistry + cells + capacity, and **lifetime = persistent baseline +
    Σ(linked flights)** (cycles, flights, flight time, mAh, charges). **Linked flights** list jumps
    to the flight in the logbook.
  - **Create / edit / add-usage** as modal popups; the **additive usage editor** only ever adds to
    the persistent baseline (cycles / hours / mAh / charges). **Delete** warns how many flights
    reference the serial.
  - **List:** grouped (Cell→Capacity / Capacity→Cell, ▲/▼ orders the groups) or **Flat** (sort by
    serial / cell count / capacity); leaf packs always serial-ascending in grouped mode. **Storage**
    and **Retired & Damaged** packs form trailing collapsible groups in every mode. Groups start
    collapsed. Search by serial / label / maker / model / notes.
  - **Logbook:** the flight detail has a **Battery** row — link/unlink by serial (unknown serials
    allowed); the manual **Refresh** button was removed (the list auto-reloads on disarm/disconnect).
  - **`.kbatt` export/import:** per-pack export (**Consolidate** folds linked-flight usage into the
    file's baseline / **Base** = baseline only; non-destructive to the source) and an import **preview**
    with serial-conflict resolution (Consolidate / Overwrite existing, or edit the serial to import as
    new). Import/Export live in the logbook toolbar (Import over the list, Export over the data view).
  - **Cross-jump navigation:** a flight's linked **mission** and **battery** are slim chip buttons that
    jump to the respective Manager (selected); the Battery Manager's linked-flights jump back to the
    Logbook, auto-expanding the flight tree and scrolling to the highlighted flight.
  - **End-Flight dialog** on **disarm:** a read-only flight summary (duration, max alt/speed/distance,
    mAh, location); when DB-recorded it also captures the **battery serial** (no autofill), a **note**,
    and a **mission link** confirmation (FC-synced missions link automatically; a non-FC mission is an
    opt-in checkbox — the old standalone "mission changed?" prompt folded in). Without recording it is
    summary-only (live arm→disarm stats). Re-arming dismisses it; sub-5 s arms are ignored.
  - **Flight-deletion consolidation:** deleting a flight with a linked battery shows an opt-in checkbox
    in the delete dialog to **consolidate its usage into the battery's lifetime totals** before deletion
    (otherwise the contribution drops from the live sum). `ConfirmDialog` gained an optional checkbox.

### Added — Mission Manager (mission library UI)
- **Mission Manager** — an alternate view of the mission planner panel (button next to Edit;
  Back returns to the editor), styled in the **Flight-Logbook design language** and sized like
  it (wide list, widest when a mission is selected). A **location-grouped, collapsible list**
  (geocoded; an "Unknown location" group for the rest); selecting a mission opens a detail with
  **editable name/notes**, computed metadata (WP count, distance, altitude diff/range, location,
  created), a **non-interactive mini-map preview** of the mission on the current map provider
  (fixed aspect-ratio = bounding box, portrait capped to a square), and the **flights that link
  this mission** — each row **jumps to that flight in the Logbook**
- **Actions:** **Load to Map** (with a replace-confirm if the map mission is unsaved), **Export**
  (INAV `.mission`), **Import** via button or **drag & drop** (popup: into the library + map, or
  map only — both dedup-match), **Delete** (unlinks referencing flights, with a warning)
- **Editor:** a **"Save to library"** button (name + notes dialog; NEW / OVERWRITE / CANCEL when
  a loaded library mission was modified). The file **"Save"** / **"Open"** buttons stay (files
  vs. library is the user's choice)
- **Logbook:** the flight detail shows the **linked mission name + waypoint count** and a
  **Link / Unlink** control (DB mission → link directly; a FILE/FC mission → "save & link");
  loading a flight also loads its linked mission onto the map (hideable via the player MISSION
  toggle)
- **State persistence:** the Manager keeps its open state + selected mission across close/reopen
  (`stores/missionManager.ts`)

### Added — Mission library & flight↔mission linking (Phase 1)
- **First-class mission library in the flight-log DB** — a new `missions` table stores each
  mission once, keyed by a **content hash** (SHA-256 of the same serialization the provenance
  system uses → deduplicated, shared across any number of flights/UAVs). Per-mission metadata:
  waypoint count, total distance, altitude diff (max−min) + max/min altitude, bounding box, and
  a **reverse-geocoded location name** (bbox centroid, same Nominatim service as the flight log)
- **Recorded flights link the flown mission** — on **arm** (with DB recording, mission FC-synced)
  the displayed mission is saved + linked to the new flight; on **disarm** the link is finalized.
  Only the FC-synced mission is linked (a stale/edited-not-reuploaded plan is not what the FC
  flies). If a different mission is uploaded mid-flight, a prompt on disarm offers to update the
  link. The recorder emits `flight-recording-started/-ended` events for this
- **Replay `WP N/X` source** — the Blackbox `H waypoints:N` header is parsed into
  `flights.logged_wp_count`; the replay readout uses the linked mission's count first, then this
  header fallback
- **Self-healing schema (v8)** — existing flight-log DBs gain the `missions` table and the
  `flights.mission_id` / `logged_wp_count` columns automatically on next open (idempotent, no
  data loss)
- _UI pending (planner Save-to-library dialog + NEW/OVERWRITE, import flow, mission browser);
  see `docs/archive/MISSION_LIBRARY_AND_DB.md` for the functional spec + manual test checklist._

### Added — Mission provenance flags + active-waypoint readout
- **3-flag provenance model (FC / FILE / DB)** — per mission slot, each flag is valid only while
  the mission's content still matches the snapshot captured at its sync event (content-hash based,
  so an edit invalidates it and Undo restores it). Gates when the active-waypoint highlight is
  trustworthy; one-time "track?" prompts (replay / flight), a connect prompt (Download / Upload /
  Nothing), and FC/FILE/DB labels in the mission panel. See
  `docs/active/MISSION_TRACKING_AND_PROVENANCE.md`
- **Active waypoint in the Flight-Mode widget** — in MISSION (NAV_WP) mode the widget shows
  **`WP N/X`** (N = active waypoint, X = mission waypoint count) or **`WP-RTH`** when there is no
  active WP; replaces the raw flight-mode-flags hex dump

### Fixed — Terrain widgets could freeze the whole UI
- The **Terrain Radar** and **Live AGL** widgets ran their telemetry update inside a tracked
  `$effect` that both read and wrote the same `$state` (`range`/`step = nextStep(speed, …)`).
  Under some replay values this tripped Svelte's `effect_update_depth_exceeded` guard and
  hard-froze the JS main thread (CSS hover/animations kept playing, but no click or panel switch
  reacted — only an app restart helped). The update now runs `untrack`ed, so the self-reads are
  not effect dependencies

### Added — Mission: Fly-by-Home (FBH) waypoints
- **Fly-by-Home support** — FBH is INAV's `NAV_WP_FLAG_HOME` (0x48) flag on a real, numbered WAYPOINT/POSHOLD_TIME/LAND that executes at the arming home location (not a separate waypoint type, and not shown in the stock INAV Configurator UI). It is added as a **modifier** in the waypoint editor: pick "Fly By Home", and a real WP is created at the home/launch point with the flag set
- **Nested editor section** — the FBH is edited under its parent WP in the same popup (like Set Heading, but richer): a sub-type dropdown (Waypoint / PosHold Time / Land), altitude (+ REL/AMSL/AGL), and the type's params (speed / hold time / user-action bits) — no coordinates
- **Map** — an orange house marker (with the WP number) sits on the inbound leg; dashed inbound + outbound legs in the flight-path blue route through a thin blue **ring around the home/launch marker** (so the legs stop at the ring instead of overdrawing it). The solid flight path breaks cleanly at the FBH instead of cutting straight across. Also fixes FBH waypoints (lat/lon 0) previously drawing a line to "Null Island"
- **Waypoint list** — FBH shows as an orange, numbered `↳ FBH` row (number kept for OSD/other-app consistency) with its altitude and "→ Home"
- **Backend** — `Mission::renumber()` no longer overwrites a Fly-by-Home flag (0x48) with the last-waypoint flag (0xA5) on the final WP; the flag round-trips through MSP upload/download and `.mission` XML
- _3D map overlay for FBH is a separate follow-up._

### Fixed — 3D map: altitude/geoid, camera, source switching & trails
- **Track altitude reworked** — the 3D track now uses the **fused, arming-relative altitude** (`nav_alt_m`, smooth — validated against decoded blackbox logs as far cleaner than GPS/baro) anchored at the first GPS fix, instead of raw GPS MSL. Fixes the stair-stepped vertical track
- **Clean terrain-derived geoid offset** — `N = cesiumGround_ellipsoid − Copernicus MSL` at the reference point (GPS-independent), replacing the single-point GPS-snap that mis-placed tower/rooftop starts and shifted the whole track. Applied to track, ground shadow/curtain and the playback marker; the mission stays `altMsl + N` (consistent)
- **Live UAV derives its own geoid** at the first live GPS fix, so on a fresh start the craft sits at the right height instead of ~tens of metres below ground (previously the offset was only computed when a log was loaded)
- **Map data clears on source switches** — replay log ↔ log and replay → live wipe the old track / trail / markers; a fresh live connect clears **only when disarmed** (an armed reconnect keeps the track for connection recovery); a disconnect never clears. Stops tracks/markers bleeding across locations and the slowdown from stacking continents. The mission overlay is kept and re-placed at the new geoid
- **Progressive shadow/curtain no longer spans a log switch** — `clearDeco()` cancels its pending grow/rebuild timers and a load guard stops the async track load from appending stale points (the old behaviour drew a wall/shadow between the two locations)
- **Camera follow (heading-lock)** — start pitch lowered to **20°** (view from behind with the horizon visible) and the **sideways-drag jitter fixed**: Cesium's own rotate is disabled in follow so it can't fight the per-frame heading lock; pitch is driven by a dedicated vertical-drag handler
- **Recenter on every 2D→3D switch** — reliably frames the UAV/track again (the old inline `flyTo` ran before the canvas was laid out on the first switch and did nothing)
- **Over-zoom placeholder tiles replaced immediately** — when a new blank-tile region is detected, the visible tiles are re-requested so the 1–3 placeholders that slipped through before hash confirmation are swapped for the parent tile, without a manual zoom
- **Live trail only while armed**; a thin plain **black pre-arm trail** shows GPS movement while disarmed (2D + 3D), cleared on arm

### Added — 3D map: altitude curtain + mission overlay
- **3D flight track**: black outline, a terrain-draped grey ground shadow, and a faint vertical **altitude curtain** (wall down to the ground, flight-mode coloured, ~22 % opacity). **Settings → Map → "Altitude Curtain (3D Map)"** toggle (global, default on). In replay the shadow + curtain **build progressively behind the UAV** to show flown progress — chunked growing build (scales to hour-long logs, no per-frame flicker) with a reverse-scrub debounce
- **3D mission overlay mirroring the 2D map**: the **same waypoint marker SVGs** as viewport-facing billboards + the **same line colours/styles** (flight path, greyed-beyond-end, launch connector, jump, RTH), drawn as an always-visible overlay; plus per-WP **drop-lines** (white dashed + black outline) to the ground. Shared `wpIconSpec` (missionIcons), shared geometry helpers (`missionGeometry`), and `resolveMissionAltitudes` (REL/AMSL/AGL → MSL)
- **"Show Mission" toggle** in the replay player (MISSION button after REC/BBX): in replay it shows/hides the loaded mission on **2D + 3D**; in planning/live a loaded mission is **always shown** (`showMission` + `replayActive` stores)
- _Planning in `docs/archive/Map3DRework.md` (archived 2026-06-19 — fully shipped, incl. the live-trail curtain and the FPV cockpit view)._

### Fixed — 2D map follow (replay + smoothing)
- **Follow / Heading-Follow now work during blackbox replay** — the follow path was driven only by the live telemetry store (empty during playback), so the 2D map didn't track the replayed UAV. It now follows the playback position too (live behaviour unchanged)
- **Smooth tracking** — map centre + UAV marker ease toward each new position via a rAF loop (~250 ms catch-up) instead of snapping on every (≈2 Hz) telemetry/playback update; heading interpolates the short way; large jumps (scrub / new flight / first fix) snap
- **Panning disabled while following** (it only fought the locked view); zoom stays enabled but anchored to the map centre (= UAV) instead of the cursor
- Track auto-framing (`fitBounds`) no longer yanks the view out of an active follow

### Added — Map: over-zoom placeholder detection & parent fallback
- **Detect ESRI over-zoom blank tiles** — ESRI World Imagery advertises zoom 1–20, but many areas only have real satellite imagery up to z17–19. Above that the server returns a fixed *"Map data not yet available"* blank (HTTP 200, not a 404), which was acceptable on the 2D map but showed as a blank ground in the 3D follow camera when it descended to UAV altitude
- **Self-calibrating detection** — a content hash (FNV-1a) of the tile bytes; the same hash from two different tile URLs is, with practical certainty, the placeholder (real imagery is never byte-identical). No hardcoded signature, so a provider changing its blank still works. Per coarse region we learn the lowest placeholder zoom + the verified real-imagery depth (in-memory, re-learned each session so newly added imagery isn't hidden). Only active at z≥19 → zero cost at normal zoom
- **Fallback to real parent imagery** instead of a blank: **3D** rejects the placeholder so Cesium keeps the parent (z-1) tile visible (native upsampling); **2D** fills the tile with the scaled real-ancestor tile (a clipping `<div>` + offset child `<img>` resolved through the IndexedDB cache, then network — so already-cached lower-zoom tiles are reused), walking down to the real level where coverage stops several zooms lower
- **ESRI satellite/hybrid `cesiumMaxZoom` raised 17 → 20** — full detail where it exists, with the detection covering the gaps
- Smoothness: fallback tiles get their own GPU layer (`will-change`/`translateZ`) + a 1px bleed, and the learned-cap redraw is deferred to gesture-idle, to avoid seam flicker during pan
- _See ADR-028._

### Added — Mission undo/redo
- **Undo/redo for mission edits** — snapshot-based history that covers **all** missions at once (active + cached multi-mission slots), so even cross-mission *Move to mission* is undoable. The launch point is intentionally excluded (it isn't part of the FC upload)
- **One snapshot = one user action**: the primitive mutators (add / insert / remove / update / reorder / clear) auto-record a step; multi-step actions — **batch edit, batch delete, move-to-mission, pattern append, terrain correction, WP-with-modifiers delete, mission remove** — are grouped into a **single** undo step via `beginUndoGroup()` / `endUndoGroup()`
- **Controls**: flat `↶` / `↷` toolbar buttons (right of the Edit button, **edit-mode only**, hidden in Pattern mode) + keyboard **Ctrl+Z / Ctrl+Y / Ctrl+Shift+Z** (ignored while a text field is focused so native input-undo still works). History limit 50 steps; **cleared on load / download / import** (fresh baseline)
- **Mission clear (🗑️) now asks for confirmation** (in-app dialog) before removing the mission
- **Backend**: new `mission_set(waypoints)` command — replaces the whole active-mission WP list in **one** IPC call, preserving every field incl. `alt_mode` (used by undo restore; faster + atomic vs clear-then-re-add)
- The Mission panel is **15 % wider** (414 px) so the full toolbar fits on one row and the WP list has room for richer entries
- _See ADR-027._

### Added — Custom context menus + waypoint multi-select & batch edit
- **Reusable custom context menu** — right-click **and** touch long-press open an in-app menu (store + `use:contextMenu` action + recursive `ContextMenu` with submenu fly-outs); the native WebView menu (print/save/inspect) is suppressed app-wide except in text inputs. Styled like the NavRail panels with a widget-style blurred background
- **Waypoint context menu** (list rows + map markers): **Move to mission** (INAV multi-mission → submenu of the other missions, moves the whole selection) and **Batch Edit**
- **Multi-select waypoints** (edit mode): list — click = single, **Ctrl/⌘** = toggle, **Shift** = range, tap the **number circle** = toggle (touch); map — tap a marker toggles it (all selected red, editor bubble only for a single selection); tap empty map / leave edit mode clears
- **Batch delete** — the ✕ button removes all selected waypoints
- **Batch Edit popup** — edit **altitude** (absolute + a **relative-change** field that keeps the relative differences), **speed**, **hold time** and **user-action bits** across the selection. Fields show `---` when values differ and apply only to applicable WP types; **one APPLY** (no live-apply, undo/redo-friendly), unit-aware (shared `UnitStepper`/`NumberStepper`, now with an empty/`---` state + display-unit step). Mixed altitude modes block the absolute field with a warning; the mode toggle converts all selected to one mode (terrain/launch-aware, via the shared `convertAltCm` helper)
- Single-WP editor popup restyled to match (blurred background, same accent border)
- _Waypoint **disable/enable** designed (kept in the file's meta, never uploaded) — plan in `docs/active/WaypointDisable.md`, not yet implemented_

### Added — Embedded video (core: router + webcam + panel)
- **Video subsystem foundation** — a source **router** (`stores/video.ts`) opens a source once and shares it with multiple display sinks (one `MediaStream` binds to many `<video>` elements → one decode feeds panel/widget/floating/swap). Layered for webcam now and network streams later
- **Webcam / USB-capture source** via `getUserMedia` — works in WebView2 (Windows) **and** WebKitGTK (Linux), no backend; device enumeration, device + resolution (auto/720p/1080p) selection, mirror
- **NavRail "Video" panel** — start/stop, device picker, resolution, mirror, **live preview**, and an info line (resolution · measured/set fps; measured via `requestVideoFrameCallback`)
- **Frame-rate fix**: the browser camera API can't request MJPEG directly, so high-res modes could land on a slow uncompressed format (13 fps @720p / 6 fps @1080p). Requesting `frameRate: { ideal: 60 }` (FPV standard) nudges the browser to the camera's MJPEG mode → full 60 fps
- **Video widget** (2×1 `wide`) — a router sink showing the shared feed in the standard widget card; crop-to-fill (`object-fit: cover`) for a full 2:1 tile, thin rounded frame, no settings (the panel owns control)
- **Persistence + auto-start** — device/resolution/mirror and the running state are remembered (localStorage); video **auto-starts with the last settings** if it was running at last close, falling back to the default device if the saved one is gone
- **Floating video window** — an in-app overlay sink: **snaps bottom-left** (the bottom dock reflows out of the way), **drag** the header to float free, **corner-resize** (aspect-locked, 10–30 % of viewport height); frosted frame matching the NavRail panels
- **Double-click map⇄video swap** — double-click the floating video → the video fills the map view and the **map moves into the (movable) floating frame** (not a fixed corner); double-click the full-size video to swap back. The map is never re-mounted (Cesium state survives); a `resize` re-fits Leaflet/Cesium. Layered so the map stays fully interactive while the frame header/resize remain usable
- **Native Picture-in-Picture** — a "Video Window" button detaches the feed into a borderless OS window (free placement anywhere on screen) via a persistently-mounted source, so it **survives closing the Video panel**
- _Planning + design in `docs/active/VideoFeature.md`. Network streams (RTSP/UDP) + native capture are v2._

### Added — Terrain Radar widget (top-down EGPWS-style)
- **New `terrainRadar` widget** (1×1) — a top-down, **track-up** terrain-awareness display: a **120° forward fan** sampled as a polar grid and coloured by terrain clearance. Fixed pointing up; terrain is sampled relative to heading so it rotates with the craft. The fan fills the square (wide flanks clipped); the same **UAV ring marker** sits at the apex
- **Two ranges**: the *forward fan distance* is **speed-driven** (300/900/1800/3600 m, shared scale + hysteresis with the Live AGL widget) — shown as range arcs + distance labels; the *clearance colour scale* is a **separate setting** (left toggle **60/120/250 m**, default 120; coarse-rounded **200/400/800 ft** in imperial) — deliberately independent of the Terrain-Analysis `groundClearance`
- **Colouring**: continuous **red→orange→yellow→green** ramp over `0…scale` (`< 0` red, `> scale` off), reference altitude toggles **REL** (current MSL) ↔ **PRED** (sink-angle predicted, averaged FC vario) — right button
- **Heatmap look**: cells textured with an SVG `feTurbulence` + `feDisplacementMap` filter (+ a very light blur), clipped to the fan — keeps terrain detail instead of smearing it like a plain blur
- **Backend**: new `terrain_fan` command — server-side polar sampling (one IPC call/refresh) over the existing tile cache; re-sampled only on meaningful change. Default **off**

### Added — Live AGL widget (forward-looking terrain HUD)
- **New `liveAgl` widget** in a new **`wide` (2×1) widget class** — a side-view terrain HUD: left 1/3 = recently flown terrain + flight history, a neutral (airframe-agnostic) **UAV marker** at the "now" divider that tracks the current altitude, right 2/3 = **estimated terrain ahead along the current heading**
- **Works live *and* in replay**: the flown history is accumulated **internally from the telemetry stream** (the shared `liveTrack` store only fills while armed on a live link, so it is empty during blackbox/flight-log playback). Resets on scrub-back / new flight
- **Forward terrain** sampled along the heading via `terrain_profile` (30 m), re-queried only on meaningful change (>5 m / >2° / scale change / >1 s) to avoid hammering the backend on yaw jitter
- **Heading source** mirrors the compass: filtered 5-point GPS track ≥ 2 m/s, compass `yaw` below
- **Projected flight line** (dashed) from the FC's own vario (the smooth baro/nav-filtered source, 5-sample averaged) — shows the actual climb/descent angle, ground-intersect warning
- **Speed-based horizontal scale** — total render distance steps 300 / 900 / 1800 / 3600 m (1:2 history:forward) with **boundary hysteresis** (step down only below 70 % of the lower step) so cruising on a scale boundary doesn't flap
- **Auto-fit vertical scale** (expand fast / shrink slow); the steep projected line is *not* a scaling reference
- **Axes**: left = altitude **relative to the UAV** (0 = current flight level, incl. negatives, like the Altitude widget); bottom = visible **distance** (0 under the UAV, positive both ways)
- Visuals follow the **Terrain Analysis panel** (grid, ground gradient) inside a standard **widget card** (blur / semi-transparent / rounded); AGL + min-clearance-ahead readouts; **text scales with widget size**. Default **off** (enable in widget settings)

### Added — Terrain Analysis: Live Track mode
- **Track mode follows the live flown track** when an FC is connected (MSP/MAVLink): a shared in-RAM `liveTrack` store accumulates lat/lon + MSL altitude **while armed** (cleared each new arm), independent of the map trail and the flight-log DB
- On arm, the Copernicus tile for the current area is **pre-fetched** so terrain is ready
- **Incremental** profiler — every 5 s only the *new* points are terrain-sampled and appended (no full re-sample); cheap clearance/min/climb folding recomputed over the accumulation
- **Follow** toggle (live only): on = pinned to the newest data (zoom-only, no pan); off = free pan + zoom over the growing range; default 250 m window builds up left→right then scrolls; full zoom-out auto-fits the whole growing range
- **Zoom fix**: the chart's max zoom-in is now a flat 50 m window on any log length (was scaled to total distance, so long logs couldn't zoom past ~500 m)

### Changed — UI & unit consistency cleanup
- **App-wide units honour the interface settings** in mission planning (previously hardcoded metric): altitude/distance/speed are stored internally in metric base (m, m/s; waypoint speed stays cm/s for the FC) and converted at the UI boundary for both display and input
- Covered: **Terrain Analysis** (Ground Clearance, chart axes + readouts), **Survey Pattern** (line spacing / radius / turn distance / base altitude / base speed), **WP editor + mission panel** (altitude, and waypoint speed now in the speed unit instead of cm/s)
- New **`UnitStepper`** wrapper around `NumberStepper` (metric value in, unit-aware display); inverse helpers `toAltitudeM` / `convertLength` / `toLengthM` in `utils/units.ts`
- **`NumberStepper`**: value centered, unit right-aligned inside the field (was sitting outside the `+` button); the Settings panel's bespoke steppers now use the shared component

### Added — Terrain Correction & Jump Simulation (Terrain Analysis)
- **Terrain Correction** (Waypoint mode): **Terrain Follow** (set WPs to a target AGL, then lift legs to clear) and **Clearance Check** (raise-only) over a WP range; corrected waypoints written in **AGL** mode
- **Fixed-wing climb/descent-angle limits** (two params, 0 = off): too-steep legs are eased by raising the lower endpoint (never costs clearance), propagated to convergence
- **Manual *Add WP***: pin a marker on the chart, add a waypoint there exactly on the track, then re-run (replaces unreliable auto-insertion)
- **Live green preview** of the corrected track (drawn behind the path), with changed-count / min-clearance readout and warnings; **APPLY** behind a confirm dialog. Vertical scaling includes the preview so raised lines never clip
- **Clearance warning at 95%** of the target (5% grace) for both the readout and the red path colouring
- **Jump-loop simulation**: one loop per jump (`4J2` → branch `4→2`, **cut**, resume `4→5`) — no duplicate WP dots; the jump-back leg is coloured like the map with a `↩N` target marker, and the resume point shows its WP dot. Correction keys altitude per WP index so revisited WPs stay consistent

### Added — Terrain Analysis (elevation profile & clearance)
- **Terrain Analysis panel**: full-width NavRail overlay showing a side-view elevation profile of the mission/track vs terrain — hand-rolled **SVG** chart, **no external runtime dependency**
- **Two view modes**: *Waypoint* (planned mission, altitudes resolved to absolute MSL via terrain + launch point) and *Track* (flown live temp-log or loaded blackbox); profiles cached per mode → instant switching
- **Clearance check**: dashed clearance floor (`terrain + Ground Clearance`) with red coloring where the path drops below it; min-clearance readout **ignores take-off/landing** (leading/trailing below-clearance runs trimmed; mid-route dips still alert)
- **MSL ↔ AGL datum toggle**: MSL side-view or an AGL *clearance curve* on a flat 0 baseline
- **Zoom/pan** (wheel / drag / double-click reset) with **resolution that scales to the zoom level** — only the visible slice is drawn, decimated to ~screen resolution (peaks + unsafe spots preserved); full-res data drives the readouts
- **Max climb angle** readout; flown-track jitter low-pass filtered (~10-sample window per ≥20 m segment)
- **Compact mode** (*Show Map*): collapses to a short, animated top-docked strip; the chart cursor is mirrored onto the 2D map (`TerrainCursorLayer`) as a transient hover dot + a click-pinned persistent marker that **stays on the map after the panel closes** (and is mirrored back into the chart)
- **Void bridging**: interior null terrain samples (tile-edge / nodata) interpolated so the profile stays continuous
- Session-persistent panel state (in-memory; reset on app close). Global text-selection blocker added (UI is app-like, inputs excepted)

### Added — AGL Waypoint Planning & Launch Reference
- **AGL altitude mode**: `alt_mode` (REL / AMSL / **AGL**) on waypoints; AGL is a GCS-only authoring concept (INAV has no AGL flag) resolved to AMSL on export (`AMSL = terrain(lat,lon) + AGL`, MSP upload + `.mission` save)
- **Editor toggle** cycles REL→AMSL→AGL, converting the value via terrain + the launch point so the physical height is preserved; survey patterns support an AGL (`ground`) option
- **Launch/home reference**: auto-placed, draggable map marker + dashed connector to the first WP; persisted in `.mission` via the mwp-compatible `<mwp home-x/home-y>` meta (round-trips, inter-app compatible)

### Added — Terrain Elevation Provider (Copernicus GLO-30)
- **Local terrain elevation** (`src-tauri/src/terrain/`): Copernicus DEM GLO-30 (AWS Open Data, Cloud Optimized GeoTIFF, no API key, EGM2008 geoid ≈ MSL) — fetch → portable-aware disk cache → `tiff`-crate decode (Float32/DEFLATE/predictor) → in-memory LRU → bilinear sample
- **Commands** `terrain_elevation` / `terrain_profile`; CPU decode + disk I/O on `spawn_blocking` (runtime never stalls), concurrent loads coalesced via async lock
- **≈MSL** throughout — GPS altitude, AMSL waypoints, and GLO-30 are directly comparable (no geoid-undulation hack, unlike Cesium's ellipsoid terrain)

### Added — Survey Pattern Generator: Polygon Lawnmower (Contour-Offset)
- **`generatePolygonLawnmower()`**: contour-offset coverage for arbitrary (concave) polygons
- **Convex decomposition** (`decomposeConvexXY`): recursive reflex-cut splits a concave polygon into convex pieces at reflex vertices, preferring diagonals between two reflex vertices
- **Hertel-Mehlhorn merge** (`mergeConvexPiecesXY`): adjacent convex pieces are re-merged where their union stays convex — avoids unnecessary triangle splits (two triangles forming a quad re-merge into one piece)
- **Robust inward offset** (`offsetConvexInwardXY`): Sutherland-Hodgman half-plane clipping — can never self-intersect, collapsed edges drop out automatically (replaces fragile miter-intersection offset)
- **Per-piece coverage**: concentric rings offset inward until the piece collapses (fills the centre), then a final spine track (`spineOfConvexXY`) along the medial axis when an elongated remainder spans > lineSpacing
- **One continuous path per zone**: all rings + spine of a convex piece form a single survey segment — transfer (turn) legs only occur between separate zones
- **Diagonal ring transitions**: each inner ring is entered one vertex past the nearest point, so ring-to-ring steps run diagonally inward (no perpendicular hop, no re-flown waypoint) — matches rectangle lawnmower / stepped circle
- **Short-edge cleanup** (`removeShortEdgesXY`): waypoints producing tracks shorter than lineSpacing are removed; tiny inner rings are dropped entirely

### Added — Survey Pattern Generator: Polygon ZigZag
- **`generatePolygonZigzag()`**: scanline sweep perpendicular to track orientation, even-odd intersection pairing handles concave polygons
- **Two concave modes** via "Stay inside area" toggle: cross-gap serpentine (default — flies all segments per scan line, good for area photography / trigger zones) vs. connected-fill DFS (stays within connected sub-regions like 3D-printer infill)
- **Interactive map editing**: independently draggable corner markers, midpoint insertion markers (click to add vertex, max 50), draggable centroid marker (moves whole polygon), right-click to delete vertex (min 3)
- **Self-intersection protection**: live preview pauses while a drag would make the polygon self-intersecting; the vertex reverts to its last valid position on drop
- **Touch-friendly delete zone**: overlay at the top of the map while dragging a vertex
- **Default shape**: equilateral pentagon, 200 m circumradius, map-centered
- **Track orientation** rotates the scan lines only, not the polygon shape

### Added — Survey Pattern Generator: Circle (Stepped) + Spiral
- **`generateCircleStepped()`**: concentric rings, `ringPoints` waypoints per ring (auto-reduced for small inner rings), center-point termination
- **`generateSpiral()`**: Archimedean spiral — fixed angular step (360°/ringPoints) in the outer phase, widening to keep arc = lineSpacing in the inner phase; stops when the UAV turn would exceed 60° (interior angle < 120°) or a track would fall below lineSpacing; always terminates at the exact center
- **Circle editing markers**: draggable center (blue) + radius handle (red)
- **Shared circle UI** for both shapes: radius, ring points, line spacing, start angle, CW/CCW, reverse, altitude, speed, user-action triggers
- **`ringPoints`** parameter (default 10) added to `CirclePatternParams`

### Added — Survey Pattern Generator: shape switching & info display
- **`switchShape()`**: clean shape switching with a per-family parameter cache — params survive switches between shape families within a session (e.g. rectangle → circle → rectangle restores the rectangle params); same-family switches (rectangle ↔ lawnmower, circle ↔ spiral) preserve all params
- **Per-shape state separation** in the panel: `rectangleParams` / `circleParams` / `polygonParams` are independent `$state` — no cross-shape sharing
- **Reactive waypoint count**: live "N WPs" readout per pattern, shown in red when the mission would exceed the 120 WP limit
- **Computed info**: rectangle shows actual line spacing + WP count; circle shows rings; spiral shows rotations — all in a single info row

### Fixed — Survey Pattern Generator
- **Shape-switch corruption**: switching to Circle/Spiral previously kept rectangle params (no `radius`), breaking the dropdown/preview; `switchShape()` now builds shape-correct defaults
- **Double-render**: merged the layer's two `$effect`s into one and made `prevShape` a plain `let`, eliminating a reschedule loop
- **Circle → rectangle layer reuse**: `L.Circle` has no `setLatLngs`; added `instanceof` guards so the shape layer is recreated when the geometry type changes
- **Reverse + user-action flags**: start/end flags now land on the correct waypoints after a reversed path (was applying flags in pre-reverse order) — fixed in all generators
- **Turn distance on collinear gaps** (polygon zigzag, U-shapes): turn extension is applied only before a real turn to the next scan line, not on intermediate cross-gap segments — keeps the end-flag trigger at the true boundary crossing
- **NumberStepper**: restored `bind:value` + `onchange` so keyboard entry and live preview work correctly

### Added — Rectangle Lawnmower (Contour-Offset) Pattern
- **`generateRectangleLawnmower()`** algorithm: concentric rectangles shrunk by `2×targetLineSpacing` per layer
- **CW/CCW flight direction**: Checkbox toggles clockwise vs counter-clockwise traversal
- **Start Corner** (1–4): Selectable corner index to position the pattern start point (replaces trackOrientation for lawnmower)
- **Full 4 corners per layer**: No shortening of the last edge — all 4 corners are visited
- **Diagonal layer transitions**: Short diagonal from E4 of one layer to E1 of the next inner layer, saves one waypoint per layer
- **New User Action flags**: 3-zone system (Start / Track / End) replaces Line-Start/Line-End for lawnmower — each zone has independent 4-bit trigger mask, applied to first WP, interior WPs, and last WP
- **Zigzag unchanged**: Rectangle pattern retains original Line Start / Line End UA system
- **Live preview**: Map layer renders lawnmower path with correct coloring (survey=blue)
- **Reactivity fix**: `clockwise` and `startCorner` parameters now trigger preview updates via `$effect`
- **CW/CCW labels swapped**: UI checkbox labels inverted to match actual flight direction behavior
- **Parameter store**: `startCorner`, `userActionStartFlags`, `userActionTrackFlags`, `userActionEndFlags` added to `BasePatternParams`
- **Rectangle shape editing**: Center, length, width, orientation via NumberStepper UI + draggable map markers (corner + center)
- **Map visualization**: Gray semi-transparent shape polygon + blue survey path preview with sawtooth turn extensions
- **Turn Distance**: Extends outbound legs beyond shape boundary for fixed-wing turn zone
- **Track Orientation**: Independent track angle within shape — tracks rotated and clipped to shape boundary
- **Altitude Type**: Dropdown with Relative / AMSL / Ground (Ground disabled, "coming soon")
- **User Action Trigger**: 4 checkbox pairs per line (start + end), encoded as bits 1–4 in p3 per INAV spec
- **Waypoint generation**: `generateRectangleZigzag()` + `generateClassicZigzag()` algorithms, deduplication of survey/turn boundary points
- **120 WP limit**: Check with ConfirmDialog + truncation option
- **Persist params**: Pattern configuration survives mode toggles (reset on app close)
- **FC buttons hidden**: Upload/Download/Save/Load hidden while in Pattern mode
- **WP placement blocked**: InavMissionLayer blocks map click WP placement when Pattern mode is active
- **Waypoint p3 encoding**: `altMode` (bit 0: 0=REL, 1=AMSL) + `userActionFlags` (bits 1–4, shifted from user mask)
- **i18n**: ~25 new keys for survey panel (en.json + de.json)
- **New files**: `surveyPattern.svelte.ts` (rune store), `surveyPatterns.ts` (geometry + generator), `SurveyPatternPanel.svelte` (UI), `SurveyPatternLayer.svelte` (map), `NumberStepper.svelte` (reusable component, replaces inline steppers in WeatherEditor)
- **Documentation**: `PatternGenerator.md` with full workflow and phased plan

### Added — Colored Flight Tracks in 3D Map View (Map3D.svelte)
- **Playback track color segmentation**: `updatePlaybackTrack3D()` now respects `trackColorMode` prop — Flight Mode, Altitude, Speed, Signal, and None modes render as multi-segment colored polylines in CesiumJS
- **Live trail flightmode coloring**: `updateTrail3D()` uses `classifyFlightMode()` for real-time trail color changes on flight mode transitions (matching Map.svelte behavior)
- **Trail reset on re-arm**: 3D trail clears on arm transition with valid GPS fix, same as 2D map
- Reuses existing `trackColors.ts` segmentation functions (`segmentTrackByFlightMode`, `segmentTrackByAltitude`, `segmentTrackBySpeed`, `segmentTrackBySignal`) — no duplication, no new abstraction needed
- Geoid correction applied to all track segment positions

### Added — CSS Grid Zone Layout System (ADR-023)
- **CSS Grid layout**: `.app` container uses a 4×4 named grid with 7 zones (Toolbar, Nav Rail, Panel Zone, Bottom Dock, Side Dock, Map Controls, Status Bar)
- **Layout store** (`src/lib/stores/layout.ts`): Layout profiles (`flight`, `mission`, `area-planner`), zone visibility toggles, CSS custom property overrides for dock sizes
- **Container-relative widget sizing**: Replaced viewport-based `vmin` CSS units with per-dock `px` sizing — `pxPerUnit = crossAxisPx / LARGE_BASE_VMIN` computed independently for each dock, fully decoupling bottom and side dock scaling
- **Panel Zone constraints**: Floating panels (Settings, UAV Info, Logbook, Mission) now derive `max-height` and `width` from grid zone variables — panels never overflow into bottom dock, side dock, or map controls
- **Zone padding**: 6px padding on dock zones keeps widgets from sitting flush against edges/status bar
- **Side dock max width**: Reduced from 300px to 250px (`clamp(150px, 15vw, 250px)`)
- **Debug overlay**: Dev-only dashed-border zone visualization showing grid area names and sizes
- Removed viewport resize listener (`winW`/`winH`/`vminPx`) — no longer needed

### Added — MAVLink Protocol Support (Phases 1–4)
- **ByteTransport trait**: Protocol-agnostic byte-level I/O trait extracted from existing transports; Serial, TCP, UDP, BLE all implement it
- **MspTransport wrapper**: MSP framing layer (`MspTransport`) now wraps `ByteTransport` instead of owning raw serial; clean separation of wire transport from protocol framing
- **MAVLink parser** (`mavlink_proto/parser.rs`): Byte-level state machine for MAVLink v1/v2 frames with CRC-Extra validation, `raw_bytes` capture for tlog recording
- **MAVLink codec** (`mavlink_proto/codec.rs`): MAVLink v2 frame serialization with CRC-Extra
- **MAVLink handshake** (`mavlink_proto/handshake.rs`): GCS heartbeat → FC heartbeat exchange, AUTOPILOT_VERSION request, FC info extraction (ArduPilot, PX4, INAV MAVLink)
- **MAVLink handler thread** (`mavlink_proto/handler.rs`): Continuous read loop with `AnalogState` accumulator, telemetry dispatch to identical Tauri events as MSP (7 event types), heartbeat writer (1 Hz)
- **Protocol dropdown in Toolbar**: UI selector for MSP / MAVLink with auto-baud switching (115200 for MSP, 57600 for MAVLink default)
- **ActiveProtocol enum** (`state.rs`): `Msp(SchedulerHandle) | Mavlink(MavlinkHandle)` — clean dual-protocol state management
- **MAVLink telemetry mapping**: HEARTBEAT, ATTITUDE, GPS_RAW_INT, GLOBAL_POSITION_INT, SYS_STATUS, RC_CHANNELS, VFR_HUD, BATTERY_STATUS, SCALED_PRESSURE → same TelemetryData as MSP; pitch negation (MAVLink up=+ → INAV down=+)
- **tlog logger** (`flightlog/tlog_logger.rs`): MAVLink `.tlog` binary format recording (Mission Planner / QGC compatible), `[u64 µs BE][raw frame]` per entry
- **Dual-protocol flight recorder**: `FlightRecorder` parameterized with `protocol: String` ("MSP"/"MAVLink"), creates `RawLogger` for MSP or `TlogLogger` for MAVLink
- **Continuous raw logging mode** (`raw_always`): Optional always-on raw recording from connect (pre-arm data included), DB only captures armed segments; loggers persist across arm/disarm cycles until disconnect
- **Continuous logging UI**: New "Continuous Raw Logging" toggle in Settings with i18n labels (en/de)

### Added — Settings & Logbook Enhancements
- **Separate Flight Recording / Flight Logbook toggles**: Recording (raw stream capture) and Logbook (SQLite database) are now independent settings — users can enable either or both (ADR-022)
- **Craft name inline editing**: Click ✎ button in LogbookPanel to edit craft name, confirm with Enter or blur, cancel with Escape
- **`flightlog_update_craft_name` Tauri command**: Persists user-edited craft name to `flights.craft_name` column
- **Blackbox import filter memory**: Last-used filter order (INAV vs ArduPilot) persisted in localStorage across sessions
- **Logbook tab conditional visibility**: Logbook tab hidden in NavRail when Flight Logbook is disabled
- **i18n updates**: "Flight Logging" split into "Flight Logbook" / "Flight Recording" labels (de + en)
- **DB schema v5**: `flights.craft_name` column for user-editable craft names (migration v4→v5)

### Added — Protocol Refactoring Plan
- **`docs/archive/PROTOCOL_REFACTORING.md`**: Comprehensive 5-phase MAVLink integration workstream document
- Architecture: ByteTransport trait + separate MspScheduler/MavlinkHandler modules
- Recording: MWP v2 Binary Capture (.raw) for MSP, standard tlog (.tlog) for MAVLink
- Firmware scope: ArduPilot + PX4 + INAV MAVLink

### Added — CesiumJS 3D Map View (M7)
- **CesiumJS integration**: Apache 2.0 licensed 3D globe renderer alongside existing Leaflet 2D map
- **Custom Vite plugin** (`cesiumPlugin()`): sirv middleware serves Cesium Workers/Assets in dev mode; `fs.cpSync` copies assets for production builds — replaced `vite-plugin-static-copy` (404 issues) and `vite-plugin-cesium` (path encoding bug with spaces)
- **2D/3D toggle button**: Switch between Leaflet and CesiumJS views (persisted preference)
- **Cesium Ion token support**: Settings panel password input for World Terrain access (ion.cesium.com)
- **Map provider sync**: 3D view uses same tile provider as 2D map with live switching support
- **IndexedDB tile cache**: Shared cache between 2D and 3D — overridden `requestImage` routes through `getCachedTile`/`putCachedTile`
- **Per-provider `cesiumMaxZoom`**: ESRI providers limited to zoom 17 in 3D to prevent "No tiles available" placeholders in sparse-coverage areas
- **Tile error handling**: `errorEvent` listener prevents render crashes; parent tiles remain visible for failed child tiles
- **World Terrain**: `Cesium.Terrain.fromWorldTerrain()` with vertex normals when Ion token is configured
- **Geoid undulation correction**: `sampleTerrainMostDetailed` at first track point computes WGS84 ellipsoid offset from GPS MSL altitude — fixes ~40m altitude error in Europe
- **Async terrain readiness**: `waitForTerrain()` awaits `terrainProviderChanged` event before sampling, avoids `"terrainProvider is required"` errors
- **UAV entity**: Colored point + SVG arrow billboard + "UAV" label, colored by flight mode flags
- **Home marker**: Green "H" point, `CLAMP_TO_GROUND` height reference
- **Live trail**: `CallbackProperty` polyline with 1m minimum distance filter
- **Playback track**: Static polyline from `TelemetryRecord[]` with geoid-corrected altitude
- **Playback marker**: Point + arrow billboard follows scrubber position with heading rotation
- **Chase camera**: Smooth follow mode with `requestAnimationFrame` lerp loop — exponential interpolation for position (lat/lon/alt) and heading (shortest-path angle wrap)
- **Chase UI**: "🎥 Follow" / "👁 Free" toggle button + range slider (50–2000m) + pitch slider (-90° to -5°)
- **Fog**: `density: 2.5e-4` hides distant terrain for performance
- **Performance**: `requestRenderMode`, `scene3DOnly`, `tileCacheSize: 100`, MSAA 2×
- `Map3D.svelte` component (~750 lines): full 3D view with all features above
- `mapProviders.ts`: added `cesiumMaxZoom` optional field to `MapProvider` interface
- `settings.ts`: added `cesiumIonToken` field to `AppSettings`
- `SettingsPanel.svelte`: Cesium Ion Token password input with signup link

### Added — Colored Flight Tracks & Mode Visualization
- **Track color modes**: Flight Mode, Altitude, Speed, Signal, None — selectable in LogPlayer dropdown
- **Flight mode track coloring**: Priority-based INAV bitmask classification (11 levels: Failsafe RTH → Acro)
- **Altitude track coloring**: Blue→green→yellow→red gradient, reference altitude from alerts settings (`warnAltitude`)
- **Speed track coloring**: Blue→red gradient scaled to max ground speed
- **Signal track coloring**: Green→red inverted gradient, prefers Link Quality over RSSI
- **"None" mode**: Single-color orange track (classic view)
- **Multi-segment rendering**: `L.layerGroup()` with merged polylines per color (typically 20–100 segments instead of 10k individual points)
- **LogPlayer track color dropdown** with 5 modes + dynamic legend (colored mode badges or gradient min/max bar)
- **Flight mode legend**: Shows only modes actually used in the loaded flight
- **UAV icon coloring by nav_state** (S7): UAV marker fill color changes based on INAV `MW_NAV_STATE_*` — Idle=blue, RTH=violet, PosHold=cyan, Landing=orange, Emergency=red, Landed=green
- **Live trail colored by flight mode** (S10): Real-time trail rendered as multi-segment colored polylines matching flight mode classification (same colors as playback track)
- `getNavStateColor()` function in `trackColors.ts` — maps 16 INAV nav states to icon colors
- `classifyFlightMode()` used for both playback track and live trail coloring
- Alerts settings group with `warnAltitude` (default 120 m) for altitude gradient reference
- `trackColors.ts` helper module: `TrackColorMode`, `FlightModeInfo`, `classifyFlightMode()`, `getGradientColor()`, `getSignalGradientColor()`, `segmentTrackByFlightMode()`, `segmentTrackByAltitude()`, `segmentTrackBySpeed()`, `segmentTrackBySignal()`, `getUsedFlightModes()`, `getNavStateColor()`
- Protocol reference doc: `docs/PROTOCOL_FLIGHT_MODES.md` — INAV bitmask vs ArduPilot enum comparison for future multi-protocol support

### Added — .kflight Data Exchange (M5)
- `.kflight` file format: self-contained SQLite database for sharing flight records between KiteGC installations
- Export: single or multi-flight export via Ctrl+click multi-select, includes all telemetry, blackbox records, and raw Blackbox BLOBs
- Import: file picker or drag & drop `.kflight` files into logbook, with duplicate detection (craft_name + start_time ±10s)
- `_kflight_meta` table in export files: schema version, app ID, export timestamp, flight count
- Export Blackbox: extract original raw binary file (.TXT/.bbl/.bfl) from `blackbox_files` BLOB
- `exchange.rs` module (~290 lines): `export_flights()`, `import_flights()`, `create_export_db()`, `copy_flight()`, `copy_blackbox_records()`, `copy_blackbox_files()`, `list_flights_in_file()`, `get_flight_from_file()`, `get_track_from_file()`
- New Tauri commands: `flightlog_export_kflight`, `flightlog_import_kflight`, `flightlog_export_blackbox`
- Frontend: `exportKflight()`, `importKflight()`, `exportBlackbox()` controller functions with native Save/Open dialogs
- Button layout: right-aligned button groups in logbook (Blackbox group | .kflight group) with gap between groups

### Added — Logbook Search & Multi-select (M5)
- Text search/filter field in logbook: filters by aircraft name, location, date across all group modes
- Ctrl+click multi-select for flights (multi-selection set, used by .kflight export)
- Flight source indicators in flight list: ◈ (blackbox only), ◉ (both), no prefix (live)

### Added — Weather at ARM Time (M5)
- Weather + reverse geocoding fetched at ARM time via `tauri::async_runtime::spawn` (non-blocking)
- Opens separate SQLite connection to avoid contention with recorder's batch writes
- Lazy fallback retained: `flightlog_geocode` and `flightlog_fetch_weather` Tauri commands for manual refresh

### Added — Telemetry Replay Pipeline (M5b)
- `telemetryAdapter.ts`: `toTelemetryData(TelemetryRecord → TelemetryData)` mapper for feeding DB records into live widgets during log replay
- Automatic live/replay switch: `$derived(telem)` selects between live telemetry store (connected) and adapter output (replaying)
- Home position automatically set from `flight.start_lat/lon` during replay, cleared on player close
- Compass uses GPS COG (`heading` column) for replay, with fallback to attitude `yaw`

### Fixed — Blackbox Import Data Quality (M5b)
- **AHI (roll/pitch)**: INAV blackbox attitude columns (`attitude[0]`, `attitude[1]`, `attitude[2]`) now resolved alongside `roll`, `pitch`, `yaw` — unconditional ÷10 conversion from decidegrees to degrees
- **Vario**: `gps_velned[2]` (NED down velocity in cm/s) now correctly negated and divided by 100 for m/s climb rate; fallback `vario` column also ÷100
- **Compass**: Adapter maps `heading` (GPS COG) for replay instead of attitude `yaw` (which may be decidegrees)
- **Home Distance**: `homePosition` store now set during replay from flight start coordinates

### Refactored — Frontend Modularization
- Frontend modularization completed: `+page.svelte` refactored to thin orchestrator (4 controllers + 1 adapter + helpers extracted)
- 4 controllers extracted: `connectionController.ts`, `logbookController.ts`, `playbackController.ts`, `widgetController.ts`
- 1 adapter: `telemetryAdapter.ts` (DB → widget data mapping)
- 1 helper: `helpers/telemetry.ts` (`isArmed()`, `hasKnownLocation()`, `isValidGpsCoordinate()`)
- 7 UI components extracted: `LogPlayer`, `LogbookPanel`, `SettingsPanel`, `Toolbar`, `UavInfoPanel`, `StatusBar`, `NavRail`

### Added — Blackbox Import & Playback (M5b)
- Blackbox import pipeline: `blackbox_decode` binary discovery (app folder first, PATH fallback), invoked with `--merge-gps --datetime --unit-height m --unit-gps-speed mps --stdout`
- Multi-log probing: `flightlog_probe_blackbox_logs` command tries indices 0–31 and returns all found logs with labels
- Import progress events: `flightlog-import-progress` Tauri event emitted at 9 stages (5–100%) during import
- Progress bar UI in Logbook tab shown during active import
- Multi-log selection: if the .TXT contains >1 session, user selects the desired log index before import starts
- CSV parsing performance overhaul: pre-built `HashMap<String, usize>` header index map resolves all column positions once — O(1) access per field per row (vs O(headers²) before)
- Downsampling to 10 Hz: reads `H looptime:` and `H P interval:` from the raw log header, computes effective sample rate, skips rows to keep ≤ 10 Hz in the DB (e.g. 500 Hz → keep 1 in 50 rows)
- Raw CSV lines stored in `blackbox_records.csv_data` (comma-joined) instead of full JSON re-serialization — significantly reduces parsing overhead
- Heading fix: INAV blackbox `heading` column is prioritised over `gps_ground_course`; auto-detects decidegrees (>360 → ÷10)
- `link_quality` field added to `TelemetryRecord` (0–100 %, maps `lq` / `link_quality` / `rxlq` from blackbox CSV; `None` for MSP live recordings)
- DB migration v3: `ALTER TABLE telemetry_records ADD COLUMN link_quality INTEGER`
- Log replay: track loaded into `selectedFlightTrack` on flight selection; orange dashed polyline rendered on map via `playbackTrack` prop
- Playback controls: Play/Pause/Reset buttons + scrubber timeline; timer-based at 120 ms/step
- Playback position marker: amber circle marker moves on map during playback
- `fitBounds` called once on new playback track load
- Wider logbook panel when a flight is selected: CSS `min()` responsive width, `nav-panel-wide` class adds ~560px extra width
- Improved logbook grid proportions (list/detail split)

### Added — Logbook UX Improvements (M5)
- Weather editor: compact read-only weather summary in flight detail + pencil edit icon that opens a weather editor form (temperature/wind steppers, wind direction/conditions dropdowns, save button)
- `flightlog_update_weather` Tauri command + `updateFlightWeather()` frontend store function
- Batch import: file picker allows multi-file selection for Blackbox logs (`.bbl`, `.bfl`, `.csv`, `.txt`)
- Drag & drop import: drop Blackbox files onto the logbook to import (Tauri `dragDropEnabled` + `tauri://drag-drop` listener)
- Logbook minimize/expand: click map → panel minimizes to 280px metadata-only view; click panel → expand back to full detail
- Notes auto-resize: textarea grows with content up to 140px, read-only in minimized mode
- Delete Flight button styled red for danger indication
- Duplicate flight detection dialog on import with force-import option
- Extended flight metadata: Firmware, Total Distance, Max Distance fields in detail panel
- All hardcoded UI strings replaced with i18n keys (duplicate dialog, import progress, weather edit title, status bar connection info)

### Added — Flight Recording & Logbook (M5, core)
- New Rust `flightlog` module: `db.rs`, `recorder.rs`, `raw_logger.rs`, `geocode.rs`, `weather.rs`, `types.rs`
- SQLite storage via `rusqlite` with bundled SQLite (no external SQLite install required)
- Migration system using `PRAGMA user_version` (schema v1)
- New `flights` and `telemetry_records` tables with flight metadata + sampled telemetry points
- Flight recorder integrated into scheduler loop (primary connection only), with ARM/DISARM transition detection from `MSPV2_INAV_STATUS` arming flags
- Automatic flight session start on ARM and finalize on DISARM
- Optional raw text log file output per flight (`raw_logs/*.txt`)
- New handshake step: `MSP_NAME` query; craft name now available in `FcInfo`
- New Tauri commands for logbook operations:
	- `flightlog_list`, `flightlog_get`, `flightlog_get_track`, `flightlog_delete`, `flightlog_update_notes`
	- `flightlog_geocode` (OSM Nominatim), `flightlog_fetch_weather` (Open-Meteo), `flightlog_default_db_path`
- Frontend store `src/lib/stores/flightlog.ts` for typed logbook command wrappers
- Settings integration:
	- `flightLoggingEnabled` (default OFF)
	- `flightLogDbPath` (custom folder or default AppData/portable path)
	- `flightLogRawEnabled` (default OFF)
- Settings UI enhancements:
	- Flight logging enable/disable toggle
	- Raw log toggle
	- Database folder picker using native directory dialog
- New Logbook tab with grouped sort modes:
	- Aircraft -> Location -> Date
	- Location -> Date -> Aircraft
	- Date -> Location -> Aircraft
	- Aircraft -> Date -> Location
- Flight detail panel with metadata, notes editing, and delete action
- English and German i18n keys for flight logging and logbook UI

### Tested — Flight Recording & Logbook
- Rust unit tests for DB schema + CRUD + telemetry batch + cascade delete (5 tests, all passing)
- `cargo check` successful
- `npm run check` successful (0 errors; existing warnings remain)

### Added — Mission Planning (M4)
- Mission module: Rust backend with `mission/types.rs`, `mission/codec.rs`, `mission/store.rs`
- Waypoint data model: all 8 INAV WP action types (Waypoint, PosholdUnlim, PosholdTime, RTH, SetPoi, Jump, SetHead, Land)
- MSP WP codec: `MSP_WP` (118) decode, `MSP_SET_WP` (209) encode, `MSP_WP_GETINFO` (20)
- MSP mission EEPROM: `MSP_WP_MISSION_SAVE` (18), `MSP_WP_MISSION_LOAD` (19)
- 13 Tauri commands: `mission_get`, `mission_clear`, `mission_add_wp`, `mission_update_wp`, `mission_remove_wp`, `mission_insert_wp`, `mission_reorder_wp`, `mission_download`, `mission_upload`, `mission_export_xml`, `mission_import_xml`, `mission_save_file`, `mission_load_file`
- 37 Rust unit tests covering codec, XML serialization, store operations
- Frontend mission store (`mission.ts`): Svelte writable stores, derived stores (`geoWaypoints`, `selectedWpIndex`, `editMode`), invoke wrappers
- MissionLayer.svelte: Leaflet map layer with SVG markers, polyline path, floating editor/labels
- MissionPanel.svelte: sidebar panel with WP table, detail view, FC/EEPROM/file controls
- Type-specific SVG marker icons: blue WP teardrop, orange PosHold circle with orbit ring, purple POI, orange Land teardrop with down-arrow, orange RTH house, grey generic fallback
- Floating editor popup per selected WP: type selector, altitude with REL/AMSL toggle, speed, hold time
- Floating parameter labels on non-selected WPs showing altitude and modifier summary
- Modifier WPs (Jump, RTH, SetHead) grouped into parent geo-WP editor popup
- Add/remove modifiers via dropdown in editor
- Display numbering skips modifier WPs (map markers + sidebar)
- Click-on-polyline to insert WP between existing waypoints
- Map click with editor open deselects WP instead of adding new
- Dashed lines for Jump (purple) and RTH (orange) modifiers on map
- WPs after first LAND/RTH greyed out (35% opacity, dashed grey polyline, non-draggable)
- Greyed WP rows in sidebar list (opacity + grayscale filter)
- FC Download / FC Upload buttons (RAM transfer)
- EEPROM Save / EEPROM Load buttons (save disabled when armed)
- Armed state detection via telemetry `armingFlags` (bit 2)
- File Open / Save via native OS file picker dialog (@tauri-apps/plugin-dialog)
- Drag & drop .mission file import
- MW XML format import/export (interoperable with INAV Configurator, mwp, ezgui)
- Max 120 WP sanity check on map click, polyline insert, and modifier add
- Warning text in modifier dropdown when WP limit reached
- WP count display (n/120) with dirty state badge
- Multi-mission support: dynamic tabs [1][+], up to 9 missions, 120 WP global limit across all missions
- Mission Control settings: Default WP Altitude (1–1000 m, default 50), Default PH Time (1–600 s, default 30), stepper +/− buttons
- Scrollable WP list with fixed (non-scrolling) control buttons at bottom
- Dark-themed scrollbars (custom WebKit styling + `color-scheme: dark`)
- Dark-themed number inputs and selects in editor popup
- Global `color-scheme: dark` on HTML root element

### Added — Installer & Portable Mode
- NSIS installer: install mode `both` — user chooses per-user (%LOCALAPPDATA%) or all-users (Program Files)
- NSIS uninstall hook: asks whether to remove application data (settings, map cache) from AppData
- Portable mode: place a `.portable` file next to the exe → all data stored in `data/` folder beside the binary
- Portable mode works on both Windows (WEBVIEW2_USER_DATA_FOLDER) and Linux (XDG_DATA_HOME/XDG_CONFIG_HOME)

### Added — Internationalization (i18n)
- `svelte-i18n` library with ICU Message Format for interpolation and plurals
- English locale file (`en.json`, ~200 translation keys across 18 namespaces)
- German locale file (`de.json`, complete translation)
- i18n initialization in `+layout.svelte` (blocks rendering until locale loaded)
- All 14 frontend component files converted: `+page.svelte`, `MissionPanel.svelte`, `MissionLayer.svelte`, `Map.svelte`, `DebugPanel.svelte`, 7 widget components
- Language picker in Settings panel (persists selection to localStorage)
- `WP_ACTION_KEYS` map in `mission.ts` for i18n-compatible waypoint action labels
- `labelKey` field in `widgetRegistry.ts` for translated widget names
- `locale` field in `AppSettings` with default `'en'`

### Fixed — Mission Planning (M4)
- Editor popup flicker on value edits: popup now on map (not layerGroup), direct DOM innerHTML update avoids Leaflet layout recalc
- Edit mode auto-disables when switching away from Mission tab or closing navigation panel

## [0.2.0] — 2026-04-15

### Added
- MSP scheduler: dedicated thread with priority-based adaptive polling
- Telemetry groups: Attitude (5 Hz), Status (1 Hz), Analog (1 Hz), GPS (2 Hz), Altitude (rotating)
- Configurable poll rates: Attitude 1–5 Hz, GPS Position 1–5 Hz
- Optional airspeed module (toggleable, rotates with altitude in secondary slot)
- Adaptive degradation: priority-based slot selection degrades low-priority groups first
- Live telemetry strip with real data: ALT, SPD, VARIO, BAT, SATS
- Aircraft position marker on map with heading arrow (SVG, rotates with yaw)
- Battery voltage/current/power display from MSPV2_INAV_ANALOG
- Arming status: pulsing ARMED widget in telemetry strip + status bar indicator
- GPS fix type display (NO GPS, NO FIX, 2D, 3D) + satellite count
- Sensor bar driven by MSP_SENSOR_STATUS (151) hardware health values
- Sensor bar states: green=OK, yellow=warning (GPS no fix), red=unhealthy, gray=none
- UAV platform type detection via MSP2_INAV_MIXER (0x2010) handshake step
- Platform type display in UAV Info panel (Multirotor, Airplane, Helicopter, etc.)
- MSP Debug Monitor panel (dev builds only, toggled via 🔧 Debug button in status bar)
- Debug: per-message LED indicators (yellow=request, green=response, red=timeout)
- Debug: MSG/s and bytes/s throughput counters (TX/RX)
- Debug: target rate vs actual rate per MSP code with throttle highlighting
- Debug: POLL/INIT status badges, request/response/timeout counters
- Debug module: zero-cost in release builds (`#[cfg(debug_assertions)]` + no-op stub)
- Settings: attitude rate, position rate, airspeed toggle (persisted in localStorage)

### Changed
- Floating navigation panel: hamburger menu opens tab rail + content panel
- Tab-based panel navigation: UAV Info, Settings, Mission Control
- Bottom telemetry overlay strip replaces placeholder widgets
- Animated hamburger icon (transforms to X when open)
- Glassmorphism UI elements (backdrop-filter blur, semi-transparent panels)
- Session persistence: panel state, active tab, telemetry rate settings
- Map zoom controls repositioned to top-right
- `MspRc` feature gate (INAV 8.0+), `AuxRc` feature gate (INAV 9.1+)
- Feature gate system: removed `MultiMission` (irrelevant, pre-7.0)

### Fixed
- MSPV2_INAV_ANALOG decode: correct byte offsets (batteryFlags:u8, vbat:u16, amperage:i16, power:u32, ...)
- GPS fix type mapping: added missing case 0 (NO GPS)
- MSPV2_INAV_STATUS decode: correct offsets for sensorStatus, cpuLoad, armingFlags
- Sensor bar: uses actual FC sensor health instead of connection state
- BARO indicator: uses hardware sensor status instead of altitude ≠ 0 heuristic
- Map resize handling on panel toggle transitions

## [0.1.0] — 2026-04-15

### Added
- Initial project setup with Tauri 2.0 + Svelte 5 + TypeScript
- Modular Rust backend structure (msp, transport, commands)
- MSP v1/v2 codec with encode/decode and unit tests
- MSP streaming parser (byte-by-byte state machine)
- Serial transport with cross-platform port listing
- Tauri IPC commands: `list_serial_ports`, `connect`, `disconnect`, `get_app_version`
- MSP handshake: API_VERSION, FC_VARIANT, FC_VERSION, BOARD_INFO
- INAV version parsing with minimum version check (≥ 7.0)
- Version-dependent feature gating (CoreTelemetry, AutolandConfig, Geozones)
- Svelte frontend with dark-themed GCS layout (INAV Configurator color scheme)
- Reactive stores for connection, telemetry, and settings state
- Leaflet map integration with OpenStreetMap tiles
- Connection status display with sensor bar
- GPLv3 license
- Build scripts for Windows and Linux
- Development documentation (DEVLOG, CHANGELOG, ARCHITECTURE, ROADMAP)
