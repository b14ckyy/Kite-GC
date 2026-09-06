# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marc Hoffmann (b14ckyy)

"""Kite-GC procedural model primitives. Frame: nose=+Z, up=+Y, port(left)=+X, starboard=-X."""
import numpy as np, trimesh

BODY  = (236,238,242,255)
DARK  = (60, 64, 72,255)
GUARD = (228,231,236,255)   # prop guards without a nav colour: light, so they read over the 2D drop shadow
CANOPY= (48, 56, 70,255)
NOSE  = (245,200, 40,255)
RED   = (225, 40, 40,255)
GREEN = (40, 195, 70,255)

def mesh(V, F, colors):
    m = trimesh.Trimesh(np.asarray(V,float), np.asarray(F,int), process=False)
    m.visual = trimesh.visual.ColorVisuals(m, face_colors=np.asarray(colors, np.uint8))
    return m

def orient(m):
    """Flip winding if normals point inward on average (loft direction / mirroring agnostic).
    Rebuilds instead of invert(): trimesh 5.1 invert() leaves cached normals stale."""
    c = m.triangles_center - m.triangles_center.mean(0)
    if np.einsum('ij,ij->i', m.face_normals, c).mean() < 0:
        m = mesh(m.vertices, m.faces[:, ::-1], m.visual.face_colors)
    return m

def loft(sections, colors, cap0=True, cap1=True):
    """sections: list of rings (same point count). colors: per-segment colour (len = len(sections)-1) or single colour."""
    if isinstance(colors, tuple): colors = [colors]*(len(sections)-1)
    n=len(sections[0]); V=[p for s in sections for p in s]; F=[]; C=[]
    for i in range(len(sections)-1):
        for j in range(n):
            a=i*n+j; b=i*n+(j+1)%n; c=(i+1)*n+(j+1)%n; d=(i+1)*n+j
            F += [[a,c,b],[a,d,c]]; C += [colors[i]]*2
    if cap0:
        k=len(V); V.append(np.mean(sections[0],0)); F += [[k,j,(j+1)%n] for j in range(n)]; C += [colors[0]]*n
    if cap1:
        k=len(V); V.append(np.mean(sections[-1],0)); o=(len(sections)-1)*n
        F += [[k,o+(j+1)%n,o+j] for j in range(n)]; C += [colors[-1]]*n
    return orient(mesh(V,F,C))

def ring(z, w, h, y0=0.0, n=12, top_flat=0.0, x0=0.0):
    t = np.linspace(0,2*np.pi,n,endpoint=False)+np.pi/n
    y = h*np.sin(t); y = np.where(y>0, y*(1-top_flat), y)
    return [(x0+w*np.cos(a), y0+yy, z) for a,yy in zip(t,y)]

def ring_xz(y, r, x0=0.0, z0=0.0, n=12, rz=None):
    """Horizontal ring (for vertical lofts: motor pods, domes)."""
    rz = r if rz is None else rz
    t = np.linspace(0,2*np.pi,n,endpoint=False)
    return [(x0+r*np.cos(a), y, z0+rz*np.sin(a)) for a in t]

def airfoil(x, y, z_le, chord, t):
    us = [(0.0,0.0),(0.08,0.55),(0.3,1.0),(0.6,0.75),(1.0,0.05)]
    ls = [(0.6,-0.25),(0.3,-0.35),(0.08,-0.2)]
    return [(x, y+t*f, z_le-chord*c) for c,f in us+ls]

def tip_ring(x, y, z, s=1.0):
    return [(x, y+s*d[0], z+s*d[1]) for d in [(0.01,0.01),(0.008,0.008),(0.004,0.004),(0,0),(-0.004,-0.004),(-0.008,-0.008),(-0.006,-0.006),(0.006,0.006)]]

def wing_panel(side, y, z_le, span, root_c, tip_c, thick, dihedral, sweep, nav=None, n_stations=6, tip_nav_len=1):
    """Tapered wing with rounded tip; optional nav colour on the last segments."""
    secs=[]; cols=[]
    for i in range(n_stations):
        f=i/(n_stations-1); c=root_c+(tip_c-root_c)*f
        secs.append(airfoil(side*span*f, y+dihedral*f, z_le-sweep*f, c, thick*(1-0.45*f)))
        if i>0: cols.append(BODY)
    x_end=side*span; navc = nav or BODY
    secs.append(airfoil(x_end+side*0.06*tip_c/0.28, y+dihedral+0.01, z_le-sweep-tip_c*0.08, tip_c*0.8, thick*0.4)); cols.append(navc)
    secs.append(tip_ring(x_end+side*0.09*tip_c/0.28, y+dihedral+0.015, z_le-sweep-tip_c*0.35, s=tip_c/0.28)); cols.append(navc)
    return loft(secs, cols, cap0=False, cap1=True)

def fin(z_le, y0, height, root_c, tip_c, sweep, thick=0.014):
    def sec(y, zl, c, t):
        return [(t*f, y, zl - c*cf) for cf,f in [(0,0),(0.08,0.5),(0.3,1),(0.6,0.7),(1,0.05),(0.6,-0.7),(0.3,-1),(0.08,-0.5)]]
    secs=[sec(y0, z_le, root_c, thick), sec(y0+height*0.5, z_le-sweep*0.5, (root_c+tip_c)/2, thick*0.85), sec(y0+height*0.95, z_le-sweep, tip_c, thick*0.6)]
    secs.append([(0, y0+height, z_le-sweep-tip_c*0.4+d) for d in [0.004,0.003,0.001,0,-0.001,-0.003,-0.002,0.002]])
    return loft(secs, BODY, cap0=False)

def motor_pod(x, y, z, r=0.07, color=DARK, bell=BODY):
    """Motor: mount disc + bell (no prop — props don't spin in Kite)."""
    parts=[loft([ring_xz(y-0.02,r*1.1,x,z), ring_xz(y+0.01,r*1.1,x,z)], color)]
    parts.append(loft([ring_xz(y+0.01,r*0.75,x,z), ring_xz(y+0.07,r*0.75,x,z), ring_xz(y+0.09,r*0.45,x,z), ring_xz(y+0.10,r*0.15,x,z)], bell))
    return parts

def arm(p0, p1, r=0.028, color=BODY, n=8):
    """Round tube from p0 to p1."""
    p0=np.array(p0,float); p1=np.array(p1,float); d=p1-p0; L=np.linalg.norm(d); d/=L
    u = np.cross(d,[0,1,0]); u = u/np.linalg.norm(u) if np.linalg.norm(u)>1e-6 else np.array([1,0,0]); v=np.cross(d,u)
    t=np.linspace(0,2*np.pi,n,endpoint=False)
    def rng(p, rr): return [tuple(p+rr*(np.cos(a)*u+np.sin(a)*v)) for a in t]
    return loft([rng(p0,r), rng(p0+d*L*0.5, r*0.9), rng(p1,r*0.8)], color)

def check(parts):
    for i,p in enumerate(parts):
        c = p.triangles_center - p.triangles_center.mean(0)
        frac = (np.einsum('ij,ij->i', p.face_normals, c)>0).mean()
        assert frac>0.5, f"part {i} looks inside-out ({frac:.2f} outward)"

def export(parts, path):
    """One primitive per colour with baseColorFactor material (2D renderer reads baseColorFactor)."""
    check(parts)
    m = trimesh.util.concatenate(parts); cols = m.visual.face_colors; scene = trimesh.Scene()
    for i,c in enumerate(np.unique(cols, axis=0)):
        idx = np.where((cols==c).all(1))[0]; sub = m.submesh([idx], append=True)
        mat = trimesh.visual.material.PBRMaterial(baseColorFactor=c/255.0, metallicFactor=0.0, roughnessFactor=0.8, name=f"mat{i}")
        sub.visual = trimesh.visual.TextureVisuals(material=mat)
        scene.add_geometry(sub, node_name=f"part{i}", geom_name=f"part{i}")
    scene.export(path)
    print(f"{path}: {len(m.vertices)} verts, {len(m.faces)} faces, bounds {m.bounds.round(2).tolist()}")
    return m

def prop_guard(x, y, z, r, color=GUARD, t=0.036, h=0.09, n=20, spokes=3, hub_r=0.075):
    """Closed annular ring (prop guard) around a motor at (x,y,z) plus thin spokes to the motor mount."""
    parts=[loft([ring_xz(y-h/2, r+t, x, z, n), ring_xz(y+h/2, r+t, x, z, n), ring_xz(y+h/2, r-t, x, z, n),
                 ring_xz(y-h/2, r-t, x, z, n), ring_xz(y-h/2, r+t, x, z, n)], color, cap0=False, cap1=False)]
    for k in range(spokes):
        a = 2*np.pi*k/spokes + np.pi/2
        parts.append(arm((x+hub_r*np.cos(a), y, z+hub_r*np.sin(a)), (x+(r-t)*np.cos(a), y, z+(r-t)*np.sin(a)), r=0.024, color=color, n=6))
    return parts

def ring_yz(x, r, y0=0.0, z0=0.0, n=12):
    """Ring in the YZ plane (normal ±X) — e.g. a helicopter tail rotor disc."""
    t = np.linspace(0,2*np.pi,n,endpoint=False)
    return [(x, y0+r*np.sin(a), z0+r*np.cos(a)) for a in t]
