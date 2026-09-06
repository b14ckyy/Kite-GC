# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marc Hoffmann (b14ckyy)

"""Preview .glb models the way Kite's 2D top-down renderer does (and a side view), into one PNG grid."""
import sys, zlib, struct, math, glob
import numpy as np
sys.path.insert(0, sys.argv[1])
ns = {}; exec(open(sys.argv[1] + "/glb_info.py").read().split("for path in sys.argv[1:]:")[0], ns)

CELL = 220; FIT = 0.66; MODEL_RADIUS = 0.65
AMBIENT, DIFFUSE = 0.5, 0.6
LIGHT = np.array([0.1, 0.55, -0.66]); LIGHT /= np.linalg.norm(LIGHT)

def prims(path):
    js, bins = ns["load"](path); out = []
    def walk(ni, parent):
        node = js['nodes'][ni]; m = parent @ ns["node_matrix"](node)
        if 'mesh' in node:
            for p in js['meshes'][node['mesh']]['primitives']:
                P = ns["accessor"](js, bins, p['attributes']['POSITION']).astype(float)
                N = ns["accessor"](js, bins, p['attributes']['NORMAL']).astype(float) if 'NORMAL' in p['attributes'] else None
                I = ns["accessor"](js, bins, p['indices']).reshape(-1, 3)
                mat = js['materials'][p['material']] if 'material' in p else {}
                col = np.array(mat.get('pbrMetallicRoughness', {}).get('baseColorFactor', [0.7, 0.7, 0.7, 1])[:3])
                out.append((P, N, I, col))
        for c in node.get('children', []): walk(c, m)
    for n in js['scenes'][js.get('scene', 0)]['nodes']: walk(n, np.eye(4))
    return out

def render(path, img, ox, oy, view):
    """view 'top': camera +Y looking down; 'side': camera +X looking at port side (nose to the right? nose +Z → screen right)."""
    cx, cy = ox + CELL / 2, oy + CELL / 2; scale = (CELL / 2) / MODEL_RADIUS * FIT
    tris = []
    for P, N, I, col in prims(path):
        for a, b, c in I:
            A, B, C = P[a], P[b], P[c]
            n = np.cross(B - A, C - A); l = np.linalg.norm(n) or 1; n = n / l
            if N is not None:
                sn = (N[a] + N[b] + N[c]) / 3
                if np.dot(n, sn) < 0: n = -n
            if view == 'top':
                if n[1] <= 0: continue
                sx = [cx - v[0] * scale for v in (A, B, C)]; sy = [cy - v[2] * scale for v in (A, B, C)]; depth = (A[1] + B[1] + C[1]) / 3
            else:
                if n[0] <= 0: continue
                sx = [cx + v[2] * scale for v in (A, B, C)]; sy = [cy - v[1] * scale for v in (A, B, C)]; depth = (A[0] + B[0] + C[0]) / 3
            sh = AMBIENT + DIFFUSE * max(0.0, float(np.dot(n, LIGHT)))
            tris.append((depth, sx, sy, np.clip(col * sh, 0, 1)))
    tris.sort(key=lambda t: t[0])
    for _, sx, sy, rgb in tris:
        fill_tri(img, sx, sy, (rgb * 255).astype(int))

def fill_tri(img, xs, ys, rgb):
    h, w, _ = img.shape
    y0, y1 = max(int(min(ys)), 0), min(int(max(ys)) + 1, h - 1)
    for y in range(y0, y1 + 1):
        xs_at = []
        for i in range(3):
            (xa, ya), (xb, yb) = (xs[i], ys[i]), (xs[(i + 1) % 3], ys[(i + 1) % 3])
            if (ya <= y < yb) or (yb <= y < ya):
                xs_at.append(xa + (y - ya) * (xb - xa) / (yb - ya))
        if len(xs_at) >= 2:
            x0, x1 = max(int(min(xs_at)), 0), min(int(max(xs_at)), w - 1)
            img[y, x0:x1 + 1] = rgb

def png(img, path):
    h, w, _ = img.shape
    raw = b''.join(b'\0' + img[y].astype(np.uint8).tobytes() for y in range(h))
    def chunk(t, d): return struct.pack('>I', len(d)) + t + d + struct.pack('>I', zlib.crc32(t + d) & 0xffffffff)
    open(path, 'wb').write(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 2, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(raw, 9)) + chunk(b'IEND', b''))

files = sys.argv[3:]
cols = len(files); img = np.full((2 * CELL, cols * CELL, 3), 40, dtype=np.uint8)
for i, f in enumerate(files):
    for r, view in enumerate(('top', 'side')):
        render(f, img, i * CELL, r * CELL, view)
        # cell border
        img[r * CELL, i * CELL:(i + 1) * CELL] = 90; img[r * CELL:(r + 1) * CELL, i * CELL] = 90
png(img, sys.argv[2]); print("wrote", sys.argv[2], img.shape)
