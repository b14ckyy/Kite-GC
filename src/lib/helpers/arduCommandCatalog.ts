// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

/**
 * ArduPilot mission command catalog — the single declarative source describing every mission command
 * Kite can edit: its category (picker grouping), whether it carries a map position, and its editable
 * params (label/units/range/enum/description). Modelled on QGroundControl's MavCmdInfo*.json + the
 * ArduPilot mission-command reference (see docs/active/ARDUPILOT_WAYPOINT_ARCHITECTURE.md).
 *
 * Curation (locked with the user): we offer a **modern** command set. ArduPilot does not formally
 * deprecate commands, so "legacy" is our call — a command is legacy when a newer preferred method
 * exists for the same job (e.g. DO_MOUNT_CONTROL → DO_GIMBAL_MANAGER_PITCHYAW). Legacy/unknown commands
 * are **not** offered in the picker but still download / load from `.waypoints` and round-trip; they
 * render raw (labelled params, no friendly editor). Runtime-only camera-protocol commands
 * (SET_CAMERA_*, IMAGE_START_CAPTURE) are not mission items → excluded entirely.
 *
 * Names are kept in English (universal MAVLink / pilot terminology). The friendly Kite name is primary;
 * the canonical MAV_CMD name (the form Mission Planner shows) is surfaced as a subtle secondary label.
 * Param ranges/defaults still want ArduPilot-operator review (⚠).
 */

// ── Types ───────────────────────────────────────────────────────────────────

export type VehicleClass = 'plane' | 'copter' | 'quadplane' | 'rover' | 'boat' | 'sub';

/** Autopilot firmware family the catalog is resolved for. ArduPilot and PX4 share the MAVLink mission
 *  protocol and the same MAV_CMD ids, but PX4 implements a smaller, standard subset (see PX4_COMMANDS)
 *  and — unlike ArduPilot — has **one** mission interpreter for all airframes (no per-vehicle command
 *  split). INAV does not use this catalog (it has its own MSP waypoint model). */
export type Firmware = 'ardupilot' | 'px4';

/** QGC-style picker categories (the grouping shown in the command list). */
export type UiCategory =
  | 'Basic' | 'Loiter' | 'Flight control' | 'Conditionals' | 'Camera' | 'Safety' | 'VTOL' | 'Advanced';

export const UI_CATEGORY_ORDER: UiCategory[] = [
  'Basic', 'Loiter', 'Flight control', 'Conditionals', 'Camera', 'Safety', 'VTOL', 'Advanced',
];

/** One editable parameter (mirrors QGC's MissionCmdParamInfo). A number field unless `enumStrings`
 *  is set (→ dropdown). Indices 1..4 are the dedicated mission params; **5/6/7 map to x/y/z (lat/lon/
 *  alt)**. A command either uses 5/6/7 as a real coordinate (`specifiesCoordinate`/`standaloneCoordinate`
 *  → map marker, edited as lat/lon/alt) OR repurposes them as labelled data fields (e.g. DIGICAM puts
 *  Shutter/Cmd-ID/Shot in x/y/z) — in that case define params 5/6/7 here and they render as plain
 *  number fields bound to the raw lat/lon/alt values. */
export interface ParamSpec {
  label: string;
  units?: string;
  default: number;
  decimals?: number;
  min?: number;
  max?: number;
  enumStrings?: string[];
  enumValues?: number[];
  tooltip?: string;    // QGC-style description, shown as an (i) hint in the editor
  advanced?: boolean;  // hidden under the "Advanced" expander by default (declutter, QGC-style)
}

export type ParamIndex = 1 | 2 | 3 | 4 | 5 | 6 | 7;

export interface ArduCmdDef {
  id: number;                 // MAV_CMD
  friendlyName: string;       // editor header + list (primary)
  short: string;              // compact list badge
  category: UiCategory;
  specifiesCoordinate?: boolean;   // has a map position + is a flight-path node (primary waypoint)
  standaloneCoordinate?: boolean;  // has coords but NOT in the flight path (e.g. ROI location)
  isLoiter?: boolean;
  isLand?: boolean;
  isTakeoff?: boolean;
  params?: Partial<Record<ParamIndex, ParamSpec>>;
  vehicles: VehicleClass[];   // classes that expose this command
  /** Display hint for non-location commands: do they act on the previous waypoint (DO_*, executed on
   *  arrival) or gate the next nav command (CONDITION_*). */
  appliesTo?: 'prev' | 'next';
}

// ── MAV_CMD ids (catalog scope) ──────────────────────────────────────────────

export const CMD = {
  NAV_WAYPOINT: 16, NAV_LOITER_UNLIM: 17, NAV_LOITER_TURNS: 18, NAV_LOITER_TIME: 19,
  NAV_RETURN_TO_LAUNCH: 20, NAV_LAND: 21, NAV_TAKEOFF: 22, NAV_CONTINUE_AND_CHANGE_ALT: 30,
  NAV_LOITER_TO_ALT: 31, NAV_SPLINE_WAYPOINT: 82, NAV_ALTITUDE_WAIT: 83,
  NAV_VTOL_TAKEOFF: 84, NAV_VTOL_LAND: 85, NAV_GUIDED_ENABLE: 92, NAV_DELAY: 93, NAV_PAYLOAD_PLACE: 94,
  CONDITION_DELAY: 112, CONDITION_DISTANCE: 114, CONDITION_YAW: 115,
  DO_JUMP: 177, DO_CHANGE_SPEED: 178, DO_SET_HOME: 179, DO_SET_RELAY: 181, DO_REPEAT_RELAY: 182,
  DO_SET_SERVO: 183, DO_REPEAT_SERVO: 184, DO_LAND_START: 189, DO_SET_ROI_LOCATION: 195,
  DO_SET_ROI_NONE: 197, DO_DIGICAM_CONTROL: 203, DO_SET_CAM_TRIGG_DIST: 206, DO_FENCE_ENABLE: 207,
  DO_PARACHUTE: 208, DO_INVERTED_FLIGHT: 210, DO_GRIPPER: 211, DO_AUTOTUNE_ENABLE: 212,
  DO_SET_RESUME_REPEAT_DIST: 215, DO_AUX_FUNCTION: 218, DO_GUIDED_LIMITS: 222,
  DO_SET_REVERSE: 594,
  JUMP_TAG: 600, DO_JUMP_TAG: 601, DO_GIMBAL_MANAGER_PITCHYAW: 1000, DO_VTOL_TRANSITION: 3000,
} as const;

// MAV_CMDs PX4 accepts in a mission. Verified against the PX4 docs ("Mission Mode" supported-command
// list) + the authoritative source `MavlinkMissionManager::parse_mavlink_mission_item` in
// firmware/src/modules/mavlink/mavlink_mission.cpp. PX4 implements a smaller, standard subset than
// ArduPilot (no JUMP_TAG, no extra LOITER variants, no relay/parachute/fence/condition commands) and
// has no per-vehicle command split — one interpreter for all airframes. The VTOL commands are offered
// for any PX4 airframe; a soft-warning at connect flags them on a non-VTOL vehicle (mirrors ArduPilot).
// PX4 rejects unsupported commands at upload (mission feasibility checker), so this set must stay tight.
const PX4_COMMANDS = new Set<number>([
  CMD.NAV_WAYPOINT, CMD.NAV_LOITER_UNLIM, CMD.NAV_LOITER_TIME, CMD.NAV_LOITER_TO_ALT,
  CMD.NAV_TAKEOFF, CMD.NAV_LAND, CMD.NAV_RETURN_TO_LAUNCH, CMD.NAV_DELAY,
  CMD.DO_JUMP, CMD.DO_CHANGE_SPEED, CMD.DO_SET_HOME, CMD.DO_SET_SERVO, CMD.DO_LAND_START,
  CMD.DO_SET_ROI_LOCATION, CMD.DO_SET_ROI_NONE, CMD.DO_DIGICAM_CONTROL, CMD.DO_SET_CAM_TRIGG_DIST,
  CMD.DO_GIMBAL_MANAGER_PITCHYAW, CMD.DO_AUTOTUNE_ENABLE,
  CMD.NAV_VTOL_TAKEOFF, CMD.NAV_VTOL_LAND, CMD.DO_VTOL_TRANSITION,
]);

// Vehicle-class groupings (readability).
const FIXED: VehicleClass[] = ['plane', 'quadplane'];
const COPTER: VehicleClass[] = ['copter'];
const VTOL: VehicleClass[] = ['quadplane'];
const AIR: VehicleClass[] = ['plane', 'copter', 'quadplane'];
const ALL: VehicleClass[] = ['plane', 'copter', 'quadplane', 'rover', 'boat', 'sub'];

// Reusable enum sets.
const ON_OFF: Pick<ParamSpec, 'enumStrings' | 'enumValues'> = { enumStrings: ['Off', 'On'], enumValues: [0, 1] };
const EXIT_LOITER: Pick<ParamSpec, 'enumStrings' | 'enumValues'> = { enumStrings: ['Loiter center', 'Cross-track'], enumValues: [0, 1] };

// ── Catalog (curated modern set) ─────────────────────────────────────────────

export const ARDU_CATALOG: ArduCmdDef[] = [
  // ── Basic ──
  {
    id: CMD.NAV_WAYPOINT, friendlyName: 'Waypoint', short: 'WP', category: 'Basic',
    specifiesCoordinate: true, vehicles: ALL,
    params: {
      1: { label: 'Hold', units: 's', default: 0, decimals: 0, min: 0, max: 600, advanced: true, tooltip: 'Seconds to wait at the waypoint before continuing.' },
      2: { label: 'Acceptance', units: 'm', default: 0, decimals: 1, min: 0, advanced: true, tooltip: 'Radius within which the waypoint counts as reached (0 = firmware default).' },
      3: { label: 'Pass Radius', units: 'm', default: 0, decimals: 1, advanced: true, tooltip: 'Fly past the point by this radius instead of stopping (0 = pass through).' },
      4: { label: 'Yaw', units: 'deg', default: 0, decimals: 0, min: -1, max: 360, advanced: true, tooltip: 'Heading to hold at the waypoint (Copter). -1 = keep current heading.' },
    },
  },
  {
    id: CMD.NAV_SPLINE_WAYPOINT, friendlyName: 'Spline Waypoint', short: 'SPL', category: 'Basic',
    specifiesCoordinate: true, vehicles: COPTER,
    params: { 1: { label: 'Hold', units: 's', default: 0, decimals: 0, min: 0, max: 600, tooltip: 'Seconds to wait at the point. The path is a smooth curve through the waypoints.' } },
  },
  {
    id: CMD.NAV_TAKEOFF, friendlyName: 'Takeoff', short: 'TKO', category: 'Basic',
    specifiesCoordinate: true, isTakeoff: true, vehicles: AIR,
    params: {
      1: { label: 'Min Pitch', units: 'deg', default: 0, decimals: 0, min: 0, max: 90, tooltip: 'Minimum climb pitch (fixed-wing). 0 = firmware default.' },
      4: { label: 'Yaw', units: 'deg', default: 0, decimals: 0, min: -1, max: 360, tooltip: 'Heading after takeoff (Copter). -1 = keep current.' },
    },
  },
  {
    id: CMD.NAV_LAND, friendlyName: 'Land', short: 'LND', category: 'Basic',
    specifiesCoordinate: true, isLand: true, vehicles: AIR,
    params: {
      1: { label: 'Abort Alt', units: 'm', default: 0, decimals: 0, min: 0, tooltip: 'Altitude to climb to if the landing is aborted (0 = firmware default).' },
      2: { label: 'Precision Land', default: 0, decimals: 0, enumStrings: ['None', 'Opportunistic', 'Required'], enumValues: [0, 1, 2], tooltip: 'Use a precision-landing sensor if available.' },
      4: { label: 'Yaw', units: 'deg', default: 0, decimals: 0, min: -1, max: 360, tooltip: 'Landing heading. -1 = keep current.' },
    },
  },

  // ── Loiter ──
  {
    id: CMD.NAV_LOITER_UNLIM, friendlyName: 'Loiter (forever)', short: 'LTR∞', category: 'Loiter',
    specifiesCoordinate: true, isLoiter: true, vehicles: AIR,
    params: {
      3: { label: 'Radius', units: 'm', default: 50, decimals: 0, tooltip: 'Loiter circle radius (fixed-wing). Sign sets direction: + clockwise, − counter-clockwise.' },
      4: { label: 'Yaw', units: 'deg', default: 0, decimals: 0, min: -1, max: 360, tooltip: 'Heading to face while loitering (Copter). -1 = keep current.' },
    },
  },
  {
    id: CMD.NAV_LOITER_TURNS, friendlyName: 'Loiter (turns)', short: 'LTRT', category: 'Loiter',
    specifiesCoordinate: true, isLoiter: true, vehicles: AIR,
    params: {
      1: { label: 'Turns', default: 1, decimals: 0, min: 0, tooltip: 'Number of full circles before continuing.' },
      3: { label: 'Radius', units: 'm', default: 50, decimals: 0, tooltip: 'Loiter circle radius. Sign sets direction (+ CW / − CCW).' },
      4: { label: 'Exit', default: 0, decimals: 0, ...EXIT_LOITER, tooltip: 'Leave from the loiter centre or tangent toward the next waypoint.' },
    },
  },
  {
    id: CMD.NAV_LOITER_TIME, friendlyName: 'Loiter (time)', short: 'LTRS', category: 'Loiter',
    specifiesCoordinate: true, isLoiter: true, vehicles: AIR,
    params: {
      1: { label: 'Time', units: 's', default: 30, decimals: 0, min: 0, tooltip: 'Seconds to loiter before continuing.' },
      3: { label: 'Radius', units: 'm', default: 50, decimals: 0, tooltip: 'Loiter circle radius. Sign sets direction (+ CW / − CCW).' },
      4: { label: 'Exit', default: 0, decimals: 0, ...EXIT_LOITER, tooltip: 'Leave from the loiter centre or tangent toward the next waypoint.' },
    },
  },
  {
    id: CMD.NAV_LOITER_TO_ALT, friendlyName: 'Loiter to Altitude', short: 'LTA', category: 'Loiter',
    specifiesCoordinate: true, isLoiter: true, vehicles: AIR,
    params: {
      1: { label: 'Heading Required', default: 0, decimals: 0, ...ON_OFF, tooltip: 'Wait until pointing toward the next waypoint before leaving.' },
      2: { label: 'Radius', units: 'm', default: 50, decimals: 0, tooltip: 'Loiter circle radius while climbing/descending to the target altitude.' },
      4: { label: 'Exit', default: 0, decimals: 0, ...EXIT_LOITER, tooltip: 'Leave from the loiter centre or tangent.' },
    },
  },

  // ── Flight control ──
  {
    id: CMD.NAV_RETURN_TO_LAUNCH, friendlyName: 'Return to Launch', short: 'RTL',
    category: 'Flight control', vehicles: ALL, appliesTo: 'prev',
  },
  {
    id: CMD.NAV_CONTINUE_AND_CHANGE_ALT, friendlyName: 'Change Altitude', short: 'ALT',
    category: 'Flight control', vehicles: FIXED, appliesTo: 'next',
    params: { 1: { label: 'Mode', default: 0, decimals: 0, enumStrings: ['Auto', 'Climb', 'Descend'], enumValues: [0, 1, 2], tooltip: 'Continue on the current heading until the target altitude (set in Alt) is reached.' } },
  },
  {
    id: CMD.NAV_ALTITUDE_WAIT, friendlyName: 'Altitude Wait', short: 'AWT', category: 'Flight control',
    vehicles: FIXED, appliesTo: 'next',
    params: {
      1: { label: 'Altitude', units: 'm', default: 0, decimals: 0, tooltip: 'Wait until this altitude is reached (e.g. balloon ascent) before continuing.' },
      2: { label: 'Descent Rate', units: 'm/s', default: 0, decimals: 1, tooltip: 'Or continue once descending faster than this rate.' },
      3: { label: 'Wiggle', units: 'deg', default: 0, decimals: 0, tooltip: 'Periodically wiggle the servos by this much to prevent freezing.' },
    },
  },
  {
    id: CMD.NAV_DELAY, friendlyName: 'Delay', short: 'DLY', category: 'Flight control', vehicles: AIR,
    appliesTo: 'next',
    params: {
      1: { label: 'Delay', units: 's', default: -1, decimals: 0, min: -1, tooltip: 'Seconds to halt the mission (-1 = use the wall-clock time below instead).' },
      2: { label: 'Hour (UTC)', default: -1, decimals: 0, min: -1, max: 23, tooltip: 'Resume at this UTC hour (-1 = ignore).' },
      3: { label: 'Minute (UTC)', default: -1, decimals: 0, min: -1, max: 59, tooltip: 'Resume at this UTC minute (-1 = ignore).' },
      4: { label: 'Second (UTC)', default: -1, decimals: 0, min: -1, max: 59, tooltip: 'Resume at this UTC second (-1 = ignore).' },
    },
  },
  {
    id: CMD.DO_JUMP, friendlyName: 'Jump', short: 'JMP', category: 'Flight control', vehicles: ALL,
    appliesTo: 'prev',
    params: {
      1: { label: 'To WP', default: 1, decimals: 0, min: 1, tooltip: 'Mission item number to jump to.' },
      2: { label: 'Repeat', default: 1, decimals: 0, min: -1, tooltip: 'How many times to perform the jump (-1 = forever).' },
    },
  },
  {
    id: CMD.DO_JUMP_TAG, friendlyName: 'Jump to Tag', short: 'JMPT', category: 'Flight control', vehicles: ALL,
    appliesTo: 'prev',
    params: {
      1: { label: 'Tag', default: 1, decimals: 0, min: 1, tooltip: 'Jump to the Jump-Tag with this id (tag-based jump survives mission edits).' },
      2: { label: 'Repeat', default: 1, decimals: 0, min: -1, tooltip: 'How many times to jump (-1 = forever).' },
    },
  },
  {
    id: CMD.JUMP_TAG, friendlyName: 'Jump Tag', short: 'TAG', category: 'Flight control', vehicles: ALL,
    appliesTo: 'prev',
    params: { 1: { label: 'Tag', default: 1, decimals: 0, min: 1, max: 65535, tooltip: 'A named marker that "Jump to Tag" can target.' } },
  },
  {
    id: CMD.DO_CHANGE_SPEED, friendlyName: 'Change Speed', short: 'SPD', category: 'Flight control', vehicles: ALL,
    appliesTo: 'prev',
    params: {
      1: { label: 'Speed Type', default: 0, decimals: 0, enumStrings: ['Airspeed', 'Groundspeed', 'Climb', 'Descent'], enumValues: [0, 1, 2, 3], tooltip: 'Which speed the value applies to.' },
      2: { label: 'Speed', units: 'm/s', default: -1, decimals: 1, min: -1, tooltip: 'Target speed (-1 = no change).' },
      3: { label: 'Throttle', units: '%', default: -1, decimals: 0, min: -1, max: 100, tooltip: 'Throttle to hold (-1 = no change).' },
    },
  },
  {
    id: CMD.DO_SET_RESUME_REPEAT_DIST, friendlyName: 'Resume Rewind', short: 'RWD', category: 'Flight control',
    vehicles: ALL, appliesTo: 'prev',
    params: { 1: { label: 'Rewind', units: 'm', default: 0, decimals: 0, min: 0, tooltip: 'On mission resume, rewind this far back along the path first (0 = disable).' } },
  },
  {
    id: CMD.DO_INVERTED_FLIGHT, friendlyName: 'Inverted Flight', short: 'INV', category: 'Flight control',
    vehicles: ['plane'], appliesTo: 'prev',
    params: { 1: { label: 'Inverted', default: 0, decimals: 0, enumStrings: ['Normal', 'Inverted'], enumValues: [0, 1], tooltip: 'Fly upright or inverted from here on.' } },
  },
  {
    // Rover/Boat-specific: drive forward or in reverse from here on (ArduRover).
    id: CMD.DO_SET_REVERSE, friendlyName: 'Set Reverse', short: 'REV', category: 'Flight control',
    vehicles: ['rover', 'boat'], appliesTo: 'prev',
    params: { 1: { label: 'Direction', default: 0, decimals: 0, enumStrings: ['Forward', 'Reverse'], enumValues: [0, 1], tooltip: 'Drive forward or in reverse from this point on.' } },
  },

  // ── Conditionals (gate the next nav command) ──
  {
    id: CMD.CONDITION_DELAY, friendlyName: 'Wait (time)', short: 'cDLY', category: 'Conditionals',
    vehicles: ALL, appliesTo: 'next',
    params: { 1: { label: 'Delay', units: 's', default: 0, decimals: 0, min: 0, tooltip: 'Delay the next DO_ command by this many seconds (does not stop the vehicle).' } },
  },
  {
    id: CMD.CONDITION_DISTANCE, friendlyName: 'Wait (distance)', short: 'cDST', category: 'Conditionals',
    vehicles: ALL, appliesTo: 'next',
    params: { 1: { label: 'Distance', units: 'm', default: 0, decimals: 0, min: 0, tooltip: 'Hold the next DO_ command until within this distance of the next waypoint.' } },
  },
  {
    id: CMD.CONDITION_YAW, friendlyName: 'Set Heading', short: 'cYAW', category: 'Conditionals',
    vehicles: ALL, appliesTo: 'next',
    params: {
      1: { label: 'Angle', units: 'deg', default: 0, decimals: 0, min: 0, max: 360, tooltip: 'Target heading.' },
      2: { label: 'Rate', units: 'deg/s', default: 0, decimals: 0, min: 0, tooltip: 'Turn rate (0 = default).' },
      3: { label: 'Direction', default: 1, decimals: 0, enumStrings: ['Clockwise', 'Counter-CW'], enumValues: [1, -1], tooltip: 'Turn direction.' },
      4: { label: 'Relative', default: 0, decimals: 0, enumStrings: ['Absolute', 'Relative'], enumValues: [0, 1], tooltip: 'Angle is absolute (compass) or relative to the current heading.' },
    },
  },

  // ── Camera ──
  {
    id: CMD.DO_SET_ROI_LOCATION, friendlyName: 'Region of Interest', short: 'ROI', category: 'Camera',
    standaloneCoordinate: true, vehicles: AIR, appliesTo: 'prev',
  },
  {
    id: CMD.DO_SET_ROI_NONE, friendlyName: 'Clear ROI', short: 'ROI⊘', category: 'Camera',
    vehicles: AIR, appliesTo: 'prev',
  },
  {
    id: CMD.DO_SET_CAM_TRIGG_DIST, friendlyName: 'Camera Auto-Trigger', short: 'CTD', category: 'Camera',
    vehicles: AIR, appliesTo: 'prev',
    params: {
      1: { label: 'Distance', units: 'm', default: 25, decimals: 1, min: 0, tooltip: 'Trigger the camera shutter every N metres of travel (0 = stop triggering).' },
      3: { label: 'Trigger Now', default: 0, decimals: 0, ...ON_OFF, tooltip: 'Also take one shot immediately.' },
    },
  },
  {
    id: CMD.DO_DIGICAM_CONTROL, friendlyName: 'Take Photo', short: 'CAM', category: 'Camera',
    vehicles: AIR, appliesTo: 'prev',
    // Not a coordinate command — params 5/6/7 (= x/y/z) carry shutter/command-id/shot-id. The shutter
    // trigger is the everyday field; session/zoom/focus + ids are valid but rare → Advanced.
    params: {
      5: { label: 'Shutter', default: 1, decimals: 0, enumStrings: ['No', 'Trigger'], enumValues: [0, 1], tooltip: 'Fire the camera shutter once.' },
      1: { label: 'Session', default: 0, decimals: 0, enumStrings: ['Off', 'On'], enumValues: [0, 1], advanced: true, tooltip: 'Open/close the camera session (lens cover / power).' },
      2: { label: 'Zoom Pos', default: 0, decimals: 0, advanced: true, tooltip: 'Absolute zoom position (camera-dependent).' },
      3: { label: 'Zoom Step', default: 0, decimals: 0, advanced: true, tooltip: 'Relative zoom step (+ in / − out).' },
      4: { label: 'Focus Lock', default: 0, decimals: 0, enumStrings: ['Unlock', 'Lock'], enumValues: [0, 1], advanced: true, tooltip: 'Lock or unlock the focus.' },
      6: { label: 'Command ID', default: 0, decimals: 0, advanced: true, tooltip: 'Command identity (increment to send a new command to the camera).' },
      7: { label: 'Shot ID', default: 0, decimals: 0, advanced: true, tooltip: 'Shot counter / extra parameter.' },
    },
  },
  {
    id: CMD.DO_GIMBAL_MANAGER_PITCHYAW, friendlyName: 'Aim Gimbal', short: 'GMB', category: 'Camera',
    vehicles: AIR, appliesTo: 'prev',
    params: {
      1: { label: 'Pitch', units: 'deg', default: 0, decimals: 0, min: -90, max: 90, tooltip: 'Gimbal pitch (negative = down). Modern gimbal-manager command (ArduPilot ≥4.3).' },
      2: { label: 'Yaw', units: 'deg', default: 0, decimals: 0, min: -180, max: 180, tooltip: 'Gimbal yaw relative to the vehicle.' },
    },
  },

  // ── Safety ──
  {
    id: CMD.DO_FENCE_ENABLE, friendlyName: 'Geofence', short: 'FNC', category: 'Safety', vehicles: AIR,
    appliesTo: 'prev',
    params: { 1: { label: 'Action', default: 1, decimals: 0, enumStrings: ['Disable', 'Enable', 'Floor only'], enumValues: [0, 1, 2], tooltip: 'Enable or disable the geofence mid-mission.' } },
  },
  {
    id: CMD.DO_PARACHUTE, friendlyName: 'Parachute', short: 'PAR', category: 'Safety', vehicles: COPTER,
    appliesTo: 'prev',
    params: { 1: { label: 'Action', default: 2, decimals: 0, enumStrings: ['Disable', 'Enable', 'Release'], enumValues: [0, 1, 2], tooltip: 'Arm/disarm the parachute or release it now.' } },
  },
  {
    id: CMD.DO_AUX_FUNCTION, friendlyName: 'Aux Function', short: 'AUX', category: 'Safety', vehicles: ALL,
    appliesTo: 'prev',
    params: {
      1: { label: 'Function', default: 0, decimals: 0, min: 0, tooltip: 'RCn_OPTION auxiliary-function number to trigger (same numbering as an RC switch).' },
      2: { label: 'Switch', default: 0, decimals: 0, enumStrings: ['Low', 'Middle', 'High'], enumValues: [0, 1, 2], tooltip: 'Switch position to simulate.' },
    },
  },

  // ── VTOL ──
  {
    id: CMD.NAV_VTOL_TAKEOFF, friendlyName: 'VTOL Takeoff', short: 'VTKO', category: 'VTOL',
    specifiesCoordinate: true, isTakeoff: true, vehicles: VTOL,
  },
  {
    id: CMD.NAV_VTOL_LAND, friendlyName: 'VTOL Land', short: 'VLND', category: 'VTOL',
    specifiesCoordinate: true, isLand: true, vehicles: VTOL,
    params: { 1: { label: 'Approach', default: 0, decimals: 0, enumStrings: ['Auto', 'Exclude approach'], enumValues: [0, 1], tooltip: 'Whether to fly the fixed-wing approach before the vertical descent.' } },
  },
  {
    id: CMD.DO_VTOL_TRANSITION, friendlyName: 'VTOL Transition', short: 'VTRN', category: 'VTOL',
    vehicles: VTOL, appliesTo: 'prev',
    params: { 1: { label: 'State', default: 4, decimals: 0, enumStrings: ['Multicopter', 'Fixed-wing'], enumValues: [3, 4], tooltip: 'Force a transition to multicopter or fixed-wing flight.' } },
  },

  // ── Advanced (outputs / less common) ──
  {
    id: CMD.NAV_PAYLOAD_PLACE, friendlyName: 'Payload Place', short: 'PLP', category: 'Advanced',
    specifiesCoordinate: true, vehicles: [...COPTER, ...VTOL],
    params: { 1: { label: 'Max Descent', units: 'm', default: 0, decimals: 1, min: 0, tooltip: 'Descend up to this far while lowering the payload, releasing on ground contact.' } },
  },
  {
    id: CMD.DO_SET_SERVO, friendlyName: 'Set Servo', short: 'SRV', category: 'Advanced', vehicles: ALL,
    appliesTo: 'prev',
    params: {
      1: { label: 'Servo', default: 9, decimals: 0, min: 1, max: 16, tooltip: 'Servo/output channel number.' },
      2: { label: 'PWM', units: 'µs', default: 1500, decimals: 0, min: 800, max: 2200, tooltip: 'Pulse width to set on that output.' },
    },
  },
  {
    id: CMD.DO_REPEAT_SERVO, friendlyName: 'Repeat Servo', short: 'rSRV', category: 'Advanced', vehicles: ALL,
    appliesTo: 'prev',
    params: {
      1: { label: 'Servo', default: 9, decimals: 0, min: 1, max: 16, tooltip: 'Servo/output channel number.' },
      2: { label: 'PWM', units: 'µs', default: 1500, decimals: 0, min: 800, max: 2200, tooltip: 'Pulse width to toggle to.' },
      3: { label: 'Count', default: 1, decimals: 0, min: 1, tooltip: 'How many cycles.' },
      4: { label: 'Cycle Time', units: 's', default: 1, decimals: 1, min: 0, tooltip: 'Seconds per cycle.' },
    },
  },
  {
    id: CMD.DO_SET_RELAY, friendlyName: 'Set Relay', short: 'RLY', category: 'Advanced', vehicles: ALL,
    appliesTo: 'prev',
    params: {
      1: { label: 'Relay', default: 0, decimals: 0, min: 0, max: 5, tooltip: 'Relay number.' },
      2: { label: 'State', default: 1, decimals: 0, ...ON_OFF, tooltip: 'Switch the relay on or off.' },
    },
  },
  {
    id: CMD.DO_REPEAT_RELAY, friendlyName: 'Repeat Relay', short: 'rRLY', category: 'Advanced', vehicles: ALL,
    appliesTo: 'prev',
    params: {
      1: { label: 'Relay', default: 0, decimals: 0, min: 0, max: 5, tooltip: 'Relay number.' },
      2: { label: 'Count', default: 1, decimals: 0, min: 1, tooltip: 'How many toggles.' },
      3: { label: 'Cycle Time', units: 's', default: 1, decimals: 1, min: 0, tooltip: 'Seconds per toggle.' },
    },
  },
  {
    id: CMD.DO_GRIPPER, friendlyName: 'Gripper', short: 'GRP', category: 'Advanced', vehicles: [...COPTER, ...VTOL],
    appliesTo: 'prev',
    params: {
      1: { label: 'Gripper', default: 1, decimals: 0, min: 1, tooltip: 'Gripper number.' },
      2: { label: 'Action', default: 1, decimals: 0, enumStrings: ['Release', 'Grab'], enumValues: [0, 1], tooltip: 'Release or grab.' },
    },
  },
  {
    id: CMD.DO_LAND_START, friendlyName: 'Landing Sequence Start', short: 'LSEQ', category: 'Advanced',
    standaloneCoordinate: true, vehicles: FIXED, appliesTo: 'next',
  },
  {
    id: CMD.DO_SET_HOME, friendlyName: 'Set Home', short: 'HOME', category: 'Advanced',
    standaloneCoordinate: true, vehicles: ALL, appliesTo: 'prev',
    params: { 1: { label: 'Use Current', default: 1, decimals: 0, enumStrings: ['Specified', 'Current'], enumValues: [0, 1], tooltip: 'Set home to the current position, or to the coordinates below.' } },
  },
  {
    id: CMD.NAV_GUIDED_ENABLE, friendlyName: 'Guided Enable', short: 'GUI', category: 'Advanced', vehicles: AIR,
    appliesTo: 'next',
    params: { 1: { label: 'Enable', default: 1, decimals: 0, ...ON_OFF, tooltip: 'Hand control to an external/companion controller (Guided) for this leg.' } },
  },
  {
    id: CMD.DO_GUIDED_LIMITS, friendlyName: 'Guided Limits', short: 'GLIM', category: 'Advanced', vehicles: AIR,
    appliesTo: 'next',
    params: {
      1: { label: 'Timeout', units: 's', default: 0, decimals: 0, min: 0, tooltip: 'Max time the external controller may have control (0 = no limit).' },
      2: { label: 'Min Alt', units: 'm', default: 0, decimals: 0, tooltip: 'Abort if it descends below this (0 = no limit).' },
      3: { label: 'Max Alt', units: 'm', default: 0, decimals: 0, tooltip: 'Abort if it climbs above this (0 = no limit).' },
      4: { label: 'Horiz Limit', units: 'm', default: 0, decimals: 0, min: 0, tooltip: 'Abort if it strays this far horizontally (0 = no limit).' },
    },
  },
  {
    id: CMD.DO_AUTOTUNE_ENABLE, friendlyName: 'Autotune', short: 'TUNE', category: 'Advanced', vehicles: AIR,
    appliesTo: 'prev',
    params: { 1: { label: 'Enable', default: 1, decimals: 0, ...ON_OFF, tooltip: 'Run autotune during this part of the mission.' } },
  },
];

// ── Lookups & helpers ────────────────────────────────────────────────────────

const BY_ID = new Map<number, ArduCmdDef>(ARDU_CATALOG.map((c) => [c.id, c]));

// id → canonical MAV_CMD short name (the form Mission Planner shows, e.g. "DO_DIGICAM_CONTROL"),
// surfaced as the subtle secondary label so MP/QGC users recognise the command.
const RAW_NAME_BY_ID = new Map<number, string>(Object.entries(CMD).map(([k, v]) => [v as number, k]));

/** Canonical MAV_CMD short name for a command id (MP-style), or "#<id>" if unknown. */
export function cmdRawName(id: number): string {
  return RAW_NAME_BY_ID.get(id) ?? `#${id}`;
}

/** Catalog definition for a command id, or undefined if not in the catalog (preserved but un-editable). */
export function cmdDef(id: number): ArduCmdDef | undefined {
  return BY_ID.get(id);
}

/** Friendly name (falls back to the canonical name for catalog-unknown commands so they still read). */
export function cmdName(id: number): string {
  return BY_ID.get(id)?.friendlyName ?? cmdRawName(id);
}

export function cmdShort(id: number): string {
  return BY_ID.get(id)?.short ?? `#${id}`;
}

/** Does this command carry a flight-path map position (primary waypoint)? */
export function cmdHasLocation(id: number): boolean {
  return BY_ID.get(id)?.specifiesCoordinate === true;
}

/** A coordinate that is NOT a flight-path node (ROI/Home/LandStart). */
export function cmdStandaloneCoordinate(id: number): boolean {
  return BY_ID.get(id)?.standaloneCoordinate === true;
}

/** A non-location command — i.e. an INAV-modifier-style "action" item (no map marker, no flight path). */
export function cmdIsModifier(id: number): boolean {
  const d = BY_ID.get(id);
  return !d || !d.specifiesCoordinate;
}

export function cmdIsLoiter(id: number): boolean { return BY_ID.get(id)?.isLoiter === true; }
export function cmdIsTakeoff(id: number): boolean { return BY_ID.get(id)?.isTakeoff === true; }

/** Default param1..4 for a freshly-added command. */
export function cmdDefaultParams(id: number): { param1: number; param2: number; param3: number; param4: number } {
  const p = BY_ID.get(id)?.params;
  return {
    param1: p?.[1]?.default ?? 0,
    param2: p?.[2]?.default ?? 0,
    param3: p?.[3]?.default ?? 0,
    param4: p?.[4]?.default ?? 0,
  };
}

/** Default x/y/z (lat/lon/alt) for a freshly-added command that repurposes params 5/6/7 as data
 *  (e.g. DIGICAM shutter). Returns undefined for each axis the command does not define. */
export function cmdDefaultCoordParams(id: number): { x?: number; y?: number; z?: number } {
  const p = BY_ID.get(id)?.params;
  return { x: p?.[5]?.default, y: p?.[6]?.default, z: p?.[7]?.default };
}

/** Commands offered for a firmware/vehicle. ArduPilot: filtered by the vehicle class. PX4: the whole
 *  PX4 subset regardless of class (one interpreter for all airframes; VTOL commands are always offered
 *  and soft-warned at connect on a non-VTOL vehicle). */
export function resolveCatalog(vehicle: VehicleClass, firmware: Firmware = 'ardupilot'): ArduCmdDef[] {
  if (firmware === 'px4') return ARDU_CATALOG.filter((c) => PX4_COMMANDS.has(c.id));
  return ARDU_CATALOG.filter((c) => c.vehicles.includes(vehicle));
}

/** Is this command part of the PX4-supported set? */
export function cmdSupportedByPx4(id: number): boolean {
  return PX4_COMMANDS.has(id);
}

/** Soft-validity: is this command valid for the given vehicle class? A catalog-unknown (legacy /
 *  round-trip-only) command is NOT flagged — we can't judge it and it must round-trip untouched. A
 *  known command whose `vehicles[]` excludes the class is flagged (warn, never block). */
export function cmdValidForVehicle(id: number, vehicle: VehicleClass): boolean {
  const d = BY_ID.get(id);
  return !d || d.vehicles.includes(vehicle);
}

/** Soft-validity for PX4. Flags two cases the FC would reject at upload: (1) a known ArduPilot-only
 *  command not in the PX4 set, and (2) a VTOL command on a non-VTOL airframe. Catalog-unknown (legacy /
 *  round-trip-only) commands are NOT flagged — we can't judge them and they must round-trip untouched. */
export function cmdValidForPx4(id: number, vehicle: VehicleClass): boolean {
  if (!PX4_COMMANDS.has(id)) return !BY_ID.has(id); // unknown → don't flag; known-but-not-PX4 → flag
  return BY_ID.get(id)?.category === 'VTOL' ? vehicle === 'quadplane' : true;
}

/** Location (primary-waypoint) commands for a firmware/vehicle, grouped by UI category in display order. */
export function locationCommandsByCategory(vehicle: VehicleClass, firmware: Firmware = 'ardupilot'): { category: UiCategory; cmds: ArduCmdDef[] }[] {
  return groupByCategory(resolveCatalog(vehicle, firmware).filter((c) => c.specifiesCoordinate));
}

/** Modifier (non-location) commands for a firmware/vehicle, grouped by UI category — drives "+ Add modifier". */
export function modifierCommandsByCategory(vehicle: VehicleClass, firmware: Firmware = 'ardupilot'): { category: UiCategory; cmds: ArduCmdDef[] }[] {
  return groupByCategory(resolveCatalog(vehicle, firmware).filter((c) => !c.specifiesCoordinate));
}

function groupByCategory(cmds: ArduCmdDef[]): { category: UiCategory; cmds: ArduCmdDef[] }[] {
  const out: { category: UiCategory; cmds: ArduCmdDef[] }[] = [];
  for (const category of UI_CATEGORY_ORDER) {
    const group = cmds.filter((c) => c.category === category);
    if (group.length) out.push({ category, cmds: group });
  }
  return out;
}

/** Format an enum value to its label, or the raw number. */
export function enumLabel(p: ParamSpec, value: number): string {
  if (p.enumStrings && p.enumValues) {
    const i = p.enumValues.indexOf(value);
    if (i >= 0) return p.enumStrings[i];
  }
  return String(value);
}
