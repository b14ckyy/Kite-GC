// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// The vehicle library's types against the platform types (`uavIcons.ts`). "flying_wing" exists only in
// the library and counts as an airplane; "other" names no platform.

import {
  PLATFORM_MULTIROTOR, PLATFORM_AIRPLANE, PLATFORM_HELICOPTER, PLATFORM_TRICOPTER, PLATFORM_ROVER,
  PLATFORM_BOAT, PLATFORM_VTOL,
} from '$lib/helpers/uavIcons';

const PLATFORM_FOR_VEHICLE_TYPE: Record<string, number> = {
  multirotor: PLATFORM_MULTIROTOR, tricopter: PLATFORM_TRICOPTER, fixed_wing: PLATFORM_AIRPLANE,
  flying_wing: PLATFORM_AIRPLANE, helicopter: PLATFORM_HELICOPTER, rover: PLATFORM_ROVER, boat: PLATFORM_BOAT,
  vtol: PLATFORM_VTOL,
};

const VEHICLE_TYPE_FOR_PLATFORM: Record<number, string> = {
  [PLATFORM_MULTIROTOR]: 'multirotor', [PLATFORM_AIRPLANE]: 'fixed_wing', [PLATFORM_HELICOPTER]: 'helicopter',
  [PLATFORM_TRICOPTER]: 'tricopter', [PLATFORM_ROVER]: 'rover', [PLATFORM_BOAT]: 'boat', [PLATFORM_VTOL]: 'vtol',
};

/** The platform a library vehicle type names; undefined for "other" and unknown values. */
export function platformForVehicleType(vehicleType: string): number | undefined {
  return PLATFORM_FOR_VEHICLE_TYPE[vehicleType];
}

/** The library type for a platform type; "other" when the platform is unknown or has no counterpart. */
export function vehicleTypeForPlatform(platformType: number | null | undefined): string {
  return (platformType != null ? VEHICLE_TYPE_FOR_PLATFORM[platformType] : undefined) ?? 'other';
}
