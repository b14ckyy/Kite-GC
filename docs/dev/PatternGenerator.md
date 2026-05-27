# Survey Pattern Generator – Workflow & Implementation Plan

**Status**: In Progress — Phase 1+2 Complete  
**Last Updated**: 2026  
**Responsible**: Grok + User  

---

## 1. Goals & Scope

The goal is to add a **Survey / Pattern Generator** to the existing unified Mission Planner.

### Core Requirements
- Allow the user to define geometric patterns (Rectangle, Rectangle Lawnmower, Polygon, Polygon Lawnmower, Circle, Spiral).
- Generate standard waypoints from these patterns.
- The generator must be **platform-agnostic** (works for INAV, ArduPilot, PX4).
- Excellent map-based editing experience with real-time preview.
- Clean integration into the existing Mission Planner without breaking current behavior.

### Out of Scope (for now)
- Terrain / Ground Clearance analysis
- 3D rendering of missions
- Re-editing of applied patterns
- Multi-aircraft or cross-mission pattern generation

---

## 2. UI Integration

### Button Placement
- The button is named **"Pattern"**.
- It appears **only when Edit Mode is active**.
- Position: Immediately to the right of the **EDIT** button (left-aligned group).
- The two DELETE buttons (Waypoint Delete + Mission Delete) remain right-aligned and are unaffected.

### Behavior
- When the user clicks **"Pattern"**, the area below the button row (currently the waypoint list) switches to the **Pattern Generation UI**.
- While in Pattern mode, the normal waypoint list and manual editing are hidden.
- After pressing **"Generate"**, the system returns immediately to normal mission editing mode.
- Generated waypoints are **appended** to the currently active mission (INAV multi-mission behavior: only the active mission is ever modified).

### Mode Protection
- The Pattern button is strictly hidden when Edit Mode is disabled.
- This maintains the existing rule that a mission is protected from accidental changes when not in edit mode.

---

## 3. Pattern Generation UI (First Version)

### Shape Selection
- Dropdown with the following options (starting with **Rectangle**):
  - Rectangle
  - Rectangle Lawnmower
  - Polygon
  - Polygon Lawnmower
  - Circle
  - Spiral

### Core Parameters (per shape)
Parameters are split into separate interfaces per shape type.

**Common parameters (all shapes):**
- Orientation (heading of the pattern)
- Base Altitude
- Base Speed
- Line Spacing / Distance between legs
- Turn Distance (Fixed Wing Turn Zone) – see section 6

**Rectangle / Rectangle Lawnmower:**
- Center Position
- Length / Width
- Orientation

**Circle / Spiral:**
- Center Position
- Radius (preferably editable via drag handle on the map)

**Polygon:**
- List of corner points (defined by dragging on the map)

### Editing Philosophy
- The majority of editing happens **directly on the map** (draggable points, radius handle, etc.).
- UI fields exist only where map interaction is not practical.
- Number inputs follow the project’s established stepper pattern (see `WeatherEditor.svelte` and the `wpe-num-ctrl` popups in the mission layers) for visual consistency with the rest of the application (dark theme, hidden native spinners, +/- buttons).
- The “Generate” action will use the project’s custom `ConfirmDialog` component (not native `alert()` / Windows popup) once real generation logic is implemented in Phase 2.

---

## 4. Map Visualization & Interaction

### Shape Display
- The defined shape (Rectangle, Circle, Polygon, etc.) is shown as a **gray, semi-transparent area**.
- The shape is **exactly** what the user defined. It is **never** visually enlarged by the Turn Distance.

### Path Preview
- The generated survey legs / path are shown in real time on the map.
- Changes to parameters update the preview immediately.

### Interactive Elements (Map)
- Corner points of the shape are draggable.
- Center point is draggable.
- For **Circle**: A radius handle should be available for direct dragging (preferred). If technically too complex in the first iteration, a numeric input in the UI is acceptable as fallback.
- All changes on the map update the parameters in the UI in real time (two-way binding).

### Turn Zone Visualization (Fixed Wing)
- The Turn Distance affects **only the generated legs**, not the shape.
- Outbound legs are visually extended beyond the shape boundary.
- This creates a "sawtooth" appearance at the edges of the survey area in the preview.
- The actual shape polygon/circle remains unchanged.

---

## 5. Generate Flow & Validation

### Generate Button
- Located inside the Pattern UI.
- Triggers a confirmation dialog (using the existing unified `ConfirmDialog` helper).

### INAV 120 Waypoint Limit Check
- Before final generation, the system calculates the number of waypoints the current pattern would produce.
- If the total would exceed 120 waypoints (INAV limit):
  - Show a warning via the unified dialog system.
  - Inform the user that the mission would be truncated.
  - Offer the choice to:
    - Stay in Pattern mode and adjust the pattern, **or**
    - Proceed anyway (accepting truncation).

### Post-Generation Behavior
- After successful generation, the system immediately exits Pattern mode.
- The user is returned to the normal waypoint list of the active mission.
- The generated waypoints are appended at the end of the current mission.

---

## 6. Fixed Wing Turn Zone (Turn Distance)

### Purpose
Fixed-wing aircraft require space to turn. Without extension, they would turn inside the survey area.

### Behavior
- The user can define a **Turn Distance** (in meters).
- This value extends the outbound leg beyond the shape boundary.
- The aircraft is expected to turn outside the survey area and re-enter the next leg at the correct angle once it has stabilized.
- Visually this produces the sawtooth pattern at the edges.

### Scope
- Only relevant for line-based patterns (Rectangle, Rectangle Lawnmower, Polygon, Polygon Lawnmower).
- Less or not relevant for pure spiral patterns.

### UI
- Numeric input field for the distance in meters.
- Visual extension of the legs in the map preview (as described in section 4).

---

## 7. Data Model & State Management

### Recommended Structure

- **Temporary Pattern State**: Stored in a dedicated Svelte 5 rune store (e.g. `surveyPattern.ts`).
- **Generation Logic**: Pure functions in a single helper file (e.g. `src/lib/helpers/surveyPatterns.ts`).
- The pattern configuration uses **separate interfaces** per shape type (discriminated union recommended).

### Lifecycle
- Pattern state exists in a dedicated Svelte 5 rune store (`surveyPattern.svelte.ts`).
- While in Pattern mode: `isActive = true`, config holds current parameters.
- Exiting Pattern mode: `isActive = false`, **config is preserved** so re-entering shows the same parameters.
- Only app close resets the config.
- On "Generate" the state is converted into normal `Waypoint` objects and pushed into the active mission via the existing mission API.

### No Re-Editing After Generation
- Once waypoints have been generated and added to the mission, there is no automatic way to reactivate the original shape parameters.
- Reason: Users can freely move individual waypoints afterward, making reverse-engineering unreliable and overly complex.

---

## 8. Phased Implementation Plan

### Phase 0 – Documentation (Current)
- Create this complete workflow document.
- User reviews and can request changes.

### Phase 1 – UI + Shape Visualization (Rectangle first)
- Pattern button integration (only visible in Edit mode).
- Switching into Pattern UI.
- Basic Rectangle shape definition (Center, Length, Width, Orientation).
- Map rendering of the gray shape area.
- Draggable center + corner points with real-time updates.
- Basic parameter panel.
- "Generate" button + confirmation flow (even if it only creates dummy/placeholder waypoints for now).

**Goal of Phase 1**: User can define and visually adjust a rectangle on the map and trigger the generation flow.

### Phase 2 – Waypoint Generation Algorithm
- Actual algorithm that turns the shape + parameters into real waypoints.
- First implementation for Rectangle (including Turn Distance logic).
- Appending generated waypoints to the active mission.

### Phase 3 – Further Shapes
- Rectangle Lawnmower
- Polygon
- Polygon Lawnmower
- Circle + Spiral

### Phase 4 – Advanced Features (later)
- Terrain-relative / Ground Clearance missions
- 3D rendering of missions
- More sophisticated survey types

---

## 9. Technical & Architectural Decisions

- Survey Pattern Generator is **platform independent**.
- It always generates standard waypoints.
- The existing unified Mission Planner (with its platform-specific adapters) remains responsible for limits, supported waypoint types, and final validation.
- All map interaction for patterns happens only while in Pattern mode.
- We reuse the existing `ConfirmDialog` system for all warnings and confirmations.
- INAV 120 WP limit is checked at generation time with user override option.

---

## 10. Open Topics / Future Considerations

- Full 3D preview of generated patterns (later)
- Terrain following / ground clearance analysis (later)
- Possibility to save/load survey patterns as reusable templates (future)
- Better visual distinction between survey-generated legs and manually added waypoints (future)
- Kurs/turn preview based on actual UAV configuration (explicitly deferred)

---

## 11. Current Open Questions (as of document creation)

None critical at this point. All major decisions have been captured above.

---

**Document Purpose**  
This document serves as the single source of truth for the Survey Pattern Generator feature. Any implementation should follow the rules and flows described here unless explicitly changed and documented.

---

*End of Document*