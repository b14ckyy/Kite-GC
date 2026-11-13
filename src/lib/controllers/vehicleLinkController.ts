// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Flight ↔ library vehicle link decisions that need the user's word (Dev-Docs active/MULTI_VEHICLE.md
// §9 item 1). A flight is linked by its stored `vehicle_id`, resolved when it is stored (FC id first,
// craft name as the fallback). Everything that would re-link EXISTING flights is offered, never done
// silently: after a vehicle gets a craft name / FC id, and after the End-Flight save. No UI here — the
// caller passes its dialog (`ConfirmDialog.show`).

import { get } from 'svelte/store';
import { t } from 'svelte-i18n';
import type { DialogOptions } from '$lib/components/ConfirmDialog.svelte';
import {
  flightSetVehicle,
  vehicleDbFindByFcUid,
  vehicleDbLinkFlights,
  vehicleDbLinkFlightsCount,
  vehicleDbSetFcUid,
} from '$lib/stores/flightlog';
import type { Vehicle, VehicleInput, VehicleResolution } from '$lib/stores/flightlogTypes';
import { vehicleLibraryChanged, type VehicleCreatePrefill } from '$lib/stores/vehicleManager';
import { hardwareUid, shortFcUid } from '$lib/helpers/fcUid';

/** Shows a dialog and resolves to the chosen button value, null = cancelled (`ConfirmDialog.show`). */
export type AskFn = (opts: DialogOptions) => Promise<string | null>;

/** A library vehicle as the create/update payload (drops id, timestamps and the lifetime baseline). */
export function vehicleToInput(v: Vehicle): VehicleInput {
  const {
    id: _id, created_at: _c, updated_at: _u,
    base_flight_count: _bf, base_total_time_s: _bt, base_total_dist_m: _bd, base_total_energy: _be,
    ...rest
  } = v;
  return rest;
}

/** A vehicle as a picker entry: its name, with the craft name — or the shortened FC id when it has
 *  none — in brackets when that differs from the name. Shared by every vehicle picker. */
export function vehiclePickLabel(v: Vehicle): string {
  const id = v.craft_name || (v.fc_uid ? shortFcUid(v.fc_uid) : '');
  return id && id !== v.name ? `${v.name} (${id})` : v.name;
}

function clean(s: string | null | undefined): string | null {
  const v = (s ?? '').trim();
  return v ? v : null;
}

/** Offer to link the existing flights that belong to no vehicle yet and were flown with `craftName`
 *  and/or `fcUid` to vehicle `vehicleId`. No prompt when there is nothing to link (no key, or no such
 *  flight; a synthetic FC id never matches). Returns the number linked. */
export async function offerLinkExistingFlights(
  ask: AskFn,
  vehicleId: number,
  craftName: string | null,
  fcUid: string | null,
  dbPath: string,
): Promise<number> {
  const craft = clean(craftName);
  const uid = clean(fcUid);
  if (!craft && !uid) return 0;
  const count = await vehicleDbLinkFlightsCount(vehicleId, craft, uid, true, dbPath);
  if (count === 0) return 0;
  const tr = get(t);
  const keys = [
    craft ? tr('vehicleLink.byCraft', { values: { craft } }) : null,
    uid ? tr('vehicleLink.byUid') : null,
  ].filter((k): k is string => k != null).join(tr('vehicleLink.or'));
  const ans = await ask({
    title: tr('vehicleLink.title'),
    message: tr('vehicleLink.message', { values: { count, keys } }),
    buttons: [{ label: tr('vehicleLink.link'), value: 'link', primary: true }],
  });
  if (ans !== 'link') return 0;
  return vehicleDbLinkFlights(vehicleId, craft, uid, true, dbPath);
}

/** The End-Flight screen's vehicle decision for one committed flight. */
export interface EndFlightVehicle {
  flightId: number;
  /** The identity the flight was stored with. */
  craftName: string;
  fcUid: string | null;
  /** What the dialog resolved (null: not resolved, e.g. the lookup failed — nothing is changed then). */
  resolution: VehicleResolution | null;
  /** The vehicle the user left selected (null = none). */
  chosen: Vehicle | null;
}

/** Apply the post-flight vehicle choice to the committed flight, then ask what the choice implies:
 *  - matched by FC id → linked, nothing to ask;
 *  - the vehicle stores no FC id but the flight has one (craft-name match or a hand pick) → linked, and
 *    offer to store the FC id in the vehicle and link the existing flights flown with it / the vehicle's
 *    craft name;
 *  - the vehicle stores a different FC id (`uid_conflict`, or a hand pick) → FC swapped (replace the
 *    vehicle's FC id, link), a second aircraft (create a new vehicle), or leave the flight unassigned —
 *    when unsure, unlinked (the automatic link at commit already left such a flight unlinked);
 *  - no vehicle matched and none picked → offer to create one.
 *  An FC id belongs to one vehicle: storing it on the chosen vehicle takes it off the vehicle that holds
 *  it now, and the question says so. Returns the create pre-fill when the user wants a new vehicle (the
 *  caller opens the Manager). */
export async function settleEndFlightVehicle(
  ask: AskFn,
  c: EndFlightVehicle,
  dbPath: string,
): Promise<VehicleCreatePrefill | null> {
  // The dialog could not resolve (lookup failed, or Save before it finished): the flight keeps what
  // the store resolved — never re-link it from a picker that never loaded.
  if (!c.resolution) return null;
  try {
    return await settleResolved(ask, c, c.resolution, dbPath);
  } finally {
    // An open Vehicle Manager / UAV Info / flight detail must not keep working on a stale vehicle.
    vehicleLibraryChanged.update((n) => n + 1);
  }
}

/** The vehicle other than `vehicleId` that stores `fcUid` now, if any. */
async function otherHolder(fcUid: string, vehicleId: number, dbPath: string): Promise<Vehicle | null> {
  const holder = await vehicleDbFindByFcUid(fcUid, dbPath);
  return holder && holder.id !== vehicleId ? holder : null;
}

async function settleResolved(
  ask: AskFn,
  c: EndFlightVehicle,
  resolution: VehicleResolution,
  dbPath: string,
): Promise<VehicleCreatePrefill | null> {
  const tr = get(t);
  const uid = hardwareUid(c.fcUid);
  const craft = clean(c.craftName);
  const link = (id: number | null) => flightSetVehicle(c.flightId, id, dbPath);

  if (!c.chosen) {
    await link(null);
    // "None" picked although a vehicle matched → the user's decision, nothing to offer.
    if (resolution.vehicle) return null;
    // Nothing identifies the aircraft (passive link, simulator): no offer after every such flight.
    if (!craft && !uid) return null;
    const ans = await ask({
      title: tr('endFlight.newVehicleTitle'),
      message: craft
        ? tr('endFlight.newVehicleMessage', { values: { craft } })
        : tr('endFlight.newVehicleMessageNoCraft'),
      buttons: [{ label: tr('endFlight.newVehicleCreate'), value: 'create', primary: true }],
    });
    return ans === 'create' ? { craftName: craft ?? '', fcUid: uid, flightId: c.flightId } : null;
  }

  const v = c.chosen;
  const stored = hardwareUid(v.fc_uid);
  // No hardware id on the flight (passive link, simulator), or the vehicle's own: link, nothing to ask.
  if (!uid || (stored && stored.toLowerCase() === uid.toLowerCase())) {
    await link(v.id);
    return null;
  }
  const holder = await otherHolder(uid, v.id, dbPath);
  const movesFrom = holder ? `\n\n${tr('endFlight.uidMovesFrom', { values: { name: holder.name } })}` : '';

  if (!stored) {
    await link(v.id);
    const count = await vehicleDbLinkFlightsCount(v.id, v.craft_name, uid, true, dbPath);
    const ans = await ask({
      title: tr('endFlight.storeUidTitle'),
      message: tr('endFlight.storeUidMessage', { values: { name: v.name, count } }) + movesFrom,
      buttons: [{ label: tr('endFlight.storeUidYes'), value: 'store', primary: true }],
    });
    if (ans !== 'store') return null;
    await vehicleDbSetFcUid(v.id, uid, dbPath);
    if (count > 0) await vehicleDbLinkFlights(v.id, v.craft_name, uid, true, dbPath);
    return null;
  }

  const ans = await ask({
    title: tr('endFlight.uidConflictTitle'),
    message: tr('endFlight.uidConflictMessage', { values: { name: v.name } }) + movesFrom,
    buttons: [
      { label: tr('endFlight.uidConflictNew'), value: 'new' },
      { label: tr('endFlight.uidConflictReplace'), value: 'replace', primary: true },
    ],
    cancelLabel: tr('endFlight.uidConflictLeave'),
  });
  if (ans === 'replace') {
    // FC swapped: the vehicle takes the new id (off the vehicle that held it); its earlier flights stay
    // linked through vehicle_id, and this flight joins them.
    await vehicleDbSetFcUid(v.id, uid, dbPath);
    await link(v.id);
    return null;
  }
  // A second aircraft (it goes to the new vehicle once that is saved) or unsure: unassigned.
  await link(null);
  return ans === 'new' ? { craftName: craft ?? '', fcUid: uid, flightId: c.flightId } : null;
}
