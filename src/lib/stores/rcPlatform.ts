// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// RC-control target platform (docs/active/MAVLINK_RC_CONTROL.md §2). RC injection is not one mechanism
// across stacks: INAV uses MSP (RAW_RC + AUX_RC), ArduPilot uses MAVLink RC_CHANNELS_OVERRIDE, PX4 uses
// MANUAL_CONTROL. The platform decides the channel split (rcLayout) and which adapter streams (rcStream).
//
// When a FC is connected the platform is DERIVED from it and LOCKED for the session; while disconnected
// the user picks it (offline config) via settings.rcControl.platform. The dropdown behind "Device" binds
// to the offline choice and shows the locked value when connected.

import { derived, get } from 'svelte/store';
import { connection } from './connection';
import { settings } from './settings';
import type { RcPlatform } from './settings';

export type { RcPlatform };

/** Map a connected FC to its RC-control platform. MSP ⇒ INAV; MAVLink ⇒ PX4 if the variant says so,
 *  else ArduPilot (ArduPlane/ArduCopter/…). Passive telemetry can't inject RC → treat as null. */
function platformFromConnection(
  status: string,
  protocol: string,
  fcVariant: string | undefined,
): RcPlatform | null {
  if (status !== 'connected') return null;
  if (protocol === 'msp') return 'inav';
  if (protocol === 'mavlink') return fcVariant?.toUpperCase().includes('PX4') ? 'px4' : 'ardupilot';
  return null;
}

/** The effective platform: the connected FC's (locked) when connected, otherwise the offline choice. */
export const rcPlatform = derived([connection, settings], ([$c, $s]): RcPlatform => {
  return (
    platformFromConnection($c.status, $c.protocolType, $c.fcInfo?.fc_variant) ??
    $s.rcControl.platform
  );
});

/** true while a connected FC dictates the platform — the offline dropdown is then read-only. */
export const rcPlatformLocked = derived(connection, ($c) => $c.status === 'connected');

/** Set the offline platform choice (no-op effect while connected/locked). */
export function setOfflinePlatform(p: RcPlatform): void {
  settings.patch({ rcControl: { ...get(settings).rcControl, platform: p } });
}
