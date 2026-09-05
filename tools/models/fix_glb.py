# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marc Hoffmann (b14ckyy)

"""Normalise a generated .glb for Kite: consistent outward winding, vertex normals, uniform scale.
Run: uv run --with trimesh --with numpy --with scipy python fix_glb.py <factor> <in.glb> <out.glb>"""
import sys
import numpy as np
import trimesh

k = float(sys.argv[1]); src, dst = sys.argv[2], sys.argv[3]
scene = trimesh.load(src, force='scene')
out = trimesh.Scene()
for name, geom in scene.geometry.items():
    m = geom.copy()
    m.merge_vertices()
    m.fix_normals()                 # consistent winding, outward where the surface encloses a volume
    m.apply_scale(k)
    _ = m.vertex_normals            # force computation so the exporter writes NORMAL
    out.add_geometry(m, node_name=name, geom_name=name)
out.export(dst, include_normals=True)
b = out.bounds
print(f"{src.split('/')[-1].split(chr(92))[-1]}: {sum(len(g.vertices) for g in out.geometry.values())} verts, bounds x{b[0][0]:+.2f}..{b[1][0]:+.2f} y{b[0][1]:+.2f}..{b[1][1]:+.2f} z{b[0][2]:+.2f}..{b[1][2]:+.2f}")
