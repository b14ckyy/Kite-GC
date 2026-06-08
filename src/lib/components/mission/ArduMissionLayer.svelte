<!--
  SPDX-License-Identifier: GPL-3.0-or-later
  Copyright (C) 2026 Marc Hoffmann (b14ckyy)
-->

<!-- ArduMissionLayer.svelte
     Renders ArduPilot mission waypoints on the Leaflet map.
     Usage: <ArduMissionLayer {map} />
-->
<script lang="ts">
  import { onDestroy } from 'svelte';
  import L from 'leaflet';
  import { get } from 'svelte/store';
  import { t } from 'svelte-i18n';
  import {
    arduMission, arduSelectedWpIndex, arduEditMode,
    arduUpdateWp, arduRemoveWp, arduMoveWp, arduAddWp,
    arduHasLocation, arduIsLoiter,
    MAV_CMD_NAV_WAYPOINT, MAV_CMD_NAV_LOITER_UNLIM, MAV_CMD_NAV_LOITER_TURNS,
    MAV_CMD_NAV_LOITER_TIME, MAV_CMD_NAV_RETURN_TO_LAUNCH, MAV_CMD_NAV_LAND,
    MAV_CMD_NAV_TAKEOFF, MAV_CMD_DO_JUMP, MAV_CMD_DO_CHANGE_SPEED,
    MAV_CMD_DO_SET_ROI, MAV_CMD_CONDITION_DELAY,
    MAV_FRAME_GLOBAL, MAV_FRAME_GLOBAL_RELATIVE_ALT, MAV_FRAME_GLOBAL_TERRAIN_ALT,
    MAV_CMD_LABELS,
    type ArduWaypoint, type MavFrame,
  } from '$lib/stores/missionArdupilot';
  import { iconForArduWp } from '$lib/helpers/missionIconsArdupilot';
  import { settings } from '$lib/stores/settings';

  interface Props { map: L.Map; }
  let { map }: Props = $props();

  let currentWps    = $state<ArduWaypoint[]>(get(arduMission));
  let currentSelIdx = $state<number>(get(arduSelectedWpIndex));
  let currentEditing = $state<boolean>(get(arduEditMode));

  const unsubMission  = arduMission.subscribe(wps => { currentWps = wps; });
  const unsubSelIdx   = arduSelectedWpIndex.subscribe(i => { currentSelIdx = i; });
  const unsubEditMode = arduEditMode.subscribe(e => { currentEditing = e; });

  // svelte-ignore state_referenced_locally
  const missionGroup = L.layerGroup().addTo(map);
  let wpMarkers: L.Marker[] = [];
  let loiterCircles: L.Circle[] = [];
  let flightPath: L.Polyline | undefined;
  let editorPopup: L.Popup | undefined;
  let editorPopupIdx = -1;

  const CMD_OPTIONS: Array<[number, string]> = [
    [MAV_CMD_NAV_WAYPOINT,         'Waypoint'],
    [MAV_CMD_NAV_LOITER_UNLIM,     'Loiter Unlim'],
    [MAV_CMD_NAV_LOITER_TURNS,     'Loiter Turns'],
    [MAV_CMD_NAV_LOITER_TIME,      'Loiter Time'],
    [MAV_CMD_NAV_RETURN_TO_LAUNCH, 'Return to Launch'],
    [MAV_CMD_NAV_LAND,             'Land'],
    [MAV_CMD_NAV_TAKEOFF,          'Takeoff'],
    [MAV_CMD_DO_JUMP,              'Jump'],
    [MAV_CMD_DO_CHANGE_SPEED,      'Change Speed'],
    [MAV_CMD_DO_SET_ROI,           'Set ROI'],
    [MAV_CMD_CONDITION_DELAY,      'Delay'],
  ];

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

  function numInputHtml(field: string, value: number, step: number, min?: number, max?: number): string {
    const minAttr = min !== undefined ? `min="${min}"` : '';
    const maxAttr = max !== undefined ? `max="${max}"` : '';
    return `<div class="wpe-num-ctrl"><button class="wpe-num-btn" data-numdir="-1" data-field="${field}">−</button><input type="number" data-field="${field}" value="${value}" step="${step}" ${minAttr} ${maxAttr}/><button class="wpe-num-btn" data-numdir="1" data-field="${field}">+</button></div>`;
  }

  function selectHtml(field: string, options: Array<[number, string]>, current: number): string {
    const opts = options.map(([v, l]) => `<option value="${v}"${v === current ? ' selected' : ''}>${l}</option>`).join('');
    return `<select data-field="${field}" class="wpe-row-select">${opts}</select>`;
  }

  function getDefaultParams(cmd: number): { param1: number; param2: number; param3: number; param4: number } {
    switch (cmd) {
      case MAV_CMD_NAV_LOITER_UNLIM:
        return { param1: 0, param2: 0, param3: 50, param4: 0 };
      case MAV_CMD_NAV_LOITER_TURNS:
        return { param1: 1, param2: 0, param3: 50, param4: 0 };
      case MAV_CMD_NAV_LOITER_TIME:
        return { param1: 30, param2: 0, param3: 50, param4: 0 };
      case MAV_CMD_DO_JUMP:
        return { param1: 1, param2: 1, param3: 0, param4: 0 };
      case MAV_CMD_DO_CHANGE_SPEED:
        return { param1: 0, param2: 10, param3: -1, param4: 0 };
      default:
        return { param1: 0, param2: 0, param3: 0, param4: 0 };
    }
  }

  function attachNumBtnEvents(el: HTMLElement) {
    el.querySelectorAll('.wpe-num-btn').forEach(btn => {
      btn.addEventListener('click', (e) => {
        e.preventDefault(); e.stopPropagation();
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

  function buildEditorHtml(wp: ArduWaypoint, idx: number, total: number, displayNum: number): string {
    const cmd = wp.command;
    const latDeg = (wp.lat / 1e7).toFixed(7);
    const lonDeg = (wp.lon / 1e7).toFixed(7);
    const frameLbl = frameLabel(wp.frame);

    const cmdOpts = CMD_OPTIONS.map(([v, lbl]) =>
      `<option value="${v}"${v === cmd ? ' selected' : ''}>${lbl}</option>`
    ).join('');

    let html = `<div class="wp-editor-popup">`;
    html += `<div class="wpe-header"><span class="wpe-num">WP${displayNum}</span><select data-field="cmdType" class="wpe-cmd-select">${cmdOpts}</select></div>`;

    if (arduHasLocation(cmd)) {
      html += `<div class="wpe-row"><label>${$t('missionLayer.alt')}</label>${numInputHtml('alt', wp.alt, 1, 0)}<button data-field="frameToggle" class="wpe-toggle">${frameLbl}</button></div>`;
      html += `<div class="wpe-row"><label>${$t('missionLayer.lat')}</label><input type="number" data-field="lat" value="${latDeg}" step="0.0000001" min="-90" max="90" class="wpe-coord-input"/></div>`;
      html += `<div class="wpe-row"><label>${$t('missionLayer.lon')}</label><input type="number" data-field="lon" value="${lonDeg}" step="0.0000001" min="-180" max="180" class="wpe-coord-input"/></div>`;
    }

    if (cmd === MAV_CMD_NAV_WAYPOINT) {
      html += `<div class="wpe-row"><label>${$t('arduMission.hold')}</label>${numInputHtml('p1', wp.param1, 0.5, 0)}<span class="wpe-unit">${$t('arduMission.sec')}</span></div>`;
      html += `<div class="wpe-row"><label>${$t('arduMission.accept')}</label>${numInputHtml('p2', wp.param2, 1, 0)}<span class="wpe-unit">m</span></div>`;
      html += `<div class="wpe-row"><label>${$t('arduMission.passRadius')}</label>${numInputHtml('p3', wp.param3, 1, 0)}<span class="wpe-unit">m</span></div>`;
      html += `<div class="wpe-row"><label>${$t('arduMission.yaw')}</label>${numInputHtml('p4', wp.param4, 1, -1, 360)}<span class="wpe-unit">°</span></div>`;
    }
    if (arduIsLoiter(cmd)) {
      html += `<div class="wpe-row"><label>${$t('arduMission.radius')} (+CW/−CCW)</label>${numInputHtml('p3', wp.param3, 1)}<span class="wpe-unit">m</span></div>`;
    }
    if (cmd === MAV_CMD_NAV_LOITER_UNLIM) {
      html += `<div class="wpe-row"><label>${$t('arduMission.yaw')}</label>${numInputHtml('p4', wp.param4, 1, -1, 360)}<span class="wpe-unit">°</span></div>`;
    }
    if (cmd === MAV_CMD_NAV_LOITER_TURNS) {
      html += `<div class="wpe-row"><label>${$t('arduMission.turns')}</label>${numInputHtml('p1', wp.param1, 1, 1)}</div>`;
      html += `<div class="wpe-row"><label>${$t('arduMission.headingReq')}</label>${selectHtml('p2', [[0, $t('arduMission.no')], [1, $t('arduMission.yes')]], wp.param2)}</div>`;
      html += `<div class="wpe-row"><label>${$t('arduMission.exitXtrack')}</label>${selectHtml('p4', [[0, $t('arduMission.exitLoiter')], [1, $t('arduMission.exitXtrack')]], wp.param4)}</div>`;
    }
    if (cmd === MAV_CMD_NAV_LOITER_TIME) {
      html += `<div class="wpe-row"><label>${$t('arduMission.time')}</label>${numInputHtml('p1', wp.param1, 1, 0)}<span class="wpe-unit">${$t('arduMission.sec')}</span></div>`;
      html += `<div class="wpe-row"><label>${$t('arduMission.headingReq')}</label>${selectHtml('p2', [[0, $t('arduMission.no')], [1, $t('arduMission.yes')]], wp.param2)}</div>`;
      html += `<div class="wpe-row"><label>${$t('arduMission.exitXtrack')}</label>${selectHtml('p4', [[0, $t('arduMission.exitLoiter')], [1, $t('arduMission.exitXtrack')]], wp.param4)}</div>`;
    }
    if (cmd === MAV_CMD_NAV_LAND) {
      html += `<div class="wpe-row"><label>${$t('arduMission.abortAlt')}</label>${numInputHtml('p1', wp.param1, 1, 0)}<span class="wpe-unit">m</span></div>`;
      html += `<div class="wpe-row"><label>${$t('arduMission.precisionLand')}</label>${selectHtml('p2', [[0, $t('arduMission.precNone')], [1, $t('arduMission.precOpport')], [2, $t('arduMission.precRequired')]], wp.param2)}</div>`;
      html += `<div class="wpe-row"><label>${$t('arduMission.yaw')}</label>${numInputHtml('p4', wp.param4, 1, -1, 360)}<span class="wpe-unit">°</span></div>`;
    }
    if (cmd === MAV_CMD_NAV_TAKEOFF) {
      html += `<div class="wpe-row"><label>${$t('arduMission.minPitch')}</label>${numInputHtml('p1', wp.param1, 1, 0, 90)}<span class="wpe-unit">°</span></div>`;
      html += `<div class="wpe-row"><label>${$t('arduMission.yaw')}</label>${numInputHtml('p4', wp.param4, 1, -1, 360)}<span class="wpe-unit">°</span></div>`;
    }
    if (cmd === MAV_CMD_DO_JUMP) {
      html += `<div class="wpe-row"><label>${$t('missionLayer.toWp')}</label>${numInputHtml('p1', wp.param1, 1, 1, total)}</div>`;
      html += `<div class="wpe-row"><label>${$t('mission.repeat')}</label>${numInputHtml('p2', wp.param2, 1, -1)}</div>`;
    }
    if (cmd === MAV_CMD_DO_CHANGE_SPEED) {
      html += `<div class="wpe-row"><label>${$t('arduMission.speedType')}</label>${selectHtml('p1', [[0, $t('arduMission.airspeed')], [1, $t('arduMission.groundspeed')], [2, $t('arduMission.climbspeed')], [3, $t('arduMission.descentspeed')]], wp.param1)}</div>`;
      html += `<div class="wpe-row"><label>${$t('arduMission.speed')}</label>${numInputHtml('p2', wp.param2, 0.5, -1)}<span class="wpe-unit">m/s</span></div>`;
      html += `<div class="wpe-row"><label>${$t('arduMission.throttle')}</label>${numInputHtml('p3', wp.param3, 1, -1, 100)}<span class="wpe-unit">%</span></div>`;
    }
    if (cmd === MAV_CMD_CONDITION_DELAY) {
      html += `<div class="wpe-row"><label>${$t('arduMission.delay')}</label>${numInputHtml('p1', wp.param1, 1, 0)}<span class="wpe-unit">${$t('arduMission.sec')}</span></div>`;
    }

    html += `<div class="wpe-actions">`;
    html += `<button data-action="moveUp" ${idx <= 0 ? 'disabled' : ''} title="${$t('missionLayer.moveUp')}">▲</button>`;
    html += `<button data-action="moveDown" ${idx >= total - 1 ? 'disabled' : ''} title="${$t('missionLayer.moveDown')}">▼</button>`;
    html += `<button data-action="remove" class="wpe-remove" title="${$t('missionLayer.removeWp')}">✕</button>`;
    html += `</div></div>`;
    return html;
  }

  function attachEditorEvents(popup: L.Popup, wp: ArduWaypoint, idx: number) {
    const el = popup.getElement();
    if (!el) return;

    const cmdSelect = el.querySelector('select[data-field="cmdType"]') as HTMLSelectElement | null;
    cmdSelect?.addEventListener('change', () => {
      const newCmd = Number(cmdSelect.value);
      arduUpdateWp(idx, { ...wp, command: newCmd, ...getDefaultParams(newCmd) });
    });

    const altInput = el.querySelector('input[data-field="alt"]') as HTMLInputElement | null;
    altInput?.addEventListener('change', () => {
      arduUpdateWp(idx, { ...wp, alt: Number(altInput.value) });
    });

    const frameBtn = el.querySelector('button[data-field="frameToggle"]') as HTMLButtonElement | null;
    frameBtn?.addEventListener('click', () => {
      arduUpdateWp(idx, { ...wp, frame: nextFrame(wp.frame) });
    });

    const latInput = el.querySelector('input[data-field="lat"]') as HTMLInputElement | null;
    const lonInput = el.querySelector('input[data-field="lon"]') as HTMLInputElement | null;
    function applyCoordChange() {
      const newLat = parseFloat(latInput?.value ?? '');
      const newLon = parseFloat(lonInput?.value ?? '');
      if (isNaN(newLat) || isNaN(newLon)) return;
      arduUpdateWp(idx, { ...wp, lat: Math.round(newLat * 1e7), lon: Math.round(newLon * 1e7) });
      map.panTo(L.latLng(newLat, newLon));
    }
    latInput?.addEventListener('change', applyCoordChange);
    lonInput?.addEventListener('change', applyCoordChange);

    // Generic binder for p1–p4: matches both <input> and <select>
    function bindParam(field: string, apply: (v: number) => ArduWaypoint) {
      const node = el!.querySelector(`[data-field="${field}"]`) as HTMLInputElement | HTMLSelectElement | null;
      node?.addEventListener('change', () => arduUpdateWp(idx, apply(Number(node.value))));
    }
    bindParam('p1', v => ({ ...wp, param1: v }));
    bindParam('p2', v => ({ ...wp, param2: v }));
    bindParam('p3', v => ({ ...wp, param3: v }));
    bindParam('p4', v => ({ ...wp, param4: v }));

    attachNumBtnEvents(el);

    el.querySelector('button[data-action="moveUp"]')?.addEventListener('click', () => {
      if (idx > 0) { arduMoveWp(idx, idx - 1); arduSelectedWpIndex.set(idx - 1); }
    });
    el.querySelector('button[data-action="moveDown"]')?.addEventListener('click', () => {
      if (idx < currentWps.length - 1) { arduMoveWp(idx, idx + 1); arduSelectedWpIndex.set(idx + 1); }
    });
    el.querySelector('button[data-action="remove"]')?.addEventListener('click', () => {
      arduRemoveWp(idx);
      arduSelectedWpIndex.set(-1);
    });

    el.querySelectorAll('input, select, button').forEach(input => {
      L.DomEvent.disableClickPropagation(input as HTMLElement);
    });
  }

  function renderMission(wps: ArduWaypoint[], selIdx: number, editing: boolean) {
    const keepPopup = editing && editorPopup && editorPopupIdx === selIdx && selIdx >= 0;
    missionGroup.clearLayers();
    wpMarkers = []; loiterCircles = [];

    if (!keepPopup) {
      if (editorPopup) map.removeLayer(editorPopup);
      editorPopup = undefined; editorPopupIdx = -1;
    }
    if (wps.length === 0) return;

    const fpPositions: L.LatLng[] = [];
    const fpWpIndices: number[] = [];

    for (let i = 0; i < wps.length; i++) {
      const wp = wps[i];
      const selected = i === selIdx;
      const displayNum = i + 1;

      if (arduHasLocation(wp.command)) {
        const latLng = L.latLng(wp.lat / 1e7, wp.lon / 1e7);

        if (wp.command !== MAV_CMD_DO_SET_ROI) {
          fpPositions.push(latLng);
          fpWpIndices.push(i);
        }

        const icon = iconForArduWp(wp, displayNum, selected);
        const marker = L.marker(latLng, {
          icon,
          draggable: editing,
          title: `WP${displayNum}: ${MAV_CMD_LABELS[wp.command] ?? ''}`,
        }).addTo(missionGroup);

        marker.on('click', () => { arduSelectedWpIndex.set(i); });
        if (editing) {
          marker.on('dragend', () => {
            const pos = marker.getLatLng();
            arduUpdateWp(i, { ...wp, lat: Math.round(pos.lat * 1e7), lon: Math.round(pos.lng * 1e7) });
          });
        }

        if (arduIsLoiter(wp.command) && wp.param3 > 0) {
          const circle = L.circle(latLng, {
            radius: Math.abs(wp.param3),
            color: '#00bcd4', weight: 1.5, fill: false, opacity: 0.5, dashArray: '4 4',
          }).addTo(missionGroup);
          loiterCircles.push(circle);
        }

        if (editing && selected) {
          const htmlContent = buildEditorHtml(wp, i, wps.length, displayNum);
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
          setTimeout(() => { if (editorPopup) attachEditorEvents(editorPopup, wp, i); }, 50);
        }

        if (!editing) {
          const frameLbl = frameLabel(wp.frame);
          marker.bindTooltip(
            `WP${displayNum} ${MAV_CMD_LABELS[wp.command] ?? ''}<br>${wp.alt.toFixed(1)}m ${frameLbl}`,
            { direction: 'top', offset: L.point(0, -20) }
          );
        }
        wpMarkers.push(marker);
      }

      if (wp.command === MAV_CMD_DO_JUMP && wp.param1 > 0) {
        const targetIdx = Math.round(wp.param1) - 1;
        const targetWp = wps[targetIdx];
        const prevLocIdx = findPrevLocationIdx(wps, i);
        const sourceWp = prevLocIdx >= 0 ? wps[prevLocIdx] : null;
        if (sourceWp && targetWp && arduHasLocation(targetWp.command)) {
          L.polyline([
            L.latLng(sourceWp.lat / 1e7, sourceWp.lon / 1e7),
            L.latLng(targetWp.lat / 1e7, targetWp.lon / 1e7),
          ], { color: '#8e44ad', weight: 2, dashArray: '8 4', opacity: 0.8 }).addTo(missionGroup)
           .bindTooltip(`Jump → WP${wp.param1} ×${wp.param2 < 0 ? '∞' : wp.param2}`, { sticky: true });
        }
      }
    }

    if (fpPositions.length > 1) {
      flightPath = L.polyline(fpPositions, {
        color: '#37a8db', weight: editing ? 6 : 3, opacity: 0.7,
      }).addTo(missionGroup);

      if (editing) {
        flightPath.on('click', (e: L.LeafletMouseEvent) => {
          L.DomEvent.stopPropagation(e);
          const clickLatLng = e.latlng;
          let bestInsertIdx = wps.length;
          let bestDist = Infinity;
          for (let s = 0; s < fpPositions.length - 1; s++) {
            const mid = L.latLng(
              (fpPositions[s].lat + fpPositions[s + 1].lat) / 2,
              (fpPositions[s].lng + fpPositions[s + 1].lng) / 2,
            );
            const d = clickLatLng.distanceTo(mid);
            if (d < bestDist) { bestDist = d; bestInsertIdx = fpWpIndices[s + 1] ?? wps.length; }
          }
          const defaultAlt = get(settings).defaultWpAltitudeM;
          const updated = [...wps];
          updated.splice(bestInsertIdx, 0, {
            command: MAV_CMD_NAV_WAYPOINT,
            frame: MAV_FRAME_GLOBAL_RELATIVE_ALT,
            param1: 0, param2: 0, param3: 0, param4: 0,
            lat: Math.round(clickLatLng.lat * 1e7),
            lon: Math.round(clickLatLng.lng * 1e7),
            alt: defaultAlt,
            autocontinue: true,
          });
          arduMission.set(updated);
          arduSelectedWpIndex.set(bestInsertIdx);
        });
      }
    }
  }

  function findPrevLocationIdx(wps: ArduWaypoint[], fromIndex: number): number {
    for (let i = fromIndex - 1; i >= 0; i--) {
      if (arduHasLocation(wps[i].command)) return i;
    }
    return -1;
  }

  function onMapClick(e: L.LeafletMouseEvent) {
    if (!currentEditing) return;
    if (currentSelIdx >= 0) { arduSelectedWpIndex.set(-1); return; }
    const defaultAlt = get(settings).defaultWpAltitudeM;
    arduAddWp({
      command: MAV_CMD_NAV_WAYPOINT,
      frame: MAV_FRAME_GLOBAL_RELATIVE_ALT,
      param1: 0, param2: 0, param3: 0, param4: 0,
      lat: Math.round(e.latlng.lat * 1e7),
      lon: Math.round(e.latlng.lng * 1e7),
      alt: defaultAlt,
      autocontinue: true,
    });
  }

  // svelte-ignore state_referenced_locally
  map.on('click', onMapClick);

  $effect(() => { renderMission(currentWps, currentSelIdx, currentEditing); });

  onDestroy(() => {
    unsubMission(); unsubSelIdx(); unsubEditMode();
    map.off('click', onMapClick);
    if (editorPopup) { map.removeLayer(editorPopup); editorPopup = undefined; }
    missionGroup.clearLayers();
    map.removeLayer(missionGroup);
  });
</script>
