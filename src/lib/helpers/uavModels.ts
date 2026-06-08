// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Shared UAV 3D-model resolution — used by the 3D map (Cesium glTF), the 2D map (canvas top-down
// renderer) and the replay model-override dropdown, so model selection lives in one place.

import {
  PLATFORM_MULTIROTOR, PLATFORM_TRICOPTER, PLATFORM_AIRPLANE, PLATFORM_VTOL,
  type PlatformType, type UavModelKind, type UavModelOverride,
} from './uavIcons';

const KIND_FILE: Record<UavModelKind, string> = {
  quad: 'uav-quad',
  tricopter: 'uav-tricopter',
  plane: 'uav-plane',
  vtol: 'uav-vtol',
  generic: 'uav-arrow',
};

/** Which model to show: an explicit override wins, otherwise derived from the platform type. */
export function resolveModelKind(platformType: PlatformType, override: UavModelOverride = 'auto'): UavModelKind {
  if (override !== 'auto') return override;
  if (platformType === PLATFORM_MULTIROTOR) return 'quad';
  if (platformType === PLATFORM_TRICOPTER) return 'tricopter';
  if (platformType === PLATFORM_AIRPLANE) return 'plane';
  if (platformType === PLATFORM_VTOL) return 'vtol';
  return 'generic';
}

/** Static asset URL for a model kind (SvelteKit serves `static/models/` at the root). */
export function modelUri(kind: UavModelKind): string {
  return `/models/${KIND_FILE[kind]}.glb`;
}

export function modelUriForPlatform(platformType: PlatformType, override: UavModelOverride = 'auto'): string {
  return modelUri(resolveModelKind(platformType, override));
}
