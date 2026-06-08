# Kite Ground Control — Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
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
- _Planning + remaining work in `docs/active/Map3DRework.md`: the live-trail curtain is deferred to simulator long-flight tests; the FPV cockpit view + follow tuning are the remaining Phase-3 items. (The clean terrain-derived geoid offset is now done — see the Fixed entry above.)_

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
