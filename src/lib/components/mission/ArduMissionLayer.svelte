<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- ArduMissionLayer.svelte
     Renders ArduPilot mission waypoints on the Leaflet map and the catalog-driven editor popup.
     Location (NAV) commands are markers; non-location commands are shown as INAV-style modifiers
     under their preceding waypoint. The editable fields come entirely from arduCommandCatalog.
     Usage: <ArduMissionLayer {map} />
-->
<script lang="ts">
  import { onDestroy } from 'svelte';
  import L from 'leaflet';
  import { get } from 'svelte/store';
  import { t } from 'svelte-i18n';
  import {
    arduMission, arduSelectedWpIndex, arduEditMode, arduVehicleClass,
    arduUpdateWp, arduRemoveWp, arduAddWp,
    groupArduMission, groupEndIndex, type ArduGroup,
    MAV_CMD_NAV_WAYPOINT,
    MAV_FRAME_GLOBAL, MAV_FRAME_GLOBAL_RELATIVE_ALT, MAV_FRAME_GLOBAL_TERRAIN_ALT,
    type ArduWaypoint, type MavFrame,
  } from '$lib/stores/missionArdupilot';
  import {
    CMD,
    cmdDef, cmdName, cmdRawName, cmdHasLocation, cmdStandaloneCoordinate, cmdIsLoiter, cmdIsTakeoff,
    cmdDefaultParams, cmdDefaultCoordParams, locationCommandsByCategory, modifierCommandsByCategory,
    enumLabel,
    type ParamSpec, type ParamIndex, type VehicleClass, type Firmware,
  } from '$lib/helpers/arduCommandCatalog';
  import { autopilotSystem, type AutopilotSystem } from '$lib/stores/autopilotContext';
  import { connection } from '$lib/stores/connection';
  import { arduWpDetailLines } from '$lib/helpers/missionWpDetails';
  import {
    newPopupState, renderEditorPopup, closeEditorPopup,
    numInputHtml, enumSelectHtml, paramRow, canonicalLabel, actionsHtml, modifierSection,
    addModifierSelect, attachNumBtnEvents, disablePopupPropagation,
  } from '$lib/helpers/missionEditorPopup';
  import { iconForArduWp } from '$lib/helpers/missionIconsArdupilot';
  import { settings } from '$lib/stores/settings';
  import { homePosition, type HomePosition } from '$lib/stores/home';
  import { activeWpNumber } from '$lib/stores/navStatus';

  interface Props { map: L.Map; }
  let { map }: Props = $props();

  let currentWps     = $state<ArduWaypoint[]>(get(arduMission));
  let currentSelIdx  = $state<number>(get(arduSelectedWpIndex));
  let currentEditing = $state<boolean>(get(arduEditMode));
  let currentHome    = $state<HomePosition>(get(homePosition));
  let currentVehicle = $state<VehicleClass>(get(arduVehicleClass));
  let currentActiveWp = $state<number>(get(activeWpNumber));
  let currentSystem  = $state<AutopilotSystem>(get(autopilotSystem));
  const firmware = $derived<Firmware>(currentSystem === 'px4' ? 'px4' : 'ardupilot');
  let connected      = $state<boolean>(get(connection).status === 'connected');

  const unsubMission  = arduMission.subscribe(wps => { currentWps = wps; });
  const unsubSelIdx   = arduSelectedWpIndex.subscribe(i => { currentSelIdx = i; });
  const unsubEditMode = arduEditMode.subscribe(e => { currentEditing = e; });
  const unsubHome     = homePosition.subscribe(h => { currentHome = h; });
  const unsubVehicle  = arduVehicleClass.subscribe(v => { currentVehicle = v; });
  const unsubActiveWp = activeWpNumber.subscribe(v => { currentActiveWp = v; });
  const unsubSystem   = autopilotSystem.subscribe(s => { currentSystem = s; });
  const unsubConn     = connection.subscribe(c => { connected = c.status === 'connected'; });

  // svelte-ignore state_referenced_locally
  const missionGroup = L.layerGroup().addTo(map);
  // Shared editor-popup lifecycle (content-signature redraw guard lives in the framework module).
  const popupState = newPopupState();
  // Which "Advanced" param sections are expanded, keyed per section ('primary' / 'mod-<idx>'). Persisted
  // here (not just in the DOM) so a value edit — which rebuilds the popup HTML — keeps them open.
  const expandedAdv = new Set<string>();

  /** Permanent on-map reference label (edit mode): altitude + frame, plus a compact summary of the
   *  command's key params (loiter radius/time, hold, …) — the ArduPilot/PX4 counterpart to INAV's
   *  edit-mode alt/speed labels. Speed isn't a per-WP field in ArduPilot (it's DO_CHANGE_SPEED), so
   *  altitude is the primary reference. */
  function arduParamLabelHtml(wp: ArduWaypoint): string {
    const lines = [`${wp.alt.toFixed(0)}m ${frameLabel(wp.frame)}`];
    const def = cmdDef(wp.command);
    if (def?.params) {
      const vals = [wp.param1, wp.param2, wp.param3, wp.param4];
      const parts: string[] = [];
      for (const pidx of [1, 2, 3, 4] as const) {
        const spec = def.params[pidx];
        if (!spec || spec.advanced) continue;
        const v = vals[pidx - 1];
        if (spec.enumStrings && spec.enumValues) parts.push(enumLabel(spec, v));
        else if (v !== 0) parts.push(`${v}${spec.units ?? ''}`);
      }
      if (parts.length) lines.push(parts.slice(0, 2).join(' · '));
    }
    return `<div class="wp-param-label">${lines.join('<br>')}</div>`;
  }

  function createParamLabel(latLng: L.LatLng, html: string): L.Marker {
    return L.marker(latLng, {
      icon: L.divIcon({ className: 'wp-param-label-wrapper', html, iconSize: [1, 1], iconAnchor: [-12, 10] }),
      interactive: false,
    });
  }

  function frameLabel(frame: MavFrame): string {
    if (frame === MAV_FRAME_GLOBAL) return 'AMSL';
    if (frame === MAV_FRAME_GLOBAL_TERRAIN_ALT) return 'TRN';
    return 'REL';
  }

  function nextFrame(frame: MavFrame): MavFrame {
    if (frame === MAV_FRAME_GLOBAL_RELATIVE_ALT) return MAV_FRAME_GLOBAL;
    if (frame === MAV_FRAME_GLOBAL) return MAV_FRAME_GLOBAL_TERRAIN_ALT;
    return MAV_FRAME_GLOBAL_RELATIVE_ALT;
  }

  // ── Catalog-driven editor HTML ──────────────────────────────────────

  function paramFieldHtml(spec: ParamSpec, value: number, dataAttr: string): string {
    if (spec.enumStrings && spec.enumValues) {
      const options = spec.enumValues.map((v, i) => [v, spec.enumStrings![i]] as [number, string]);
      return enumSelectHtml(dataAttr, options, value);
    }
    const step = spec.decimals && spec.decimals > 0 ? Math.pow(10, -spec.decimals) : 1;
    return numInputHtml(dataAttr, value, { step, min: spec.min, max: spec.max });
  }

  function paramRowsHtml(wp: ArduWaypoint, target: 'primary' | 'mod', modIdx = -1): string {
    const def = cmdDef(wp.command);
    if (!def?.params) return '';
    let normal = '';
    let advanced = '';
    for (const pidx of [1, 2, 3, 4, 5, 6, 7] as ParamIndex[]) {
      const spec = def.params[pidx];
      if (!spec) continue;
      // params 5/6/7 are stored in x/y/z (lat/lon/alt) — read them raw for non-coordinate commands.
      const value = pidx <= 4
        ? [wp.param1, wp.param2, wp.param3, wp.param4][pidx - 1]
        : [wp.lat, wp.lon, wp.alt][pidx - 5];
      const dataAttr = target === 'primary'
        ? `data-target="primary" data-pidx="${pidx}"`
        : `data-target="mod" data-modidx="${modIdx}" data-pidx="${pidx}"`;
      const row = paramRow(spec.label, paramFieldHtml(spec, value, dataAttr), { unit: spec.units, tooltip: spec.tooltip });
      if (spec.advanced) advanced += row; else normal += row;
    }
    // Advanced (rare) params collapse under an expander, QGC-style — common fields stay uncluttered.
    if (advanced) {
      const key = target === 'primary' ? 'primary' : `mod-${modIdx}`;
      const open = expandedAdv.has(key);
      normal += `<button type="button" class="wpe-adv-toggle" data-advtoggle data-advkey="${key}">${open ? '▴' : '▾'} ${$t('missionLayer.advanced')}</button>`
        + `<div class="wpe-adv-body" data-advbody${open ? '' : ' hidden'}>${advanced}</div>`;
    }
    return normal;
  }

  function categorizedOptions(
    groups: { category: string; cmds: { id: number; friendlyName: string }[] }[],
    selected: number,
  ): string {
    return groups.map(g =>
      `<optgroup label="${g.category}">`
      + g.cmds.map(c => `<option value="${c.id}"${c.id === selected ? ' selected' : ''}>${c.friendlyName}</option>`).join('')
      + `</optgroup>`,
    ).join('');
  }

  function primaryCmdSelectHtml(cmd: number, vehicle: VehicleClass): string {
    const groups = locationCommandsByCategory(vehicle, firmware);
    let opts = categorizedOptions(groups, cmd);
    // Keep an out-of-catalog / non-location current command selectable so it round-trips.
    if (!groups.some(g => g.cmds.some(c => c.id === cmd))) {
      opts = `<option value="${cmd}" selected>${cmdName(cmd)}</option>` + opts;
    }
    return `<select data-field="cmdType" class="wpe-cmd-select">${opts}</select>`;
  }

  function addModSelectHtml(vehicle: VehicleClass): string {
    const groups = modifierCommandsByCategory(vehicle, firmware);
    const opts = `<option value="" selected disabled>${$t('missionLayer.addModifier')}</option>`
      + categorizedOptions(groups, -1);
    return addModifierSelect(opts);
  }

  function buildGroupEditorHtml(group: ArduGroup, total: number, vehicle: VehicleClass): string {
    const wp = group.anchor!;
    const idx = group.anchorIdx;
    const def = cmdDef(wp.command);
    const displayNum = idx + 1;

    let html = `<div class="wp-editor-popup wp-editor-popup-ardu">`;
    html += `<div class="wpe-header"><span class="wpe-num">WP${displayNum}</span>${primaryCmdSelectHtml(wp.command, vehicle)}</div>`;
    html += `<div class="wpe-canonical-line"><span class="wpe-canonical">${cmdRawName(wp.command)}</span></div>`;

    if (def?.specifiesCoordinate) {
      html += `<div class="wpe-row"><label>${$t('missionLayer.alt')}</label>`
        + `<div class="wpe-num-ctrl"><button class="wpe-num-btn" data-numdir="-1">−</button>`
        + `<input type="number" data-field="alt" value="${wp.alt}" step="1" min="0"/>`
        + `<button class="wpe-num-btn" data-numdir="1">+</button></div>`
        + `<button data-field="frameToggle" class="wpe-toggle">${frameLabel(wp.frame)}</button></div>`;
      // Takeoff coords are hidden when connected (it takes off from the FC home), but editable offline
      // so the operator can place it precisely (drag on the map or type here).
      if (!def.isTakeoff || !connected) {
        const latDeg = (wp.lat / 1e7).toFixed(7);
        const lonDeg = (wp.lon / 1e7).toFixed(7);
        html += `<div class="wpe-row"><label>${$t('missionLayer.lat')}</label><input type="number" data-field="lat" value="${latDeg}" step="0.0000001" min="-90" max="90" class="wpe-coord-input"/></div>`;
        html += `<div class="wpe-row"><label>${$t('missionLayer.lon')}</label><input type="number" data-field="lon" value="${lonDeg}" step="0.0000001" min="-180" max="180" class="wpe-coord-input"/></div>`;
      }
    }

    html += paramRowsHtml(wp, 'primary');

    // Modifiers (non-location commands attached to this waypoint), INAV-style — numbered (ArduPilot
    // numbers every mission item).
    for (const m of group.modifiers) {
      const def2 = cmdDef(m.wp.command);
      const applies = def2?.appliesTo === 'next' ? ` <span class="wpe-applies">${$t('missionLayer.appliesNext')}</span>` : '';
      const header = `${m.idx + 1} ${canonicalLabel(cmdName(m.wp.command), cmdRawName(m.wp.command))}${applies}`;
      html += modifierSection(header, paramRowsHtml(m.wp, 'mod', m.idx), `data-modremove="${m.idx}"`, $t('missionLayer.removeModifier'));
    }

    html += addModSelectHtml(vehicle);
    html += actionsHtml({
      disableUp: idx <= 0, disableDown: idx >= total - 1,
      upTitle: $t('missionLayer.moveUp'), downTitle: $t('missionLayer.moveDown'), removeTitle: $t('missionLayer.removeWp'),
    });
    html += `</div>`;
    return html;
  }

  function setParam(targetIdx: number, pidx: number, value: number) {
    const cur = get(arduMission)[targetIdx];
    if (!cur) return;
    const patch = { ...cur };
    if (pidx === 1) patch.param1 = value;
    else if (pidx === 2) patch.param2 = value;
    else if (pidx === 3) patch.param3 = value;
    else if (pidx === 4) patch.param4 = value;
    else if (pidx === 5) patch.lat = value; // params 5/6/7 = x/y/z
    else if (pidx === 6) patch.lon = value;
    else patch.alt = value;
    arduUpdateWp(targetIdx, patch);
  }

  function moveGroup(group: ArduGroup, dir: -1 | 1) {
    const groups = groupArduMission(get(arduMission));
    const gi = groups.findIndex(g => g.anchorIdx === group.anchorIdx);
    const target = gi + dir;
    if (gi < 0 || target < 0 || target >= groups.length) return;
    const order = [...groups];
    const [moved] = order.splice(gi, 1);
    order.splice(target, 0, moved);
    const flat: ArduWaypoint[] = [];
    let newAnchor = -1;
    for (const g of order) {
      if (g === moved) newAnchor = flat.length;
      if (g.anchor) flat.push(g.anchor);
      for (const m of g.modifiers) flat.push(m.wp);
    }
    arduMission.set(flat);
    arduSelectedWpIndex.set(newAnchor);
  }

  function attachGroupEditorEvents(popup: L.Popup, group: ArduGroup) {
    const el = popup.getElement();
    if (!el) return;
    const idx = group.anchorIdx;
    const wp = () => get(arduMission)[idx];

    (el.querySelector('select[data-field="cmdType"]') as HTMLSelectElement | null)?.addEventListener('change', (e) => {
      const newCmd = Number((e.target as HTMLSelectElement).value);
      const cur = wp(); if (!cur) return;
      arduUpdateWp(idx, { ...cur, command: newCmd, ...cmdDefaultParams(newCmd) });
    });

    (el.querySelector('input[data-field="alt"]') as HTMLInputElement | null)?.addEventListener('change', (e) => {
      const cur = wp(); if (!cur) return;
      arduUpdateWp(idx, { ...cur, alt: Number((e.target as HTMLInputElement).value) });
    });

    (el.querySelector('button[data-field="frameToggle"]') as HTMLButtonElement | null)?.addEventListener('click', () => {
      const cur = wp(); if (!cur) return;
      arduUpdateWp(idx, { ...cur, frame: nextFrame(cur.frame) });
    });

    const latInput = el.querySelector('input[data-field="lat"]') as HTMLInputElement | null;
    const lonInput = el.querySelector('input[data-field="lon"]') as HTMLInputElement | null;
    function applyCoordChange() {
      const newLat = parseFloat(latInput?.value ?? '');
      const newLon = parseFloat(lonInput?.value ?? '');
      if (isNaN(newLat) || isNaN(newLon)) return;
      const cur = wp(); if (!cur) return;
      arduUpdateWp(idx, { ...cur, lat: Math.round(newLat * 1e7), lon: Math.round(newLon * 1e7) });
      map.panTo(L.latLng(newLat, newLon));
    }
    latInput?.addEventListener('change', applyCoordChange);
    lonInput?.addEventListener('change', applyCoordChange);

    // Generic param binding (primary + modifiers), via data-target / data-modidx / data-pidx.
    el.querySelectorAll('[data-pidx]').forEach((node) => {
      node.addEventListener('change', () => {
        const n = node as HTMLInputElement | HTMLSelectElement;
        const pidx = Number(n.dataset.pidx);
        const targetIdx = n.dataset.target === 'mod' ? Number(n.dataset.modidx) : idx;
        setParam(targetIdx, pidx, Number(n.value));
      });
    });

    el.querySelectorAll('button[data-modremove]').forEach((btn) => {
      btn.addEventListener('click', () => {
        arduRemoveWp(Number((btn as HTMLElement).dataset.modremove));
        arduSelectedWpIndex.set(idx);
      });
    });

    (el.querySelector('select[data-field="addMod"]') as HTMLSelectElement | null)?.addEventListener('change', (e) => {
      const newCmd = Number((e.target as HTMLSelectElement).value);
      if (!newCmd) return;
      const cur = wp();
      const standalone = cmdStandaloneCoordinate(newCmd);
      const coord = cmdDefaultCoordParams(newCmd); // params 5/6/7 defaults (data-in-x/y/z commands)
      const mod: ArduWaypoint = {
        command: newCmd, frame: MAV_FRAME_GLOBAL_RELATIVE_ALT, ...cmdDefaultParams(newCmd),
        lat: standalone && cur ? cur.lat : (coord.x ?? 0),
        lon: standalone && cur ? cur.lon : (coord.y ?? 0),
        alt: coord.z ?? 0, autocontinue: true,
      };
      // A JUMP_TAG marks the waypoint being edited as the jump target, so it must go BEFORE the anchor
      // (the FC resumes at the next nav waypoint after the tag = this waypoint). All other modifiers
      // act on / gate from this waypoint, so they trail it.
      const leading = newCmd === CMD.JUMP_TAG;
      const at = leading ? group.anchorIdx : groupEndIndex(group);
      const updated = [...get(arduMission)];
      updated.splice(at, 0, mod);
      arduMission.set(updated);
      // Inserting before the anchor shifts it down one; keep the editor on the same waypoint.
      arduSelectedWpIndex.set(leading ? idx + 1 : idx);
    });

    // Advanced-params expander(s): toggle the sibling body AND remember the state in `expandedAdv`, so a
    // subsequent value edit (which rebuilds the popup HTML) re-renders the section already open.
    el.querySelectorAll('[data-advtoggle]').forEach((btn) => {
      btn.addEventListener('click', (e) => {
        e.preventDefault(); e.stopPropagation();
        const b = btn as HTMLElement;
        const body = b.nextElementSibling as HTMLElement | null;
        if (!body) return;
        const key = b.dataset.advkey ?? 'primary';
        const show = body.hasAttribute('hidden');
        if (show) { body.removeAttribute('hidden'); expandedAdv.add(key); }
        else { body.setAttribute('hidden', ''); expandedAdv.delete(key); }
        b.textContent = `${show ? '▴' : '▾'} ${$t('missionLayer.advanced')}`;
      });
    });

    attachNumBtnEvents(el);

    el.querySelector('button[data-action="moveUp"]')?.addEventListener('click', () => moveGroup(group, -1));
    el.querySelector('button[data-action="moveDown"]')?.addEventListener('click', () => moveGroup(group, 1));
    el.querySelector('button[data-action="remove"]')?.addEventListener('click', () => {
      // Remove the waypoint and its attached modifiers (back-to-front to keep indices valid).
      const idxs = [group.anchorIdx, ...group.modifiers.map(m => m.idx)].sort((a, b) => b - a);
      for (const i of idxs) arduRemoveWp(i);
      arduSelectedWpIndex.set(-1);
    });

    disablePopupPropagation(el);
  }

  // ── Takeoff position resolution (no real coords → anchor on home/centroid) ──

  function resolveTakeoffLatLng(wps: ArduWaypoint[], home: HomePosition): L.LatLng | null {
    if (home.set) return L.latLng(home.lat, home.lon);
    let sumLat = 0, sumLon = 0, n = 0;
    for (const w of wps) {
      if (cmdIsTakeoff(w.command) || !cmdHasLocation(w.command)) continue;
      if (w.lat === 0 && w.lon === 0) continue;
      sumLat += w.lat / 1e7; sumLon += w.lon / 1e7; n++;
    }
    return n > 0 ? L.latLng(sumLat / n, sumLon / n) : null;
  }

  function wpDisplayLatLng(wp: ArduWaypoint, wps: ArduWaypoint[], home: HomePosition, conn: boolean): L.LatLng | null {
    if (cmdIsTakeoff(wp.command)) {
      // Offline the operator may position the takeoff freely (stored coords win) so it stays out of the
      // way — also the correct target for PX4 / ArduPlane takeoff. With a UAV connected the real takeoff
      // is the FC home, so anchor there and lock it (mirrors the "locked when connected" mission rule).
      if (!conn && (wp.lat !== 0 || wp.lon !== 0)) return L.latLng(wp.lat / 1e7, wp.lon / 1e7);
      return resolveTakeoffLatLng(wps, home);
    }
    return L.latLng(wp.lat / 1e7, wp.lon / 1e7);
  }

  function findPrevLocationIdx(wps: ArduWaypoint[], fromIndex: number): number {
    for (let i = fromIndex - 1; i >= 0; i--) {
      if (cmdHasLocation(wps[i].command)) return i;
    }
    return -1;
  }

  /** First location waypoint at or after `fromIndex` (the visual target of a jump that lands on a
   *  non-location item such as a Jump Tag). */
  function findNextLocationIdx(wps: ArduWaypoint[], fromIndex: number): number {
    for (let i = Math.max(0, fromIndex); i < wps.length; i++) {
      if (cmdHasLocation(wps[i].command)) return i;
    }
    return -1;
  }

  /** Resolve where a jump points, as a mission-item index: DO_JUMP targets an absolute item number
   *  (param1, 1-based); DO_JUMP_TAG targets the JUMP_TAG item whose id matches param1. -1 if none. */
  function jumpTargetIdx(wps: ArduWaypoint[], jump: ArduWaypoint): number {
    if (jump.command === CMD.DO_JUMP) return Math.round(jump.param1) - 1;
    if (jump.command === CMD.DO_JUMP_TAG) {
      const tag = Math.round(jump.param1);
      return wps.findIndex((w) => w.command === CMD.JUMP_TAG && Math.round(w.param1) === tag);
    }
    return -1;
  }

  // ── Render ──────────────────────────────────────────────────────────

  function renderMission(wps: ArduWaypoint[], selIdx: number, editing: boolean, home: HomePosition, vehicle: VehicleClass, activeWp: number, conn: boolean) {
    const groups = groupArduMission(wps);
    const selGroup = groups.find(g => g.anchorIdx === selIdx || g.modifiers.some(m => m.idx === selIdx)) ?? null;

    missionGroup.clearLayers(); // markers/lines only — the editor popup is a separate map layer
    if (wps.length === 0) { closeEditorPopup(map, popupState); return; }

    const fpPositions: L.LatLng[] = [];
    const fpWpIndices: number[] = [];
    // Count of jump badges already placed per source waypoint, so multiple jumps stack instead of overlap.
    const jumpBadgesBySource = new Map<number, number>();

    for (let i = 0; i < wps.length; i++) {
      const wp = wps[i];
      const hasLoc = cmdHasLocation(wp.command);
      const standalone = cmdStandaloneCoordinate(wp.command);
      const displayNum = i + 1;

      if (hasLoc || standalone) {
        const isTakeoff = cmdIsTakeoff(wp.command);
        const latLng = wpDisplayLatLng(wp, wps, home, conn);
        if (!latLng) continue;

        if (hasLoc) { fpPositions.push(latLng); fpWpIndices.push(i); }

        // Takeoff is draggable too while planning offline (a connected UAV locks it to its FC home).
        const draggable = editing && (!isTakeoff || !conn);
        const marker = L.marker(latLng, {
          icon: iconForArduWp(wp, displayNum, i === selIdx, activeWp > 0 && displayNum === activeWp),
          draggable,
          title: `WP${displayNum}: ${cmdName(wp.command)}`,
        }).addTo(missionGroup);

        marker.on('click', () => { arduSelectedWpIndex.set(i); });
        if (draggable) {
          marker.on('dragend', () => {
            const pos = marker.getLatLng();
            const cur = get(arduMission)[i];
            if (cur) arduUpdateWp(i, { ...cur, lat: Math.round(pos.lat * 1e7), lon: Math.round(pos.lng * 1e7) });
          });
        }

        // Loiter radius is in param3 for most loiters, but param2 for NAV_LOITER_TO_ALT.
        const loiterRadius = wp.command === CMD.NAV_LOITER_TO_ALT ? wp.param2 : wp.param3;
        if (cmdIsLoiter(wp.command) && loiterRadius > 0) {
          L.circle(latLng, {
            radius: Math.abs(loiterRadius),
            color: '#f39c12', weight: 1.5, fill: false, opacity: 0.5, dashArray: '4 4',
          }).addTo(missionGroup);
        }

        if (!editing) {
          // Hover (view mode): list every parameter — shared with the panel footer so they never drift.
          const detail = arduWpDetailLines(wp, $t).map((l) => `${l.label}: ${l.value}`).join('<br>');
          marker.bindTooltip(
            `<b>WP${displayNum} ${cmdName(wp.command)}</b>${detail ? '<br>' + detail : ''}`,
            { direction: 'top', offset: L.point(0, -20) },
          );
        }

        // Edit mode: permanent reference label (alt + key params) on every location WP except the
        // selected one (its editor popup already shows the details). Mirrors the INAV edit labels.
        if (editing && hasLoc && i !== selIdx) {
          createParamLabel(latLng, arduParamLabelHtml(wp)).addTo(missionGroup);
        }
      }

      // Jump connector line — DO_JUMP (→ absolute item) and DO_JUMP_TAG (→ matching Jump Tag). Both
      // draw from the jump's preceding waypoint to the first location at/after the resolved target.
      if ((wp.command === CMD.DO_JUMP || wp.command === CMD.DO_JUMP_TAG) && wp.param1 > 0) {
        const targetIdx = jumpTargetIdx(wps, wp);
        const targetLocIdx = targetIdx >= 0 ? findNextLocationIdx(wps, targetIdx) : -1;
        const prevLocIdx = findPrevLocationIdx(wps, i);
        const sourceWp = prevLocIdx >= 0 ? wps[prevLocIdx] : null;
        const targetWp = targetLocIdx >= 0 ? wps[targetLocIdx] : null;
        const src = sourceWp ? wpDisplayLatLng(sourceWp, wps, home, conn) : null;
        const dst = targetWp ? wpDisplayLatLng(targetWp, wps, home, conn) : null;
        if (src && dst) {
          const repeat = wp.param2 < 0 ? '∞' : wp.param2;
          const label = wp.command === CMD.DO_JUMP
            ? `Jump → WP${wp.param1} ×${repeat}`
            : `Jump → Tag ${wp.param1} ×${repeat}`;
          L.polyline([src, dst], { color: '#8e44ad', weight: 2, dashArray: '8 4', opacity: 0.8 })
            .addTo(missionGroup)
            .bindTooltip(label, { sticky: true });
          // Repeat-count badge on the source waypoint marker — ArduPilot otherwise shows no count.
          // Stacked upward when a waypoint has several jumps so the pills don't overlap.
          const stack = jumpBadgesBySource.get(prevLocIdx) ?? 0;
          L.marker(src, {
            icon: L.divIcon({
              className: 'mission-jump-badge',
              html: `↺${repeat}`,
              iconSize: [34, 18],
              iconAnchor: [-4, 64 + stack * 20],
            }),
            interactive: false,
            zIndexOffset: 1000,
          }).addTo(missionGroup);
          jumpBadgesBySource.set(prevLocIdx, stack + 1);
        }
      }
    }

    // Flight path (location commands only), takeoff-adjacent leg dashed.
    if (fpPositions.length > 1) {
      const insertOnPath = (e: L.LeafletMouseEvent) => {
        L.DomEvent.stopPropagation(e);
        let bestInsertIdx = wps.length;
        let bestDist = Infinity;
        for (let s = 0; s < fpPositions.length - 1; s++) {
          const mid = L.latLng((fpPositions[s].lat + fpPositions[s + 1].lat) / 2, (fpPositions[s].lng + fpPositions[s + 1].lng) / 2);
          const d = e.latlng.distanceTo(mid);
          if (d < bestDist) { bestDist = d; bestInsertIdx = fpWpIndices[s + 1] ?? wps.length; }
        }
        const updated = [...wps];
        updated.splice(bestInsertIdx, 0, newWaypointAt(e.latlng));
        arduMission.set(updated);
        arduSelectedWpIndex.set(bestInsertIdx);
      };

      for (let s = 0; s < fpPositions.length - 1; s++) {
        const loose = cmdIsTakeoff(wps[fpWpIndices[s]]?.command) || cmdIsTakeoff(wps[fpWpIndices[s + 1]]?.command);
        const leg = L.polyline([fpPositions[s], fpPositions[s + 1]], {
          color: '#37a8db', weight: editing ? 6 : 3, opacity: 0.7,
          ...(loose ? { dashArray: '8 6' } : {}),
        }).addTo(missionGroup);
        if (editing) leg.on('click', insertOnPath);
      }
    }

    // Editor popup for the selected group (anchored on its primary waypoint) — shared framework with
    // the content-signature redraw guard, so live redraws don't close an open dropdown.
    const anchorLatLng = editing && selGroup?.anchor ? wpDisplayLatLng(selGroup.anchor, wps, home, conn) : null;
    if (editing && selGroup?.anchor && anchorLatLng) {
      const g = selGroup;
      renderEditorPopup(
        map, popupState, g.anchorIdx, anchorLatLng,
        buildGroupEditorHtml(g, wps.length, vehicle),
        (popup) => attachGroupEditorEvents(popup, g),
        { popupOptions: { maxWidth: 260, minWidth: 200 } },
      );
    } else {
      closeEditorPopup(map, popupState);
    }
  }

  function newWaypointAt(latlng: L.LatLng): ArduWaypoint {
    return {
      command: MAV_CMD_NAV_WAYPOINT, frame: MAV_FRAME_GLOBAL_RELATIVE_ALT,
      param1: 0, param2: 0, param3: 0, param4: 0,
      lat: Math.round(latlng.lat * 1e7), lon: Math.round(latlng.lng * 1e7),
      alt: get(settings).defaultWpAltitudeM, autocontinue: true,
    };
  }

  function onMapClick(e: L.LeafletMouseEvent) {
    if (!currentEditing) return;
    if (currentSelIdx >= 0) { arduSelectedWpIndex.set(-1); return; }
    arduAddWp(newWaypointAt(e.latlng));
  }

  // svelte-ignore state_referenced_locally
  map.on('click', onMapClick);

  $effect(() => { renderMission(currentWps, currentSelIdx, currentEditing, currentHome, currentVehicle, currentActiveWp, connected); });

  onDestroy(() => {
    unsubMission(); unsubSelIdx(); unsubEditMode(); unsubHome(); unsubVehicle(); unsubActiveWp(); unsubSystem(); unsubConn();
    map.off('click', onMapClick);
    closeEditorPopup(map, popupState);
    missionGroup.clearLayers();
    map.removeLayer(missionGroup);
  });
</script>

<style>
  /* Permanent edit-mode reference labels (same look as the INAV layer; only one mission layer is
     mounted at a time, so the duplicate :global rule never conflicts). */
  :global(.wp-param-label-wrapper) { background: none !important; border: none !important; overflow: visible !important; width: auto !important; height: auto !important; }
  :global(.wp-param-label) { background: rgba(30,30,30,0.88); color: #ccc; padding: 3px 8px; border-radius: 4px; font-size: 12px; line-height: 1.4; white-space: nowrap; border: 1px solid rgba(55,168,219,0.35); pointer-events: none; transform: scale(var(--ui-scale, 1)); transform-origin: top left; }

  /* New popup-framework classes (the base wpe-* classes are global in InavMissionLayer). */
  /* ArduPilot param labels (Acceptance / Pass Radius / …) are longer than INAV's — give them a wider,
     no-wrap label column so they stay on one line and the value controls line up. Scoped to the Ardu
     popup so INAV's narrower layout is untouched. */
  :global(.wp-editor-popup-ardu .wpe-row label) { width: 78px; white-space: nowrap; }
  :global(.wpe-applies) { color: #888; font-size: 10px; font-weight: 400; margin-left: 4px; }
  :global(.wpe-canonical) { color: #7a7a7a; font-size: 10px; font-weight: 400; font-family: 'Consolas', monospace; }
  :global(.wpe-canonical-line) { margin: 1px 0 4px; padding-left: 2px; }
  :global(.wpe-info) { color: #37a8db; font-size: 10px; cursor: help; opacity: 0.8; }
  :global(.wpe-adv-toggle) { display: block; width: 100%; text-align: left; background: none; border: none; border-top: 1px dashed #555; margin-top: 4px; padding: 4px 0 2px; color: #37a8db; font-size: 11px; cursor: pointer; }
  :global(.wpe-adv-toggle:hover) { color: #5cc0ec; }
  :global(.wpe-adv-body) { margin-top: 2px; }
  /* Jump repeat-count badge (↺N) pinned near the source waypoint — purple to match the jump line. */
  :global(.mission-jump-badge) {
    display: flex; align-items: center; justify-content: center;
    background: #8e44ad; color: #fff;
    font-size: 11px; font-weight: 700; line-height: 1;
    border: 1px solid rgba(255, 255, 255, 0.85); border-radius: 9px;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.45);
    white-space: nowrap;
  }
</style>
