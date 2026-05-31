<!-- InavMissionLayer.svelte
     Renders INAV mission waypoints on the Leaflet map.
     Usage: <InavMissionLayer {map} />
-->
<script lang="ts">
  import { onDestroy } from 'svelte';
  import L from 'leaflet';
  import {
    mission, geoWaypoints, selectedWpIndex, selectedWpIndices,
    selectWpSingle, toggleWpSelection, clearWpSelection, editMode, showMission, replayActive, launchPoint,
    missionAddWp, missionUpdateWp, missionRemoveWp, missionInsertWp,
    missionReorderWp, beginUndoGroup, endUndoGroup,
    getTotalWpCount, MAX_WAYPOINTS_TOTAL,
    ALT_MODE_REL, ALT_MODE_AMSL, ALT_MODE_AGL,
    type Waypoint, type Mission, type LaunchPoint, WpAction, hasLocation, isModifier, toDeg, fromDeg, altFromM,
    WP_ACTION_LABELS, WP_ACTION_KEYS,
  } from '$lib/stores/mission';
  import { homePosition } from '$lib/stores/home';
  import { activeSurveyPattern } from '$lib/stores/surveyPattern.svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { iconForWp } from '$lib/helpers/missionIcons';
  import { openContextMenu } from '$lib/stores/contextMenu';
  import { buildWaypointMenu } from '$lib/helpers/waypointMenu';
  import { convertAltCm } from '$lib/helpers/altConvert';
  import { get } from 'svelte/store';
  import { settings } from '$lib/stores/settings';
  import type { InterfaceSettings } from '$lib/stores/settings';
  import { convertAltitude, toAltitudeM, convertSpeed, toSpeedMs } from '$lib/utils/units';
  import { t } from 'svelte-i18n';

  const IFACE_FALLBACK: InterfaceSettings = {
    speedUnit: 'kmh', altitudeUnit: 'm', distanceUnit: 'metric', verticalSpeedUnit: 'ms', temperatureUnit: 'c',
  };
  function iface(): InterfaceSettings {
    return get(settings).interface ?? IFACE_FALLBACK;
  }
  /** Altitude (m internal) → display unit. */
  function altDisp(m: number) {
    return convertAltitude(m, iface().altitudeUnit);
  }
  /** Waypoint speed (cm/s internal) → display speed unit. */
  function spdDisp(cms: number) {
    return convertSpeed(cms / 100, iface().speedUnit);
  }

  interface Props {
    map: L.Map;
  }

  let { map }: Props = $props();

  const MAX_WAYPOINTS = MAX_WAYPOINTS_TOTAL;

  let currentMission = $state<Mission>(get(mission));
  let currentSelIdx = $state<number>(get(selectedWpIndex));
  let currentSelSet = $state<Set<number>>(get(selectedWpIndices));
  let currentEditing = $state<boolean>(get(editMode));
  let currentShowMission = $state<boolean>(get(showMission));
  let currentReplayActive = $state<boolean>(get(replayActive));

  // ── Launch / home reference marker (planning-time, for REL↔AGL + clearance) ──
  // Declared before the store subscriptions below, which fire immediately on
  // subscribe and call renderLaunchMarker() (avoids a temporal-dead-zone crash).
  let launchMarker: L.Marker | undefined;
  let currentLaunch = $state<LaunchPoint | null>(get(launchPoint));

  const unsubMission = mission.subscribe(m => { currentMission = m; });
  const unsubSelIdx = selectedWpIndex.subscribe(i => { currentSelIdx = i; });
  const unsubSelSet = selectedWpIndices.subscribe(s => { currentSelSet = s; });
  const unsubEditMode = editMode.subscribe(e => {
    currentEditing = e;
    if (e) autoPlaceLaunch();
    renderLaunchMarker();
  });
  const unsubShowMission = showMission.subscribe(v => { currentShowMission = v; });
  const unsubReplayActive = replayActive.subscribe(v => { currentReplayActive = v; });

  /** Auto-place the launch point when none is set: FC home → first geo-WP → map center. */
  function autoPlaceLaunch() {
    if (get(launchPoint)) return;
    const home = get(homePosition);
    if (home.set && (home.lat !== 0 || home.lon !== 0)) {
      launchPoint.set({ lat: home.lat, lng: home.lon });
      return;
    }
    const geo = get(geoWaypoints);
    if (geo.length > 0) {
      launchPoint.set({ lat: toDeg(geo[0].lat), lng: toDeg(geo[0].lon) });
      return;
    }
    const c = map.getCenter();
    launchPoint.set({ lat: c.lat, lng: c.lng });
  }

  /** Always-visible draggable launch marker (the mission's home reference). */
  function renderLaunchMarker() {
    const lp = get(launchPoint);
    if (!lp) {
      if (launchMarker) { try { map.removeLayer(launchMarker); } catch {} launchMarker = undefined; }
      return;
    }
    if (!launchMarker) {
      launchMarker = L.marker([lp.lat, lp.lng], {
        draggable: true,
        zIndexOffset: 500,
        icon: L.divIcon({
          className: 'launch-marker',
          html: '<div style="background:#f39c12;width:18px;height:18px;border-radius:50% 50% 50% 0;transform:rotate(-45deg);border:2px solid white;box-shadow:0 0 4px rgba(0,0,0,0.6);display:flex;align-items:center;justify-content:center;"><span style="transform:rotate(45deg);color:#fff;font-size:11px;font-weight:bold;line-height:1;">L</span></div>',
          iconSize: [22, 22], iconAnchor: [11, 22],
        }),
      }).addTo(map);
      launchMarker.bindTooltip('Launch / home reference', { direction: 'top', offset: [0, -20] });
      launchMarker.on('dragend', () => {
        const p = launchMarker!.getLatLng();
        launchPoint.set({ lat: p.lat, lng: p.lng });
      });
    } else {
      launchMarker.setLatLng([lp.lat, lp.lng]);
    }
  }

  const unsubLaunch = launchPoint.subscribe(lp => { currentLaunch = lp; renderLaunchMarker(); });

  // svelte-ignore state_referenced_locally
  const missionGroup = L.layerGroup().addTo(map);
  let wpMarkers: L.Marker[] = [];
  let flightPath: L.Polyline | undefined;
  let modifierLines: L.Polyline[] = [];
  let paramLabels: L.Marker[] = [];
  let editorPopup: L.Popup | undefined;
  let editorPopupIdx: number = -1;

  function buildDisplayNumbers(waypoints: Waypoint[]): Map<number, number> {
    const nums = new Map<number, number>();
    let dn = 1;
    for (let i = 0; i < waypoints.length; i++) {
      if (!isModifier(waypoints[i].action)) nums.set(i, dn++);
    }
    return nums;
  }

  function getModifiersForWp(waypoints: Waypoint[], geoIdx: number): { wp: Waypoint; idx: number }[] {
    const mods: { wp: Waypoint; idx: number }[] = [];
    for (let j = geoIdx + 1; j < waypoints.length; j++) {
      if (isModifier(waypoints[j].action)) mods.push({ wp: waypoints[j], idx: j });
      else break;
    }
    return mods;
  }

  function isFlightPathWp(action: WpAction): boolean {
    return hasLocation(action) && action !== WpAction.SetPoi;
  }

  function findMissionEndIndex(waypoints: Waypoint[]): number {
    for (let i = 0; i < waypoints.length; i++) {
      if (waypoints[i].action === WpAction.Land || waypoints[i].action === WpAction.Rth) return i;
    }
    return -1;
  }

  /** Altitude reference label REL/AMSL/AGL from a waypoint's alt_mode (falls back to p3 bit0). */
  function altLabel(wp: Waypoint): string {
    const m = wp.alt_mode ?? ((wp.p3 & 1) ? 1 : 0);
    return m === 2 ? 'AGL' : m === 1 ? $t('missionLayer.amsl') : $t('missionLayer.rel');
  }

  function paramLabelHtml(wp: Waypoint, modifiers: { wp: Waypoint; idx: number }[]): string {
    const a = altDisp(wp.altitude / 100);
    const altType = altLabel(wp);
    let lines = [`${Math.round(a.value)}${a.unit} ${altType}`];
    switch (wp.action) {
      case WpAction.Waypoint:
      case WpAction.Land:
        if (wp.p1 > 0) { const s = spdDisp(wp.p1); lines.push(`${s.value.toFixed(1)} ${s.unit}`); }
        break;
      case WpAction.PosholdTime:
        lines.push($t('missionLayer.holdTime', { values: { seconds: wp.p1 } }));
        break;
      case WpAction.PosholdUnlim:
        lines.push($t('missionLayer.holdUnlimited'));
        break;
    }
    for (const mod of modifiers) {
      switch (mod.wp.action) {
        case WpAction.Rth:
          lines.push(mod.wp.p1 ? $t('missionLayer.rthLand') : $t('missionLayer.rthHover'));
          break;
        case WpAction.Jump:
          lines.push($t('missionLayer.jumpTo', { values: { target: mod.wp.p1, count: mod.wp.p2 === -1 ? '∞' : mod.wp.p2 } }));
          break;
        case WpAction.SetHead:
          lines.push(mod.wp.p1 === -1 ? $t('missionLayer.hdgFree') : $t('missionLayer.hdgValue', { values: { degrees: mod.wp.p1 } }));
          break;
      }
    }
    return `<div class="wp-param-label">${lines.join('<br>')}</div>`;
  }

  function createParamLabel(wp: Waypoint, modifiers: { wp: Waypoint; idx: number }[], latLng: L.LatLng): L.Marker {
    const icon = L.divIcon({
      className: 'wp-param-label-wrapper',
      html: paramLabelHtml(wp, modifiers),
      iconSize: [1, 1],
      iconAnchor: [-12, 10],
    });
    return L.marker(latLng, { icon, interactive: false });
  }

  function numInputHtml(field: string, value: number, step: number, min?: number, max?: number, modIdx?: number): string {
    const dataAttrs = modIdx !== undefined ? `data-field="${field}" data-mod-idx="${modIdx}"` : `data-field="${field}"`;
    const minAttr = min !== undefined ? `min="${min}"` : '';
    const maxAttr = max !== undefined ? `max="${max}"` : '';
    return `<div class="wpe-num-ctrl"><button class="wpe-num-btn" data-numdir="-1" ${dataAttrs}>−</button><input type="number" ${dataAttrs} value="${value}" step="${step}" ${minAttr} ${maxAttr}/><button class="wpe-num-btn" data-numdir="1" ${dataAttrs}>+</button></div>`;
  }

  function attachNumBtnEvents(el: HTMLElement) {
    el.querySelectorAll('.wpe-num-btn').forEach(btn => {
      btn.addEventListener('click', (e) => {
        e.preventDefault();
        e.stopPropagation();
        const b = btn as HTMLElement;
        const parent = b.closest('.wpe-num-ctrl');
        if (!parent) return;
        const input = parent.querySelector('input') as HTMLInputElement;
        if (!input) return;
        const dir = Number(b.dataset.numdir);
        const step = Number(input.step) || 1;
        const min = input.min !== '' ? Number(input.min) : -Infinity;
        const max = input.max !== '' ? Number(input.max) : Infinity;
        let val = Number(input.value) + dir * step;
        val = Math.max(min, Math.min(max, val));
        input.value = String(val);
        input.dispatchEvent(new Event('change', { bubbles: true }));
      });
    });
  }

  function buildEditorHtml(wp: Waypoint, idx: number, total: number, displayNum: number,
    modifiers: { wp: Waypoint; idx: number }[]): string {
    const altC = altDisp(wp.altitude / 100);
    const altVal = Math.round(altC.value);
    const altType = altLabel(wp);
    const geoTypes: WpAction[] = [WpAction.Waypoint, WpAction.PosholdUnlim, WpAction.PosholdTime, WpAction.SetPoi, WpAction.Land];
    const typeOptions = geoTypes.map(v => `<option value="${v}" ${v === wp.action ? 'selected' : ''}>${$t(WP_ACTION_KEYS[v])}</option>`).join('');

    let html = `<div class="wp-editor-popup">`;
    html += `<div class="wpe-header">${$t('missionLayer.wpHeader', { values: { number: displayNum } })} <span class="wpe-type-name">${$t(WP_ACTION_KEYS[wp.action])}</span></div>`;
    html += `<div class="wpe-row"><label>${$t('missionLayer.type')}</label><select data-field="action">${typeOptions}</select></div>`;
    html += `<div class="wpe-row"><label>${$t('missionLayer.alt')}</label>${numInputHtml('altitude', altVal, 1)}<span class="wpe-unit">${altC.unit}</span><button data-field="altToggle" class="wpe-toggle">${altType}</button></div>`;

    const latDeg = toDeg(wp.lat).toFixed(7);
    const lonDeg = toDeg(wp.lon).toFixed(7);
    html += `<div class="wpe-row"><label>${$t('missionLayer.lat')}</label><input type="number" data-field="lat" value="${latDeg}" step="0.0000001" min="-90" max="90" class="wpe-coord-input"/></div>`;
    html += `<div class="wpe-row"><label>${$t('missionLayer.lon')}</label><input type="number" data-field="lon" value="${lonDeg}" step="0.0000001" min="-180" max="180" class="wpe-coord-input"/></div>`;

    if (wp.action === WpAction.Waypoint || wp.action === WpAction.Land) {
      const spd = spdDisp(wp.p1);
      html += `<div class="wpe-row"><label>${$t('missionLayer.speed')}</label>${numInputHtml('p1', Math.round(spd.value * 10) / 10, 1, 0)}<span class="wpe-unit">${spd.unit}</span></div>`;
    }
    if (wp.action === WpAction.PosholdTime) {
      html += `<div class="wpe-row"><label>${$t('missionLayer.hold')}</label>${numInputHtml('p1', wp.p1, 1, 0)}<span class="wpe-unit">${$t('missionLayer.sec')}</span></div>`;
    }

    if (wp.action === WpAction.Waypoint || wp.action === WpAction.PosholdUnlim ||
        wp.action === WpAction.PosholdTime || wp.action === WpAction.Land) {
      const ua1 = (wp.p3 >> 1) & 1; const ua2 = (wp.p3 >> 2) & 1;
      const ua3 = (wp.p3 >> 3) & 1; const ua4 = (wp.p3 >> 4) & 1;
      html += `<div class="wpe-row wpe-ua-row"><label>${$t('missionLayer.actions')}</label>`;
      html += `<button data-field="ua" data-ua-bit="1" class="wpe-ua-btn ${ua1 ? 'active' : ''}">UA1</button>`;
      html += `<button data-field="ua" data-ua-bit="2" class="wpe-ua-btn ${ua2 ? 'active' : ''}">UA2</button>`;
      html += `<button data-field="ua" data-ua-bit="3" class="wpe-ua-btn ${ua3 ? 'active' : ''}">UA3</button>`;
      html += `<button data-field="ua" data-ua-bit="4" class="wpe-ua-btn ${ua4 ? 'active' : ''}">UA4</button>`;
      html += `</div>`;
    }

    for (const mod of modifiers) {
      const mi = mod.idx;
      html += `<div class="wpe-mod-section">`;
      html += `<div class="wpe-mod-header">${$t(WP_ACTION_KEYS[mod.wp.action])}<button data-action="removeMod" data-mod-idx="${mi}" class="wpe-mod-remove" title="${$t('missionLayer.removeModifier')}">✕</button></div>`;
      if (mod.wp.action === WpAction.Rth) {
        html += `<div class="wpe-row"><label>${$t('missionLayer.landField')}</label><button data-field="rthLand" data-mod-idx="${mi}" class="wpe-toggle">${mod.wp.p1 ? $t('missionLayer.yes') : $t('missionLayer.no')}</button></div>`;
      }
      if (mod.wp.action === WpAction.Jump) {
        html += `<div class="wpe-row"><label>${$t('missionLayer.toWp')}</label>${numInputHtml('mod-p1', mod.wp.p1, 1, 1, undefined, mi)}</div>`;
        html += `<div class="wpe-row"><label>${$t('mission.repeat')}</label>${numInputHtml('mod-p2', mod.wp.p2, 1, -1, undefined, mi)}<span class="wpe-unit">${mod.wp.p2 === -1 ? '∞' : ''}</span></div>`;
      }
      if (mod.wp.action === WpAction.SetHead) {
        html += `<div class="wpe-row"><label>${$t('missionLayer.headingField')}</label>${numInputHtml('mod-p1', mod.wp.p1, 1, -1, 359, mi)}<span class="wpe-unit">${mod.wp.p1 === -1 ? $t('missionLayer.free') : '°'}</span></div>`;
      }
      html += `</div>`;
    }

    const atLimit = getTotalWpCount() >= MAX_WAYPOINTS;
    html += `<div class="wpe-add-mod"><select data-field="addModType" ${atLimit ? 'disabled' : ''}>`;
    html += `<option value="">${atLimit ? $t('missionLayer.maxWpReached') : $t('missionLayer.addModifier')}</option>`;
    if (!atLimit) {
      html += `<option value="${WpAction.SetHead}">${$t('missionLayer.modSetHead')}</option>`;
      html += `<option value="${WpAction.Jump}">${$t('missionLayer.modJump')}</option>`;
      html += `<option value="${WpAction.Rth}">${$t('missionLayer.modRth')}</option>`;
    }
    html += `</select></div>`;

    html += `<div class="wpe-actions">`;
    html += `<button data-action="moveUp" ${idx <= 0 ? 'disabled' : ''} title="${$t('missionLayer.moveUp')}">▲</button>`;
    html += `<button data-action="moveDown" ${idx >= total - 1 ? 'disabled' : ''} title="${$t('missionLayer.moveDown')}">▼</button>`;
    html += `<button data-action="remove" class="wpe-remove" title="${$t('missionLayer.removeWp')}">✕</button>`;
    html += `</div></div>`;
    return html;
  }

  function attachEditorEvents(popup: L.Popup, wp: Waypoint, idx: number,
    modifiers: { wp: Waypoint; idx: number }[]) {
    const el = popup.getElement();
    if (!el) return;

    const typeSelect = el.querySelector('select[data-field="action"]') as HTMLSelectElement | null;
    typeSelect?.addEventListener('change', () => {
      const newAction = Number(typeSelect.value) as WpAction;
      const updated = { ...wp, action: newAction };
      if (newAction === WpAction.PosholdTime) { updated.p1 = get(settings).defaultPhTimeSec; updated.p2 = 0; }
      else { updated.p1 = 0; updated.p2 = 0; }
      missionUpdateWp(idx, updated);
    });

    const altInput = el.querySelector('input[data-field="altitude"]') as HTMLInputElement | null;
    altInput?.addEventListener('change', () => {
      const m = toAltitudeM(Number(altInput.value), iface().altitudeUnit);
      missionUpdateWp(idx, { ...wp, altitude: altFromM(m) });
    });

    const altToggle = el.querySelector('button[data-field="altToggle"]') as HTMLButtonElement | null;
    altToggle?.addEventListener('click', async () => {
      // Cycle REL → AMSL → AGL → REL, converting the altitude so the waypoint
      // stays at the same physical height (terrain + launch point as references).
      const cur = wp.alt_mode ?? ((wp.p3 & 1) ? 1 : 0);
      const next = (cur + 1) % 3;
      const newAlt = await convertAltCm(wp, cur, next);
      missionUpdateWp(idx, { ...wp, alt_mode: next, altitude: newAlt });
    });

    const p1Input = el.querySelector('input[data-field="p1"]') as HTMLInputElement | null;
    p1Input?.addEventListener('change', () => {
      // Waypoint/Land p1 is speed (display unit → cm/s); otherwise raw (e.g. hold seconds)
      const isSpeed = wp.action === WpAction.Waypoint || wp.action === WpAction.Land;
      const p1 = isSpeed
        ? Math.round(toSpeedMs(Number(p1Input.value), iface().speedUnit) * 100)
        : Number(p1Input.value);
      missionUpdateWp(idx, { ...wp, p1 });
    });

    const latInput = el.querySelector('input[data-field="lat"]') as HTMLInputElement | null;
    const lonInput = el.querySelector('input[data-field="lon"]') as HTMLInputElement | null;
    function applyCoordChange() {
      const newLat = parseFloat(latInput?.value ?? '');
      const newLon = parseFloat(lonInput?.value ?? '');
      if (isNaN(newLat) || isNaN(newLon)) return;
      missionUpdateWp(idx, { ...wp, lat: fromDeg(newLat), lon: fromDeg(newLon) });
      map.panTo(L.latLng(newLat, newLon));
    }
    latInput?.addEventListener('change', applyCoordChange);
    lonInput?.addEventListener('change', applyCoordChange);

    el.querySelectorAll('button[data-field="ua"]').forEach(btn => {
      btn.addEventListener('click', () => {
        const bit = Number((btn as HTMLElement).dataset.uaBit);
        missionUpdateWp(idx, { ...wp, p3: wp.p3 ^ (1 << bit) });
      });
    });

    attachNumBtnEvents(el);

    for (const mod of modifiers) {
      const mi = mod.idx;
      const rthBtn = el.querySelector(`button[data-field="rthLand"][data-mod-idx="${mi}"]`) as HTMLButtonElement | null;
      rthBtn?.addEventListener('click', () => { missionUpdateWp(mi, { ...mod.wp, p1: mod.wp.p1 ? 0 : 1 }); });
      const modP1 = el.querySelector(`input[data-field="mod-p1"][data-mod-idx="${mi}"]`) as HTMLInputElement | null;
      modP1?.addEventListener('change', () => { missionUpdateWp(mi, { ...mod.wp, p1: Number(modP1.value) }); });
      const modP2 = el.querySelector(`input[data-field="mod-p2"][data-mod-idx="${mi}"]`) as HTMLInputElement | null;
      modP2?.addEventListener('change', () => { missionUpdateWp(mi, { ...mod.wp, p2: Number(modP2.value) }); });
      const rmBtn = el.querySelector(`button[data-action="removeMod"][data-mod-idx="${mi}"]`) as HTMLButtonElement | null;
      rmBtn?.addEventListener('click', () => { missionRemoveWp(mi); });
    }

    const addModSelect = el.querySelector('select[data-field="addModType"]') as HTMLSelectElement | null;
    addModSelect?.addEventListener('change', () => {
      const modAction = Number(addModSelect.value) as WpAction;
      if (!modAction) return;
      if (getTotalWpCount() >= MAX_WAYPOINTS) { addModSelect.value = ''; return; }
      const insertIdx = modifiers.length > 0 ? modifiers[modifiers.length - 1].idx + 1 : idx + 1;
      let p1 = 0, p2 = 0;
      if (modAction === WpAction.SetHead) p1 = -1;
      else if (modAction === WpAction.Jump) { p1 = 1; p2 = 1; }
      missionInsertWp(insertIdx, modAction, 0, 0, 0, p1, p2);
    });

    el.querySelector('button[data-action="moveUp"]')?.addEventListener('click', () => {
      if (idx > 0) { missionReorderWp(idx, idx - 1); selectWpSingle(idx - 1); }
    });
    el.querySelector('button[data-action="moveDown"]')?.addEventListener('click', () => {
      if (idx < currentMission.waypoints.length - 1) { missionReorderWp(idx, idx + 1); selectWpSingle(idx + 1); }
    });
    el.querySelector('button[data-action="remove"]')?.addEventListener('click', () => {
      // WP + its attached modifiers = a single undo step.
      beginUndoGroup();
      for (let k = modifiers.length - 1; k >= 0; k--) missionRemoveWp(modifiers[k].idx);
      missionRemoveWp(idx);
      endUndoGroup();
      selectedWpIndex.set(-1);
    });

    el.querySelectorAll('input, select, button').forEach(input => {
      L.DomEvent.disableClickPropagation(input as HTMLElement);
    });
  }

  function renderMission(m: Mission, selIdx: number, editing: boolean) {
    const keepPopup = editing && editorPopup && editorPopupIdx === selIdx && selIdx >= 0;
    missionGroup.clearLayers();
    wpMarkers = []; modifierLines = []; paramLabels = [];

    if (!keepPopup) {
      if (editorPopup) map.removeLayer(editorPopup);
      editorPopup = undefined; editorPopupIdx = -1;
    }
    // In replay the mission follows the "Show Mission" toggle; in planning/live
    // a loaded mission is always shown.
    if (currentReplayActive && !currentShowMission) return;
    if (m.waypoints.length === 0) return;

    // Launch → first waypoint connector (orange dashed, matching pattern turn legs)
    if (currentLaunch) {
      const firstGeo = m.waypoints.find(w => hasLocation(w.action) && !(w.lat === 0 && w.lon === 0));
      if (firstGeo) {
        L.polyline(
          [[currentLaunch.lat, currentLaunch.lng], [toDeg(firstGeo.lat), toDeg(firstGeo.lon)]],
          { color: '#f39c12', weight: 2, opacity: 0.7, dashArray: '6 5' }
        ).addTo(missionGroup);
      }
    }

    const displayNums = buildDisplayNumbers(m.waypoints);
    const missionEndIdx = findMissionEndIndex(m.waypoints);
    const fpPositions: L.LatLng[] = [];
    const fpIndices: number[] = [];
    const fpGreyed: boolean[] = [];

    for (let i = 0; i < m.waypoints.length; i++) {
      const wp = m.waypoints[i];
      const inSel = currentSelSet.has(i); // any selected → red icon
      const isPrimary = i === selIdx; // sole selection → editor popup
      const dn = displayNums.get(i) ?? 0;
      const greyed = missionEndIdx >= 0 && i > missionEndIdx;

      if (hasLocation(wp.action)) {
        const latLng = L.latLng(toDeg(wp.lat), toDeg(wp.lon));
        if (isFlightPathWp(wp.action)) { fpPositions.push(latLng); fpIndices.push(i); fpGreyed.push(greyed); }

        const icon = iconForWp(wp, dn, inSel);
        const marker = L.marker(latLng, {
          icon, draggable: editing && !greyed, opacity: greyed ? 0.35 : 1.0,
          title: `WP${dn}: ${$t(WP_ACTION_KEYS[wp.action]) || 'Unknown'}`,
        }).addTo(missionGroup);

        marker.on('click', () => { if (editing) toggleWpSelection(i); else selectWpSingle(i); });
        marker.on('contextmenu', (e: L.LeafletMouseEvent) => {
          // Right-click on an unselected marker selects it; on a selected one
          // keeps the (multi-)selection so the menu can act on all of it.
          if (!currentSelSet.has(i)) selectWpSingle(i);
          openContextMenu(e.originalEvent.clientX, e.originalEvent.clientY, buildWaypointMenu());
        });
        if (editing) {
          marker.on('dragend', () => {
            const pos = marker.getLatLng();
            missionUpdateWp(i, { ...wp, lat: fromDeg(pos.lat), lon: fromDeg(pos.lng) });
          });
        }

        const modifiers = getModifiersForWp(m.waypoints, i);
        if (editing && !isPrimary && !greyed) {
          createParamLabel(wp, modifiers, latLng).addTo(missionGroup);
        }

        if (editing && isPrimary && !greyed) {
          const htmlContent = buildEditorHtml(wp, i, m.waypoints.length, dn, modifiers);
          if (keepPopup && editorPopup) {
            editorPopup.setLatLng(latLng);
            const contentEl = editorPopup.getElement()?.querySelector('.leaflet-popup-content');
            if (contentEl) contentEl.innerHTML = htmlContent;
          } else {
            if (editorPopup) map.removeLayer(editorPopup);
            editorPopup = L.popup({
              closeButton: false, autoClose: false, closeOnClick: false,
              className: 'wp-editor-popup-container', offset: L.point(0, -30), maxWidth: 240, minWidth: 190,
            }).setLatLng(latLng).setContent(htmlContent).addTo(map);
          }
          editorPopupIdx = i;
          setTimeout(() => { if (editorPopup) attachEditorEvents(editorPopup, wp, i, modifiers); }, 50);
        }

        if (!editing) {
          const a = altDisp(wp.altitude / 100);
          const altType = altLabel(wp);
          const mods = getModifiersForWp(m.waypoints, i);
          let tip = `WP${dn} ${$t(WP_ACTION_KEYS[wp.action])}<br>${a.value.toFixed(1)}${a.unit} ${altType}`;
          for (const mod of mods) tip += `<br>${$t(WP_ACTION_KEYS[mod.wp.action])}`;
          marker.bindTooltip(tip, { direction: 'top', offset: L.point(0, -20) });
        }
        wpMarkers.push(marker);
      }

      if (wp.action === WpAction.Jump && wp.p1 > 0) {
        const targetIdx = wp.p1 - 1;
        const sourceWp = findPreviousGeoWp(m.waypoints, i);
        const targetWp = m.waypoints[targetIdx];
        if (sourceWp && targetWp && hasLocation(targetWp.action)) {
          const line = L.polyline([
            L.latLng(toDeg(sourceWp.lat), toDeg(sourceWp.lon)),
            L.latLng(toDeg(targetWp.lat), toDeg(targetWp.lon)),
          ], { color: '#8e44ad', weight: 2, dashArray: '8 4', opacity: 0.8 }).addTo(missionGroup);
          const jLabel = wp.p2 === -1 ? '∞' : `×${wp.p2}`;
          line.bindTooltip($t('missionLayer.jumpLineTooltip', { values: { target: wp.p1, label: jLabel } }), { sticky: true });
          modifierLines.push(line);
        }
      }

      if (wp.action === WpAction.Rth) {
        const sourceWp = findPreviousGeoWp(m.waypoints, i);
        if (sourceWp && fpPositions.length > 0) {
          const line = L.polyline([
            L.latLng(toDeg(sourceWp.lat), toDeg(sourceWp.lon)), fpPositions[0],
          ], { color: '#e67e22', weight: 2, dashArray: '8 4', opacity: 0.7 }).addTo(missionGroup);
          line.bindTooltip($t('missionLayer.rthLineTooltip'), { sticky: true });
          modifierLines.push(line);
        }
      }
    }

    if (fpPositions.length > 1) {
      const activePositions: L.LatLng[] = [];
      const greyedPositions: L.LatLng[] = [];
      for (let s = 0; s < fpPositions.length; s++) {
        if (!fpGreyed[s]) {
          activePositions.push(fpPositions[s]);
        } else {
          if (greyedPositions.length === 0 && activePositions.length > 0) greyedPositions.push(activePositions[activePositions.length - 1]);
          greyedPositions.push(fpPositions[s]);
        }
      }
      if (activePositions.length > 1) {
        flightPath = L.polyline(activePositions, { color: '#37a8db', weight: editing ? 6 : 3, opacity: 0.7 }).addTo(missionGroup);
        if (editing) {
          flightPath.on('click', (e: L.LeafletMouseEvent) => {
            L.DomEvent.stopPropagation(e);
            const clickLatLng = e.latlng;
            let bestInsertIdx = fpIndices.length;
            let bestDist = Infinity;
            for (let s = 0; s < fpPositions.length - 1; s++) {
              if (fpGreyed[s] || fpGreyed[s + 1]) continue;
              const mid = L.latLng((fpPositions[s].lat + fpPositions[s + 1].lat) / 2, (fpPositions[s].lng + fpPositions[s + 1].lng) / 2);
              const d = clickLatLng.distanceTo(mid);
              if (d < bestDist) { bestDist = d; bestInsertIdx = fpIndices[s + 1]; }
            }
            if (getTotalWpCount() < MAX_WAYPOINTS) {
              missionInsertWp(bestInsertIdx, WpAction.Waypoint, fromDeg(clickLatLng.lat), fromDeg(clickLatLng.lng), altFromM(get(settings).defaultWpAltitudeM));
            }
          });
        }
      }
      if (greyedPositions.length > 1) {
        L.polyline(greyedPositions, { color: '#666', weight: editing ? 4 : 2, opacity: 0.4, dashArray: '6 4' }).addTo(missionGroup);
      }
    }
  }

  function findPreviousGeoWp(waypoints: Waypoint[], fromIndex: number): Waypoint | null {
    for (let i = fromIndex - 1; i >= 0; i--) {
      if (hasLocation(waypoints[i].action)) return waypoints[i];
    }
    return null;
  }

  function onMapClick(e: L.LeafletMouseEvent) {
    if (!currentEditing) return;
    // Block waypoint placement while pattern mode is active
    if (activeSurveyPattern.isActive) return;
    if (currentSelSet.size > 0) { clearWpSelection(); return; }
    if (getTotalWpCount() >= MAX_WAYPOINTS) return;
    const lat = fromDeg(e.latlng.lat);
    const lon = fromDeg(e.latlng.lng);
    const altitude = altFromM(get(settings).defaultWpAltitudeM);
    missionAddWp(WpAction.Waypoint, lat, lon, altitude);
  }

  // svelte-ignore state_referenced_locally
  map.on('click', onMapClick);

  $effect(() => { void currentLaunch; void currentSelSet; void currentShowMission; void currentReplayActive; renderMission(currentMission, currentSelIdx, currentEditing); });

  onDestroy(() => {
    unsubMission(); unsubSelIdx(); unsubSelSet(); unsubEditMode(); unsubShowMission(); unsubReplayActive(); unsubLaunch();
    if (launchMarker) { try { map.removeLayer(launchMarker); } catch {} launchMarker = undefined; }
    map.off('click', onMapClick);
    if (editorPopup) { map.removeLayer(editorPopup); editorPopup = undefined; }
    missionGroup.clearLayers();
    map.removeLayer(missionGroup);
  });
</script>

<style>
  :global(.mission-wp-icon) { background: none !important; border: none !important; }
  :global(.wp-param-label-wrapper) { background: none !important; border: none !important; overflow: visible !important; width: auto !important; height: auto !important; }
  :global(.wp-param-label) { background: rgba(30,30,30,0.88); color: #ccc; padding: 3px 8px; border-radius: 4px; font-size: 12px; line-height: 1.4; white-space: nowrap; border: 1px solid rgba(55,168,219,0.35); pointer-events: none; }
  :global(.wp-editor-popup-container .leaflet-popup-content-wrapper) { background: rgba(30,30,30,0.82); backdrop-filter: blur(10px); color: #ccc; border: 1px solid rgba(55,168,219,0.35); border-radius: 8px; box-shadow: 0 6px 24px rgba(0,0,0,0.5); padding: 0; }
  :global(.wp-editor-popup-container .leaflet-popup-content) { margin: 0; width: auto !important; }
  :global(.wp-editor-popup-container .leaflet-popup-tip) { background: rgba(30,30,30,0.82); backdrop-filter: blur(10px); border: 1px solid rgba(55,168,219,0.35); }
  :global(.wp-editor-popup) { padding: 10px; font-size: 13px; min-width: 190px; }
  :global(.wpe-header) { display: flex; align-items: center; gap: 6px; font-weight: bold; font-size: 14px; color: #37a8db; margin-bottom: 6px; border-bottom: 1px solid #444; padding-bottom: 4px; }
  :global(.wpe-num) { flex-shrink: 0; }
  :global(.wpe-cmd-select) { flex: 1; background: #1e1e1e; color: #ccc; border: 1px solid #555; border-radius: 3px; padding: 2px 4px; font-size: 12px; font-weight: normal; cursor: pointer; color-scheme: dark; }
  :global(.wpe-cmd-select:focus) { border-color: #37a8db; outline: none; }
  :global(.wpe-type-name) { color: #888; font-weight: normal; font-size: 12px; margin-left: 4px; }
  :global(.wpe-row) { display: flex; align-items: center; gap: 6px; margin-bottom: 5px; }
  :global(.wpe-row label) { width: 52px; color: #888; font-size: 12px; flex-shrink: 0; }
  :global(.wpe-row input) { background: #2a2a2a; color: #ccc; border: 1px solid #555; border-radius: 0; padding: 3px 4px; font-size: 13px; width: 52px; text-align: center; appearance: textfield; -moz-appearance: textfield; }
  :global(.wpe-row input::-webkit-inner-spin-button), :global(.wpe-row input::-webkit-outer-spin-button) { -webkit-appearance: none; margin: 0; }
  :global(.wpe-row input:focus) { border-color: #37a8db; outline: none; }
  :global(.wpe-coord-input) { width: 110px !important; text-align: right !important; }
  :global(.wpe-num-ctrl) { display: flex; align-items: stretch; border-radius: 4px; overflow: hidden; border: 1px solid #555; }
  :global(.wpe-num-ctrl input) { border: none; border-left: 1px solid #555; border-right: 1px solid #555; border-radius: 0; }
  :global(.wpe-num-btn) { background: #333; color: #aaa; border: none; width: 24px; cursor: pointer; font-size: 14px; font-weight: bold; line-height: 1; display: flex; align-items: center; justify-content: center; padding: 0; user-select: none; }
  :global(.wpe-num-btn:hover) { background: #37a8db; color: #fff; }
  :global(.wpe-num-btn:active) { background: #2980b9; }
  :global(.wpe-row select) { background: #2a2a2a; color: #ccc; border: 1px solid #555; border-radius: 3px; padding: 3px 4px; font-size: 13px; flex: 1; }
  :global(.wpe-toggle) { background: #2a2a2a; color: #ccc; border: 1px solid #555; border-radius: 3px; padding: 3px 8px; font-size: 12px; cursor: pointer; }
  :global(.wpe-toggle:hover) { background: #3a3a3a; }
  :global(.wpe-unit) { color: #888; font-size: 12px; white-space: nowrap; }
  :global(.wpe-ua-row) { gap: 4px; }
  :global(.wpe-ua-btn) { padding: 2px 5px; border: 1px solid #555; border-radius: 3px; background: #2a2a2a; color: #777; cursor: pointer; font-size: 10px; font-weight: 600; transition: all 0.15s; }
  :global(.wpe-ua-btn.active) { background: #37a8db; color: #fff; border-color: #37a8db; }
  :global(.wpe-ua-btn:hover:not(.active)) { background: #3a3a3a; color: #ccc; }
  :global(.wpe-mod-section) { margin-top: 6px; padding-top: 5px; border-top: 1px dashed #555; }
  :global(.wpe-mod-header) { display: flex; align-items: center; justify-content: space-between; font-size: 12px; font-weight: 600; color: #e67e22; margin-bottom: 4px; }
  :global(.wpe-mod-remove) { background: none; border: none; color: #c0392b; cursor: pointer; font-size: 12px; padding: 0 4px; line-height: 1; }
  :global(.wpe-mod-remove:hover) { color: #e74c3c; }
  :global(.wpe-add-mod) { margin-top: 4px; }
  :global(.wpe-add-mod select) { width: 100%; background: #2a2a2a; color: #888; border: 1px dashed #555; border-radius: 3px; padding: 3px 4px; font-size: 12px; cursor: pointer; }
  :global(.wpe-actions) { display: flex; gap: 4px; margin-top: 6px; padding-top: 4px; border-top: 1px solid #444; }
  :global(.wpe-actions button) { padding: 3px 10px; border: 1px solid #555; border-radius: 3px; background: #2a2a2a; color: #ccc; cursor: pointer; font-size: 13px; }
  :global(.wpe-actions button:hover:not(:disabled)) { background: #3a3a3a; }
  :global(.wpe-actions button:disabled) { opacity: 0.3; cursor: not-allowed; }
  :global(.wpe-remove) { margin-left: auto; border-color: #c0392b !important; color: #e74c3c !important; }
  :global(.wpe-remove:hover) { background: #c0392b !important; color: #fff !important; }
  :global(.wp-editor-popup select) { color-scheme: dark; }
</style>
