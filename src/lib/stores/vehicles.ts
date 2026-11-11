// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Vehicle registry mirror — which vehicles the backend currently knows and which one the singleton UI
// (widgets, control panel, untargeted commands) follows. Fed by the backend's `vehicle-discovered` /
// `vehicle-lost` / `active-vehicle-changed` events; see docs/02-design/features/multi-vehicle.design.md §6.
//
// Phase A: the `telemetry` store still holds ONE vehicle — the active one. Every telemetry listener
// runs its payload through `isActive()` so a second vehicle on the same link (or a second link) can't
// bleed into the widgets. Phase B turns the store into a per-vehicle map behind the same API.

import { writable, derived, get } from 'svelte/store';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import type { FcInfo } from '$lib/stores/connection';

export interface VehicleSummary {
  vehicleId: string;
  linkId: number;
  sysid: number;
  compid: number;
  fcVariant: string;
  platformType: number;
  mavType: number;
  /** "MSP" | "MAVLink" | "Telemetry" (the backend's protocol label). */
  protocol: string;
  /** The vehicle the link was opened for (handshake target). */
  primary: boolean;
  /** Its own FC hardware id and firmware version, when known — the primary's from the handshake, a
   *  vehicle discovered on a shared link from its own AUTOPILOT_VERSION (re-announced on arrival). */
  fcUid: string | null;
  fcVersion: string | null;
  /** Display name — `${fcVariant} #${sysid}` until the user renames it (Phase D). */
  name: string;
  lastSeen: number;
}

/** One open link as the backend's `list_links` reports it. */
export interface LinkSummary {
  linkId: number;
  /** "MSP" | "MAVLink" | "Telemetry". */
  protocol: string;
  /** Human-readable transport ("Serial COM7 @ 57600", "UDP 127.0.0.1:14550"). */
  transport: string;
  fcInfo: FcInfo;
  primaryVehicleId: string;
}

/** Every open link (mirror of the backend registry; refreshed on connect/close events). */
export const links = writable<LinkSummary[]>([]);

export async function refreshLinks(): Promise<LinkSummary[]> {
  const l = await invoke<LinkSummary[]>('list_links');
  links.set(l);
  return l;
}

/** Every vehicle the backend has announced, keyed by `"L{link}:S{sysid}"`. */
export const vehicles = writable<Map<string, VehicleSummary>>(new Map());

/** The vehicle the singleton UI follows, or null when nothing is connected. */
export const activeVehicleId = writable<string | null>(null);

export const activeVehicle = derived([vehicles, activeVehicleId], ([$vehicles, $id]) =>
  $id ? $vehicles.get($id) ?? null : null
);

/** Module-level mirror so the hot telemetry listeners don't pay for `get()` on every event. */
let activeKey: string | null = null;
activeVehicleId.subscribe((v) => { activeKey = v; });

/** Whether an event payload belongs to the active vehicle.
 *  - No `vehicleId` on the payload (an event that isn't per-vehicle) → true.
 *  - No active vehicle yet → true: a link's first events (home, EKF type one-shots) arrive before the
 *    connect call returns, and dropping them would lose those one-shots. The backend announces the
 *    active vehicle as soon as it registers the link, so this window is a few milliseconds. */
export function isActive(payload: unknown): boolean {
  if (activeKey == null) return true;
  if (payload == null || typeof payload !== 'object') return true;
  const id = (payload as { vehicleId?: unknown }).vehicleId;
  return typeof id !== 'string' || id === activeKey;
}

interface VehicleDiscovered {
  vehicleId: string; linkId: number; sysid: number; compid: number;
  fcVariant: string; platformType: number; mavType: number; protocol: string; primary: boolean;
  fcUid?: string | null; fcVersion?: string | null;
}

function defaultName(v: VehicleDiscovered): string {
  const variant = v.fcVariant || v.protocol;
  return v.sysid > 0 ? `${variant} #${v.sysid}` : variant;
}

let unlisteners: UnlistenFn[] = [];

/** Subscribe to the backend's registry events. Idempotent; call before the first `connect` so the
 *  active-vehicle announcement isn't missed. */
export async function startVehicleListeners(): Promise<void> {
  if (unlisteners.length) return;
  unlisteners.push(
    await listen<VehicleDiscovered>('vehicle-discovered', (e) => {
      const p = e.payload;
      vehicles.update((m) => {
        const next = new Map(m);
        const prev = next.get(p.vehicleId);
        next.set(p.vehicleId, {
          vehicleId: p.vehicleId, linkId: p.linkId, sysid: p.sysid, compid: p.compid,
          fcVariant: p.fcVariant, platformType: p.platformType, mavType: p.mavType,
          protocol: p.protocol, primary: p.primary,
          fcUid: p.fcUid ?? null, fcVersion: p.fcVersion ?? null,
          name: prev?.name ?? defaultName(p),
          lastSeen: Date.now(),
        });
        return next;
      });
    }),
    await listen<{ vehicleId: string; linkId: number; reason: string }>('vehicle-lost', (e) => {
      vehicles.update((m) => {
        if (!m.has(e.payload.vehicleId)) return m;
        const next = new Map(m);
        next.delete(e.payload.vehicleId);
        return next;
      });
    }),
    await listen<{ vehicleId: string | null }>('active-vehicle-changed', (e) => {
      activeVehicleId.set(e.payload.vehicleId);
    }),
    await listen<{ linkId: number; reason: string }>('link-closed', () => {
      void refreshLinks().catch(() => {});
    }),
  );
  // Catch up in case the backend already has links (e.g. a page reload while connected).
  try {
    const current = await invoke<string | null>('get_active_vehicle');
    if (current !== undefined) activeVehicleId.set(current);
  } catch {
    // backend not ready — the events will fill it in
  }
}

export function stopVehicleListeners(): void {
  for (const u of unlisteners) u();
  unlisteners = [];
}

/** Make `vehicleId` the active vehicle (backend + local). The `active-vehicle-changed` event that
 *  follows updates every consumer; callers that need the switch synchronously can await this. */
export async function selectVehicle(vehicleId: string): Promise<void> {
  if (get(activeVehicleId) === vehicleId) return;
  await invoke('set_active_vehicle', { vehicleId });
  activeVehicleId.set(vehicleId);
}

/** Drop all local registry state (disconnect-all). */
export function resetVehicles(): void {
  vehicles.set(new Map());
  links.set([]);
  activeVehicleId.set(null);
}
