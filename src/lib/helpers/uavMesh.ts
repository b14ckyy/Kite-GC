// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Minimal binary glTF (.glb) loader for the procedural UAV models — just enough to feed the 2D
// top-down canvas renderer (positions + normals + indices + flat material colours). The same .glb
// assets are used by Cesium in 3D, so there is a single source of truth for the geometry.

export interface UavMeshPrimitive {
  positions: Float32Array; // xyz triplets (model frame: nose +Z, up +Y, port +X)
  normals: Float32Array;   // xyz triplets
  indices: Uint16Array;
  color: [number, number, number];    // baseColorFactor rgb
  emissive: [number, number, number]; // emissiveFactor rgb
}
export interface UavMesh { primitives: UavMeshPrimitive[]; }

const cache = new Map<string, Promise<UavMesh>>();

/** Load + parse a .glb (cached per URL). */
export function loadUavMesh(url: string): Promise<UavMesh> {
  let p = cache.get(url);
  if (!p) {
    p = fetchAndParse(url).catch((e) => { cache.delete(url); throw e; });
    cache.set(url, p);
  }
  return p;
}

async function fetchAndParse(url: string): Promise<UavMesh> {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`glb fetch ${url}: ${res.status}`);
  const buf = await res.arrayBuffer();
  const dv = new DataView(buf);
  if (dv.getUint32(0, true) !== 0x46546c67) throw new Error(`not a glb: ${url}`);

  const jsonLen = dv.getUint32(12, true);
  const gltf = JSON.parse(new TextDecoder().decode(new Uint8Array(buf, 20, jsonLen)));
  const binStart = 20 + jsonLen + 8; // skip JSON chunk + BIN chunk header (both 4-aligned in our files)

  const acc = gltf.accessors as any[];
  const views = gltf.bufferViews as any[];
  const mats = (gltf.materials ?? []) as any[];

  const read = (i: number): Float32Array | Uint16Array => {
    const a = acc[i];
    const v = views[a.bufferView];
    const off = binStart + (v.byteOffset ?? 0) + (a.byteOffset ?? 0);
    const n = a.type === 'VEC3' ? 3 : 1;
    return a.componentType === 5126
      ? new Float32Array(buf, off, a.count * n)
      : new Uint16Array(buf, off, a.count * n);
  };

  const primitives: UavMeshPrimitive[] = [];
  for (const mesh of gltf.meshes) {
    for (const prim of mesh.primitives) {
      const m = mats[prim.material] ?? {};
      const base = m.pbrMetallicRoughness?.baseColorFactor ?? [0.7, 0.7, 0.7, 1];
      const emi = m.emissiveFactor ?? [0, 0, 0];
      primitives.push({
        positions: read(prim.attributes.POSITION) as Float32Array,
        normals: read(prim.attributes.NORMAL) as Float32Array,
        indices: read(prim.indices) as Uint16Array,
        color: [base[0], base[1], base[2]],
        emissive: [emi[0], emi[1], emi[2]],
      });
    }
  }
  return { primitives };
}
