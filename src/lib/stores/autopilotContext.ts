import { writable, derived, get, type Readable } from 'svelte/store';
import { connection } from './connection';
import { settings } from './settings';
import {
  mission, missionResetMemory, resetMultiMission, getTotalWpCount,
  selectedWpIndex, MAX_WAYPOINTS_TOTAL,
} from './mission';
import { arduMission, arduMissionClear, arduSelectedWpIndex } from './missionArdupilot';
import { inavToArdu, arduToInav } from '../helpers/missionConverter';

export type AutopilotSystem = 'inav' | 'ardupilot' | 'px4';

export interface SystemSwitchRequest {
  from: AutopilotSystem;
  to: AutopilotSystem;
  trigger: 'connection' | 'manual';
  hasMission: boolean;
}

function variantToSystem(variant: string): AutopilotSystem | null {
  const v = variant.toLowerCase();
  if (v === 'inav') return 'inav';
  if (v === 'ardupilot') return 'ardupilot';
  if (v === 'px4') return 'px4';
  return null;
}

function coerceSystem(s: string): AutopilotSystem {
  if (s === 'ardupilot' || s === 'px4') return s;
  return 'inav';
}

function sourceHasMission(system: AutopilotSystem): boolean {
  if (system === 'inav') return getTotalWpCount() > 0;
  return get(arduMission).length > 0;
}

function sameFamily(a: AutopilotSystem, b: AutopilotSystem): boolean {
  return (a === 'ardupilot' || a === 'px4') && (b === 'ardupilot' || b === 'px4');
}

const _system = writable<AutopilotSystem>(coerceSystem(get(settings).lastAutopilotSystem));
const _pendingSwitch = writable<SystemSwitchRequest | null>(null);

export const autopilotSystem: Readable<AutopilotSystem> = { subscribe: _system.subscribe };
export const autopilotLocked: Readable<boolean> = derived(connection, c => c.status === 'connected');
export const pendingSystemSwitch: Readable<SystemSwitchRequest | null> = { subscribe: _pendingSwitch.subscribe };

export function setAutopilotSystem(system: AutopilotSystem): void {
  if (get(autopilotLocked)) return;
  const current = get(_system);
  if (current === system) return;
  if (sameFamily(current, system)) {
    _applySwitch(system);
    return;
  }
  const hasMission = sourceHasMission(current);
  if (hasMission) {
    _pendingSwitch.set({ from: current, to: system, trigger: 'manual', hasMission: true });
    return;
  }
  _applySwitch(system);
}

export function confirmSystemSwitch(action: 'keep' | 'clear' | 'convert' = 'clear'): void {
  const pending = get(_pendingSwitch);
  if (!pending) return;

  if (action === 'convert') {
    _convertMission(pending.from, pending.to);
  }
  if (action === 'clear' || action === 'convert') {
    _clearSourceMission(pending.from);
  }

  _pendingSwitch.set(null);
  _applySwitch(pending.to);
}

export function cancelSystemSwitch(): void {
  _pendingSwitch.set(null);
}

function _applySwitch(system: AutopilotSystem): void {
  _system.set(system);
  settings.patch({ lastAutopilotSystem: system });
}

function _clearSourceMission(from: AutopilotSystem): void {
  if (from === 'inav') {
    missionResetMemory();
    resetMultiMission();
    selectedWpIndex.set(-1);
  } else {
    arduMissionClear();
    arduSelectedWpIndex.set(-1);
  }
}

function _convertMission(from: AutopilotSystem, to: AutopilotSystem): void {
  if (from === 'inav') {
    const wps = get(mission).waypoints;
    if (wps.length > 0) {
      arduMission.set(inavToArdu(wps));
      arduSelectedWpIndex.set(-1);
    }
  } else {
    const wps = get(arduMission);
    if (wps.length > 0) {
      const converted = arduToInav(wps);
      mission.set({
        waypoints: converted,
        info: { max_waypoints: MAX_WAYPOINTS_TOTAL, is_valid: true, wp_count: converted.length },
        dirty: true,
      });
      selectedWpIndex.set(-1);
    }
  }
}

let _lastDetectedVariant = '';

connection.subscribe(conn => {
  if (conn.status === 'disconnected') {
    _lastDetectedVariant = '';
    return;
  }
  if (conn.status !== 'connected' || !conn.fcInfo) return;
  if (conn.fcInfo.fc_variant === _lastDetectedVariant) return;
  _lastDetectedVariant = conn.fcInfo.fc_variant;

  const detected = variantToSystem(conn.fcInfo.fc_variant);
  if (!detected) return;

  const current = get(_system);
  if (detected === current) return;

  const hasMission = sourceHasMission(current);
  if (hasMission) {
    _pendingSwitch.set({ from: current, to: detected, trigger: 'connection', hasMission: true });
  } else {
    _applySwitch(detected);
  }
});
