<!-- SurveyPatternLayer.svelte
     Renders the live preview of the survey pattern shape on the map.
     Only active while the user is in Pattern mode.
-->
<script lang="ts">
  import { onDestroy } from 'svelte';
  import L from 'leaflet';
  import { activeSurveyPattern, applyRectangleDragUpdate, type LngLat } from '$lib/stores/surveyPattern.svelte';
  import {
    computeRectangleCorners,
    generateRectangleZigzag,
    generateRectangleLawnmower,
    updateRectangleFromDraggedCenter,
    updateRectangleFromDraggedCorner,
    type SurveyPathSegment,
  } from '$lib/helpers/surveyPatterns';

  interface Props {
    map: L.Map;
  }
  let { map }: Props = $props();

  // ── Layers ──────────────────────────────────────────
  let shapeLayer: L.Polygon | undefined;
  let pathLines: L.Polyline[] = [];             // the continuous preview path lines
  let pathMarkers: L.Marker[] = [];             // waypoint dot markers (start/end/wp)

  let hasAutoFitted = false;

  // ── Editing markers ────────────────────────────────
  let centerMarker: L.Marker | undefined;
  let cornerMarkers: L.Marker[] = [];
  let isDragging = false;

  // Temp values for drag preview (avoids store mutation during drag)
  let _dragTemp: { center: LngLat | null; length: number | null; width: number | null } = {
    center: null, length: null, width: null,
  };

  // ══════════════════════════════════════════════════════
  //  MARKER MANAGEMENT
  // ══════════════════════════════════════════════════════

  function removeEditingMarkers() {
    if (isDragging) return;
    if (centerMarker) { try { map.removeLayer(centerMarker); } catch {} centerMarker = undefined; }
    cornerMarkers.forEach(m => { try { map.removeLayer(m); } catch {} });
    cornerMarkers = [];
  }

  function buildEditingMarkers(corners: LngLat[], center: LngLat) {
    if (cornerMarkers.length === 0) {
      const cLL: L.LatLngExpression = [center.lat, center.lng];

      centerMarker = L.marker(cLL, {
        draggable: true,
        icon: L.divIcon({
          className: 'pattern-center-marker',
          html: '<div style="background:#37a8db;width:12px;height:12px;border-radius:50%;border:2px solid white;box-shadow:0 0 4px rgba(0,0,0,0.5);"></div>',
          iconSize: [16, 16], iconAnchor: [8, 8],
        }),
      }).addTo(map);

      centerMarker.on('dragstart', () => { isDragging = true; });
      centerMarker.on('drag', () => {
        const pos = centerMarker!.getLatLng();
        const baseParams = activeSurveyPattern.config!.params as any;
        const u = updateRectangleFromDraggedCenter(baseParams, { lat: pos.lat, lng: pos.lng });
        _dragTemp.center = u.center!;
        syncDragPreview();
      });
      centerMarker.on('dragend', () => {
        isDragging = false;
        const pos = centerMarker!.getLatLng();
        applyRectangleDragUpdate(
          updateRectangleFromDraggedCenter(activeSurveyPattern.config!.params as any, { lat: pos.lat, lng: pos.lng })
        );
        _dragTemp.center = null;
      });

      for (let i = 0; i < 4; i++) {
        const m = L.marker([corners[i].lat, corners[i].lng], {
          draggable: true,
          icon: L.divIcon({
            className: 'pattern-corner-marker',
            html: '<div style="background:#f1c40f;width:10px;height:10px;border:2px solid white;box-shadow:0 0 3px rgba(0,0,0,0.6);"></div>',
            iconSize: [14, 14], iconAnchor: [7, 7],
          }),
        }).addTo(map);

        const idx = i as 0 | 1 | 2 | 3;
        m.on('dragstart', () => { isDragging = true; });
        m.on('drag', () => {
          const pos = m.getLatLng();
          const baseParams = activeSurveyPattern.config!.params as any;
          const u = updateRectangleFromDraggedCorner(baseParams, idx, { lat: pos.lat, lng: pos.lng });
          if (u.center) _dragTemp.center = u.center;
          if (u.length) _dragTemp.length = u.length;
          if (u.width) _dragTemp.width = u.width;
          syncDragPreview();
        });
        m.on('dragend', () => {
          isDragging = false;
          const pos = m.getLatLng();
          applyRectangleDragUpdate(
            updateRectangleFromDraggedCorner(activeSurveyPattern.config!.params as any, idx, { lat: pos.lat, lng: pos.lng })
          );
          _dragTemp.center = null;
          _dragTemp.length = null;
          _dragTemp.width = null;
        });
        cornerMarkers.push(m);
      }
    } else {
      centerMarker!.setLatLng([center.lat, center.lng]);
      corners.forEach((c, i) => cornerMarkers[i]?.setLatLng([c.lat, c.lng]));
    }
  }

  // ══════════════════════════════════════════════════════
  //  PATH VISUALISATION
  // ══════════════════════════════════════════════════════

  function clearPath() {
    pathLines.forEach(l => { try { map.removeLayer(l); } catch {} });
    pathLines = [];
    pathMarkers.forEach(m => { try { map.removeLayer(m); } catch {} });
    pathMarkers = [];
  }

  function clearShapeLines() {
    if (shapeLayer) { try { map.removeLayer(shapeLayer); } catch {} shapeLayer = undefined; }
    clearPath();
  }

  function clearAll() {
    clearShapeLines();
    if (!isDragging) removeEditingMarkers();
  }

  /** Build the continuous preview path from SurveyPathSegment[] */
  function buildPreviewPath(segments: SurveyPathSegment[]) {
    clearPath();
    if (segments.length === 0) return;

    // Collect all unique waypoints along the path
    const allPoints: { lat: number; lng: number }[] = [];
    for (const seg of segments) {
      for (const wp of seg.points) {
        allPoints.push({ lat: wp.lat, lng: wp.lng });
      }
    }
    if (allPoints.length < 2) return;

    // Draw one polyline per segment with kind-based coloring
    for (const seg of segments) {
      if (seg.points.length < 2) continue;
      const ll: [number, number][] = seg.points.map(p => [p.lat, p.lng]);
      const color = seg.kind === 'survey' ? '#37a8db' : '#f39c12';
      const line = L.polyline(ll, {
        color, weight: seg.kind === 'survey' ? 3 : 2,
        opacity: seg.kind === 'survey' ? 0.9 : 0.6,
      }).addTo(map);
      pathLines.push(line);
    }

    // --- Waypoint dot markers (skip consecutive duplicates) ---
    const uniqPoints: { lat: number; lng: number }[] = [];
    for (const p of allPoints) {
      if (uniqPoints.length === 0) {
        uniqPoints.push(p);
      } else {
        const last = uniqPoints[uniqPoints.length - 1];
        const dLat = Math.abs(p.lat - last.lat);
        const dLng = Math.abs(p.lng - last.lng);
        if (dLat > 1e-8 || dLng > 1e-8) uniqPoints.push(p);
      }
    }

    for (let i = 0; i < uniqPoints.length; i++) {
      const p = uniqPoints[i];
      let html: string;
      let size: [number, number];
      let anchor: [number, number];

      if (i === 0) {
        // Start marker — green circle with "S"
        html = '<div style="background:#2ecc71;width:14px;height:14px;border-radius:50%;border:3px solid white;box-shadow:0 0 5px rgba(0,0,0,0.6);display:flex;align-items:center;justify-content:center;font-size:10px;color:white;font-weight:bold;">S</div>';
        size = [20, 20]; anchor = [10, 10];
      } else if (i === uniqPoints.length - 1) {
        // End marker — checkerboard flag
        html = '<div style="background:repeating-conic-gradient(#000 0% 25%, #fff 0% 50%) 50% / 10px 10px;width:16px;height:16px;border:2px solid white;border-radius:2px;box-shadow:0 0 4px rgba(0,0,0,0.6);"></div>';
        size = [20, 20]; anchor = [10, 10];
      } else {
        // Waypoint dot — small filled circle
        html = '<div style="background:#888;width:6px;height:6px;border-radius:50%;border:1px solid white;"></div>';
        size = [10, 10]; anchor = [5, 5];
      }

      const marker = L.marker([p.lat, p.lng], {
        icon: L.divIcon({ className: 'path-wp-marker', html, iconSize: size, iconAnchor: anchor }),
        interactive: false,
      }).addTo(map);
      pathMarkers.push(marker);
    }
  }

  // ══════════════════════════════════════════════════════
  //  PREVIEW RENDERING
  // ══════════════════════════════════════════════════════

  function updatePreview() {
    const config = activeSurveyPattern.config;
    if (!activeSurveyPattern.isActive || !config) { clearAll(); return; }

    const shape = config.shape;
    const p: any = config.params;

    if (shape !== 'rectangle' && shape !== 'rectangle-lawnmower') {
      if (!isDragging) removeEditingMarkers();
    }

    // Snap dummy center
    const isDummy = Math.abs(p.center?.lat - 48) < 0.5 && Math.abs(p.center?.lng - 11) < 0.5;
    if (isDummy || !p.center) {
      const mc = map.getCenter();
      p.center = { lat: mc.lat, lng: mc.lng };
    }

    let shapeLatLngs: [number, number][] = [];
    const centerLL: [number, number] = [p.center.lat, p.center.lng];

    if (shape === 'rectangle' || shape === 'rectangle-lawnmower') {
      const { corners } = computeRectangleCorners(
        p.center, p.length ?? 400, p.width ?? 200, p.shapeOrientation ?? 0
    );
    shapeLatLngs = corners.map(c => [c.lat, c.lng] as [number, number]);

      buildEditingMarkers(corners, p.center);

      // Gray shape
      if (shapeLayer) { shapeLayer.setLatLngs(shapeLatLngs); }
      else {
        shapeLayer = L.polygon(shapeLatLngs, {
          color: '#888888', weight: 2, fillColor: '#666666', fillOpacity: 0.25, dashArray: '4 2'
        }).addTo(map);
      }

      // Generate and show preview path (only when not dragging to avoid jank)
      if (!isDragging) {
        const segments = (shape === 'rectangle-lawnmower')
          ? generateRectangleLawnmower(p)
          : generateRectangleZigzag(p);
        buildPreviewPath(segments);
      }

    } else if (shape === 'circle' || shape === 'spiral') {
      clearShapeLines();
      const radius = p.radius ?? 200;
      shapeLayer = L.circle(centerLL, {
        color: '#888888', weight: 2, fillColor: '#666666', fillOpacity: 0.25, radius
      }).addTo(map) as any;
      // TODO: circle preview path

    } else {
      clearShapeLines();
      const size = 0.002;
      const pts: [number, number][] = [];
      for (let i = 0; i < 5; i++) {
        const a = (i / 5) * Math.PI * 2 - Math.PI / 2;
        pts.push([p.center.lat + size * Math.cos(a), p.center.lng + size * Math.sin(a) * 0.6]);
      }
      shapeLatLngs = pts;
      shapeLayer = L.polygon(shapeLatLngs, {
        color: '#888888', weight: 2, fillColor: '#666666', fillOpacity: 0.25, dashArray: '4 2'
      }).addTo(map);
    }

    if (!hasAutoFitted) {
      hasAutoFitted = true;
      if (shapeLatLngs.length >= 3) {
        try { map.fitBounds(L.latLngBounds(shapeLatLngs), { padding: [80, 80], maxZoom: 18 }); } catch {}
      } else { map.panTo(centerLL); if (map.getZoom() < 15) map.setZoom(15); }
    }
  }

  /** Sync drag without reactivity — only marker + polygon positions, and path during drag */
  function syncDragPreview() {
    const config = activeSurveyPattern.config;
    if (!activeSurveyPattern.isActive || !config) return;
    const p: any = config.params;
    if (!p.center) return;

    if (config.shape === 'rectangle' || config.shape === 'rectangle-lawnmower') {
      // Use temp values if available (during drag), else store values
      const center = _dragTemp.center || p.center;
      const length = _dragTemp.length ?? p.length ?? 400;
      const width = _dragTemp.width ?? p.width ?? 200;

      const { corners } = computeRectangleCorners(
        center, length, width, p.shapeOrientation ?? 0
      );
      const shapeLL = corners.map(c => [c.lat, c.lng] as [number, number]);
      if (centerMarker) centerMarker.setLatLng([center.lat, center.lng]);
      corners.forEach((c, i) => cornerMarkers[i]?.setLatLng([c.lat, c.lng]));
      if (shapeLayer) shapeLayer.setLatLngs(shapeLL);

      // Rebuild path during drag
      const dragParams = { ...p, center: _dragTemp.center || p.center, length: _dragTemp.length ?? p.length ?? 400, width: _dragTemp.width ?? p.width ?? 200 };
      const segments = (config.shape === 'rectangle-lawnmower')
        ? generateRectangleLawnmower(dragParams)
        : generateRectangleZigzag(dragParams);
      buildPreviewPath(segments);
    }
  }

  // ══════════════════════════════════════════════════════
  //  REACTIVITY
  // ══════════════════════════════════════════════════════

  let prevShape = $state<string | undefined>();

  // Effect 1: shape changes → full render
  $effect(() => {
    const isActive = activeSurveyPattern.isActive;
    const shape = activeSurveyPattern.config?.shape;

    if (!isActive || !activeSurveyPattern.config) {
      clearAll(); hasAutoFitted = false; prevShape = undefined;
      return;
    }

    if (prevShape !== shape) {
      prevShape = shape;
      updatePreview();
    }
  });

  // Effect 2: param changes from sidebar → rebuild path (but not during drag)
  $effect(() => {
    const isActive = activeSurveyPattern.isActive;
    const config = activeSurveyPattern.config;
    if (!isActive || !config) return;
    if (prevShape !== config.shape) return; // handled by effect 1

    // Subscribe to params by reading them (triggers reactivity on any param change)
    const p = config.params as any;
    const _trigger = p.length + p.width + p.shapeOrientation + p.targetLineSpacing + p.turnDistance + p.reverse + p.clockwise + p.startCorner + p.trackOrientationEnabled + p.trackOrientation + p.userActionStartFlags + p.userActionTrackFlags + p.userActionEndFlags + p.userActionLineStartFlags + p.userActionLineEndFlags;

    // Compute actualLineSpacing
    if ((config.shape === 'rectangle' || config.shape === 'rectangle-lawnmower') && p.targetLineSpacing > 0 && p.width > 0) {
      const numTracks = Math.max(1, Math.ceil(p.width / p.targetLineSpacing));
      const actualSpacing = p.width / Math.max(1, numTracks - 1);
      if (Math.abs(p.actualLineSpacing - actualSpacing) > 0.01) {
        p.actualLineSpacing = actualSpacing;
      }
    }

    // Rebuild path (always � updatePreview handles drag vs non-drag internally)
    updatePreview();
  });

  onDestroy(() => { clearAll(); });
</script>
