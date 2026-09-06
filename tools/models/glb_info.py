# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marc Hoffmann (b14ckyy)

"""Print node transforms and mesh extents of .glb files (no dependencies)."""
import json, struct, sys, math
import numpy as np

def load(path):
    d = open(path, 'rb').read()
    assert d[:4] == b'glTF', path
    length = struct.unpack('<I', d[8:12])[0]
    off = 12; js = None; bins = None
    while off < length:
        clen, ctype = struct.unpack('<II', d[off:off+8]); off += 8
        chunk = d[off:off+clen]; off += clen
        if ctype == 0x4E4F534A: js = json.loads(chunk)
        elif ctype == 0x004E4942: bins = chunk
    return js, bins

def accessor(js, bins, idx):
    a = js['accessors'][idx]; bv = js['bufferViews'][a['bufferView']]
    start = bv.get('byteOffset', 0) + a.get('byteOffset', 0)
    n = a['count']; comp = {5126: ('f4', 4), 5123: ('u2', 2), 5125: ('u4', 4)}[a['componentType']]
    ncomp = {'SCALAR': 1, 'VEC2': 2, 'VEC3': 3, 'VEC4': 4}[a['type']]
    stride = bv.get('byteStride', comp[1] * ncomp)
    arr = np.frombuffer(bins, dtype=np.dtype('<' + comp[0]), count=n * ncomp if stride == comp[1] * ncomp else None, offset=start) if stride == comp[1] * ncomp else None
    if arr is None:
        rows = [np.frombuffer(bins, dtype=np.dtype('<' + comp[0]), count=ncomp, offset=start + i * stride) for i in range(n)]
        arr = np.stack(rows)
    return arr.reshape(n, ncomp)

def quat_to_mat(q):
    x, y, z, w = q
    return np.array([[1-2*(y*y+z*z), 2*(x*y-z*w), 2*(x*z+y*w)], [2*(x*y+z*w), 1-2*(x*x+z*z), 2*(y*z-x*w)], [2*(x*z-y*w), 2*(y*z+x*w), 1-2*(x*x+y*y)]])

def node_matrix(node):
    if 'matrix' in node:
        return np.array(node['matrix']).reshape(4, 4).T
    m = np.eye(4)
    s = node.get('scale', [1, 1, 1]); r = node.get('rotation', [0, 0, 0, 1]); t = node.get('translation', [0, 0, 0])
    m[:3, :3] = quat_to_mat(r) @ np.diag(s); m[:3, 3] = t
    return m

def world_vertices(js, bins):
    pts = []; colors = []
    def walk(ni, parent):
        node = js['nodes'][ni]; m = parent @ node_matrix(node)
        if 'mesh' in node:
            for prim in js['meshes'][node['mesh']]['primitives']:
                p = accessor(js, bins, prim['attributes']['POSITION']).astype(float)
                p4 = np.c_[p, np.ones(len(p))] @ m.T
                pts.append(p4[:, :3])
                mat = js['materials'][prim['material']] if 'material' in prim else {}
                bc = mat.get('pbrMetallicRoughness', {}).get('baseColorFactor', [1, 1, 1, 1])
                colors.append((tuple(round(c, 2) for c in bc[:3]), p4[:, :3]))
        for c in node.get('children', []): walk(c, m)
    scene = js['scenes'][js.get('scene', 0)]
    for n in scene['nodes']: walk(n, np.eye(4))
    return np.vstack(pts), colors

for path in sys.argv[1:]:
    js, bins = load(path)
    roots = js['scenes'][js.get('scene', 0)]['nodes']
    rot = [js['nodes'][n].get('rotation') for n in roots]
    v, colors = world_vertices(js, bins)
    mn, mx = v.min(0), v.max(0)
    # where do the pointy ends sit? count vertices in the outer 10 % slice per axis direction
    ext = mx - mn
    tips = {}
    for ax, name in enumerate('XYZ'):
        hi = (v[:, ax] > mx[ax] - 0.1 * ext[ax]).sum(); lo = (v[:, ax] < mn[ax] + 0.1 * ext[ax]).sum()
        tips[name] = (int(lo), int(hi))
    # red/green marker centroids (nav lights) → tells port/starboard axis
    def centroid(rgb_test):
        sel = [p for c, p in colors if rgb_test(c)]
        return np.vstack(sel).mean(0).round(3).tolist() if sel else None
    red = centroid(lambda c: c[0] > 0.6 and c[1] < 0.3 and c[2] < 0.3)
    green = centroid(lambda c: c[1] > 0.5 and c[0] < 0.3 and c[2] < 0.4)
    print(f"{path.split('/')[-1]:22s} nodes={len(js['nodes'])} root_rot={rot} verts={len(v)}")
    print(f"   extent X {mn[0]:+.2f}..{mx[0]:+.2f} | Y {mn[1]:+.2f}..{mx[1]:+.2f} | Z {mn[2]:+.2f}..{mx[2]:+.2f}   verts in outer 10% (lo,hi): {tips}")
    print(f"   red centroid {red}  green centroid {green}")
