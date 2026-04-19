# Flight Mode Protocol Reference

How different autopilot stacks report flight modes via telemetry. This document captures protocol-specific details needed when adding support for new autopilots.

---

## INAV (current, implemented)

**Source**: `active_flight_mode_flags` from Blackbox CSV / `MSPV2_INAV_STATUS` payload

**Format**: **Bitmask** (uint32) — multiple flags can be active simultaneously.

| Bit | Constant | Value | Description |
|-----|----------|-------|-------------|
| 0 | `ANGLE_MODE` | 1 | Self-leveling (pitch/roll limited) |
| 1 | `HORIZON_MODE` | 2 | Self-leveling with acro at full stick |
| 2 | `HEADING_MODE` | 4 | Heading hold |
| 3 | `NAV_ALTHOLD_MODE` | 8 | Altitude hold (modifier, never standalone) |
| 4 | `NAV_RTH_MODE` | 16 | Return to home |
| 5 | `NAV_POSHOLD_MODE` | 32 | Position hold |
| 6 | `HEADFREE_MODE` | 64 | Headfree / carefree |
| 7 | `NAV_LAUNCH_MODE` | 128 | Auto launch (fixed wing) |
| 8 | `MANUAL_MODE` | 256 | Manual (fixed wing passthrough) |
| 9 | `FAILSAFE_MODE` | 512 | Failsafe active |
| 10 | `AUTO_TUNE` | 1024 | Auto-tune PID |
| 11 | `NAV_WP_MODE` | 2048 | Waypoint mission |
| 12 | `NAV_COURSE_HOLD` | 4096 | Course hold / cruise |
| 13 | `FLAPERON` | 8192 | Flaperon mode |
| 14 | `TURN_ASSISTANT` | 16384 | Turn assistant |
| 15 | `TURTLE_MODE` | 32768 | Turtle / flip over after crash |
| 16 | `SOARING_MODE` | 65536 | Soaring / thermal assist |
| 17 | `ANGLEHOLD_MODE` | 131072 | Angle hold |
| 18 | `NAV_FW_AUTOLAND` | 262144 | Fixed wing auto-land |

**Classification**: Priority-based — highest matching flag wins. See `classifyFlightMode()` in `trackColors.ts`.

**Key property**: Flags combine — e.g. `ANGLE | NAV_ALTHOLD = 9` means primary mode `ANGLE` with `ALT` as modifier. A single integer can mean multiple things simultaneously. This is fundamentally different from ArduPilot.

**DB column**: `telemetry_records.active_flight_mode_flags` (INTEGER, nullable)

**Live MSP**: `MSPV2_INAV_STATUS` parsing is implemented in `scheduler/telemetry.rs`.

Important details for correct live decoding:
- INAV `activeModes` is **index-based**, not permanent box-ID based.
- Index→box-id mapping is queried once on connect via `MSP_BOXIDS` (119).
- During decode, bit index → permanent box ID → runtime mode flag mapping.
- INAV implicit ANGLE behavior is mirrored: nav modes without explicit stabilization force ANGLE.

---

## ArduPilot / MAVLink (planned, NOT yet implemented)

### HEARTBEAT (msg #0) — Legacy Mode Reporting

Two fields in every heartbeat (1 Hz):

| Field | Type | Description |
|-------|------|-------------|
| `base_mode` | uint8 | `MAV_MODE_FLAG` bitmask: generic state flags |
| `custom_mode` | uint32 | **Autopilot-specific enum** (NOT a bitmask!) |

#### `base_mode` — MAV_MODE_FLAG (bitmask)

| Bit | Flag | Value | Meaning |
|-----|------|-------|---------|
| 0 | CUSTOM_MODE_ENABLED | 1 | `custom_mode` field is valid |
| 1 | TEST_ENABLED | 2 | System in test mode |
| 2 | AUTO_ENABLED | 4 | Autonomous mode active |
| 3 | GUIDED_ENABLED | 8 | Guided mode active |
| 4 | STABILIZE_ENABLED | 16 | Stabilization active |
| 5 | HIL_ENABLED | 32 | Hardware-in-loop sim |
| 6 | MANUAL_INPUT_ENABLED | 64 | Manual input accepted |
| 7 | SAFETY_ARMED | 128 | Motors can be actuated |

#### `custom_mode` — Vehicle-Type-Specific Enums

**CRITICAL**: Same integer = different mode depending on vehicle type. Must know `MAV_TYPE` from heartbeat.

##### COPTER_MODE (MAV_TYPE = 2, 13, 14, etc.)

| Value | Name | Description |
|-------|------|-------------|
| 0 | STABILIZE | Self-leveling |
| 1 | ACRO | Rate control |
| 2 | ALT_HOLD | Altitude hold |
| 3 | AUTO | Mission |
| 4 | GUIDED | GCS-controlled |
| 5 | LOITER | Position hold |
| 6 | RTL | Return to launch |
| 7 | CIRCLE | Circle around point |
| 9 | LAND | Auto landing |
| 11 | DRIFT | Drift mode |
| 13 | SPORT | Sport mode |
| 14 | FLIP | Flip |
| 15 | AUTOTUNE | PID auto-tune |
| 16 | POSHOLD | Position hold (GPS) |
| 17 | BRAKE | Stop immediately |
| 18 | THROW | Throw to start |
| 19 | AVOID_ADSB | ADSB avoidance |
| 20 | GUIDED_NOGPS | Guided without GPS |
| 21 | SMART_RTL | Smart RTL (retrace) |
| 22 | FLOWHOLD | Optical flow hold |
| 23 | FOLLOW | Follow mode |
| 24 | ZIGZAG | Zigzag survey |
| 25 | SYSTEMID | System identification |
| 26 | AUTOROTATE | Heli autorotation |
| 27 | AUTO_RTL | Auto RTL |
| 28 | TURTLE | Turtle / flip-over |

##### PLANE_MODE (MAV_TYPE = 1)

| Value | Name | Description |
|-------|------|-------------|
| 0 | MANUAL | Manual |
| 1 | CIRCLE | Circle |
| 2 | STABILIZE | Stabilize |
| 3 | TRAINING | Training |
| 4 | ACRO | Acro |
| 5 | FLY_BY_WIRE_A | FBWA |
| 6 | FLY_BY_WIRE_B | FBWB |
| 7 | CRUISE | Cruise |
| 8 | AUTOTUNE | Auto-tune |
| 10 | AUTO | Mission |
| 11 | RTL | Return to launch |
| 12 | LOITER | Loiter |
| 13 | TAKEOFF | Auto takeoff |
| 14 | AVOID_ADSB | ADSB avoidance |
| 15 | GUIDED | GCS-controlled |
| 17 | QSTABILIZE | VTOL stabilize |
| 18 | QHOVER | VTOL hover |
| 19 | QLOITER | VTOL loiter |
| 20 | QLAND | VTOL land |
| 21 | QRTL | VTOL RTL |
| 24 | THERMAL | Thermal soaring |
| 26 | AUTOLAND | Auto-land |

##### ROVER_MODE (MAV_TYPE = 10)

| Value | Name | Description |
|-------|------|-------------|
| 0 | MANUAL | Manual |
| 1 | ACRO | Acro |
| 3 | STEERING | Steering |
| 4 | HOLD | Hold position |
| 5 | LOITER | Loiter |
| 6 | FOLLOW | Follow |
| 7 | SIMPLE | Simple mode |
| 10 | AUTO | Mission |
| 11 | RTL | Return to launch |
| 12 | SMART_RTL | Smart RTL |
| 15 | GUIDED | GCS-controlled |

### CURRENT_MODE (msg #436) — New Standard Modes

Newer MAVLink extension. `MAV_STANDARD_MODE` enum — vehicle-agnostic:

| Value | Name | Description |
|-------|------|-------------|
| 0 | NON_STANDARD | Mode has no standard equivalent |
| 1 | POSITION_HOLD | Hold position |
| 2 | ORBIT | Circle a point |
| 3 | CRUISE | Maintain heading + altitude |
| 4 | ALTITUDE_HOLD | Hold altitude |
| 5 | SAFE_RECOVERY | RTL / return to safe point |
| 6 | MISSION | Follow mission waypoints |
| 7 | LAND | Auto landing |
| 8 | TAKEOFF | Auto takeoff |

Also includes `custom_mode` (uint32) for autopilot-specific detail.

### AVAILABLE_MODES (msg #435)

Lists all available modes with `standard_mode`, `custom_mode`, and `mode_name` (char[35] human-readable string).

---

## Planned DB Changes for ArduPilot Support

Two new columns in `telemetry_records`:

| Column | Type | Source | Description |
|--------|------|--------|-------------|
| `custom_mode` | INTEGER | HEARTBEAT.custom_mode | ArduPilot vehicle-specific mode enum |
| `base_mode` | INTEGER | HEARTBEAT.base_mode | MAV_MODE_FLAG bitmask (ARMED, STABILIZE, etc.) |

Plus metadata on the `flights` table to identify autopilot type + vehicle type, so the frontend can dispatch to the correct classifier:

- INAV → `classifyFlightMode(active_flight_mode_flags)` (existing bitmask logic)
- ArduPilot → `classifyArduMode(custom_mode, vehicleType)` (new lookup table)

---

## Key Difference Summary

| Aspect | INAV | ArduPilot |
|--------|------|-----------|
| Mode encoding | **Bitmask** (multiple flags OR'd) | **Enum integer** (single active mode) |
| Same number = same mode? | No — combination matters | No — depends on vehicle type |
| Example: value `9` | ANGLE + NAV_ALTHOLD | COPTER: LAND / PLANE: n/a |
| Active modes | Many simultaneous | One primary mode |
| DB field | `active_flight_mode_flags` | `custom_mode` + `base_mode` |
| Classification | Priority-based flag matching | Lookup table per vehicle type |
