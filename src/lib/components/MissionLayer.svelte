<!-- MissionLayer.svelte
     Renders INAV mission waypoints on the Leaflet map:
     - SVG markers for each waypoint type (sized per type)
     - Display numbering skips modifier WPs (Jump, RTH, SetHead)
     - Floating parameter labels (incl. attached modifiers) next to each WP in edit mode
     - Floating editor popup (incl. modifier sections) on selected WP in edit mode
     - Polyline connecting flight-path waypoints (excludes POI)
     - Dashed lines for JUMP / RTH modifiers
     - Click-to-add + drag in edit mode

     Usage: <MissionLayer {map} />
-->
<script lang="ts">
  import { onDestroy } from 'svelte';
  import L from 'leaflet';
  import {
    mission, geoWaypoints, selectedWpIndex, editMode,
    missionAddWp, missionUpdateWp, missionRemoveWp, missionInsertWp,
    missionReorderWp,
    getTotalWpCount, MAX_WAYPOINTS_TOTAL,
    type Waypoint, type Mission, WpAction, hasLocation, isModifier, toDeg, fromDeg, altFromM,
    WP_ACTION_LABELS,
  } from '$lib/stores/mission';
  import { get } from 'svelte/store';
  import { settings } from '$lib/stores/settings';

  interface Props {
    map: L.Map;
  }

  let { map }: Props = $props();

  // Local reactive state mirroring stores
  let currentMission = $state<Mission>(get(mission));
  let currentSelIdx = $state<number>(get(selectedWpIndex));
  let currentEditing = $state<boolean>(get(editMode));

  const unsubMission = mission.subscribe(m => { currentMission = m; });
  const unsubSelIdx = selectedWpIndex.subscribe(i => { currentSelIdx = i; });
  const unsubEditMode = editMode.subscribe(e => { currentEditing = e; });

  // ── Layer group for all mission elements ─────────────────────────
  const missionGroup = L.layerGroup().addTo(map);
  let wpMarkers: L.Marker[] = [];
  let flightPath: L.Polyline | undefined;
  let modifierLines: L.Polyline[] = [];
  let paramLabels: L.Marker[] = [];
  let editorPopup: L.Popup | undefined;
  let editorPopupIdx: number = -1;  // which WP index the popup is for

  // ── Display numbering (skip modifiers) ───────────────────────────

  /** Build map: array-index → display number (modifiers get no number) */
  function buildDisplayNumbers(waypoints: Waypoint[]): Map<number, number> {
    const nums = new Map<number, number>();
    let dn = 1;
    for (let i = 0; i < waypoints.length; i++) {
      if (!isModifier(waypoints[i].action)) {
        nums.set(i, dn++);
      }
    }
    return nums;
  }

  /** Collect modifier WPs attached after a given geo-WP index */
  function getModifiersForWp(waypoints: Waypoint[], geoIdx: number): { wp: Waypoint; idx: number }[] {
    const mods: { wp: Waypoint; idx: number }[] = [];
    for (let j = geoIdx + 1; j < waypoints.length; j++) {
      if (isModifier(waypoints[j].action)) {
        mods.push({ wp: waypoints[j], idx: j });
      } else break;
    }
    return mods;
  }

  /** Whether a WP is part of the flight path polyline (excludes POI and modifiers) */
  function isFlightPathWp(action: WpAction): boolean {
    return hasLocation(action) && action !== WpAction.SetPoi;
  }

  /** Find the index of the first mission-terminating WP (LAND or RTH).
   *  Returns -1 if no terminal WP exists. */
  function findMissionEndIndex(waypoints: Waypoint[]): number {
    for (let i = 0; i < waypoints.length; i++) {
      if (waypoints[i].action === WpAction.Land || waypoints[i].action === WpAction.Rth) {
        return i;
      }
    }
    return -1;
  }

  // ── SVG Icon Factories ───────────────────────────────────────────

  const MAX_WAYPOINTS = MAX_WAYPOINTS_TOTAL;

  /** WAYPOINT — upside-down teardrop with WP number (48×66) */
  function waypointIcon(num: number, selected: boolean): L.DivIcon {
    const fill = selected ? '#ff4444' : '#37a8db';
    const stroke = selected ? '#cc0000' : '#1a5276';
    return L.divIcon({
      className: 'mission-wp-icon',
      html: `<svg viewBox="0 0 32 44" width="48" height="66">
        <path d="M16 44 C16 44 2 24 2 16 A14 14 0 1 1 30 16 C30 24 16 44 16 44Z"
              fill="${fill}" stroke="${stroke}" stroke-width="2"/>
        <text x="16" y="20" text-anchor="middle" fill="white" font-size="12" font-weight="bold"
              font-family="sans-serif">${num}</text>
      </svg>`,
      iconSize: [48, 66],
      iconAnchor: [24, 66],
    });
  }

  /** POSHOLD — circle with orbit ring (88×88, tighter text) */
  function posholdIcon(num: number, seconds: number | null, selected: boolean): L.DivIcon {
    const fill = selected ? '#ff4444' : '#f39c12';
    const stroke = selected ? '#cc0000' : '#d68910';
    const label = seconds !== null ? `${seconds}s` : '∞';
    return L.divIcon({
      className: 'mission-wp-icon',
      html: `<svg viewBox="0 0 40 40" width="88" height="88">
        <circle cx="20" cy="20" r="17" fill="none" stroke="${stroke}" stroke-width="2" stroke-dasharray="4 2"/>
        <circle cx="20" cy="20" r="11" fill="${fill}" stroke="${stroke}" stroke-width="1.5"/>
        <text x="20" y="18" text-anchor="middle" fill="white" font-size="9" font-weight="bold"
              font-family="sans-serif">${num}</text>
        <text x="20" y="26" text-anchor="middle" fill="white" font-size="7"
              font-family="sans-serif">${label}</text>
      </svg>`,
      iconSize: [88, 88],
      iconAnchor: [44, 44],
    });
  }

  /** SET_POI — purple marker with eye icon (48×48) */
  function poiIcon(num: number, selected: boolean): L.DivIcon {
    const fill = selected ? '#ff4444' : '#8e44ad';
    const stroke = selected ? '#cc0000' : '#6c3483';
    return L.divIcon({
      className: 'mission-wp-icon',
      html: `<svg viewBox="0 0 32 32" width="48" height="48">
        <circle cx="16" cy="16" r="13" fill="${fill}" stroke="${stroke}" stroke-width="2"/>
        <text x="16" y="13" text-anchor="middle" fill="white" font-size="10"
              font-family="sans-serif">👁</text>
        <text x="16" y="25" text-anchor="middle" fill="white" font-size="9" font-weight="bold"
              font-family="sans-serif">${num}</text>
      </svg>`,
      iconSize: [48, 48],
      iconAnchor: [24, 24],
    });
  }

  /** LAND — orange teardrop with down-arrow icon (48×66) */
  function landIcon(selected: boolean): L.DivIcon {
    const fill = selected ? '#ff4444' : '#f39c12';
    const stroke = selected ? '#cc0000' : '#d68910';
    return L.divIcon({
      className: 'mission-wp-icon',
      html: `<svg viewBox="0 0 32 44" width="48" height="66">
        <path d="M16 44 C16 44 2 24 2 16 A14 14 0 1 1 30 16 C30 24 16 44 16 44Z"
              fill="${fill}" stroke="${stroke}" stroke-width="2"/>
        <path d="M16 10 L16 25 M11 20 L16 25 L21 20" fill="none" stroke="white"
              stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"/>
      </svg>`,
      iconSize: [48, 66],
      iconAnchor: [24, 66],
    });
  }

  /** RTH — house icon (42×42) */
  function rthIcon(selected: boolean): L.DivIcon {
    const fill = selected ? '#ff4444' : '#e67e22';
    return L.divIcon({
      className: 'mission-wp-icon',
      html: `<svg viewBox="0 0 32 32" width="42" height="42">
        <path d="M16 4 L4 16 L8 16 L8 28 L24 28 L24 16 L28 16 Z" fill="${fill}" stroke="#7e5109" stroke-width="1.5"/>
        <text x="16" y="22" text-anchor="middle" fill="white" font-size="8" font-weight="bold"
              font-family="sans-serif">RTH</text>
      </svg>`,
      iconSize: [42, 42],
      iconAnchor: [21, 21],
    });
  }

  /** Generic fallback icon (48×48) */
  function genericIcon(num: number, label: string, selected: boolean): L.DivIcon {
    const fill = selected ? '#ff4444' : '#7f8c8d';
    return L.divIcon({
      className: 'mission-wp-icon',
      html: `<svg viewBox="0 0 32 32" width="48" height="48">
        <circle cx="16" cy="16" r="13" fill="${fill}" stroke="#2c3e50" stroke-width="2"/>
        <text x="16" y="13" text-anchor="middle" fill="white" font-size="8"
              font-family="sans-serif">${label}</text>
        <text x="16" y="24" text-anchor="middle" fill="white" font-size="10" font-weight="bold"
              font-family="sans-serif">${num}</text>
      </svg>`,
      iconSize: [48, 48],
      iconAnchor: [24, 24],
    });
  }

  /** Pick the right icon for a waypoint (displayNum used for numbered types) */
  function iconForWp(wp: Waypoint, displayNum: number, selected: boolean): L.DivIcon {
    switch (wp.action) {
      case WpAction.Waypoint:
        return waypointIcon(displayNum, selected);
      case WpAction.PosholdUnlim:
        return posholdIcon(displayNum, null, selected);
      case WpAction.PosholdTime:
        return posholdIcon(displayNum, wp.p1, selected);
      case WpAction.SetPoi:
        return poiIcon(displayNum, selected);
      case WpAction.Land:
        return landIcon(selected);
      case WpAction.Rth:
        return rthIcon(selected);
      case WpAction.Jump:
        return genericIcon(displayNum, 'JMP', selected);
      case WpAction.SetHead:
        return genericIcon(displayNum, 'HDG', selected);
      default:
        return genericIcon(displayNum, '?', selected);
    }
  }

  // ── Floating param label (compact info box next to WP) ───────────

  function paramLabelHtml(wp: Waypoint, modifiers: { wp: Waypoint; idx: number }[]): string {
    const altM = (wp.altitude / 100).toFixed(0);
    const altType = (wp.p3 & 1) ? 'AMSL' : 'REL';
    let lines = [`${altM}m ${altType}`];

    switch (wp.action) {
      case WpAction.Waypoint:
      case WpAction.Land:
        if (wp.p1 > 0) lines.push(`${wp.p1} cm/s`);
        break;
      case WpAction.PosholdTime:
        lines.push(`Hold ${wp.p1}s`);
        break;
      case WpAction.PosholdUnlim:
        lines.push('Hold ∞');
        break;
    }

    // Attached modifier info
    for (const mod of modifiers) {
      switch (mod.wp.action) {
        case WpAction.Rth:
          lines.push(`⮐ RTH ${mod.wp.p1 ? 'Land' : 'Hover'}`);
          break;
        case WpAction.Jump:
          lines.push(`⮐ Jump→WP${mod.wp.p1} ×${mod.wp.p2 === -1 ? '∞' : mod.wp.p2}`);
          break;
        case WpAction.SetHead:
          lines.push(`⮐ HDG ${mod.wp.p1 === -1 ? 'Free' : mod.wp.p1 + '°'}`);
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

  // ── Custom number input HTML helper ──────────────────────────────

  function numInputHtml(field: string, value: number, step: number, min?: number, max?: number, modIdx?: number): string {
    const dataAttrs = modIdx !== undefined ? `data-field="${field}" data-mod-idx="${modIdx}"` : `data-field="${field}"`;
    const minAttr = min !== undefined ? `min="${min}"` : '';
    const maxAttr = max !== undefined ? `max="${max}"` : '';
    return `<div class="wpe-num-ctrl"><button class="wpe-num-btn" data-numdir="-1" ${dataAttrs}>−</button><input type="number" ${dataAttrs} value="${value}" step="${step}" ${minAttr} ${maxAttr}/><button class="wpe-num-btn" data-numdir="1" ${dataAttrs}>+</button></div>`;
  }

  /** Attach +/- button events for a custom number input group */
  function attachNumBtnEvents(el: HTMLElement) {
    el.querySelectorAll('.wpe-num-btn').forEach(btn => {
      btn.addEventListener('click', (e) => {
        e.preventDefault();
        e.stopPropagation();
        const b = btn as HTMLElement;
        const dir = Number(b.dataset.numdir);
        const field = b.dataset.field!;
        const modIdx = b.dataset.modIdx;
        // Find the sibling input with matching data attributes
        const parent = b.closest('.wpe-num-ctrl');
        if (!parent) return;
        const input = parent.querySelector('input') as HTMLInputElement;
        if (!input) return;
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

  // ── Floating editor popup ────────────────────────────────────────

  function buildEditorHtml(wp: Waypoint, idx: number, total: number, displayNum: number,
    modifiers: { wp: Waypoint; idx: number }[]): string {
    const altM = (wp.altitude / 100).toFixed(0);
    const altType = (wp.p3 & 1) ? 'AMSL' : 'REL';

    // Only geo-WP types in the type selector (modifiers managed separately)
    const geoTypes: WpAction[] = [
      WpAction.Waypoint, WpAction.PosholdUnlim, WpAction.PosholdTime,
      WpAction.SetPoi, WpAction.Land,
    ];
    const typeOptions = geoTypes
      .map(v => `<option value="${v}" ${v === wp.action ? 'selected' : ''}>${WP_ACTION_LABELS[v]}</option>`)
      .join('');

    let html = `<div class="wp-editor-popup">`;
    html += `<div class="wpe-header">WP ${displayNum} <span class="wpe-type-name">${WP_ACTION_LABELS[wp.action]}</span></div>`;
    html += `<div class="wpe-row"><label>Type</label><select data-field="action">${typeOptions}</select></div>`;
    html += `<div class="wpe-row"><label>Alt</label>${numInputHtml('altitude', Number(altM), 1)}<button data-field="altToggle" class="wpe-toggle">${altType}</button></div>`;

    if (wp.action === WpAction.Waypoint || wp.action === WpAction.Land) {
      html += `<div class="wpe-row"><label>Speed</label>${numInputHtml('p1', wp.p1, 10, 0)}<span class="wpe-unit">cm/s</span></div>`;
    }
    if (wp.action === WpAction.PosholdTime) {
      html += `<div class="wpe-row"><label>Hold</label>${numInputHtml('p1', wp.p1, 1, 0)}<span class="wpe-unit">sec</span></div>`;
    }

    // ── User Actions (P3 bits 1-4) ── only for geo-WP types
    if (wp.action === WpAction.Waypoint || wp.action === WpAction.PosholdUnlim ||
        wp.action === WpAction.PosholdTime || wp.action === WpAction.Land) {
      const ua1 = (wp.p3 >> 1) & 1;
      const ua2 = (wp.p3 >> 2) & 1;
      const ua3 = (wp.p3 >> 3) & 1;
      const ua4 = (wp.p3 >> 4) & 1;
      html += `<div class="wpe-row wpe-ua-row"><label>Actions</label>`;
      html += `<button data-field="ua" data-ua-bit="1" class="wpe-ua-btn ${ua1 ? 'active' : ''}">UA1</button>`;
      html += `<button data-field="ua" data-ua-bit="2" class="wpe-ua-btn ${ua2 ? 'active' : ''}">UA2</button>`;
      html += `<button data-field="ua" data-ua-bit="3" class="wpe-ua-btn ${ua3 ? 'active' : ''}">UA3</button>`;
      html += `<button data-field="ua" data-ua-bit="4" class="wpe-ua-btn ${ua4 ? 'active' : ''}">UA4</button>`;
      html += `</div>`;
    }

    // ── Modifier sections ──
    for (const mod of modifiers) {
      const mi = mod.idx;
      html += `<div class="wpe-mod-section">`;
      html += `<div class="wpe-mod-header">${WP_ACTION_LABELS[mod.wp.action]}`;
      html += `<button data-action="removeMod" data-mod-idx="${mi}" class="wpe-mod-remove" title="Remove modifier">✕</button></div>`;

      if (mod.wp.action === WpAction.Rth) {
        html += `<div class="wpe-row"><label>Land</label><button data-field="rthLand" data-mod-idx="${mi}" class="wpe-toggle">${mod.wp.p1 ? 'Yes' : 'No'}</button></div>`;
      }
      if (mod.wp.action === WpAction.Jump) {
        html += `<div class="wpe-row"><label>To WP</label>${numInputHtml('mod-p1', mod.wp.p1, 1, 1, undefined, mi)}</div>`;
        html += `<div class="wpe-row"><label>Repeat</label>${numInputHtml('mod-p2', mod.wp.p2, 1, -1, undefined, mi)}<span class="wpe-unit">${mod.wp.p2 === -1 ? '∞' : ''}</span></div>`;
      }
      if (mod.wp.action === WpAction.SetHead) {
        html += `<div class="wpe-row"><label>Heading</label>${numInputHtml('mod-p1', mod.wp.p1, 1, -1, 359, mi)}<span class="wpe-unit">${mod.wp.p1 === -1 ? 'Free' : '°'}</span></div>`;
      }
      html += `</div>`;
    }

    // Add modifier dropdown
    const atLimit = getTotalWpCount() >= MAX_WAYPOINTS;
    html += `<div class="wpe-add-mod"><select data-field="addModType" ${atLimit ? 'disabled' : ''}>`;
    html += `<option value="">${atLimit ? '⚠ Max 120 WPs reached' : '+ Add modifier…'}</option>`;
    if (!atLimit) {
      html += `<option value="${WpAction.SetHead}">Set Heading</option>`;
      html += `<option value="${WpAction.Jump}">Jump</option>`;
      html += `<option value="${WpAction.Rth}">RTH</option>`;
    }
    html += `</select></div>`;

    // Actions
    html += `<div class="wpe-actions">`;
    html += `<button data-action="moveUp" ${idx <= 0 ? 'disabled' : ''} title="Move up">▲</button>`;
    html += `<button data-action="moveDown" ${idx >= total - 1 ? 'disabled' : ''} title="Move down">▼</button>`;
    html += `<button data-action="remove" class="wpe-remove" title="Remove WP">✕</button>`;
    html += `</div>`;
    html += `</div>`;
    return html;
  }

  function attachEditorEvents(popup: L.Popup, wp: Waypoint, idx: number,
    modifiers: { wp: Waypoint; idx: number }[]) {
    const el = popup.getElement();
    if (!el) return;

    // ── Main WP fields ──
    const typeSelect = el.querySelector('select[data-field="action"]') as HTMLSelectElement | null;
    typeSelect?.addEventListener('change', () => {
      const newAction = Number(typeSelect.value) as WpAction;
      const updated = { ...wp, action: newAction };
      if (newAction === WpAction.PosholdTime) { updated.p1 = get(settings).defaultPhTimeSec; updated.p2 = 0; }
      else if (newAction === WpAction.PosholdUnlim) { updated.p1 = 0; updated.p2 = 0; }
      else { updated.p1 = 0; updated.p2 = 0; }
      missionUpdateWp(idx, updated);
    });

    const altInput = el.querySelector('input[data-field="altitude"]') as HTMLInputElement | null;
    altInput?.addEventListener('change', () => {
      missionUpdateWp(idx, { ...wp, altitude: altFromM(Number(altInput.value)) });
    });

    const altToggle = el.querySelector('button[data-field="altToggle"]') as HTMLButtonElement | null;
    altToggle?.addEventListener('click', () => {
      missionUpdateWp(idx, { ...wp, p3: (wp.p3 & 1) ? (wp.p3 & ~1) : (wp.p3 | 1) });
    });

    const p1Input = el.querySelector('input[data-field="p1"]') as HTMLInputElement | null;
    p1Input?.addEventListener('change', () => {
      missionUpdateWp(idx, { ...wp, p1: Number(p1Input.value) });
    });

    // User Action toggle buttons
    el.querySelectorAll('button[data-field="ua"]').forEach(btn => {
      btn.addEventListener('click', () => {
        const bit = Number((btn as HTMLElement).dataset.uaBit);
        const newP3 = wp.p3 ^ (1 << bit); // toggle the bit
        missionUpdateWp(idx, { ...wp, p3: newP3 });
      });
    });

    // +/- button events
    attachNumBtnEvents(el);

    // ── Modifier fields ──
    for (const mod of modifiers) {
      const mi = mod.idx;

      // RTH land toggle
      const rthBtn = el.querySelector(`button[data-field="rthLand"][data-mod-idx="${mi}"]`) as HTMLButtonElement | null;
      rthBtn?.addEventListener('click', () => {
        missionUpdateWp(mi, { ...mod.wp, p1: mod.wp.p1 ? 0 : 1 });
      });

      // p1 for modifier (jump target, heading)
      const modP1 = el.querySelector(`input[data-field="mod-p1"][data-mod-idx="${mi}"]`) as HTMLInputElement | null;
      modP1?.addEventListener('change', () => {
        missionUpdateWp(mi, { ...mod.wp, p1: Number(modP1.value) });
      });

      // p2 for modifier (jump repeat)
      const modP2 = el.querySelector(`input[data-field="mod-p2"][data-mod-idx="${mi}"]`) as HTMLInputElement | null;
      modP2?.addEventListener('change', () => {
        missionUpdateWp(mi, { ...mod.wp, p2: Number(modP2.value) });
      });

      // Remove modifier
      const rmBtn = el.querySelector(`button[data-action="removeMod"][data-mod-idx="${mi}"]`) as HTMLButtonElement | null;
      rmBtn?.addEventListener('click', () => {
        missionRemoveWp(mi);
      });
    }

    // Add modifier
    const addModSelect = el.querySelector('select[data-field="addModType"]') as HTMLSelectElement | null;
    addModSelect?.addEventListener('change', () => {
      const modAction = Number(addModSelect.value) as WpAction;
      if (!modAction) return;
      // Enforce 120 WP limit for modifiers too
      if (getTotalWpCount() >= MAX_WAYPOINTS) {
        addModSelect.value = '';
        return;
      }
      const insertIdx = modifiers.length > 0 ? modifiers[modifiers.length - 1].idx + 1 : idx + 1;
      let p1 = 0, p2 = 0;
      if (modAction === WpAction.SetHead) { p1 = -1; }
      else if (modAction === WpAction.Jump) { p1 = 1; p2 = 1; }
      missionInsertWp(insertIdx, modAction, 0, 0, 0, p1, p2);
    });

    // Move / remove WP
    el.querySelector('button[data-action="moveUp"]')?.addEventListener('click', () => {
      if (idx > 0) {
        missionReorderWp(idx, idx - 1);
        selectedWpIndex.set(idx - 1);
      }
    });
    el.querySelector('button[data-action="moveDown"]')?.addEventListener('click', () => {
      if (idx < currentMission.waypoints.length - 1) {
        missionReorderWp(idx, idx + 1);
        selectedWpIndex.set(idx + 1);
      }
    });
    el.querySelector('button[data-action="remove"]')?.addEventListener('click', () => {
      // Also remove attached modifiers (in reverse to keep indices stable)
      for (let k = modifiers.length - 1; k >= 0; k--) {
        missionRemoveWp(modifiers[k].idx);
      }
      missionRemoveWp(idx);
      selectedWpIndex.set(-1);
    });

    // Stop map click propagation
    el.querySelectorAll('input, select, button').forEach(input => {
      L.DomEvent.disableClickPropagation(input as HTMLElement);
    });
  }

  // ── Render Logic ─────────────────────────────────────────────────

  function renderMission(m: Mission, selIdx: number, editing: boolean) {
    // Preserve existing popup if it's for the same WP (avoids flicker on value edits)
    const keepPopup = editing && editorPopup && editorPopupIdx === selIdx && selIdx >= 0;

    missionGroup.clearLayers();
    wpMarkers = [];
    modifierLines = [];
    paramLabels = [];

    if (!keepPopup) {
      if (editorPopup) {
        map.removeLayer(editorPopup);
      }
      editorPopup = undefined;
      editorPopupIdx = -1;
    }

    if (m.waypoints.length === 0) return;

    const displayNums = buildDisplayNumbers(m.waypoints);
    const missionEndIdx = findMissionEndIndex(m.waypoints);

    // Flight path positions (excludes POI and modifiers)
    const fpPositions: L.LatLng[] = [];
    const fpIndices: number[] = [];
    // Track which fp positions are after mission end
    const fpGreyed: boolean[] = [];

    for (let i = 0; i < m.waypoints.length; i++) {
      const wp = m.waypoints[i];
      const selected = i === selIdx;
      const dn = displayNums.get(i) ?? 0;
      const greyed = missionEndIdx >= 0 && i > missionEndIdx;

      if (hasLocation(wp.action)) {
        const latLng = L.latLng(toDeg(wp.lat), toDeg(wp.lon));

        // Collect to flight path (POI excluded)
        if (isFlightPathWp(wp.action)) {
          fpPositions.push(latLng);
          fpIndices.push(i);
          fpGreyed.push(greyed);
        }

        const icon = iconForWp(wp, dn, selected);
        const marker = L.marker(latLng, {
          icon,
          draggable: editing && !greyed,
          opacity: greyed ? 0.35 : 1.0,
          title: `WP${dn}: ${WP_ACTION_LABELS[wp.action] || 'Unknown'}`,
        }).addTo(missionGroup);

        marker.on('click', () => {
          selectedWpIndex.set(i);
        });

        if (editing) {
          marker.on('dragend', () => {
            const pos = marker.getLatLng();
            missionUpdateWp(i, { ...wp, lat: fromDeg(pos.lat), lon: fromDeg(pos.lng) });
          });
        }

        // Collect modifiers attached to this WP
        const modifiers = getModifiersForWp(m.waypoints, i);

        // Edit mode: floating param label for non-selected WPs
        if (editing && !selected && !greyed) {
          const label = createParamLabel(wp, modifiers, latLng);
          label.addTo(missionGroup);
          paramLabels.push(label);
        }

        // Edit mode: floating editor popup for selected WP
        if (editing && selected && !greyed) {
          const htmlContent = buildEditorHtml(wp, i, m.waypoints.length, dn, modifiers);

          if (keepPopup && editorPopup) {
            // Direct DOM update — avoids Leaflet's setContent which triggers layout recalc + flicker
            editorPopup.setLatLng(latLng);
            const contentEl = editorPopup.getElement()?.querySelector('.leaflet-popup-content');
            if (contentEl) {
              contentEl.innerHTML = htmlContent;
            }
          } else {
            if (editorPopup) {
              map.removeLayer(editorPopup);
            }
            editorPopup = L.popup({
              closeButton: false,
              autoClose: false,
              closeOnClick: false,
              className: 'wp-editor-popup-container',
              offset: L.point(0, -30),
              maxWidth: 240,
              minWidth: 190,
            })
              .setLatLng(latLng)
              .setContent(htmlContent)
              .addTo(map);
          }

          editorPopupIdx = i;

          setTimeout(() => {
            if (editorPopup) attachEditorEvents(editorPopup, wp, i, modifiers);
          }, 50);
        }

        // Non-edit: simple hover tooltip
        if (!editing) {
          const altM = (wp.altitude / 100).toFixed(1);
          const altType = (wp.p3 & 1) ? 'AMSL' : 'REL';
          const modifiers = getModifiersForWp(m.waypoints, i);
          let tip = `WP${dn} ${WP_ACTION_LABELS[wp.action]}<br>${altM}m ${altType}`;
          for (const mod of modifiers) {
            tip += `<br>${WP_ACTION_LABELS[mod.wp.action]}`;
          }
          marker.bindTooltip(tip, { direction: 'top', offset: L.point(0, -20) });
        }

        wpMarkers.push(marker);
      }

      // ── Modifier dashed lines ──
      if (wp.action === WpAction.Jump && wp.p1 > 0) {
        const targetIdx = wp.p1 - 1;
        const sourceWp = findPreviousGeoWp(m.waypoints, i);
        const targetWp = m.waypoints[targetIdx];
        if (sourceWp && targetWp && hasLocation(targetWp.action)) {
          const line = L.polyline(
            [
              L.latLng(toDeg(sourceWp.lat), toDeg(sourceWp.lon)),
              L.latLng(toDeg(targetWp.lat), toDeg(targetWp.lon)),
            ],
            { color: '#8e44ad', weight: 2, dashArray: '8 4', opacity: 0.8 }
          ).addTo(missionGroup);
          const jLabel = wp.p2 === -1 ? '∞' : `×${wp.p2}`;
          line.bindTooltip(`Jump → WP${wp.p1} ${jLabel}`, { sticky: true });
          modifierLines.push(line);
        }
      }

      if (wp.action === WpAction.Rth) {
        const sourceWp = findPreviousGeoWp(m.waypoints, i);
        if (sourceWp && fpPositions.length > 0) {
          const line = L.polyline(
            [
              L.latLng(toDeg(sourceWp.lat), toDeg(sourceWp.lon)),
              fpPositions[0],
            ],
            { color: '#e67e22', weight: 2, dashArray: '8 4', opacity: 0.7 }
          ).addTo(missionGroup);
          line.bindTooltip('RTH', { sticky: true });
          modifierLines.push(line);
        }
      }
    }

    // Flight path polyline (excludes POI)
    if (fpPositions.length > 1) {
      // Split into active and greyed segments
      const activePositions: L.LatLng[] = [];
      const greyedPositions: L.LatLng[] = [];
      for (let s = 0; s < fpPositions.length; s++) {
        if (!fpGreyed[s]) {
          activePositions.push(fpPositions[s]);
        } else {
          // Include the last active point as start of grey segment for continuity
          if (greyedPositions.length === 0 && activePositions.length > 0) {
            greyedPositions.push(activePositions[activePositions.length - 1]);
          }
          greyedPositions.push(fpPositions[s]);
        }
      }

      if (activePositions.length > 1) {
        flightPath = L.polyline(activePositions, {
          color: '#37a8db',
          weight: editing ? 6 : 3,
          opacity: 0.7,
        }).addTo(missionGroup);

        if (editing) {
          flightPath.on('click', (e: L.LeafletMouseEvent) => {
            L.DomEvent.stopPropagation(e);
            const clickLatLng = e.latlng;
            let bestInsertIdx = fpIndices.length;
            let bestDist = Infinity;
            for (let s = 0; s < fpPositions.length - 1; s++) {
              if (fpGreyed[s] || fpGreyed[s + 1]) continue;
              const mid = L.latLng(
                (fpPositions[s].lat + fpPositions[s + 1].lat) / 2,
                (fpPositions[s].lng + fpPositions[s + 1].lng) / 2,
              );
              const d = clickLatLng.distanceTo(mid);
              if (d < bestDist) {
                bestDist = d;
                bestInsertIdx = fpIndices[s + 1];
              }
            }
            const lat = fromDeg(clickLatLng.lat);
            const lon = fromDeg(clickLatLng.lng);
            const altitude = altFromM(get(settings).defaultWpAltitudeM);
            if (getTotalWpCount() < MAX_WAYPOINTS) {
              missionInsertWp(bestInsertIdx, WpAction.Waypoint, lat, lon, altitude);
            }
          });
        }
      }

      // Greyed portion of flight path
      if (greyedPositions.length > 1) {
        L.polyline(greyedPositions, {
          color: '#666',
          weight: editing ? 4 : 2,
          opacity: 0.4,
          dashArray: '6 4',
        }).addTo(missionGroup);
      }
    }
  }

  function findPreviousGeoWp(waypoints: Waypoint[], fromIndex: number): Waypoint | null {
    for (let i = fromIndex - 1; i >= 0; i--) {
      if (hasLocation(waypoints[i].action)) return waypoints[i];
    }
    return null;
  }

  // ── Map click handler ────────────────────────────────────────────
  function onMapClick(e: L.LeafletMouseEvent) {
    if (!currentEditing) return;
    // If a WP is selected (editor popup open), just deselect
    if (currentSelIdx >= 0) {
      selectedWpIndex.set(-1);
      return;
    }
    // Max 120 WPs (INAV limit across all missions)
    if (getTotalWpCount() >= MAX_WAYPOINTS) return;
    const lat = fromDeg(e.latlng.lat);
    const lon = fromDeg(e.latlng.lng);
    const altitude = altFromM(get(settings).defaultWpAltitudeM);
    missionAddWp(WpAction.Waypoint, lat, lon, altitude);
  }

  map.on('click', onMapClick);

  // ── Reactive re-render ───────────────────────────────────────────
  $effect(() => {
    renderMission(currentMission, currentSelIdx, currentEditing);
  });

  // ── Cleanup ──────────────────────────────────────────────────────
  onDestroy(() => {
    unsubMission();
    unsubSelIdx();
    unsubEditMode();
    map.off('click', onMapClick);
    if (editorPopup) {
      map.removeLayer(editorPopup);
      editorPopup = undefined;
    }
    missionGroup.clearLayers();
    map.removeLayer(missionGroup);
  });
</script>

<style>
  :global(.mission-wp-icon) {
    background: none !important;
    border: none !important;
  }

  /* Floating param label wrapper — must overflow to show content */
  :global(.wp-param-label-wrapper) {
    background: none !important;
    border: none !important;
    overflow: visible !important;
    width: auto !important;
    height: auto !important;
  }
  :global(.wp-param-label) {
    background: rgba(30, 30, 30, 0.88);
    color: #ccc;
    padding: 3px 8px;
    border-radius: 4px;
    font-size: 12px;
    line-height: 1.4;
    white-space: nowrap;
    border: 1px solid rgba(55, 168, 219, 0.35);
    pointer-events: none;
  }

  /* Floating editor popup */
  :global(.wp-editor-popup-container .leaflet-popup-content-wrapper) {
    background: rgba(30, 30, 30, 0.95);
    color: #ccc;
    border: 1px solid rgba(55, 168, 219, 0.5);
    border-radius: 6px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.5);
    padding: 0;
  }
  :global(.wp-editor-popup-container .leaflet-popup-content) {
    margin: 0;
    width: auto !important;
  }
  :global(.wp-editor-popup-container .leaflet-popup-tip) {
    background: rgba(30, 30, 30, 0.95);
    border: 1px solid rgba(55, 168, 219, 0.5);
  }
  :global(.wp-editor-popup) {
    padding: 10px;
    font-size: 13px;
    min-width: 190px;
  }
  :global(.wpe-header) {
    font-weight: bold;
    font-size: 14px;
    color: #37a8db;
    margin-bottom: 6px;
    border-bottom: 1px solid #444;
    padding-bottom: 4px;
  }
  :global(.wpe-type-name) {
    color: #888;
    font-weight: normal;
    font-size: 12px;
    margin-left: 4px;
  }
  :global(.wpe-row) {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 5px;
  }
  :global(.wpe-row label) {
    width: 52px;
    color: #888;
    font-size: 12px;
    flex-shrink: 0;
  }
  :global(.wpe-row input) {
    background: #2a2a2a;
    color: #ccc;
    border: 1px solid #555;
    border-radius: 0;
    padding: 3px 4px;
    font-size: 13px;
    width: 52px;
    text-align: center;
    -moz-appearance: textfield;
  }
  :global(.wpe-row input::-webkit-inner-spin-button),
  :global(.wpe-row input::-webkit-outer-spin-button) {
    -webkit-appearance: none;
    margin: 0;
  }
  :global(.wpe-row input:focus) {
    border-color: #37a8db;
    outline: none;
  }
  /* Custom +/- number control */
  :global(.wpe-num-ctrl) {
    display: flex;
    align-items: stretch;
    border-radius: 4px;
    overflow: hidden;
    border: 1px solid #555;
  }
  :global(.wpe-num-ctrl input) {
    border: none;
    border-left: 1px solid #555;
    border-right: 1px solid #555;
    border-radius: 0;
  }
  :global(.wpe-num-btn) {
    background: #333;
    color: #aaa;
    border: none;
    width: 24px;
    cursor: pointer;
    font-size: 14px;
    font-weight: bold;
    line-height: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    user-select: none;
  }
  :global(.wpe-num-btn:hover) {
    background: #37a8db;
    color: #fff;
  }
  :global(.wpe-num-btn:active) {
    background: #2980b9;
  }
  :global(.wpe-row select) {
    background: #2a2a2a;
    color: #ccc;
    border: 1px solid #555;
    border-radius: 3px;
    padding: 3px 4px;
    font-size: 13px;
    flex: 1;
  }
  :global(.wpe-toggle) {
    background: #2a2a2a;
    color: #ccc;
    border: 1px solid #555;
    border-radius: 3px;
    padding: 3px 8px;
    font-size: 12px;
    cursor: pointer;
  }
  :global(.wpe-toggle:hover) {
    background: #3a3a3a;
  }
  :global(.wpe-unit) {
    color: #888;
    font-size: 12px;
    white-space: nowrap;
  }

  /* ── User Action toggle buttons ── */
  :global(.wpe-ua-row) {
    gap: 4px;
  }
  :global(.wpe-ua-btn) {
    padding: 2px 5px;
    border: 1px solid #555;
    border-radius: 3px;
    background: #2a2a2a;
    color: #777;
    cursor: pointer;
    font-size: 10px;
    font-weight: 600;
    transition: all 0.15s;
  }
  :global(.wpe-ua-btn.active) {
    background: #37a8db;
    color: #fff;
    border-color: #37a8db;
  }
  :global(.wpe-ua-btn:hover:not(.active)) {
    background: #3a3a3a;
    color: #ccc;
  }

  /* ── Modifier sections inside editor ── */
  :global(.wpe-mod-section) {
    margin-top: 6px;
    padding-top: 5px;
    border-top: 1px dashed #555;
  }
  :global(.wpe-mod-header) {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: 12px;
    font-weight: 600;
    color: #e67e22;
    margin-bottom: 4px;
  }
  :global(.wpe-mod-remove) {
    background: none;
    border: none;
    color: #c0392b;
    cursor: pointer;
    font-size: 12px;
    padding: 0 4px;
    line-height: 1;
  }
  :global(.wpe-mod-remove:hover) {
    color: #e74c3c;
  }

  /* Add modifier dropdown */
  :global(.wpe-add-mod) {
    margin-top: 4px;
  }
  :global(.wpe-add-mod select) {
    width: 100%;
    background: #2a2a2a;
    color: #888;
    border: 1px dashed #555;
    border-radius: 3px;
    padding: 3px 4px;
    font-size: 12px;
    cursor: pointer;
  }

  :global(.wpe-actions) {
    display: flex;
    gap: 4px;
    margin-top: 6px;
    padding-top: 4px;
    border-top: 1px solid #444;
  }
  :global(.wpe-actions button) {
    padding: 3px 10px;
    border: 1px solid #555;
    border-radius: 3px;
    background: #2a2a2a;
    color: #ccc;
    cursor: pointer;
    font-size: 13px;
  }
  :global(.wpe-actions button:hover:not(:disabled)) {
    background: #3a3a3a;
  }
  :global(.wpe-actions button:disabled) {
    opacity: 0.3;
    cursor: not-allowed;
  }
  :global(.wpe-remove) {
    margin-left: auto;
    border-color: #c0392b !important;
    color: #e74c3c !important;
  }
  :global(.wpe-remove:hover) {
    background: #c0392b !important;
    color: #fff !important;
  }

  /* Dark-themed number input spinners */
  :global(.wp-editor-popup select) {
    color-scheme: dark;
  }
</style>
