// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// What the mini view renders. Pure on purpose: the desktop mini window imports this without the app's stores.

import type { InterfaceSettings } from '$lib/stores/settings';
import type { ArmingLevel } from '$lib/helpers/arming';

/** `off` = nothing connected, `waiting` = connected without telemetry yet, `stale` = no data for 5 s. */
export type MiniLink = 'off' | 'waiting' | 'live' | 'stale';

/** Raw values in SI units (m, m/s, V); the component formats them with `units`. */
export interface MiniSnapshot {
  /** Date.now() of the main window when it was taken. */
  at: number;
  link: MiniLink;
  /** null until a status frame has arrived. */
  arming: ArmingLevel | null;
  /** FC-native charge, null when the FC reports none. */
  batteryPct: number | null;
  voltage: number | null;
  /** Canonical flight-mode id (flightModeRegistry), '' when unknown. */
  mode: string;
  homeDistM: number | null;
  /** Bearing to home relative to the aircraft's heading, degrees clockwise. */
  homeRelDeg: number | null;
  altM: number | null;
  varioMs: number | null;
  speedMs: number | null;
  /** The speed is airspeed (else ground speed), by the Speed widget's rule. */
  speedIsAir: boolean;
  sats: number | null;
  /** Unified fix scale: 0 none · 1 2D · 2 3D · 3 DGPS/RTK. */
  fixType: number;
  /** MAVLink GPS_FIX_TYPE behind the fix, null where only the unified scale exists. */
  fixRaw: number | null;
  /** RC link: LQ when the protocol has it, else RSSI %; null when neither. */
  rcPct: number | null;
  rcIsLq: boolean;
  units: InterfaceSettings;
}

/** Must match `commands::mini_view::MINI_LABEL`. */
export const MINI_LABEL = 'mini';
export const MINI_SNAPSHOT_EVENT = 'mini-view-snapshot';
export const MINI_VISIBLE_EVENT = 'mini-view-visible';
/** Dispatched on `window` by MainActivity when Android enters or leaves picture-in-picture. */
export const MINI_PIP_EVENT = 'kite-pip';
export const MINI_SNAPSHOT_MS = 500;
/** The mini window counts the feed as stale after this long without a snapshot. */
export const MINI_SNAPSHOT_TIMEOUT_MS = 3000;
