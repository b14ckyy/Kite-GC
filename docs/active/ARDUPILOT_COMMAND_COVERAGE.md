# ArduPilot Mission Command Coverage

> **Raw reference (dev) — destined for user docs.** Which ArduPilot/MAVLink mission commands Kite offers
> in the editor, which it omits, and why. Status reflects the curated catalog in
> `src/lib/helpers/arduCommandCatalog.ts`. See `ARDUPILOT_WAYPOINT_ARCHITECTURE.md` for the model + the
> curation rationale. **Params/ids still want ArduPilot-operator review (⚠).**
>
> _Last updated: 2026-06-14._

## Legend

| Status | Meaning |
|---|---|
| ✅ **Supported** | Selectable in the editor; friendly name + editable params + tooltips. |
| 🟡 **Round-trip only** | Not offered in the picker (legacy — a newer preferred command exists), but **preserved** on download / `.waypoints` load and re-upload. Renders raw (no friendly editor). |
| ⛔ **Excluded** | Not a mission item (runtime / camera-video-protocol / bench command) — ArduPilot does not execute it from a mission even if a GCS lists it. |
| ⏳ **Candidate** | Real mission command, niche/specialised — not yet added; could be on request. |

**Curation principle:** ArduPilot does not formally deprecate commands (legacy + modern coexist).
"Legacy" is *our* call — a command is 🟡 when a newer preferred command does the same job.

---

## Navigation (`NAV_*`)

| ID | MAV_CMD | Status | Vehicles | Note |
|---|---|---|---|---|
| 16 | NAV_WAYPOINT | ✅ | all | core |
| 17 | NAV_LOITER_UNLIM | ✅ | air | |
| 18 | NAV_LOITER_TURNS | ✅ | air | |
| 19 | NAV_LOITER_TIME | ✅ | air | |
| 20 | NAV_RETURN_TO_LAUNCH | ✅ | all | |
| 21 | NAV_LAND | ✅ | air | |
| 22 | NAV_TAKEOFF | ✅ | air | position anchored on home (no real coord) |
| 30 | NAV_CONTINUE_AND_CHANGE_ALT | ✅ | Plane | |
| 31 | NAV_LOITER_TO_ALT | ✅ | air | |
| 82 | NAV_SPLINE_WAYPOINT | ✅ | Copter | |
| 83 | NAV_ALTITUDE_WAIT | ✅ | Plane | balloon/launch wait |
| 84 | NAV_VTOL_TAKEOFF | ✅ | QuadPlane | |
| 85 | NAV_VTOL_LAND | ✅ | QuadPlane | |
| 92 | NAV_GUIDED_ENABLE | ✅ | air | hand control to a companion |
| 93 | NAV_DELAY | ✅ | air | |
| 94 | NAV_PAYLOAD_PLACE | ✅ | Copter/VTOL | |
| 213 | NAV_SET_YAW_SPEED | ⏳ | Rover | Rover heading+speed; niche ⚠ |
| 42702 | NAV_SCRIPT_TIME | ⏳ | all | Lua mission scripting hook |
| ⚠ | NAV_ATTITUDE_TIME | ⏳ | Copter | GPS-free attitude hold; newer FW; id verify |

## Conditional (`CONDITION_*`)

| ID | MAV_CMD | Status | Note |
|---|---|---|---|
| 112 | CONDITION_DELAY | ✅ | |
| 113 | CONDITION_CHANGE_ALT | 🟡 | superseded by NAV_CONTINUE_AND_CHANGE_ALT / NAV_LOITER_TO_ALT ⚠ |
| 114 | CONDITION_DISTANCE | ✅ | |
| 115 | CONDITION_YAW | ✅ | |

## Flow control / Jump

| ID | MAV_CMD | Status | Note |
|---|---|---|---|
| 177 | DO_JUMP | ✅ | jump by mission-item index |
| 600 | JUMP_TAG | ✅ | named marker |
| 601 | DO_JUMP_TAG | ✅ | jump by tag (survives mission edits) |

## DO — flight / speed / home

| ID | MAV_CMD | Status | Note |
|---|---|---|---|
| 176 | DO_SET_MODE | ⛔ | runtime mode change, not a mission item |
| 178 | DO_CHANGE_SPEED | ✅ | |
| 179 | DO_SET_HOME | ✅ | |
| 180 | DO_SET_PARAMETER | ⛔ | deprecated in MAVLink |
| 191 | DO_GO_AROUND | ⏳ | Plane go-around; could add ⚠ |
| 192 | DO_REPOSITION | ⛔ | guided/runtime reposition |
| 193 | DO_PAUSE_CONTINUE | ⛔ | runtime pause/continue |
| 194 | DO_SET_REVERSE | ⏳ | Rover reverse; niche |
| 210 | DO_INVERTED_FLIGHT | ✅ | Plane |
| 212 | DO_AUTOTUNE_ENABLE | ✅ | |
| 215 | DO_SET_RESUME_REPEAT_DIST | ✅ | mission rewind-on-resume |
| 218 | DO_AUX_FUNCTION | ✅ | trigger an RC aux function |
| 221 | DO_GUIDED_MASTER | ⛔ | obscure/legacy |
| 222 | DO_GUIDED_LIMITS | ✅ | |
| 223 | DO_ENGINE_CONTROL | ⏳ | ICE engine start/stop; could add |
| 224 | DO_SET_MISSION_CURRENT | ⛔ | runtime |
| 185 | DO_FLIGHTTERMINATION | ⛔ | runtime safety |
| 209 | DO_MOTOR_TEST | ⛔ | bench/runtime test |
| 216 | DO_SPRAYER | ⏳ | sprayer payload; niche |
| 217 | DO_SEND_SCRIPT_MESSAGE | ⏳ | Lua scripting |

## DO — outputs

| ID | MAV_CMD | Status | Note |
|---|---|---|---|
| 181 | DO_SET_RELAY | ✅ | |
| 182 | DO_REPEAT_RELAY | ✅ | |
| 183 | DO_SET_SERVO | ✅ | |
| 184 | DO_REPEAT_SERVO | ✅ | |
| 211 | DO_GRIPPER | ✅ | Copter/VTOL |

## ROI / Camera / Gimbal

| ID | MAV_CMD | Status | Note |
|---|---|---|---|
| 195 | DO_SET_ROI_LOCATION | ✅ | modern ROI |
| 196 | DO_SET_ROI_WPNEXT_OFFSET | ⏳ | ROI offset from next WP; niche ⚠ |
| 197 | DO_SET_ROI_NONE | ✅ | clears ROI |
| 201 | DO_SET_ROI | 🟡 | legacy ROI (→ 195/197) |
| 202 | DO_DIGICAM_CONFIGURE | 🟡 | camera **settings** (shutter speed / aperture / ISO) — legacy, round-trip only (decision: not in picker) |
| 203 | DO_DIGICAM_CONTROL | ✅ | camera **trigger** (Shutter) + zoom/focus/session/ids under **Advanced** |
| 204 | DO_MOUNT_CONFIGURE | 🟡 | legacy mount config (→ gimbal manager) |
| 205 | DO_MOUNT_CONTROL | 🟡 | legacy gimbal aim (→ DO_GIMBAL_MANAGER_PITCHYAW) |
| 206 | DO_SET_CAM_TRIGG_DIST | ✅ | auto-trigger every N m |
| 220 | DO_MOUNT_CONTROL_QUAT | 🟡 | legacy quaternion mount control |
| 1000 | DO_GIMBAL_MANAGER_PITCHYAW | ✅ | modern gimbal aim (Gimbal v2, ≥4.3) |
| 1001 | DO_GIMBAL_MANAGER_CONFIGURE | ⏳ | take gimbal control; could add ⚠ |

## Safety / landing

| ID | MAV_CMD | Status | Note |
|---|---|---|---|
| 189 | DO_LAND_START | ✅ | Plane landing-sequence marker |
| 190 | DO_RALLY_LAND | ⏳ | Plane rally land; niche ⚠ |
| 207 | DO_FENCE_ENABLE | ✅ | |
| 208 | DO_PARACHUTE | ✅ | Copter |

## VTOL

| ID | MAV_CMD | Status | Note |
|---|---|---|---|
| 3000 | DO_VTOL_TRANSITION | ✅ | force MC ↔ FW |

## Camera / Video protocol — excluded (not mission items)

These are runtime camera/gimbal-protocol commands. ArduPilot does **not** execute them from a mission
(Mission Planner lists some anyway). Use `DO_DIGICAM_CONTROL` / `DO_SET_CAM_TRIGG_DIST` /
`DO_GIMBAL_MANAGER_PITCHYAW` in missions instead.

| ID | MAV_CMD | Status |
|---|---|---|
| 531 | SET_CAMERA_ZOOM | ⛔ |
| 532 | SET_CAMERA_FOCUS | ⛔ |
| 534 | SET_CAMERA_SOURCE | ⛔ |
| 2000 | IMAGE_START_CAPTURE | ⛔ |
| 2001 | IMAGE_STOP_CAPTURE | ⛔ |
| 2500 | VIDEO_START_CAPTURE | ⛔ |
| 2501 | VIDEO_STOP_CAPTURE | ⛔ |

---

## Open questions to resolve with operators

- ~~`DO_DIGICAM_CONTROL` zoom/focus/session~~ → done (behind the Advanced expander).
- ~~`DO_DIGICAM_CONFIGURE`~~ → decided: stays 🟡 (round-trip only).
- Verify ids/availability marked ⚠ against MAVLink `common.xml` + the per-vehicle ArduPilot source.
- Rover/Boat/Sub command sets are sparse here (Plane-first) — flesh out when those vehicles are targeted.
