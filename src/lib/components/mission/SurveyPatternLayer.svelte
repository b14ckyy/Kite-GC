<!-- SurveyPatternLayer.svelte
     Renders the live preview of the survey pattern shape on the map.
     Only active while the user is in Pattern mode.
-->
<script lang="ts">
  import { onDestroy } from 'svelte';
  import L from 'leaflet';
  import { activeSurveyPattern, applyRectangleDragUpdate, applyCircleDragUpdate, type LngLat, type CirclePatternParams } from '$lib/stores/surveyPattern.svelte';
  import {
    computeRectangleCorners,
    generateRectangleZigzag,
    generateRectangleLawnmower,
    generateCircleStepped,
    generateSpiral,
    updateRectangleFromDraggedCenter,
    updateRectangleFromDraggedCorner,
    type SurveyPathSegment,
  } from '$lib/helpers/surveyPatterns';

  interface Props {
    map: L.Map;
  }
  let { map }: Props = $props();

  // ── Layers ──────────────────────────────────────────
  let shapeLayer: L.Polygon | L.Circle | undefined;
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

  // ── Circle editing markers ─────────────────────────
  let centerMarkerCircle: L.Marker | undefined;
  let radiusMarker: L.Marker | undefined;

  // Temp values for circle drag preview
  let _circleDragTemp: { center: LngLat | null; radius: number | null } = {
    center: null, radius: null,
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

  function removeCircleEditingMarkers() {
    if (isDragging) return;
    if (centerMarkerCircle) { try { map.removeLayer(centerMarkerCircle); } catch {} centerMarkerCircle = undefined; }
    if (radiusMarker) { try { map.removeLayer(radiusMarker); } catch {} radiusMarker = undefined; }
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

  function buildCircleEditingMarkers(center: LngLat, radius: number) {
    const centerLL: L.LatLngExpression = [center.lat, center.lng];
    // Place radius handle due North of center
    const radiusLL: L.LatLngExpression = [center.lat + radius / 111320, center.lng];

    if (!centerMarkerCircle) {
      centerMarkerCircle = L.marker(centerLL, {
        draggable: true,
        icon: L.divIcon({
          className: 'pattern-center-marker',
          html: '<div style="background:#37a8db;width:12px;height:12px;border-radius:50%;border:2px solid white;box-shadow:0 0 4px rgba(0,0,0,0.5);"></div>',
          iconSize: [16, 16], iconAnchor: [8, 8],
        }),
      }).addTo(map);

      centerMarkerCircle.on('dragstart', () => { isDragging = true; });
      centerMarkerCircle.on('drag', () => {
        const pos = centerMarkerCircle!.getLatLng();
        _circleDragTemp.center = { lat: pos.lat, lng: pos.lng };
        syncDragPreview();
      });
      centerMarkerCircle.on('dragend', () => {
        isDragging = false;
        const pos = centerMarkerCircle!.getLatLng();
        applyCircleDragUpdate({ center: { lat: pos.lat, lng: pos.lng } });
        _circleDragTemp.center = null;
      });
    } else {
      centerMarkerCircle.setLatLng(centerLL);
    }

    if (!radiusMarker) {
      radiusMarker = L.marker(radiusLL, {
        draggable: true,
        icon: L.divIcon({
          className: 'pattern-radius-marker',
          html: '<div style="background:#e74c3c;width:10px;height:10px;border-radius:50%;border:2px solid white;box-shadow:0 0 3px rgba(0,0,0,0.6);"></div>',
          iconSize: [14, 14], iconAnchor: [7, 7],
        }),
      }).addTo(map);

      radiusMarker.on('dragstart', () => { isDragging = true; });
      radiusMarker.on('drag', () => {
        const cp = (activeSurveyPattern.config!.params as any).center;
        const pos = radiusMarker!.getLatLng();
        const dLat = (pos.lat - cp.lat) * 111320;
        const dLng = (pos.lng - cp.lng) * 111320 * Math.cos((cp.lat * Math.PI) / 180);
        _circleDragTemp.radius = Math.max(10, Math.round(Math.sqrt(dLat * dLat + dLng * dLng)));
        syncDragPreview();
      });
      radiusMarker.on('dragend', () => {
        isDragging = false;
        const cp = (activeSurveyPattern.config!.params as any).center;
        const pos = radiusMarker!.getLatLng();
        const dLat = (pos.lat - cp.lat) * 111320;
        const dLng = (pos.lng - cp.lng) * 111320 * Math.cos((cp.lat * Math.PI) / 180);
        const newRadius = Math.max(10, Math.round(Math.sqrt(dLat * dLat + dLng * dLng)));
        applyCircleDragUpdate({ radius: newRadius });
        _circleDragTemp.radius = null;
      });
    } else {
      radiusMarker.setLatLng(radiusLL);
    }
  }

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
    removeEditingMarkers();
    removeCircleEditingMarkers();
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

    // Snap dummy center (default 48,11) to current map center on first render
    const isDummy = Math.abs(p.center?.lat - 48) < 0.5 && Math.abs(p.center?.lng - 11) < 0.5;
    if (isDummy || !p.center) {
      const mc = map.getCenter();
      p.center = { lat: mc.lat, lng: mc.lng };
    }

    // Remove editing markers that don't belong to the current shape
    if (!isDragging) {
      if (shape !== 'rectangle' && shape !== 'rectangle-lawnmower') removeEditingMarkers();
      if (shape !== 'circle' && shape !== 'spiral') removeCircleEditingMarkers();
    }

    const centerLL: L.LatLngExpression = [p.center.lat, p.center.lng];

    if (shape === 'rectangle' || shape === 'rectangle-lawnmower') {
      const { corners } = computeRectangleCorners(
        p.center, p.length ?? 400, p.width ?? 200, p.shapeOrientation ?? 0
      );
      const shapeLatLngs = corners.map(c => [c.lat, c.lng] as [number, number]);

      buildEditingMarkers(corners, p.center);

      // Gray shape — must be L.Polygon (L.Circle has no setLatLngs)
      if (shapeLayer instanceof L.Polygon) {
        shapeLayer.setLatLngs(shapeLatLngs);
      } else {
        if (shapeLayer) { try { map.removeLayer(shapeLayer); } catch {} shapeLayer = undefined; }
        shapeLayer = L.polygon(shapeLatLngs, {
          color: '#888888', weight: 2, fillColor: '#666666', fillOpacity: 0.25, dashArray: '4 2'
        }).addTo(map);
      }

      if (!hasAutoFitted) {
        hasAutoFitted = true;
        try { map.fitBounds(L.latLngBounds(shapeLatLngs), { padding: [80, 80], maxZoom: 18 }); } catch {}
      }

      if (!isDragging) {
        const segments = (shape === 'rectangle-lawnmower')
          ? generateRectangleLawnmower(p)
          : generateRectangleZigzag(p);
        buildPreviewPath(segments);
      }

    } else if (shape === 'circle') {
      const radius = p.radius ?? 200;

      // Ensure shapeLayer is an L.Circle
      if (shapeLayer instanceof L.Circle) {
        shapeLayer.setLatLng(centerLL);
        shapeLayer.setRadius(radius);
      } else {
        if (shapeLayer) { try { map.removeLayer(shapeLayer); } catch {} shapeLayer = undefined; }
        shapeLayer = L.circle(centerLL, {
          color: '#888888', weight: 2, fillColor: '#666666', fillOpacity: 0.25, radius,
        }).addTo(map);
      }

      buildCircleEditingMarkers(p.center, radius);

      if (!hasAutoFitted) {
        hasAutoFitted = true;
        try { map.fitBounds((shapeLayer as L.Circle).getBounds(), { padding: [80, 80], maxZoom: 18 }); } catch {}
      }

      const segments = generateCircleStepped(p as CirclePatternParams);
      buildPreviewPath(segments);

    } else if (shape === 'spiral') {
      const radius = p.radius ?? 200;

      if (shapeLayer instanceof L.Circle) {
        shapeLayer.setLatLng(centerLL);
        shapeLayer.setRadius(radius);
      } else {
        if (shapeLayer) { try { map.removeLayer(shapeLayer); } catch {} shapeLayer = undefined; }
        shapeLayer = L.circle(centerLL, {
          color: '#888888', weight: 2, fillColor: '#666666', fillOpacity: 0.25, radius,
        }).addTo(map);
      }

      buildCircleEditingMarkers(p.center, radius);

      if (!hasAutoFitted) {
        hasAutoFitted = true;
        try { map.fitBounds((shapeLayer as L.Circle).getBounds(), { padding: [80, 80], maxZoom: 18 }); } catch {}
      }

      const segments = generateSpiral(p as CirclePatternParams);
      buildPreviewPath(segments);

    } else {
      // Polygon and others: pentagon placeholder
      clearShapeLines();
      const size = 0.002;
      const pts: [number, number][] = [];
      for (let i = 0; i < 5; i++) {
        const a = (i / 5) * Math.PI * 2 - Math.PI / 2;
        pts.push([p.center.lat + size * Math.cos(a), p.center.lng + size * Math.sin(a) * 0.6]);
      }
      shapeLayer = L.polygon(pts, {
        color: '#888888', weight: 2, fillColor: '#666666', fillOpacity: 0.25, dashArray: '4 2'
      }).addTo(map);

      if (!hasAutoFitted) {
        hasAutoFitted = true;
        map.panTo(centerLL); if (map.getZoom() < 15) map.setZoom(15);
      }
    }
  }

  function syncDragPreview() {
    const config = activeSurveyPattern.config;
    if (!activeSurveyPattern.isActive || !config) return;
    const p: any = config.params;
    if (!p.center) return;

    if (config.shape === 'rectangle' || config.shape === 'rectangle-lawnmower') {
      const center = _dragTemp.center || p.center;
      const length = _dragTemp.length ?? p.length ?? 400;
      const width  = _dragTemp.width  ?? p.width  ?? 200;

      const { corners } = computeRectangleCorners(center, length, width, p.shapeOrientation ?? 0);
      const shapeLL = corners.map(c => [c.lat, c.lng] as [number, number]);
      if (centerMarker) centerMarker.setLatLng([center.lat, center.lng]);
      corners.forEach((c, i) => cornerMarkers[i]?.setLatLng([c.lat, c.lng]));
      if (shapeLayer instanceof L.Polygon) shapeLayer.setLatLngs(shapeLL);

      const dragParams = { ...p, center, length, width };
      const segments = (config.shape === 'rectangle-lawnmower')
        ? generateRectangleLawnmower(dragParams)
        : generateRectangleZigzag(dragParams);
      buildPreviewPath(segments);

    } else if (config.shape === 'circle' || config.shape === 'spiral') {
      const center: LngLat = _circleDragTemp.center ?? p.center;
      const radius = _circleDragTemp.radius ?? (p.radius ?? 200);
      const centerLL: L.LatLngExpression = [center.lat, center.lng];

      if (shapeLayer instanceof L.Circle) {
        shapeLayer.setLatLng(centerLL);
        shapeLayer.setRadius(radius);
      }
      if (centerMarkerCircle) centerMarkerCircle.setLatLng(centerLL);
      const radiusLL: L.LatLngExpression = [center.lat + radius / 111320, center.lng];
      if (radiusMarker) radiusMarker.setLatLng(radiusLL);

      const dragParams: CirclePatternParams = { ...p, center, radius };
      const segments = config.shape === 'spiral'
        ? generateSpiral(dragParams)
        : generateCircleStepped(dragParams);
      buildPreviewPath(segments);
    }
  }

  // Plain let — not reactive itself; only used to detect shape transitions inside the effect.
  let prevShape: string | undefined;

  // Single effect: fires on shape change AND on any param change.
  // Using a single effect avoids the double-render that two separate effects caused
  // (prevShape as $state would reschedule a second effect via reactivity).
  $effect(() => {
    const isActive = activeSurveyPattern.isActive;
    const config = activeSurveyPattern.config;

    if (!isActive || !config) {
      clearAll(); hasAutoFitted = false; prevShape = undefined;
      return;
    }

    const shape = config.shape;
    const p = config.params as any;

    // Subscribe to all params so the effect re-runs on any param change.
    // Use ?? 0 for shape-specific props absent on other shapes (e.g. circle has no length/width).
    const _trigger = (p.length ?? 0) + (p.width ?? 0) + (p.radius ?? 0) + (p.ringPoints ?? 0)
      + p.shapeOrientation + p.targetLineSpacing
      + p.turnDistance + (p.reverse ? 1 : 0) + (p.clockwise ? 1 : 0) + p.startCorner
      + (p.trackOrientationEnabled ? 1 : 0) + p.trackOrientation
      + p.userActionStartFlags + p.userActionTrackFlags + p.userActionEndFlags
      + p.userActionLineStartFlags + p.userActionLineEndFlags;
    void _trigger;

    // Keep actualLineSpacing in sync for rectangle shapes
    if ((shape === 'rectangle' || shape === 'rectangle-lawnmower') && p.targetLineSpacing > 0 && p.width > 0) {
      const numTracks = Math.max(1, Math.ceil(p.width / p.targetLineSpacing));
      const actualSpacing = p.width / Math.max(1, numTracks - 1);
      if (Math.abs(p.actualLineSpacing - actualSpacing) > 0.01) {
        p.actualLineSpacing = actualSpacing;
      }
    }

    prevShape = shape;
    updatePreview();
  });

    onDestroy(() => { clearAll(); });
</script>
