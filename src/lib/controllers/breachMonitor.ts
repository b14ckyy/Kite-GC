// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// In-flight breach toast. INAV does not expose its geozone-avoidance flight mode (NAV_SEND_TO) over MSP,
// so the GCS detects an actual breach itself, geometrically + altitude-aware (3D), from the live UAV
// position vs. the loaded geozones (INAV) / geofence (ArduPilot/PX4). One toast on breach onset, latched
// until it clears (no spam). For ArduPilot the FC's STATUSTEXT is the primary cue (stores/statusText.ts);
// this is the shared fallback. Only runs live + armed. See docs/active/GEOZONES.md / GEOFENCE.md.

import { get } from 'svelte/store';
import { t } from 'svelte-i18n';
import { telemetry } from '$lib/stores/telemetry';
import { connection } from '$lib/stores/connection';
import { homePosition } from '$lib/stores/home';
import { geozoneWorking } from '$lib/stores/geozone';
import { fenceWorking } from '$lib/stores/fence';
import { pushLocalStatus } from '$lib/stores/statusText';
import { checkLiveGeozoneBreach } from '$lib/helpers/geozoneMissionCheck';
import { checkLiveFenceBreach } from '$lib/helpers/fenceBreach';

const ARMING_FLAG_ARMED = 2;
const SEVERITY_CRITICAL = 2; // MAV_SEVERITY: red toast + alarm tone

type BreachKind = 'nfz' | 'inclusion' | 'fence' | null;

let unsub: (() => void) | undefined;
let latched: BreachKind = null; // current breach (hysteresis: toast only on a fresh onset)

function evaluate(): void {
  const tel = get(telemetry);
  const armed = tel.lastUpdate > 0 && (tel.armingFlags & (1 << ARMING_FLAG_ARMED)) !== 0;
  // Live + armed + a real fix only — never during planning or log replay.
  if (get(connection).status !== 'connected' || !armed || tel.fixType < 2) {
    latched = null;
    return;
  }

  let kind: BreachKind = null;

  const gz = get(geozoneWorking);
  if (gz?.has_geozones && gz.zones.length > 0) {
    const home = get(homePosition);
    // Launch ground MSL for AMSL zone bands: the FC home altitude, else derive from MSL − relative alt.
    const homeAmsl = home.set ? home.alt : tel.altMsl - tel.altitude;
    const r = checkLiveGeozoneBreach(
      gz.zones,
      { lat: tel.lat, lon: tel.lon, relM: tel.altitude },
      home.set ? { lat: home.lat, lon: home.lon } : null,
      homeAmsl,
    );
    if (r.nfz) kind = 'nfz';
    else if (r.inclusion) kind = 'inclusion';
  }

  const fc = get(fenceWorking);
  if (!kind && fc?.has_fence && fc.zones.length > 0) {
    if (checkLiveFenceBreach(fc, { lat: tel.lat, lon: tel.lon, relM: tel.altitude })) kind = 'fence';
  }

  if (kind && kind !== latched) {
    const tr = get(t);
    const msg =
      kind === 'nfz' ? tr('geozone.breachNfz')
      : kind === 'inclusion' ? tr('geozone.breachInclusion')
      : tr('fence.breach');
    pushLocalStatus(SEVERITY_CRITICAL, msg);
  }
  latched = kind;
}

/** Start watching the live telemetry stream for breaches. Idempotent. */
export function startBreachMonitor(): void {
  if (unsub) return;
  unsub = telemetry.subscribe(() => evaluate());
}

export function stopBreachMonitor(): void {
  unsub?.();
  unsub = undefined;
  latched = null;
}
