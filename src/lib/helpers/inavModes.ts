// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// INAV flight-mode boxes — permanent box-ID → name + safety category (docs/active/RC_CONTROL.md
// safety section). IDs/names from INAV `fc/fc_msp_box.c`. Used for the mode labels under each channel
// and the RC safety locks/warnings:
//   • critical — RTH/FAILSAFE override every other mode (incl. MANUAL); ARM on a latched AUX channel
//     could never be disarmed on GCS loss → if on a controlled AUX channel, block ALL RC output.
//   • gps — autonomous/auto-throttle modes (CRUISE/WP/POSHOLD/ALTHOLD); overridable only by MANUAL or
//     RTH on link loss → yellow warning (escalates to a block if MANUAL isn't configured).
//   • manual — MANUAL: the manual-control fallback.

export type BoxCategory = 'critical' | 'gps' | 'manual' | 'other';

interface InavBox {
  name: string;
  category: BoxCategory;
}

const BOXES: Record<number, InavBox> = {
  0: { name: 'ARM', category: 'critical' },
  1: { name: 'ANGLE', category: 'other' },
  2: { name: 'HORIZON', category: 'other' },
  3: { name: 'ALTHOLD', category: 'gps' },
  5: { name: 'HEADING HOLD', category: 'other' },
  6: { name: 'HEADFREE', category: 'other' },
  7: { name: 'HEADADJ', category: 'other' },
  8: { name: 'CAMSTAB', category: 'other' },
  10: { name: 'NAV RTH', category: 'critical' },
  11: { name: 'NAV POSHOLD', category: 'gps' },
  12: { name: 'MANUAL', category: 'manual' },
  13: { name: 'BEEPER', category: 'other' },
  15: { name: 'LEDS OFF', category: 'other' },
  16: { name: 'LIGHTS', category: 'other' },
  19: { name: 'OSD OFF', category: 'other' },
  20: { name: 'TELEMETRY', category: 'other' },
  21: { name: 'AUTO TUNE', category: 'other' },
  26: { name: 'BLACKBOX', category: 'other' },
  27: { name: 'FAILSAFE', category: 'critical' },
  28: { name: 'NAV WP', category: 'gps' },
  29: { name: 'AIR MODE', category: 'other' },
  30: { name: 'HOME RESET', category: 'other' },
  31: { name: 'GCS NAV', category: 'other' },
  32: { name: 'FPV ANGLE MIX', category: 'other' },
  33: { name: 'SURFACE', category: 'other' },
  34: { name: 'FLAPERON', category: 'other' },
  35: { name: 'TURN ASSIST', category: 'other' },
  36: { name: 'NAV LAUNCH', category: 'other' },
  37: { name: 'SERVO AUTOTRIM', category: 'other' },
  45: { name: 'NAV COURSE HOLD', category: 'other' },
  46: { name: 'MC BRAKING', category: 'other' },
  47: { name: 'USER1', category: 'other' },
  48: { name: 'USER2', category: 'other' },
  49: { name: 'LOITER CHANGE', category: 'other' },
  50: { name: 'MSP RC OVERRIDE', category: 'other' },
  51: { name: 'PREARM', category: 'other' },
  52: { name: 'TURTLE', category: 'other' },
  53: { name: 'NAV CRUISE', category: 'gps' },
  54: { name: 'AUTO LEVEL TRIM', category: 'other' },
  55: { name: 'WP PLANNER', category: 'other' },
  56: { name: 'SOARING', category: 'other' },
  57: { name: 'USER3', category: 'other' },
  58: { name: 'USER4', category: 'other' },
  64: { name: 'ANGLE HOLD', category: 'other' },
};

/** Permanent ID of the MANUAL box (the manual-control fallback). */
export const BOX_MANUAL = 12;
/** Permanent ID of the MSP RC OVERRIDE box (activates GCS override of a normal RX). */
export const BOX_MSP_RC_OVERRIDE = 50;

export function boxName(permanentId: number): string {
  return BOXES[permanentId]?.name ?? `Mode ${permanentId}`;
}

export function boxCategory(permanentId: number): BoxCategory {
  return BOXES[permanentId]?.category ?? 'other';
}
