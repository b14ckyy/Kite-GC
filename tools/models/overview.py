# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marc Hoffmann (b14ckyy)

"""Labelled model overview sheets (top + side view per model) for the docs.
Run: uv run --with pillow --with numpy python overview.py <scratch dir> <out.png> <title> name=file[=caption] ..."""
import sys, numpy as np
from PIL import Image, ImageDraw, ImageFont
scratch = sys.argv[1]
src = open(scratch + "/render_topdown.py").read().split("files = sys.argv[3:]")[0]
ns = {"__name__": "overview"}; sys.argv_backup = sys.argv; sys.argv = [sys.argv[0], scratch]; exec(src, ns); sys.argv = sys.argv_backup
CELL = ns["CELL"]; ns["MODEL_RADIUS"] = 1.1  # sheet scale: the longest model (tricopter, glider) must stay inside its cell
out_path, title = sys.argv[2], sys.argv[3]
items = []
for spec in sys.argv[4:]:
    parts = spec.split("=", 2)
    items.append((parts[0], parts[1], parts[2] if len(parts) > 2 else ""))
cols = min(6, len(items)); rows = (len(items) + cols - 1) // cols
LABEL = 62; PAD = 16; TITLE = 44
W = PAD + cols * (CELL + PAD); H = TITLE + rows * (2 * CELL + LABEL + PAD) + PAD
img = np.full((H, W, 3), 46, dtype=np.uint8)
for i, (name, path, cap) in enumerate(items):
    r, c = divmod(i, cols)
    ox = PAD + c * (CELL + PAD); oy = TITLE + r * (2 * CELL + LABEL + PAD)
    img[oy:oy + 2 * CELL, ox:ox + CELL] = 58
    for k, view in enumerate(("top", "side")):
        ns["render"](path, img, ox, oy + k * CELL, view)
pil = Image.fromarray(img)
d = ImageDraw.Draw(pil)
def font(size, bold=False):
    for f in (["segoeuib.ttf", "arialbd.ttf"] if bold else ["segoeui.ttf", "arial.ttf"]):
        try: return ImageFont.truetype(f, size)
        except OSError: pass
    return ImageFont.load_default()
F_T, F_N, F_C = font(22, True), font(17, True), font(13)
d.text((PAD, 10), title, fill=(230, 232, 236), font=F_T)
for i, (name, path, cap) in enumerate(items):
    r, c = divmod(i, cols)
    ox = PAD + c * (CELL + PAD); oy = TITLE + r * (2 * CELL + LABEL + PAD) + 2 * CELL
    d.text((ox + 8, oy + 6), name, fill=(235, 237, 240), font=F_N)
    d.text((ox + 8, oy + 28), path.split("/")[-1], fill=(150, 154, 160), font=F_C)
    if cap: d.text((ox + 8, oy + 44), cap, fill=(150, 154, 160), font=F_C)
    d.text((ox + CELL - 46, TITLE + r * (2 * CELL + LABEL + PAD) + 6), "top", fill=(120, 124, 130), font=F_C)
    d.text((ox + CELL - 46, TITLE + r * (2 * CELL + LABEL + PAD) + CELL + 6), "side", fill=(120, 124, 130), font=F_C)
pil.save(out_path); print("wrote", out_path, pil.size)
