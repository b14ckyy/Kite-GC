# RF Link / Radio-Shadow Analysis — Plan

**Status:** **Phase 1 shipped (2026-06-15).** Realises **Feature 4 (LOS analysis)** of
[TerrainFeatures.md](TerrainFeatures.md), expanded from naïve line-of-sight into a layered RF
propagation analysis (LOS occlusion + Fresnel/diffraction + two-ray ground reflection) rendered
**inside the existing Terrain Analyzer**. Phase 2 (link budget / range) deferred.
**Created:** 2026-06-15

> The "Confirmed design decisions" + "Computation core" below are the original plan. See
> **§As-built (Phase 1)** at the end for what actually shipped and where it deviates.

---

## Goal

For a mission (planning) or a flown track (replay/live), show **where terrain degrades or blocks the
radio link** between the launch point and the aircraft, as a **background "rainbow" loss field** in
the Terrain Analyzer's profile chart — green (good) → yellow → red (blocked / ≤ −18 dB). Phase 1 is
**obstacle/loss only** (relative dB); an absolute link budget / range estimate comes later.

### Why beyond naïve LOS
Simple geometric LOS is the floor, not the answer. Two terrain-driven interference mechanisms matter
and are computable from DEM + frequency:
- **Diffraction** — terrain intruding the **Fresnel zone** loses signal even when the straight ray is
  clear (60 %-clearance rule; knife-edge loss when blocked).
- **Two-ray ground reflection** — a ground-bounced ray interferes with the direct ray → the classic
  lobing pattern (nulls/peaks vs distance & altitude). **Empirically dominant and predictable**: at
  the grazing angles of long-range flight the reflection coefficient `Γ → −1` for *both*
  polarisations regardless of ground material, so the lobe *positions* come almost purely from
  geometry + frequency (validated by real RSSI-vs-distance logs — clean over water, clearly visible
  over land). See the discussion + ADR-050-era notes; physics refs in §Sources of this plan's commit.

Honest accuracy: geometry (LOS, Fresnel clearance) is reliable to DEM resolution; diffraction dB has
~9 dB scatter vs reality; two-ray *lobe positions* are good, *null depth* is approximate (worse at
5.8 GHz). The tool presents losses, the operator judges reliability.

---

## Where it lives (existing code — no new panel)

The Terrain Analyzer already has everything we build on (`docs/active/TerrainFeatures.md`):
- `components/terrain/TerrainProfileChart.svelte` — hand-rolled SVG chart. X = cumulative ground
  distance (route/track, "unrolled"); Y = altitude MSL. Terrain fill, path line, clearance colouring,
  zoom/pan (SVG viewBox on X), hover/pin cursor, visible-slice decimation.
- `helpers/terrainProfile.ts` — builds `ProfileData` (`terrain[]` samples with `lat/lon/dist/elev`,
  sparse `path[]`, per-sample path MSL, jump `cuts`) from the Rust `terrain_profile` command.
- `helpers/terrainCorrection.ts`, `stores/terrainAnalysis.ts` (in-memory session params),
  `components/terrain/TerrainCursorLayer.svelte`.
- Modes already present: **Waypoint (mission)** + **Track (live/replay)** — both feed one render
  pipeline. No new input mode needed.
- Backend `src-tauri/src/terrain/mod.rs`: `terrain_elevation`, `terrain_profile(points, spacing)`,
  `terrain_fan(...)` over the Copernicus GLO-30 tile cache (MSL ≈ EGM2008, so home/UAV/terrain
  altitudes are directly comparable — no datum conversion).

---

## Confirmed design decisions

**D1 — Compute radially, display on the unrolled track.**
The link analysis is intrinsically **radial from the launch point**. Compute terrain radials in
**1° azimuth bins** from home; only bins the route/track actually occupies are sampled, each once out
to its farthest point in that bin. The expensive part (DEM sampling) is thus bounded (≤ one radial per
occupied degree) and reused for every point sharing that bearing.
- **Attenuation is computed over distance + azimuth from home**, but **displayed over cumulative
  track/mission distance** (the existing X-axis). Out-and-back / zig-zag points sharing a bearing get
  the same colour at different X positions — awkward on the map, natural on the unrolled chart.
- Consequence (expected): the coloured stripes get **wider with distance, narrower near home** (1° =
  more arc length far out). Fine for now; visual polish later.

**D2 — Method toggles + band selector (left param column).**
Independently enable, via **ON/OFF buttons** (not switches): **LOS blocking** (straight-ray occlusion,
the naïve view) · **Fresnel/diffraction** · **Two-ray reflection**. **Frequency band** is a
**single-select `SegmentedToggle`** (`components/panel/SegmentedToggle.svelte`) with the four bands
labelled bluntly by frequency — **5.8 GHz · 2.4 GHz · 900 MHz · 433 MHz** — exactly one active (drives
λ). All enabled effects are combined into one excess-dB value per sample (see D3).

**D3 — Combination + colour scale.**
- **Two-ray is signed** — include constructive gains (up to ~+6 dB), not just nulls; the operator
  weighs reliability. Obstacle mechanisms are losses only (≥ 0).
- **No double-count of the obstacle term:** "LOS blocking" (binary: 0 dB clear / blocked) and
  "Fresnel/diffraction" (continuous knife-edge loss) model the *same* mechanism. Rule: **when
  Fresnel/diffraction is enabled, the LOS-blocking toggle is ignored** (diffraction already encodes
  blockage as a large continuous loss). LOS-only = the naïve mode for comparison. **Two-ray adds on
  top** (a different mechanism).
- **Colour scale:** green → yellow → red. **Red = geometrically blocked OR total ≤ −24 dB.** Green
  end anchored at **0 dB** for now; if 0 dB is effectively never reached, clip the green end to
  **−3 dB**. Values ≥ 0 dB (incl. two-ray peaks) clamp to green.
- The field is rendered **pale / darkened** so it never overpowers the terrain and clearance lines.

**D4 — Rendering: background rainbow.**
Drawn **behind** the terrain fill and clearance lines, as a **vertical-stripe area fill with
horizontal gradient** between adjacent samples; each stripe's colour = that sample's combined dB.
**Top-bounded by the path line** (mission altitude in Waypoint mode, flight track in Track mode), from
the chart baseline up. Colour is per-X (constant vertically within a stripe) — a 1-D colour sequence
extruded vertically, not a 2-D field.

**D5 — New clearance line.**
Add a line for the **line-of-sight clearance**: at each sample P, the *minimum* clearance of the
home→P sightline above the terrain it passes over (the binding ridge; negative = blocked). This is
**in addition** to the existing "terrain directly under the UAV" line, and complements the rainbow
(the rainbow says *how bad*, the line says *by how much the ray clears/violates*).

**D6 — RF power / range = later.**
Phase 1 is relative obstacle loss only. A configurable **link budget** (TX power, antenna gains, cable
loss, RX sensitivity, per band) turning the excess-dB field into **absolute margin / predicted range**
is Phase 2.

---

## Computation core

Per occupied 1° azimuth bin: `terrain_profile([home, farthestPointInBin], 30 m)` → a radial terrain
profile (reuse the existing command; a batched `terrain_radial` command is a possible later
optimisation). Then per chart sample P (already has `lat/lon/dist/elev` + interpolated path MSL):

1. **Geometry** — straight home→P line with Earth-curvature correction (k = 4/3). Clearance at each
   profile step = ray height − terrain (+ effective-earth bulge).
2. **LOS blocking** — any step with clearance < 0 → blocked.
3. **Fresnel** — first-zone radius `r₁ = √(λ·d₁·d₂/d)`; clearance ratio = clearance / r₁; worst over
   the profile (60 % rule onset).
4. **Diffraction** — knife-edge Fresnel-Kirchhoff parameter `v`; ITU-R P.526 single-edge loss;
   multi-obstacle via Bullington/Deygout. Encodes "blocked → large loss" continuously.
5. **Two-ray** — fit a reflecting plane to the radial (over water/flat = exact; rugged = dominant
   facet / effective heights, else fall back to diffraction-dominated). `Δ ≈ 2·h_t·h_r/d`,
   `Γ ≈ −1` at grazing, relative gain `= 20·log₁₀|1 + Γ·e^{j·2π·Δ/λ}|`. Beyond the breakpoint
   `d > 4·h_t·h_r/λ` → monotonic. (`h_t` = UAV MSL − reflecting-plane MSL, `h_r` = launch height.)
6. **Combine** per D3 → excess dB → colour.

**Frequency band presets** (λ): 5.8 GHz ≈ 5.2 cm · 2.4 GHz ≈ 12.5 cm · 900 MHz ≈ 33 cm · 433 MHz ≈
69 cm. Lower band → larger Fresnel zone (needs more clearance) but diffracts better; higher band →
smaller zone, sharper shadows, finer two-ray fringes.

---

## Honest limits (surface in the UI)
- **Near field (~first km)** isn't modelled — manoeuvring, body shadowing, antenna angle dominate.
- **Null depth** is approximate (rough ground decorrelates the reflection, esp. 5.8 GHz); lobe
  *positions* are the trustworthy part.
- **GLO-30 is ~30 m DSM** — fine clutter (single trees, masts, the pilot) isn't in it.
- **Antenna pattern + aircraft attitude** (banking in turns) is a major real factor with **no terrain
  component** — out of scope; the tool models the *terrain* contribution only.

---

## Phasing

1. **Phase 1 — obstacle/loss rainbow (this plan):** radial 1° core; LOS + Fresnel/diffraction +
   two-ray (signed); method toggles + band selector; background rainbow (D4); LOS-clearance line (D5);
   relative dB, no link budget. **Logged-RSSI line:** in **Track mode**, when the analysed log carries
   RSSI, draw it as an extra line on the chart (predicted rainbow vs measured RSSI side by side — the
   mLRS-style out-and-back curve, auto-generated). It's just another line graph, so it rides along with
   this phase rather than being a separate milestone.
2. **Phase 2 (later) — link budget / range:** per-band TX/gain/RX presets → absolute margin + predicted
   range; "first link-critical point at X".

---

## Touch list (Phase 1) — shipped

| File | Change | Status |
|---|---|---|
| `src/lib/helpers/rfLink.ts` *(new)* | Radial-bin orchestration + per-sample RF math (LOS/Fresnel/diffraction/two-ray → excess dB), `computeRfField`, `rfColor`, clutter offset | ✅ |
| `src/lib/stores/terrainAnalysis.ts` | Session params: `rfLos` / `rfFresnel` / `rfTworay`, `rfBand`, `rfClutterM` (10 m) | ✅ |
| `src/lib/helpers/terrainProfile.ts` | `rssi` on `TrackPoint` + `ProfileData`; per-terrain-sample RSSI (nearest fix) in `buildTrackProfile` | ✅ |
| `src/lib/components/terrain/TerrainProfileChart.svelte` | Background rainbow gradient (behind terrain/path) + LOS-clearance line (AGL) + logged-RSSI line (Track) | ✅ |
| `src/lib/components/terrain/TerrainAnalysisPanel.svelte` | ON/OFF method buttons + band `SegmentedToggle` (`full`) + clutter `UnitStepper` + RF compute `$effect` + home resolution | ✅ |
| `src/lib/i18n/locales/{en,de}.json` | Method/band/clutter/RSSI labels + disclaimers | ✅ |
| `src-tauri/src/terrain/mod.rs` | *(not needed)* — per-bin `terrain_profile` reuse was fast enough; no new command | — |

---

## As-built (Phase 1)

What actually shipped, and where it deviates from the plan above (refinements found during
implementation + live validation against real logs):

**Combination rule (final, supersedes D3's wording).** Geometric blocking is treated as
**fundamental**: a shadowed sample is **red under any active method** — the ground cannot reflect *up
to* a blocked point, so two-ray cannot rescue it.
- *LOS only:* clear → 0 dB; blocked → red.
- *Fresnel/diffraction on:* the continuous knife-edge loss governs (covers blockage as a finite,
  realistic loss); the LOS-blocking toggle is ignored (D3).
- *Two-ray:* added **only where LOS is clear**; on a blocked sample it contributes nothing, and if no
  Fresnel model is active the sample is forced to the blocked floor (red). Signed (nulls + up to +6 dB).

**Colour scale = 0 → −18 dB** (green→red), not −24 (tightened after validation: small real losses were
washed out as green). Two-ray null clamp and the geometric-block floor map to red.

**D7 — Clutter / vegetation offset (added).** A `rfClutterM` parameter (default **10 m**, `UnitStepper`)
adds a flat height to bare terrain in the obstacle analysis (forest / small buildings) — GLO-30
under-represents canopy, and via the knife-edge `1/d₁` term a near forested ridge kills the link far
earlier than bare terrain predicts. **The home/GCS endpoint is raised by the same offset** (the operator
launches from a clearing, antenna above local vegetation) — otherwise the clutter-raised terrain beside
the GCS would block every sample. Clutter is **not** applied to two-ray (bare-ground reflection).

**Home (GCS) reference.** **Track mode → the track's first fix** (the actual take-off), *not* the
mission `launchPoint` (a persistent planning store that is often stale). Waypoint mode → `launchPoint`.
Home ground = `terrain_elevation(home)`.

**Two-ray model (as built).** Flat-earth far-field `Δ ≈ 2·h_t·h_r/D`, `Γ = −1` (grazing), with a
**fixed GCS antenna height of 2 m** (`GCS_ANTENNA_M`) and `h_t` = UAV height above local ground — not a
fitted reflecting plane. Good for the long-range/grazing regime it targets. The fitted-plane / Deygout
multi-obstacle refinements from the plan were **not** needed for Phase 1 (single worst knife-edge).

**RSSI overlay.** In Track mode, per-terrain-sample RSSI (nearest fix by cumulative distance) drawn as a
line **normalised to the visible plot height** (shape overlay, rescales on zoom) — enough for pattern
comparison vs the predicted rainbow.

**Validation (2026-06-15).** Matched real RSSI well for a loiter behind a slope; long-range patterns
track the two-ray lobing. Confirmed the model is a **risk indicator, not an exact RSSI predictor** —
near-field, antenna pattern/attitude, and per-pixel canopy height are out of scope.

**Parked for later (separate steps):** GCS antenna height as a parameter; fixed vs. window-normalised
RSSI scale; differentiated clutter layer; **Phase 2** link budget / absolute range.
