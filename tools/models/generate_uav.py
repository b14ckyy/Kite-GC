# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marc Hoffmann (b14ckyy)

"""Own-UAV models for Kite-GC: plane, quad, tricopter, vtol, arrow.
Run:  uv run --with trimesh --with numpy --with scipy python generate_uav.py <outdir> [model names...]"""
import sys, os, numpy as np
from kitemodels import *

def scaled(parts, k):
    for m in parts: m.apply_scale(k)
    return parts

def fuselage(scale=1.0, tail=True):
    fz = [ 1.05, 0.98, 0.85, 0.65, 0.35, 0.05, -0.30, -0.65, -1.00, -1.25]
    fw = [ 0.07, 0.10, 0.125,0.135,0.135,0.125,0.10, 0.075,0.055,0.045]
    fh = [ 0.075,0.105,0.13, 0.14, 0.14, 0.13, 0.105,0.08, 0.06, 0.05]
    fy = [ 0.0,  0.0,  0.0,  0.0,  0.0,  0.0,  0.005,0.015,0.03, 0.04]
    if not tail: fz,fw,fh,fy = fz[:7],fw[:7],fh[:7],fy[:7]
    return loft([ring(z*scale,w*scale,h*scale,y*scale,top_flat=0.15) for z,w,h,y in zip(fz,fw,fh,fy)], BODY)

def canopy(scale=1.0):
    cz=[0.80,0.72,0.55,0.35,0.15,0.00]; cw=[0.07,0.10,0.11,0.11,0.09,0.06]; ch=[0.01,0.06,0.09,0.09,0.07,0.02]; cy=[0.12,0.12,0.125,0.125,0.12,0.115]
    return loft([ring(z*scale,w*scale,h*scale,y*scale) for z,w,h,y in zip(cz,cw,ch,cy)], CANOPY)

def spinner(z0=1.05, r=0.055):
    return loft([ring(z0,r,r), ring(z0+0.08,r*0.82,r*0.82), ring(z0+0.17,r*0.36,r*0.36), ring(z0+0.21,r*0.07,r*0.07)], NOSE, cap0=False)

def tailplane():
    out=[]
    for s in (+1,-1):
        secs=[airfoil(0,0.04,-0.98,0.22,0.018), airfoil(s*0.30,0.04,-1.00,0.18,0.014), airfoil(s*0.44,0.045,-1.03,0.13,0.010), tip_ring(s*0.47,0.045,-1.12,0.5)]
        out.append(loft(secs,BODY,cap0=False))
    return out

# ------------------------------------------------------------------ plane
def plane():
    p=[fuselage(), canopy(), spinner()]
    for s,nav in ((+1,RED),(-1,GREEN)):
        p.append(wing_panel(s, y=0.11, z_le=0.42, span=1.35, root_c=0.42, tip_c=0.28, thick=0.045, dihedral=0.06, sweep=0.04, nav=nav))
    p += tailplane(); p.append(fin(-0.90, 0.04, 0.36, 0.36, 0.22, 0.30))
    return p

# ------------------------------------------------------------------ quad (X frame, camera nose +Z)
def quad():
    p=[]
    # flattened rounded body
    bz=[0.30,0.26,0.15,0.0,-0.15,-0.26,-0.30]; bw=[0.10,0.15,0.18,0.19,0.18,0.15,0.10]; bh=[0.02,0.05,0.065,0.07,0.065,0.05,0.02]
    p.append(loft([ring(z,w,h,0.0,top_flat=0.3) for z,w,h in zip(bz,bw,bh)], BODY))
    # top plate / battery hump
    p.append(loft([ring(0.20,0.08,0.01,0.06), ring(0.15,0.10,0.04,0.07), ring(-0.15,0.10,0.04,0.07), ring(-0.22,0.08,0.01,0.06)], DARK))
    # nose: camera pod / spike forward
    p.append(loft([ring(0.28,0.07,0.035,0.0), ring(0.42,0.05,0.03,0.005), ring(0.50,0.015,0.015,0.01)], NOSE, cap0=False))
    # arms + motors + prop guards: front pair body colour, rear pair nav (red port / green stbd)
    for sx,sz in ((+1,+1),(-1,+1),(+1,-1),(-1,-1)):
        end=(sx*0.62, 0.0, sz*0.55)
        col = BODY if sz>0 else (RED if sx>0 else GREEN)
        p.append(arm((sx*0.14,0.0,sz*0.16), end, r=0.03, color=BODY))
        p += motor_pod(end[0], 0.0, end[2], r=0.075, color=col if sz<0 else DARK, bell=col if sz<0 else BODY)
        p += prop_guard(end[0], 0.06, end[2], r=0.40, color=col if sz<0 else GUARD)
    return scaled(p, 1.3)

# ------------------------------------------------------------------ tricopter (Y frame, two front arms, one tail arm)
def tricopter():
    p=[]
    bz=[0.30,0.22,0.05,-0.15,-0.30,-0.36]; bw=[0.09,0.14,0.17,0.15,0.10,0.06]; bh=[0.02,0.05,0.07,0.065,0.045,0.02]
    p.append(loft([ring(z,w,h,0.0,top_flat=0.3) for z,w,h in zip(bz,bw,bh)], BODY))
    p.append(loft([ring(0.15,0.08,0.01,0.06), ring(0.10,0.09,0.04,0.07), ring(-0.14,0.09,0.04,0.07), ring(-0.20,0.07,0.01,0.06)], DARK))
    p.append(loft([ring(0.28,0.06,0.035,0.0), ring(0.42,0.045,0.03,0.005), ring(0.50,0.015,0.015,0.01)], NOSE, cap0=False))
    for sx in (+1,-1):   # front arms, nav colours on motors + guards
        end=(sx*0.66, 0.0, 0.40)
        p.append(arm((sx*0.13,0.0,0.12), end, r=0.03))
        nav = RED if sx>0 else GREEN
        p += motor_pod(end[0],0.0,end[2], r=0.075, color=nav, bell=nav)
        p += prop_guard(end[0], 0.06, end[2], r=0.40, color=nav)
    end=(0.0,0.0,-0.95)   # tail arm, longer, with tilt servo block
    p.append(arm((0,0.0,-0.30), end, r=0.03))
    p.append(loft([ring(-0.86,0.05,0.04,-0.01), ring(-0.98,0.05,0.04,-0.01)], DARK))
    p += motor_pod(0.0, 0.03, -0.95, r=0.07, color=DARK, bell=BODY)
    p += prop_guard(0.0, 0.09, -0.95, r=0.40, color=GUARD)
    return scaled(p, 1.3)

# ------------------------------------------------------------------ VTOL (quadplane: plane + 4 lift motors with guards on wing booms)
def vtol():
    p=[fuselage(), canopy(), spinner()]
    for s,nav in ((+1,RED),(-1,GREEN)):
        p.append(wing_panel(s, y=0.11, z_le=0.42, span=1.30, root_c=0.42, tip_c=0.28, thick=0.045, dihedral=0.03, sweep=0.04, nav=nav))
        bx=s*0.62   # boom on the wing, fore and aft
        p.append(loft([ring(0.85,0.03,0.03,0.09,x0=bx), ring(0.70,0.04,0.04,0.09,x0=bx), ring(-0.65,0.04,0.04,0.09,x0=bx), ring(-0.85,0.025,0.025,0.09,x0=bx)], BODY))
        for mz in (0.72, -0.66):
            p += motor_pod(bx, 0.13, mz, r=0.065, color=DARK, bell=BODY)
            p += prop_guard(bx, 0.19, mz, r=0.40, color=GUARD, hub_r=0.07)
    p += tailplane(); p.append(fin(-0.90, 0.04, 0.36, 0.36, 0.22, 0.30))
    return p

# ------------------------------------------------------------------ arrow (generic fallback: classic notched nav-arrow, ridged)
def arrow():
    """Same silhouette as the old placeholder (tip +Z, notched tail), extruded with a centre ridge."""
    L_tip, L_notch, L_rear, W = 0.80, -0.30, -0.65, 0.55
    p=[]
    for s,nav in ((+1,RED),(-1,GREEN)):
        secs=[]
        for f in (0.0, 0.3, 0.6, 0.82, 0.95):
            x=s*W*f; zle=L_tip+(L_rear-L_tip)*f; zte=L_notch+(L_rear-L_notch)*f
            h=0.04*(1-0.5*f); ridge=0.12*(1-f)+0.045*f
            secs.append([(x,h,zle),(x,ridge,(zle+zte)/2),(x,h,zte),(x,-h,zte),(x,-h,zle)])
        secs.append([(s*W, 0.005, L_rear+0.005),(s*W,0.01,L_rear),(s*W,0.005,L_rear-0.005),(s*W,-0.005,L_rear-0.005),(s*W,-0.005,L_rear+0.005)])
        p.append(loft(secs,[BODY,BODY,BODY,nav,nav],cap0=False,cap1=True))
    # yellow nose spike
    p.append(loft([ring(0.55,0.045,0.045,0.02), ring(0.75,0.03,0.03,0.015), ring(0.98,0.004,0.004,0.005)], NOSE, cap0=False))
    return p

MODELS = {"uav-plane": plane, "uav-quad": quad, "uav-tricopter": tricopter, "uav-vtol": vtol, "uav-arrow": arrow}

if __name__=="__main__":
    out = sys.argv[1] if len(sys.argv)>1 else "."; os.makedirs(out, exist_ok=True)
    names = sys.argv[2:] or MODELS.keys()
    for n in names: export(MODELS[n](), os.path.join(out, n+".glb"))
