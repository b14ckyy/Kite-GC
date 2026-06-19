// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

/**
 * Stick-input adapter — turns a replay TelemetryRecord's recorded RC channels into normalized Mode-2
 * stick positions for the GimbalStick overlay. **Replay-only**: live recording does not capture RC
 * (MAVLink RC_CHANNELS arrives at ~1 Hz — useless for a stick view, not worth the bandwidth), so this
 * reads the log-imported `rc_command_json` / `rc_data_json` columns. Only INAV blackbox and ArduPilot
 * `.bin` imports populate them today.
 *
 * Two layers can be shown:
 *  - **primary** (theme blue): the everyday signal. INAV blackbox → `rcCommand` (always logged);
 *    ArduPilot `.bin` → `rc_data` (RCIN, the only RC it logs).
 *  - **secondary** (orange, behind): INAV `rcData` (RAW RC straight from the TX) when the log has it —
 *    so you can see the FC overriding the stick in self-level / nav modes (rcCommand ≠ rcData).
 *
 * Channel ORDER differs by firmware: INAV arrays are [Roll, Pitch, Yaw, Throttle]; ArduPilot RCIN is
 * AETR = [Roll, Pitch, Throttle, Yaw]. `rcCommand` only exists for INAV (always INAV order).
 * NORMALIZATION also differs: rcData / RCIN are µs (1000–2000, centre 1500); INAV rcCommand has
 * roll/pitch/yaw already centred at 0 (±500) and throttle in µs.
 */

export interface StickPos { x: number; y: number; }     // each axis −1…+1, +y = up

export interface StickAxes {
  left: StickPos;   // Mode 2: x = yaw, y = throttle
  right: StickPos;  // Mode 2: x = roll, y = pitch
}

export interface StickData {
  primary: StickAxes;
  secondary: StickAxes | null;
}

interface Controls { roll: number; pitch: number; yaw: number; throttle: number; } // each −1…+1

const clamp1 = (v: number) => Math.max(-1, Math.min(1, v));

function parseChannels(json: string | null | undefined): number[] | null {
  if (!json) return null;
  try {
    const arr: unknown = JSON.parse(json);
    if (Array.isArray(arr) && arr.length >= 4 && arr.every((n) => typeof n === 'number')) {
      return arr as number[];
    }
  } catch { /* malformed — treat as absent */ }
  return null;
}

/** Mode-2 stick layout from normalized controls. */
function toAxes(c: Controls): StickAxes {
  return {
    left:  { x: clamp1(c.yaw),  y: clamp1(c.throttle) },
    right: { x: clamp1(c.roll), y: clamp1(c.pitch) },
  };
}

/** INAV rcCommand: roll/pitch/yaw ±500 centred at 0, throttle µs (1000–2000). Order [R,P,Y,T]. */
function fromRcCommand(a: number[]): Controls {
  return { roll: a[0] / 500, pitch: a[1] / 500, yaw: a[2] / 500, throttle: (a[3] - 1500) / 500 };
}

/** Raw RC in µs (1000–2000, centre 1500). `aetr` = ArduPilot RCIN order [R,P,T,Y]; else INAV [R,P,Y,T]. */
function fromRawUs(a: number[], aetr: boolean): Controls {
  const us = (v: number) => (v - 1500) / 500;
  return {
    roll: us(a[0]),
    pitch: us(a[1]),
    throttle: us(aetr ? a[2] : a[3]),
    yaw: us(aetr ? a[3] : a[2]),
  };
}

/** Build the overlay's stick data from a replay record's RC columns. Returns null when the log has no
 *  RC channels (e.g. a .tlog or a live-recorded flight). */
export function computeStickData(
  rcCommandJson: string | null | undefined,
  rcDataJson: string | null | undefined,
  fcVariant: string | null | undefined,
): StickData | null {
  const isArdu = !!fcVariant && fcVariant.toLowerCase().startsWith('ardu');
  const cmd = parseChannels(rcCommandJson);
  const raw = parseChannels(rcDataJson);

  if (cmd) {
    // INAV: rcCommand is the primary; raw rcData (if logged) is the secondary RAW overlay (INAV order).
    return { primary: toAxes(fromRcCommand(cmd)), secondary: raw ? toAxes(fromRawUs(raw, false)) : null };
  }
  if (raw) {
    // Only raw RC (ArduPilot .bin / RCIN): the raw stick is the primary, no secondary.
    return { primary: toAxes(fromRawUs(raw, isArdu)), secondary: null };
  }
  return null;
}
