// Procedural low-poly VTOL (quadplane) glTF (.glb) for the 3D-map UAV marker.
// = the fixed-wing model + a lift-rotor boom on each main wing with a ring fore & aft of the wing,
// coloured like the quad (left/port red, right/starboard green). Same frame pipeline as the others.
// Run: `node scripts/gen-uav-vtol.mjs` → static/models/uav-vtol.glb
import { writeFileSync, mkdirSync } from 'node:fs';

function box(sx, sy, sz, cx = 0, cy = 0, cz = 0) {
  const x = sx / 2, y = sy / 2, z = sz / 2;
  const faces = [
    [[0, 0, 1], [[-x, -y, z], [x, -y, z], [x, y, z], [-x, y, z]]],
    [[0, 0, -1], [[-x, y, -z], [x, y, -z], [x, -y, -z], [-x, -y, -z]]],
    [[1, 0, 0], [[x, -y, z], [x, -y, -z], [x, y, -z], [x, y, z]]],
    [[-1, 0, 0], [[-x, y, z], [-x, y, -z], [-x, -y, -z], [-x, -y, z]]],
    [[0, 1, 0], [[x, y, z], [x, y, -z], [-x, y, -z], [-x, y, z]]],
    [[0, -1, 0], [[-x, -y, z], [-x, -y, -z], [x, -y, -z], [x, -y, z]]],
  ];
  const p = [], n = [], idx = []; let b = 0;
  for (const [nor, vs] of faces) {
    for (const v of vs) { p.push(v[0] + cx, v[1] + cy, v[2] + cz); n.push(...nor); }
    idx.push(b, b + 1, b + 2, b, b + 2, b + 3); b += 4;
  }
  return { p: new Float32Array(p), n: new Float32Array(n), i: new Uint16Array(idx) };
}
// Torus in the XY plane (disc normal = +Z) — a lift-rotor ring.
function torusZ(R, r, RS, TS, cx = 0, cy = 0, cz = 0) {
  const p = [], n = [], idx = [];
  for (let i = 0; i <= RS; i++) {
    const u = (i / RS) * 2 * Math.PI, cu = Math.cos(u), su = Math.sin(u);
    for (let j = 0; j <= TS; j++) {
      const v = (j / TS) * 2 * Math.PI, cv = Math.cos(v), sv = Math.sin(v);
      p.push((R + r * cv) * cu + cx, (R + r * cv) * su + cy, r * sv + cz);
      n.push(cv * cu, cv * su, sv);
    }
  }
  const cols = TS + 1;
  for (let i = 0; i < RS; i++) for (let j = 0; j < TS; j++) {
    const a = i * cols + j, b = (i + 1) * cols + j, c = (i + 1) * cols + (j + 1), d = i * cols + (j + 1);
    idx.push(a, b, d, b, c, d);
  }
  return { p: new Float32Array(p), n: new Float32Array(n), i: new Uint16Array(idx) };
}
function rotateX(geo, ang) {
  const c = Math.cos(ang), s = Math.sin(ang);
  const rot = (a) => { for (let i = 0; i < a.length; i += 3) { const y = a[i + 1], z = a[i + 2]; a[i + 1] = y * c - z * s; a[i + 2] = y * s + z * c; } };
  rot(geo.p); rot(geo.n); return geo;
}
function rotateY(geo, ang) {
  const c = Math.cos(ang), s = Math.sin(ang);
  const rot = (a) => { for (let i = 0; i < a.length; i += 3) { const x = a[i], z = a[i + 2]; a[i] = x * c + z * s; a[i + 2] = -x * s + z * c; } };
  rot(geo.p); rot(geo.n); return geo;
}
const ROOT_ROLL_X = -Math.PI / 2, ROOT_YAW_Y = -Math.PI / 2;

// ── glb assembly ──
const chunks = []; let binLen = 0;
const bufferViews = [], accessors = [], primitives = [], materials = [];
function pushView(ta, target) {
  const buf = Buffer.from(ta.buffer, ta.byteOffset, ta.byteLength);
  const byteOffset = binLen; chunks.push(buf); binLen += buf.length;
  const pad = (4 - (binLen % 4)) % 4; if (pad) { chunks.push(Buffer.alloc(pad)); binLen += pad; }
  bufferViews.push({ buffer: 0, byteOffset, byteLength: ta.byteLength, target });
  return bufferViews.length - 1;
}
function mat(rgb, emissive = [0, 0, 0], metallic = 0.1, rough = 0.7) {
  materials.push({
    pbrMetallicRoughness: { baseColorFactor: [...rgb, 1], metallicFactor: metallic, roughnessFactor: rough },
    emissiveFactor: emissive, doubleSided: true,
  });
  return materials.length - 1;
}
function addPrim(geo, material) {
  rotateX(geo, ROOT_ROLL_X);
  rotateY(geo, ROOT_YAW_Y);
  let mn = [Infinity, Infinity, Infinity], mx = [-Infinity, -Infinity, -Infinity];
  for (let i = 0; i < geo.p.length; i += 3) for (let k = 0; k < 3; k++) { const v = geo.p[i + k]; if (v < mn[k]) mn[k] = v; if (v > mx[k]) mx[k] = v; }
  const pv = pushView(geo.p, 34962), nv = pushView(geo.n, 34962), iv = pushView(geo.i, 34963);
  const pa = accessors.push({ bufferView: pv, componentType: 5126, count: geo.p.length / 3, type: 'VEC3', min: [...mn], max: [...mx] }) - 1;
  const na = accessors.push({ bufferView: nv, componentType: 5126, count: geo.n.length / 3, type: 'VEC3' }) - 1;
  const ia = accessors.push({ bufferView: iv, componentType: 5123, count: geo.i.length, type: 'SCALAR' }) - 1;
  primitives.push({ attributes: { POSITION: pa, NORMAL: na }, indices: ia, material, mode: 4 });
}

const C = {
  light: mat([0.60, 0.62, 0.64]),
  dark: mat([0.06, 0.06, 0.07]),
  boom: mat([0.20, 0.21, 0.23]),
  red: mat([0.80, 0.10, 0.10], [0.34, 0.04, 0.04]),
  green: mat([0.12, 0.70, 0.18], [0.05, 0.30, 0.08]),
};
function panel(chord, span, t, cx, cy, cz) {
  const h = t / 2;
  addPrim(box(chord, span, h, cx, cy, cz + h / 2), C.light);
  addPrim(box(chord, span, h, cx, cy, cz - h / 2), C.dark);
}

// ── fuselage: lofted hexagonal body (chamfered edges, flat top/bottom) tapering to a pointed nose ──
const buckets = { light: { p: [], n: [], i: [], b: 0 }, dark: { p: [], n: [], i: [], b: 0 } };
function triTo(bk, P0, P1, P2, ref) {
  const ux = P1[0] - P0[0], uy = P1[1] - P0[1], uz = P1[2] - P0[2];
  const vx = P2[0] - P0[0], vy = P2[1] - P0[1], vz = P2[2] - P0[2];
  let nx = uy * vz - uz * vy, ny = uz * vx - ux * vz, nz = ux * vy - uy * vx;
  const l = Math.hypot(nx, ny, nz) || 1; nx /= l; ny /= l; nz /= l;
  if (nx * ref[0] + ny * ref[1] + nz * ref[2] < 0) { nx = -nx; ny = -ny; nz = -nz; }
  for (const P of [P0, P1, P2]) { bk.p.push(P[0], P[1], P[2]); bk.n.push(nx, ny, nz); }
  bk.i.push(bk.b, bk.b + 1, bk.b + 2); bk.b += 3;
}
const quadTo = (bk, A, B, C2, D, ref) => { triTo(bk, A, B, C2, ref); triTo(bk, A, C2, D, ref); };
const A2 = 0.05, B2 = 0.06, C3 = 0.075;
const prof = [[C3, 0], [A2, B2], [-A2, B2], [-C3, 0], [-A2, -B2], [A2, -B2]];
const stations = [[-0.45, 0.45], [-0.30, 0.80], [0.05, 1.0], [0.35, 0.80], [0.48, 0.45]];
const rings = stations.map(([x, s]) => prof.map(([py, pz]) => [x, s * py, s * pz]));
for (let i = 0; i < rings.length - 1; i++) {
  for (let k = 0; k < 6; k++) {
    const k1 = (k + 1) % 6;
    const A = rings[i][k], B = rings[i][k1], C2 = rings[i + 1][k1], D = rings[i + 1][k];
    const cy = (A[1] + B[1] + C2[1] + D[1]) / 4, cz = (A[2] + B[2] + C2[2] + D[2]) / 4;
    quadTo((prof[k][1] + prof[k1][1]) > 0 ? buckets.light : buckets.dark, A, B, C2, D, [0, cy, cz]);
  }
}
const last = rings[rings.length - 1], apex = [0.62, 0, 0];
for (let k = 0; k < 6; k++) {
  const k1 = (k + 1) % 6;
  triTo((prof[k][1] + prof[k1][1]) > 0 ? buckets.light : buckets.dark, last[k], last[k1], apex, [1, 0, 0]);
}
const tail = [stations[0][0], 0, 0];
for (let k = 0; k < 6; k++) triTo(buckets.dark, rings[0][(k + 1) % 6], rings[0][k], tail, [-1, 0, 0]);
const toGeo = (bk) => ({ p: new Float32Array(bk.p), n: new Float32Array(bk.n), i: new Uint16Array(bk.i) });
addPrim(toGeo(buckets.light), C.light);
addPrim(toGeo(buckets.dark), C.dark);

// main wings (top/bottom split) — no nav-light tips here; the coloured rotor rings show port/stbd
panel(0.26, 0.56, 0.04, 0.10, 0.33, 0);
panel(0.26, 0.56, 0.04, 0.10, -0.33, 0);
// horizontal stabiliser + vertical fin
panel(0.14, 0.40, 0.03, -0.40, 0, 0);
addPrim(box(0.16, 0.03, 0.16, -0.40, 0, 0.11), C.light);

// ── VTOL lift booms: one per wing, with a rotor ring fore & aft of the wing (quad colours) ──
const RR = 0.13, RT = 0.025;
for (const bm of [{ y: 0.32, col: C.red }, { y: -0.32, col: C.green }]) {
  addPrim(box(0.60, 0.035, 0.035, 0.10, bm.y, 0.02), C.boom); // fore-aft boom
  addPrim(torusZ(RR, RT, 16, 6, 0.36, bm.y, 0.02), bm.col);   // front ring (ahead of wing LE)
  addPrim(torusZ(RR, RT, 16, 6, -0.16, bm.y, 0.02), bm.col);  // rear ring (behind wing TE)
}

const gltf = {
  asset: { version: '2.0', generator: 'kite-gcs gen-uav-vtol' },
  scene: 0, scenes: [{ nodes: [0] }], nodes: [{ mesh: 0, name: 'uav-vtol' }],
  meshes: [{ name: 'uav-vtol', primitives }], materials, accessors, bufferViews,
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
writeFileSync('static/models/uav-vtol.glb', Buffer.concat([head, jh, json, bh, bin]));
const tris = primitives.reduce((s, p) => s + accessors[p.indices].count / 3, 0);
console.log(`wrote static/models/uav-vtol.glb — ${(total / 1024).toFixed(1)} KB, ${primitives.length} primitives, ${tris} triangles, ${materials.length} materials`);
