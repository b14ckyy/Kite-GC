// Procedural generic UAV marker: a flat extruded 3D arrow (the 2D play-arrow icon, extruded).
// Used for replay of non-multirotor / unknown craft until dedicated models exist. Y-up glTF
// convention, nose = +X. Run: `node scripts/gen-uav-arrow.mjs` → static/models/uav-arrow.glb
import { writeFileSync, mkdirSync } from 'node:fs';

// Extrude a 2D polygon (points in the model XZ plane, X=forward, Z=lateral) along Y (up) into a
// flat 3D plate: top (+Y) + bottom (−Y) + perimeter walls. Flat normals; materials are doubleSided.
function extrudeY(poly, tris, thickness) {
  const h = thickness / 2;
  const p = [], n = [], idx = []; let base = 0;
  const tri = (a, b, c, nor) => { for (const v of [a, b, c]) { p.push(v[0], v[1], v[2]); n.push(...nor); } idx.push(base, base + 1, base + 2); base += 3; };
  for (const [i, j, k] of tris) {
    const A = poly[i], B = poly[j], C = poly[k];
    tri([A[0], h, A[1]], [B[0], h, B[1]], [C[0], h, C[1]], [0, 1, 0]);     // top
    tri([A[0], -h, A[1]], [C[0], -h, C[1]], [B[0], -h, B[1]], [0, -1, 0]); // bottom
  }
  const N = poly.length;
  for (let e = 0; e < N; e++) {
    const A = poly[e], B = poly[(e + 1) % N];
    const ex = B[0] - A[0], ez = B[1] - A[1]; const l = Math.hypot(ez, -ex) || 1;
    const nor = [ez / l, 0, -ex / l];
    const At = [A[0], h, A[1]], Bt = [B[0], h, B[1]], Bb = [B[0], -h, B[1]], Ab = [A[0], -h, A[1]];
    tri(At, Bt, Bb, nor); tri(At, Bb, Ab, nor);
  }
  return { p: new Float32Array(p), n: new Float32Array(n), i: new Uint16Array(idx) };
}

// 90° yaw about the up axis (Y) — aligns the model frame with the 3D-map's explicit body-axis
// orientation construction (same correction as the quad). Flip the sign if the nose is reversed.
const ROOT_YAW_Y = -Math.PI / 2;
function rotateY(geo, ang) {
  const c = Math.cos(ang), s = Math.sin(ang);
  const rot = (arr) => { for (let i = 0; i < arr.length; i += 3) { const x = arr[i], z = arr[i + 2]; arr[i] = x * c + z * s; arr[i + 2] = -x * s + z * c; } };
  rot(geo.p); rot(geo.n); return geo;
}

// ── glb assembly (single mesh) ──
const chunks = []; let binLen = 0;
const bufferViews = [], accessors = [], primitives = [], materials = [];
function pushView(ta, target) {
  const buf = Buffer.from(ta.buffer, ta.byteOffset, ta.byteLength);
  const byteOffset = binLen; chunks.push(buf); binLen += buf.length;
  const pad = (4 - (binLen % 4)) % 4; if (pad) { chunks.push(Buffer.alloc(pad)); binLen += pad; }
  bufferViews.push({ buffer: 0, byteOffset, byteLength: ta.byteLength, target });
  return bufferViews.length - 1;
}
function addPrim(geo, material) {
  let mn = [Infinity, Infinity, Infinity], mx = [-Infinity, -Infinity, -Infinity];
  for (let i = 0; i < geo.p.length; i += 3) for (let k = 0; k < 3; k++) { const v = geo.p[i + k]; if (v < mn[k]) mn[k] = v; if (v > mx[k]) mx[k] = v; }
  const pv = pushView(geo.p, 34962), nv = pushView(geo.n, 34962), iv = pushView(geo.i, 34963);
  const pa = accessors.push({ bufferView: pv, componentType: 5126, count: geo.p.length / 3, type: 'VEC3', min: [...mn], max: [...mx] }) - 1;
  const na = accessors.push({ bufferView: nv, componentType: 5126, count: geo.n.length / 3, type: 'VEC3' }) - 1;
  const ia = accessors.push({ bufferView: iv, componentType: 5123, count: geo.i.length, type: 'SCALAR' }) - 1;
  primitives.push({ attributes: { POSITION: pa, NORMAL: na }, indices: ia, material, mode: 4 });
}

// 2D play-arrow (same shape as the 2D icon SHAPE_MULTIROTOR "M12 2 L5 20 L12 16 L19 20 Z"),
// mapped to model XZ: X_forward = (12 − sy)·s, Z_lateral = (sx − 12)·s.
const S = 0.05;
const map = (sx, sy) => [(12 - sy) * S, (sx - 12) * S];
const poly = [map(12, 2), map(5, 20), map(12, 16), map(19, 20)]; // nose, bottom-left, mid-notch, bottom-right
const tris = [[0, 1, 2], [0, 2, 3]];
materials.push({
  pbrMetallicRoughness: { baseColorFactor: [0.22, 0.66, 0.86, 1], metallicFactor: 0.1, roughnessFactor: 0.7 },
  emissiveFactor: [0.10, 0.32, 0.42], doubleSided: true,
});
addPrim(rotateY(extrudeY(poly, tris, 0.09), ROOT_YAW_Y), 0);

const gltf = {
  asset: { version: '2.0', generator: 'kite-gcs gen-uav-arrow' },
  scene: 0, scenes: [{ nodes: [0] }], nodes: [{ mesh: 0, name: 'uav-arrow' }],
  meshes: [{ name: 'uav-arrow', primitives }], materials, accessors, bufferViews,
  buffers: [{ byteLength: binLen }],
};
const bin = Buffer.concat(chunks);
let json = Buffer.from(JSON.stringify(gltf), 'utf8');
if (json.length % 4) json = Buffer.concat([json, Buffer.alloc(4 - (json.length % 4), 0x20)]);
const total = 12 + 8 + json.length + 8 + bin.length;
const head = Buffer.alloc(12); head.writeUInt32LE(0x46546c67, 0); head.writeUInt32LE(2, 4); head.writeUInt32LE(total, 8);
const jh = Buffer.alloc(8); jh.writeUInt32LE(json.length, 0); jh.writeUInt32LE(0x4e4f534a, 4);
const bh = Buffer.alloc(8); bh.writeUInt32LE(bin.length, 0); bh.writeUInt32LE(0x004e4942, 4);
mkdirSync('static/models', { recursive: true });
writeFileSync('static/models/uav-arrow.glb', Buffer.concat([head, jh, json, bh, bin]));
const tcount = primitives.reduce((s, p) => s + accessors[p.indices].count / 3, 0);
console.log(`wrote static/models/uav-arrow.glb — ${(total / 1024).toFixed(1)} KB, ${tcount} triangles`);
