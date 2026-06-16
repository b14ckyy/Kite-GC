// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// UAV platform types + 3D-model override types.
//
// The old 2D Leaflet SVG DivIcon factory (createUavIcon / uavShapeForPlatform / SHAPE_*) was
// removed when the 2D map switched to rendering the same procedural glTF models top-down — see
// uavModels.ts (selection), uavMesh.ts (.glb loader) and uavTopDown.ts (canvas renderer).

// ── INAV Platform Types (from mixerConfig.platformType / flyingPlatformType_e) ─────────────
export const PLATFORM_MULTIROTOR = 0;
export const PLATFORM_AIRPLANE   = 1;
export const PLATFORM_HELICOPTER = 2;
export const PLATFORM_TRICOPTER  = 3;
export const PLATFORM_ROVER      = 4;
export const PLATFORM_BOAT       = 5;
export const PLATFORM_OTHER      = 6;
export const PLATFORM_VTOL       = 7; // not an INAV-parsed type — manual override only (quadplane)
export const PLATFORM_GENERIC    = 255; // no FC identity (passive telemetry) → generic arrow marker

export type PlatformType = number;

// 3D-model override (Replay control): 'auto' = pick from the flight's platform type, otherwise
// force a specific model. 'generic' = the flat arrow marker.
export type UavModelKind = 'quad' | 'tricopter' | 'plane' | 'vtol' | 'generic';
export type UavModelOverride = 'auto' | UavModelKind;
