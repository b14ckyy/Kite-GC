// Procedural low-poly fixed-wing (airplane) glTF (.glb) for the 3D-map UAV marker.
// Same design language / frame pipeline as the quad: authored Z-up (nose +X, up +Z, left +Y),
// then rolled −90° about X (→ Y-up glTF) and yawed −90° about Y to match the map's orientation
// construction. Colours: tops light grey (PBR-shaded like the quad frame), undersides near-black,
// only tiny wing-tips red (left/port) / green (right/starboard). Run: `node scripts/gen-uav-plane.mjs`.
import { writeFileSync, mkdirSync } from 'node:fs';

// Axis-aligned box (single material), centered at (cx,cy,cz).
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
const ROOT_ROLL_X = -Math.PI / 2; // Z-up authoring → Y-up glTF
const ROOT_YAW_Y = -Math.PI / 2;  // align with the 3D-map orientation construction

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

// ── materials ──
const C = {
  light: mat([0.86, 0.87, 0.89]),                 // upper surfaces — near-white base so the flight-mode tint reads
  dark: mat([0.06, 0.06, 0.07]),                  // undersides — near-black
  red: mat([0.80, 0.10, 0.10], [0.34, 0.04, 0.04]),   // left/port wing-tip
  green: mat([0.12, 0.70, 0.18], [0.05, 0.30, 0.08]), // right/starboard wing-tip
};

// helper: a flat panel split into a light TOP half and a dark BOTTOM half (chord=X, span=Y, t=Z)
function panel(chord, span, t, cx, cy, cz) {
  const h = t / 2;
  addPrim(box(chord, span, h, cx, cy, cz + h / 2), C.light);
  addPrim(box(chord, span, h, cx, cy, cz - h / 2), C.dark);
}

// ── fuselage: a lofted body with a hexagonal cross-section (chamfered longitudinal edges, flat
// top + flat bottom so the light/dark split stays clean) tapering to a pointed nose. Built into a
// light bucket (upper faces) and a dark bucket (lower faces), each emitted as one primitive. ──
const buckets = { light: { p: [], n: [], i: [], b: 0 }, dark: { p: [], n: [], i: [], b: 0 } };
function triTo(bk, P0, P1, P2, ref) {
  const ux = P1[0] - P0[0], uy = P1[1] - P0[1], uz = P1[2] - P0[2];
  const vx = P2[0] - P0[0], vy = P2[1] - P0[1], vz = P2[2] - P0[2];
  let nx = uy * vz - uz * vy, ny = uz * vx - ux * vz, nz = ux * vy - uy * vx;
  const l = Math.hypot(nx, ny, nz) || 1; nx /= l; ny /= l; nz /= l;
  if (nx * ref[0] + ny * ref[1] + nz * ref[2] < 0) { nx = -nx; ny = -ny; nz = -nz; } // force outward
  for (const P of [P0, P1, P2]) { bk.p.push(P[0], P[1], P[2]); bk.n.push(nx, ny, nz); }
  bk.i.push(bk.b, bk.b + 1, bk.b + 2); bk.b += 3;
}
const quadTo = (bk, A, B, C, D, ref) => { triTo(bk, A, B, C, ref); triTo(bk, A, C, D, ref); };
// hexagon profile (y=lateral, z=vertical): flat top/bottom, pointed sides → chamfered edges
const A2 = 0.05, B2 = 0.06, C2 = 0.075;
const prof = [[C2, 0], [A2, B2], [-A2, B2], [-C2, 0], [-A2, -B2], [A2, -B2]];
const stations = [[-0.45, 0.45], [-0.30, 0.80], [0.05, 1.0], [0.35, 0.80], [0.48, 0.45]];
const apexX = 0.62;
const rings = stations.map(([x, s]) => prof.map(([py, pz]) => [x, s * py, s * pz]));
for (let i = 0; i < rings.length - 1; i++) {
  for (let k = 0; k < 6; k++) {
    const k1 = (k + 1) % 6;
    const A = rings[i][k], B = rings[i][k1], C = rings[i + 1][k1], D = rings[i + 1][k];
    const cy = (A[1] + B[1] + C[1] + D[1]) / 4, cz = (A[2] + B[2] + C[2] + D[2]) / 4;
    quadTo((prof[k][1] + prof[k1][1]) > 0 ? buckets.light : buckets.dark, A, B, C, D, [0, cy, cz]);
  }
}
const last = rings[rings.length - 1], apex = [apexX, 0, 0];
for (let k = 0; k < 6; k++) {
  const k1 = (k + 1) % 6;
  triTo((prof[k][1] + prof[k1][1]) > 0 ? buckets.light : buckets.dark, last[k], last[k1], apex, [1, 0, 0]);
}
const tail = [stations[0][0], 0, 0];
for (let k = 0; k < 6; k++) triTo(buckets.dark, rings[0][(k + 1) % 6], rings[0][k], tail, [-1, 0, 0]);
const toGeo = (bk) => ({ p: new Float32Array(bk.p), n: new Float32Array(bk.n), i: new Uint16Array(bk.i) });
addPrim(toGeo(buckets.light), C.light);
addPrim(toGeo(buckets.dark), C.dark);
// main wings (slightly forward of centre), split top/bottom
panel(0.26, 0.50, 0.04, 0.10, 0.32, 0);   // left  (+Y)
panel(0.26, 0.50, 0.04, 0.10, -0.32, 0);  // right (−Y)
// tiny wing-tips — port red (+Y), starboard green (−Y)
addPrim(box(0.12, 0.06, 0.04, 0.10, 0.60, 0), C.red);
addPrim(box(0.12, 0.06, 0.04, 0.10, -0.60, 0), C.green);
// horizontal stabiliser (tailplane), split top/bottom
panel(0.14, 0.40, 0.03, -0.40, 0, 0);
// vertical fin (rises from the tail) — light
addPrim(box(0.16, 0.03, 0.16, -0.40, 0, 0.11), C.light);

const gltf = {
  asset: { version: '2.0', generator: 'kite-gcs gen-uav-plane' },
  scene: 0, scenes: [{ nodes: [0] }], nodes: [{ mesh: 0, name: 'uav-plane' }],
  meshes: [{ name: 'uav-plane', primitives }], materials, accessors, bufferViews,
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
writeFileSync('static/models/uav-plane.glb', Buffer.concat([head, jh, json, bh, bin]));
const tris = primitives.reduce((s, p) => s + accessors[p.indices].count / 3, 0);
console.log(`wrote static/models/uav-plane.glb — ${(total / 1024).toFixed(1)} KB, ${primitives.length} primitives, ${tris} triangles, ${materials.length} materials`);
