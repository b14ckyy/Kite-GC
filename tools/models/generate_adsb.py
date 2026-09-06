# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marc Hoffmann (b14ckyy)

"""Radar / ADS-B contact models for Kite-GC (static/models/radar/). Bodies are pure WHITE: the 3D map
tints contacts with Cesium's HIGHLIGHT blend (lit surface × altitude colour), so any grey in the material
would darken the colour scale. Only silhouette + orientation matter otherwise. Frame: nose=+Z, up=+Y, port=+X.
Run:  uv run --with trimesh --with numpy --with scipy python generate_adsb.py <outdir> [names...]"""
import sys, os, numpy as np
import kitemodels
from kitemodels import *
from generate_uav import scaled

BODY = kitemodels.BODY = (255, 255, 255, 255)   # see the module note (HIGHLIGHT blend)

N = 10   # ring resolution for these simpler models

def tube(zs, ws, hs, ys=None, n=N, top_flat=0.0):
    ys = ys or [0.0]*len(zs)
    return loft([ring(z,w,h,y,n=n,top_flat=top_flat) for z,w,h,y in zip(zs,ws,hs,ys)], BODY)

def swept_wing(side, y, z_le, span, root_c, tip_c, sweep, dihedral=0.05, thick=0.05, stations=4):
    return wing_panel(side, y, z_le, span, root_c, tip_c, thick, dihedral, sweep, nav=None, n_stations=stations)

def engine(x, y, z, r=0.09, L=0.42, n=N):
    """Turbofan nacelle along +Z centred at z."""
    return loft([ring(z+L/2, r*0.9, r*0.9, y, n=n, x0=x), ring(z+L*0.3, r, r, y, n=n, x0=x),
                 ring(z-L*0.3, r, r, y, n=n, x0=x), ring(z-L/2, r*0.7, r*0.7, y, n=n, x0=x)], BODY)

def airliner(L, fus_r, span, root_c, tip_c, sweep, engines, engine_r=0.09, fin_h=0.45):
    """Generic jet airliner. engines: list of spanwise fractions (0..1) for nacelles, both sides."""
    zs=[L*0.5, L*0.46, L*0.38, L*0.15, -L*0.15, -L*0.3, -L*0.43, -L*0.5]
    ws=[fus_r*0.2, fus_r*0.6, fus_r*0.95, fus_r, fus_r, fus_r*0.85, fus_r*0.5, fus_r*0.25]
    ys=[0,0,0,0,0,fus_r*0.15,fus_r*0.55,fus_r*0.8]
    p=[tube(zs, ws, ws, ys, top_flat=0.05)]
    wing_z = L*0.08
    for s in (+1,-1):
        p.append(swept_wing(s, -fus_r*0.4, wing_z, span, root_c, tip_c, sweep, dihedral=span*0.09, thick=0.06))
        for f in engines:
            x=s*span*f; zle=wing_z-sweep*f; c=root_c+(tip_c-root_c)*f
            p.append(engine(x, -fus_r*0.4 + span*0.09*f - engine_r*0.95, zle-c*0.1, r=engine_r, L=c*0.9))
        p.append(swept_wing(s, fus_r*0.6, -L*0.40, span*0.36, root_c*0.5, tip_c*0.6, sweep*0.4, dihedral=0.03, thick=0.03, stations=3))
    p.append(fin(-L*0.36, fus_r*0.9, fin_h, root_c*0.7, tip_c*0.7, fin_h*0.7, thick=0.03))
    return p

# ---------------------------------------------------------------------------- models
def heavy():   # 747-style: 4 engines, long body, big span
    return airliner(L=3.0, fus_r=0.20, span=1.55, root_c=0.75, tip_c=0.22, sweep=0.70, engines=(0.36, 0.62), engine_r=0.10, fin_h=0.55)

def small():   # A320-style: 2 engines
    return airliner(L=2.4, fus_r=0.15, span=1.20, root_c=0.55, tip_c=0.18, sweep=0.45, engines=(0.36,), engine_r=0.10, fin_h=0.45)

def jet():     # high-performance: pointed nose, swept wings, low aspect
    p=[tube([1.30,1.10,0.6,0.0,-0.6,-1.0,-1.25],[0.02,0.06,0.13,0.14,0.12,0.08,0.05],[0.02,0.06,0.13,0.14,0.11,0.07,0.04])]
    p.append(loft([ring(0.15,0.08,0.05,0.12,n=N), ring(0.45,0.09,0.09,0.13,n=N), ring(0.75,0.06,0.04,0.11,n=N)], BODY))
    for s in (+1,-1):
        p.append(swept_wing(s, 0.0, 0.45, 0.95, 0.85, 0.22, 0.75, dihedral=0.0, thick=0.04))
        p.append(swept_wing(s, 0.02, -0.75, 0.42, 0.35, 0.12, 0.30, dihedral=0.0, thick=0.025, stations=3))
    p.append(fin(-0.65, 0.10, 0.45, 0.45, 0.15, 0.40, thick=0.025))
    return p

def light():   # Cessna-style high wing with prop disc
    p=[tube([0.95,0.88,0.7,0.3,-0.2,-0.7,-1.05],[0.06,0.10,0.14,0.15,0.11,0.06,0.04],[0.07,0.12,0.16,0.17,0.13,0.07,0.05],[0,0,0,0,0.03,0.06,0.08],top_flat=0.1)]
    for s in (+1,-1):
        p.append(swept_wing(s, 0.17, 0.30, 1.35, 0.42, 0.34, 0.03, dihedral=0.02, thick=0.05, stations=3))
        p.append(swept_wing(s, 0.10, -0.85, 0.45, 0.25, 0.18, 0.05, dihedral=0.0, thick=0.02, stations=3))
    p.append(fin(-0.78, 0.10, 0.40, 0.32, 0.20, 0.22, thick=0.02))
    # prop disc: thin annulus at the nose
    r=0.42; t=0.012; z=0.97
    p.append(loft([ring(z,r,r,0,n=16), ring(z+0.015,r,r,0,n=16), ring(z+0.015,r-t*2,r-t*2,0,n=16), ring(z,r-t*2,r-t*2,0,n=16), ring(z,r,r,0,n=16)], BODY, cap0=False, cap1=False))
    p.append(loft([ring(0.95,0.05,0.05,n=N), ring(1.08,0.01,0.01,n=N)], BODY, cap0=False))
    return p

def glider():  # sailplane: slim body, long high-aspect wings, T-tail
    p=[tube([0.9,0.8,0.5,0.1,-0.4,-1.0,-1.35],[0.04,0.09,0.11,0.09,0.05,0.03,0.025],[0.05,0.10,0.13,0.10,0.06,0.035,0.03],[0,0,0,0,0.02,0.05,0.07])]
    p.append(loft([ring(0.35,0.06,0.03,0.10,n=N), ring(0.6,0.07,0.07,0.11,n=N), ring(0.8,0.04,0.03,0.09,n=N)], BODY))
    for s in (+1,-1):
        p.append(swept_wing(s, 0.06, 0.2, 2.4, 0.30, 0.12, 0.05, dihedral=0.10, thick=0.035, stations=5))
        p.append(swept_wing(s, 0.42, -1.22, 0.38, 0.16, 0.10, 0.03, dihedral=0.0, thick=0.015, stations=3))
    p.append(fin(-1.05, 0.08, 0.35, 0.30, 0.18, 0.15, thick=0.02))
    return p

def heli():    # helicopter: cabin, tail boom, rotor ring, tail rotor ring
    p=[tube([0.75,0.65,0.35,0.0,-0.35,-0.6],[0.10,0.19,0.24,0.22,0.15,0.08],[0.12,0.22,0.27,0.25,0.17,0.09],[0,0,0,0,0.03,0.06],top_flat=0.1)]
    p.append(tube([-0.5,-1.2,-1.55],[0.07,0.05,0.035],[0.08,0.06,0.04],[0.06,0.12,0.16]))
    p.append(fin(-1.40, 0.16, 0.30, 0.25, 0.14, 0.18, thick=0.02))
    p.append(loft([ring_xz(0.26,0.06,n=N), ring_xz(0.40,0.045,n=N), ring_xz(0.46,0.02,n=N)], BODY))   # mast
    r=1.05; t=0.14; y=0.44   # rotor disc: wide floating ring, no spokes (blades aren't visible anyway)
    p.append(loft([ring_xz(y,r,n=24), ring_xz(y+0.015,r,n=24), ring_xz(y+0.015,r-t,n=24), ring_xz(y,r-t,n=24), ring_xz(y,r,n=24)], BODY, cap0=False, cap1=False))
    # tail rotor ring (vertical, on the port side)
    rt=0.22; tt=0.02; x=0.06
    p.append(loft([ring_yz(x,rt,0.16,-1.48,n=16), ring_yz(x,rt-tt,0.16,-1.48,n=16), ring_yz(x-0.02,rt-tt,0.16,-1.48,n=16), ring_yz(x-0.02,rt,0.16,-1.48,n=16), ring_yz(x,rt,0.16,-1.48,n=16)], BODY, cap0=False, cap1=False))
    # skids
    for s in (+1,-1):
        p.append(arm((s*0.22,-0.32,0.5),(s*0.22,-0.32,-0.35), r=0.02, n=6))
        p.append(arm((s*0.16,-0.15,0.3),(s*0.22,-0.32,0.3), r=0.015, n=6)); p.append(arm((s*0.16,-0.15,-0.2),(s*0.22,-0.32,-0.2), r=0.015, n=6))
    return p

def balloon():  # hot-air balloon: envelope + basket, symmetric
    p=[loft([ring_xz(-0.30,0.12,n=12), ring_xz(-0.1,0.42,n=12), ring_xz(0.25,0.62,n=12), ring_xz(0.65,0.60,n=12), ring_xz(0.95,0.40,n=12), ring_xz(1.10,0.12,n=12)], BODY)]
    p.append(loft([ring_xz(-0.75,0.16,n=8), ring_xz(-0.50,0.17,n=8)], BODY))     # basket
    for k in range(4):                                                            # lines
        a=np.pi/4+k*np.pi/2
        p.append(arm((0.16*np.cos(a),-0.5,0.16*np.sin(a)),(0.12*np.cos(a),-0.30,0.12*np.sin(a)), r=0.008, n=4))
    return p

def dot():      # no heading: flat donut (distinguishable from the balloon)
    R, r, h = 0.5, 0.22, 0.08
    return [loft([ring_xz(-h/2,R-0.04,n=16), ring_xz(0,R,n=16), ring_xz(h/2,R-0.04,n=16), ring_xz(h/2,r+0.04,n=16),
                  ring_xz(0,r,n=16), ring_xz(-h/2,r+0.04,n=16), ring_xz(-h/2,R-0.04,n=16)], BODY, cap0=False, cap1=False)]

def ground():   # simple pickup: box body + cab + sloped hood + 4 wheels, nose +Z
    p=[]
    def box(x0,x1,y0,y1,z0,z1):
        V=[(x,y,z) for z in (z0,z1) for y in (y0,y1) for x in (x0,x1)]
        F=[[0,2,1],[1,2,3],[4,5,6],[5,7,6],[0,1,4],[1,5,4],[2,6,3],[3,6,7],[0,4,2],[2,4,6],[1,3,5],[3,7,5]]
        return orient(mesh(V,F,[BODY]*12))
    p.append(box(-0.28,0.28,0.02,0.22,-0.65,0.65))     # lower body
    p.append(box(-0.25,0.25,0.22,0.45,-0.55,0.10))     # cab / cargo box
    p.append(loft([ring(0.10,0.25,0.115,0.335,n=4), ring(0.30,0.24,0.09,0.31,n=4)], BODY))   # sloped hood
    for sx in (+1,-1):
        for z in (0.40,-0.42):
            w=trimesh.creation.cylinder(radius=0.11, height=0.08, sections=10); w.apply_transform(trimesh.transformations.rotation_matrix(np.pi/2,[0,1,0])); w.apply_translation((sx*0.28,0.0,z))
            p.append(mesh(w.vertices,w.faces,np.tile(np.array(BODY,np.uint8),(len(w.faces),1))))
    return p

def block_arrow():  # plain flat block arrow (same silhouette as the old placeholder), slightly thicker
    V=[(0,-0.05,0.7),(0.5,-0.05,-0.6),(0,-0.05,-0.3),(-0.5,-0.05,-0.6),(0,0.05,0.7),(0.5,0.05,-0.6),(0,0.05,-0.3),(-0.5,0.05,-0.6)]
    F=[[0,1,2],[0,2,3],[4,6,5],[4,7,6],[0,4,5],[0,5,1],[1,5,6],[1,6,2],[2,6,7],[2,7,3],[3,7,4],[3,4,0]]
    return [orient(mesh(V,F,[BODY]*12))]

def uav():      # multirotor (lighter version of uav-quad). README wanted a paper plane — swap here if preferred.
    p=[loft([ring(z,w,h,0.0,n=8,top_flat=0.3) for z,w,h in zip([0.3,0.15,0,-0.15,-0.3],[0.10,0.18,0.19,0.18,0.10],[0.02,0.065,0.07,0.065,0.02])], BODY)]
    p.append(loft([ring(0.28,0.07,0.035,0.0,n=8), ring(0.50,0.015,0.015,0.01,n=8)], BODY, cap0=False))
    for sx,sz in ((+1,+1),(-1,+1),(+1,-1),(-1,-1)):
        end=(sx*0.62,0.0,sz*0.55)
        p.append(arm((sx*0.14,0.0,sz*0.16), end, r=0.03, n=6))
        p += motor_pod(end[0],0.0,end[2], r=0.075, color=BODY, bell=BODY)
        p += prop_guard(end[0],0.06,end[2], r=0.40, n=14, color=BODY)
    return scaled(p, 1.3)

MODELS = {"adsb-light": light, "adsb-small": small, "adsb-heavy": heavy, "adsb-jet": jet, "adsb-heli": heli,
          "adsb-glider": glider, "adsb-balloon": balloon, "adsb-arrow": block_arrow, "adsb-ground": ground,
          "adsb-dot": dot, "ff-uav": uav}

if __name__=="__main__":
    out = sys.argv[1] if len(sys.argv)>1 else "radar"; os.makedirs(out, exist_ok=True)
    names = sys.argv[2:] or MODELS.keys()
    for n in names: export(MODELS[n](), os.path.join(out, n+".glb"))
