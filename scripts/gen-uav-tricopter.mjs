// Procedural low-poly tricopter (Y-frame) glTF (.glb) for the 3D-map UAV marker.
// Body + 3 arms in a Y (two front, one longer tail) + rotor rings: front-left red (port),
// front-right green (starboard), tail white; cyan nose arrow (+X = forward). Same frame pipeline
// as the quad. Run: `node scripts/gen-uav-tricopter.mjs` → static/models/uav-tricopter.glb
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
function coneX(r, baseX, apexX, segs, cy = 0, cz = 0) {
  const apex = [apexX, cy, cz], ring = [];
  for (let k = 0; k < segs; k++) { const a = (k / segs) * 2 * Math.PI; ring.push([baseX, cy + r * Math.cos(a), cz + r * Math.sin(a)]); }
  const p = [], n = [], idx = []; let b = 0;
  const sub = (u, v) => [u[0] - v[0], u[1] - v[1], u[2] - v[2]];
  const cross = (u, v) => [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
  const norm = (v) => { const l = Math.hypot(...v) || 1; return [v[0] / l, v[1] / l, v[2] / l]; };
  for (let k = 0; k < segs; k++) {
    const v0 = ring[k], v1 = ring[(k + 1) % segs];
    const nr = norm(cross(sub(v0, apex), sub(v1, apex)));
    for (const v of [apex, v0, v1]) { p.push(...v); n.push(...nr); }
    idx.push(b, b + 1, b + 2); b += 3;
  }
  return { p: new Float32Array(p), n: new Float32Array(n), i: new Uint16Array(idx) };
}
function rotateZ(geo, ang) {
  const c = Math.cos(ang), s = Math.sin(ang);
  const rot = (a) => { for (let i = 0; i < a.length; i += 3) { const x = a[i], y = a[i + 1]; a[i] = x * c - y * s; a[i + 1] = x * s + y * c; } };
  rot(geo.p); rot(geo.n); return geo;
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

// ── the tricopter ──
const C = {
  body: mat([0.86, 0.87, 0.89]), // near-white base so the flight-mode tint reads
  arm: mat([0.55, 0.56, 0.58]),
  green: mat([0.12, 0.70, 0.18], [0.05, 0.30, 0.08]),
  red: mat([0.80, 0.10, 0.10], [0.34, 0.04, 0.04]),
  white: mat([0.90, 0.90, 0.92], [0.14, 0.14, 0.15]),
  arrow: mat([0.22, 0.66, 0.86], [0.10, 0.32, 0.42]),
};
const RING_R = 0.17, RING_T = 0.03;
// body (smaller centre block than the quad)
addPrim(box(0.26, 0.26, 0.10), C.body);
// Y-frame arms (120° apart): front-left red (port), front-right green (starboard), longer tail white.
const motors = [
  { ang: Math.PI / 3, len: 0.42, col: C.red },     // front-left  (port, +60°)
  { ang: -Math.PI / 3, len: 0.42, col: C.green },  // front-right (starboard, −60°)
  { ang: Math.PI, len: 0.54, col: C.white },       // tail (180°)
];
for (const m of motors) {
  addPrim(rotateZ(box(m.len, 0.05, 0.04, m.len / 2, 0, 0), m.ang), C.arm);
  const mx = m.len * Math.cos(m.ang), my = m.len * Math.sin(m.ang);
  addPrim(torusZ(RING_R, RING_T, 16, 6, mx, my, 0.02), m.col);
}
// nose arrow (forward pointer)
addPrim(coneX(0.06, 0.17, 0.60, 14, 0, 0.03), C.arrow);

const gltf = {
  asset: { version: '2.0', generator: 'kite-gcs gen-uav-tricopter' },
  scene: 0, scenes: [{ nodes: [0] }], nodes: [{ mesh: 0, name: 'uav-tricopter' }],
  meshes: [{ name: 'uav-tricopter', primitives }], materials, accessors, bufferViews,
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
writeFileSync('static/models/uav-tricopter.glb', Buffer.concat([head, jh, json, bh, bin]));
const tris = primitives.reduce((s, p) => s + accessors[p.indices].count / 3, 0);
console.log(`wrote static/models/uav-tricopter.glb — ${(total / 1024).toFixed(1)} KB, ${primitives.length} primitives, ${tris} triangles, ${materials.length} materials`);
