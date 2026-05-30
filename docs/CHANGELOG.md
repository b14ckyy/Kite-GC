# Kite Ground Control — Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
- **`docs/PROTOCOL_REFACTORING.md`**: Comprehensive 5-phase MAVLink integration workstream document
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
