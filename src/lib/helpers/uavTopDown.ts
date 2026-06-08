// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Top-down flat-shaded software renderer for the UAV models on the 2D (Leaflet) map.
// Draws a UavMesh into a 2D canvas: orthographic view looking straight down, back-face culling +
// painter's depth sort, flat Lambert shading from an overhead light, and a soft silhouette drop
// shadow for contrast on bright maps. Heading rotates it in-plane; pitch/roll tilt it so the
// shading + the now-visible dark underside reveal the attitude (the "gimmick"). No WebGL / deps.
//
// Model frame (final .glb): nose = +Z, up = +Y, port = +X (red), starboard = −X.
// Screen mapping (top-down, looking down −Y): +Z(nose)→up, +X(port)→left, depth = +Y (toward top).

import type { UavMesh } from './uavMesh';

const DEG = Math.PI / 180;
const AMBIENT = 0.5, DIFFUSE = 0.6; // a bit more directional so the side light reveals the attitude
const FIT = 0.66;                    // model fits this fraction of the icon (leaves room for the soft shadow)
const MODEL_RADIUS = 0.65;
// Light from the south (screen-down = −Z) + overhead (+Y): gives side shading so roll/pitch read.
const LIGHT = norm3(0.1, 0.55, -0.66);
// Attitude sign mapping — flip if a manoeuvre reads inverted (top-down ≠ the 3D ENU projection).
const SIGN_YAW = -1, SIGN_PITCH = -1, SIGN_ROLL = 1;

type M3 = number[][];
function norm3(x: number, y: number, z: number): [number, number, number] {
  const l = Math.hypot(x, y, z) || 1; return [x / l, y / l, z / l];
}
function mul3(a: M3, b: M3): M3 {
  const r: M3 = [[0, 0, 0], [0, 0, 0], [0, 0, 0]];
  for (let i = 0; i < 3; i++) for (let j = 0; j < 3; j++) { let s = 0; for (let k = 0; k < 3; k++) s += a[i][k] * b[k][j]; r[i][j] = s; }
  return r;
}
// Body→world rotation: yaw about +Y, pitch about +X, roll about +Z (roll applied first, then pitch, then yaw).
function rotMatrix(yaw: number, pitch: number, roll: number): M3 {
  const cy = Math.cos(yaw), sy = Math.sin(yaw), cx = Math.cos(pitch), sx = Math.sin(pitch), cz = Math.cos(roll), sz = Math.sin(roll);
  const Rz: M3 = [[cz, -sz, 0], [sz, cz, 0], [0, 0, 1]];
  const Rx: M3 = [[1, 0, 0], [0, cx, -sx], [0, sx, cx]];
  const Ry: M3 = [[cy, 0, sy], [0, 1, 0], [-sy, 0, cy]];
  return mul3(Ry, mul3(Rx, Rz));
}
const clamp255 = (v: number) => Math.max(0, Math.min(255, Math.round(v * 255)));

// Reused offscreen buffer for the z-buffered model raster (avoids per-frame allocation at 60 fps).
let zbCanvas: HTMLCanvasElement | undefined, zbW = 0;
let zbuf: Float32Array | undefined, zImg: ImageData | undefined;

/**
 * Render `mesh` top-down into `ctx` (canvas already sized `size`×`size`).
 * @param tint  optional flight-mode colour [r,g,b] 0..1 mixed in by `tintAmount` (mirrors 3D MIX).
 */
export function renderUavTopDown(
  ctx: CanvasRenderingContext2D, mesh: UavMesh, size: number,
  headingDeg: number, pitchDeg: number, rollDeg: number,
  tint?: [number, number, number], tintAmount = 0,
) {
  ctx.clearRect(0, 0, size, size);
  const R = rotMatrix(SIGN_YAW * headingDeg * DEG, SIGN_PITCH * pitchDeg * DEG, SIGN_ROLL * rollDeg * DEG);
  const cx = size / 2, cy = size / 2, scale = (size / 2) / MODEL_RADIUS * FIT;
  const rot = (x: number, y: number, z: number) => [
    R[0][0] * x + R[0][1] * y + R[0][2] * z,
    R[1][0] * x + R[1][1] * y + R[1][2] * z,
    R[2][0] * x + R[2][1] * y + R[2][2] * z,
  ] as [number, number, number];

  interface Tri { x: [number, number, number]; y: [number, number, number]; d: [number, number, number]; r: number; g: number; b: number; }
  const tris: Tri[] = [];
  for (const prim of mesh.primitives) {
    const P = prim.positions, N = prim.normals, I = prim.indices;
    for (let t = 0; t < I.length; t += 3) {
      const a = I[t] * 3, b = I[t + 1] * 3, c = I[t + 2] * 3;
      const A = rot(P[a], P[a + 1], P[a + 2]);
      const B = rot(P[b], P[b + 1], P[b + 2]);
      const C = rot(P[c], P[c + 1], P[c + 2]);
      // geometric face normal (robust for smooth-normal prims like the rotor rings)
      let nx = (B[1] - A[1]) * (C[2] - A[2]) - (B[2] - A[2]) * (C[1] - A[1]);
      let ny = (B[2] - A[2]) * (C[0] - A[0]) - (B[0] - A[0]) * (C[2] - A[2]);
      let nz = (B[0] - A[0]) * (C[1] - A[1]) - (B[1] - A[1]) * (C[0] - A[0]);
      const nl = Math.hypot(nx, ny, nz) || 1; nx /= nl; ny /= nl; nz /= nl;
      // orient outward using the stored (outward) vertex normal, then cull faces pointing away
      const sn = rot(N[a], N[a + 1], N[a + 2]);
      if (nx * sn[0] + ny * sn[1] + nz * sn[2] < 0) { nx = -nx; ny = -ny; nz = -nz; }
      if (ny <= 0) continue; // back-face cull: only faces toward the top-down camera (+Y)
      const sh = AMBIENT + DIFFUSE * Math.max(0, nx * LIGHT[0] + ny * LIGHT[1] + nz * LIGHT[2]);
      let r = prim.color[0] * sh + prim.emissive[0];
      let g = prim.color[1] * sh + prim.emissive[1];
      let bl = prim.color[2] * sh + prim.emissive[2];
      if (tint && tintAmount > 0) {
        r = r * (1 - tintAmount) + tint[0] * tintAmount;
        g = g * (1 - tintAmount) + tint[1] * tintAmount;
        bl = bl * (1 - tintAmount) + tint[2] * tintAmount;
      }
      tris.push({
        x: [cx - A[0] * scale, cx - B[0] * scale, cx - C[0] * scale],
        y: [cy - A[2] * scale, cy - B[2] * scale, cy - C[2] * scale],
        d: [A[1], B[1], C[1]], // world +Y depth per vertex (larger = nearer to the top-down camera)
        r: clamp255(r), g: clamp255(g), b: clamp255(bl),
      });
    }
  }

  // Shadow: show ONLY the soft (blurred) shadow — a canvas fill always has a hard edge, so we draw
  // the sharp silhouette OFF-canvas (translated left by `push`) and use shadowOffsetX = push to land
  // its blurred shadow back on the model, scaled slightly larger → a soft dark patch rings the model
  // with no hard edge. Visible on street + satellite maps.
  const SH = 1.3, push = size, oy = size * 0.02;
  ctx.save();
  ctx.shadowColor = 'rgba(0,0,0,0.72)';
  ctx.shadowBlur = size * 0.12;
  ctx.shadowOffsetX = push;
  ctx.shadowOffsetY = oy;
  ctx.fillStyle = '#000';
  ctx.beginPath();
  for (const tr of tris) {
    ctx.moveTo(cx + (tr.x[0] - cx) * SH - push, cy + (tr.y[0] - cy) * SH);
    ctx.lineTo(cx + (tr.x[1] - cx) * SH - push, cy + (tr.y[1] - cy) * SH);
    ctx.lineTo(cx + (tr.x[2] - cx) * SH - push, cy + (tr.y[2] - cy) * SH);
    ctx.closePath();
  }
  ctx.fill();
  ctx.restore();

  // Model: rasterised with a per-pixel Z-BUFFER so interpenetrating parts (e.g. a tilted multirotor's
  // arms vs the centre box) occlude correctly — painter's sorting can't resolve interpenetration.
  // Rendered 2× super-sampled into an offscreen buffer, then composited (downscaled → anti-aliased)
  // over the shadow.
  const SS = 2, W = size * SS;
  if (!zbCanvas) zbCanvas = document.createElement('canvas');
  const zctx = zbCanvas.getContext('2d');
  if (!zctx) return;
  if (zbW !== W || !zbuf || !zImg) { zbCanvas.width = W; zbCanvas.height = W; zbW = W; zbuf = new Float32Array(W * W); zImg = zctx.createImageData(W, W); }
  zbuf.fill(-Infinity);
  const data = zImg.data; data.fill(0); // transparent
  const edge = (ax: number, ay: number, bx: number, by: number, px: number, py: number) => (bx - ax) * (py - ay) - (by - ay) * (px - ax);
  for (const tr of tris) {
    const x0 = tr.x[0] * SS, y0 = tr.y[0] * SS, x1 = tr.x[1] * SS, y1 = tr.y[1] * SS, x2 = tr.x[2] * SS, y2 = tr.y[2] * SS;
    const area = edge(x0, y0, x1, y1, x2, y2);
    if (area === 0) continue;
    const minX = Math.max(0, Math.floor(Math.min(x0, x1, x2))), maxX = Math.min(W - 1, Math.ceil(Math.max(x0, x1, x2)));
    const minY = Math.max(0, Math.floor(Math.min(y0, y1, y2))), maxY = Math.min(W - 1, Math.ceil(Math.max(y0, y1, y2)));
    for (let y = minY; y <= maxY; y++) {
      for (let x = minX; x <= maxX; x++) {
        const px = x + 0.5, py = y + 0.5;
        const w0 = edge(x1, y1, x2, y2, px, py) / area;
        const w1 = edge(x2, y2, x0, y0, px, py) / area;
        const w2 = 1 - w0 - w1;
        if (w0 < 0 || w1 < 0 || w2 < 0) continue; // outside the triangle (works for either winding)
        const depth = w0 * tr.d[0] + w1 * tr.d[1] + w2 * tr.d[2];
        const idx = y * W + x;
        if (depth <= zbuf[idx]) continue;
        zbuf[idx] = depth;
        const o = idx * 4;
        data[o] = tr.r; data[o + 1] = tr.g; data[o + 2] = tr.b; data[o + 3] = 255;
      }
    }
  }
  zctx.putImageData(zImg, 0, 0);
  ctx.imageSmoothingEnabled = true;
  ctx.drawImage(zbCanvas, 0, 0, W, W, 0, 0, size, size);
}
