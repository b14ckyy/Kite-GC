// Procedural low-poly quadcopter glTF (.glb) generator for the 3D map UAV marker.
// Stylised, recognisable-at-small-size: body box + 4 arms + 4 rotor RINGS (front green,
// rear red) + a cyan nose arrow (+X = forward). No rotor blades. Run: `node scripts/gen-uav-quad.mjs`.
// Re-run to iterate on shape/size/colours. Output: static/models/uav-quad.glb
//
// Axis convention (Cesium-friendly): +X = nose/forward, +Z = up.
import { writeFileSync, mkdirSync } from 'node:fs';

// ── geometry helpers (return {p:Float32Array, n:Float32Array, i:Uint16Array}) ──
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

// Torus in the XY plane (disc normal = +Z) — represents a rotor ring.
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

// Cone pointing +X (apex at apexX), flat-shaded. Base ring radius r at x=baseX.
function coneX(r, baseX, apexX, segs, cy = 0, cz = 0) {
  const apex = [apexX, cy, cz];
  const ring = [];
  for (let k = 0; k < segs; k++) {
    const a = (k / segs) * 2 * Math.PI;
    ring.push([baseX, cy + r * Math.cos(a), cz + r * Math.sin(a)]);
  }
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
  const rot = (arr) => { for (let i = 0; i < arr.length; i += 3) { const x = arr[i], y = arr[i + 1]; arr[i] = x * c - y * s; arr[i + 1] = x * s + y * c; } };
  rot(geo.p); rot(geo.n); return geo;
}
function rotateX(geo, ang) {
  const c = Math.cos(ang), s = Math.sin(ang);
  const rot = (arr) => { for (let i = 0; i < arr.length; i += 3) { const y = arr[i + 1], z = arr[i + 2]; arr[i + 1] = y * c - z * s; arr[i + 2] = y * s + z * c; } };
  rot(geo.p); rot(geo.n); return geo;
}
function rotateY(geo, ang) {
  const c = Math.cos(ang), s = Math.sin(ang);
  const rot = (arr) => { for (let i = 0; i < arr.length; i += 3) { const x = arr[i], z = arr[i + 2]; arr[i] = x * c + z * s; arr[i + 2] = -x * s + z * c; } };
  rot(geo.p); rot(geo.n); return geo;
}

// The geometry above is authored Z-up (easy to reason about); glTF convention is Y-up, so the
// whole model is rolled −90° about its forward (X) axis on assembly. Then a 90° yaw about the up
// axis (Y) aligns the model frame with the 3D-map's explicit body-axis orientation construction
// (so no runtime heading offset is needed). Flip ROOT_YAW_Y's sign if the nose ends up reversed.
const ROOT_ROLL_X = -Math.PI / 2;
const ROOT_YAW_Y = -Math.PI / 2;

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
  rotateX(geo, ROOT_ROLL_X); // Z-up authoring → Y-up glTF convention
  rotateY(geo, ROOT_YAW_Y);  // yaw-align the model frame with the 3D-map orientation construction
  let mn = [Infinity, Infinity, Infinity], mx = [-Infinity, -Infinity, -Infinity];
  for (let i = 0; i < geo.p.length; i += 3) for (let k = 0; k < 3; k++) { const v = geo.p[i + k]; if (v < mn[k]) mn[k] = v; if (v > mx[k]) mx[k] = v; }
  const pv = pushView(geo.p, 34962), nv = pushView(geo.n, 34962), iv = pushView(geo.i, 34963);
  const pa = accessors.push({ bufferView: pv, componentType: 5126, count: geo.p.length / 3, type: 'VEC3', min: [...mn], max: [...mx] }) - 1;
  const na = accessors.push({ bufferView: nv, componentType: 5126, count: geo.n.length / 3, type: 'VEC3' }) - 1;
  const ia = accessors.push({ bufferView: iv, componentType: 5123, count: geo.i.length, type: 'SCALAR' }) - 1;
  primitives.push({ attributes: { POSITION: pa, NORMAL: na }, indices: ia, material, mode: 4 });
}

// ── the quad ──
const C = {
  body: mat([0.13, 0.14, 0.16]),
  arm: mat([0.22, 0.23, 0.25]),
  green: mat([0.12, 0.70, 0.18], [0.05, 0.30, 0.08]),
  red: mat([0.80, 0.10, 0.10], [0.34, 0.04, 0.04]),
  arrow: mat([0.22, 0.66, 0.86], [0.10, 0.32, 0.42]),
};
const ARM = 0.45, RING_R = 0.17, RING_T = 0.03;
// body
addPrim(box(0.34, 0.34, 0.12), C.body);
// arms + rings at 4 diagonals; nose = +X so front = ±45°, rear = ±135°.
// Aviation nav-light convention: left/port (+Y) = RED, right/starboard (−Y) = GREEN.
// Same colour front & rear so an upside-down roll visibly swaps the red/green sides.
const motors = [
  { ang: Math.PI / 4, col: C.red },      // front-left  (port)
  { ang: -Math.PI / 4, col: C.green },   // front-right (starboard)
  { ang: 3 * Math.PI / 4, col: C.red },  // rear-left   (port)
  { ang: -3 * Math.PI / 4, col: C.green },// rear-right  (starboard)
];
for (const m of motors) {
  addPrim(rotateZ(box(ARM, 0.05, 0.04, ARM / 2, 0, 0), m.ang), C.arm);
  const mx = ARM * Math.cos(m.ang), my = ARM * Math.sin(m.ang);
  addPrim(torusZ(RING_R, RING_T, 16, 6, mx, my, 0.02), m.col);
}
// nose arrow (forward pointer)
addPrim(coneX(0.06, 0.17, 0.60, 14, 0, 0.03), C.arrow);

const gltf = {
  asset: { version: '2.0', generator: 'kite-gcs gen-uav-quad' },
  scene: 0, scenes: [{ nodes: [0] }],
  nodes: [{ mesh: 0, name: 'uav-quad' }],
  meshes: [{ name: 'uav-quad', primitives }],
  materials, accessors, bufferViews,
  buffers: [{ byteLength: binLen }],
};

const bin = Buffer.concat(chunks);
let json = Buffer.from(JSON.stringify(gltf), 'utf8');
if (json.length % 4) json = Buffer.concat([json, Buffer.alloc(4 - (json.length % 4), 0x20)]);
const total = 12 + 8 + json.length + 8 + bin.length;
const head = Buffer.alloc(12);
head.writeUInt32LE(0x46546c67, 0); head.writeUInt32LE(2, 4); head.writeUInt32LE(total, 8);
const jh = Buffer.alloc(8); jh.writeUInt32LE(json.length, 0); jh.writeUInt32LE(0x4e4f534a, 4);
const bh = Buffer.alloc(8); bh.writeUInt32LE(bin.length, 0); bh.writeUInt32LE(0x004e4942, 4);

mkdirSync('static/models', { recursive: true });
writeFileSync('static/models/uav-quad.glb', Buffer.concat([head, jh, json, bh, bin]));
const tris = primitives.reduce((s, p) => s + accessors[p.indices].count / 3, 0);
console.log(`wrote static/models/uav-quad.glb — ${(total / 1024).toFixed(1)} KB, ${primitives.length} primitives, ${tris} triangles, ${materials.length} materials`);
