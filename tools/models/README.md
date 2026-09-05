# 3D model pipeline

Procedural low-poly models for `static/models/` (own UAV) and `static/models/radar/` (ADS-B / radar
contacts). Everything runs with [uv](https://docs.astral.sh/uv/); no project-level Python setup.

| Script               | Purpose |
|----------------------|---------|
| `kitemodels.py`      | Shared primitives (lofts, wings, prop guards) and the colour constants (`BODY`, `GUARD`, `RED`, `GREEN`, …). |
| `generate_uav.py`    | Own-UAV models: `uav-plane`, `uav-quad`, `uav-tricopter`, `uav-vtol`, `uav-arrow`. |
| `generate_adsb.py`   | Radar models: `adsb-light` … `adsb-dot`, `ff-uav`. |
| `fix_glb.py`         | Post-process one `.glb` for Kite: consistent outward winding, vertex normals, uniform scale. |
| `glb_u16.py`         | Rewrite index accessors to uint16 (Kite's 2D loader reads `Uint16Array`). |
| `glb_info.py`        | Print extents, nav-light positions and node data of `.glb` files (orientation check). |
| `render_topdown.py`  | Raster preview (top + side) with the same maths as the 2D top-down renderer. |
| `overview.py`        | Labelled overview sheets for the user docs. |

## Regenerate the set

```sh
cd tools/models
uv run --with trimesh --with numpy --with scipy python generate_uav.py out
uv run --with trimesh --with numpy --with scipy python generate_adsb.py out/radar
for f in out/*.glb out/radar/*.glb; do
  uv run --with trimesh --with numpy --with scipy python fix_glb.py 0.66 "$f" "$f.fixed"
  python glb_u16.py "$f.fixed" "$f"        # numpy only
  rm "$f.fixed"
done
cp out/uav-*.glb ../../static/models/ && cp out/radar/adsb-*.glb ../../static/models/radar/
cp ../../static/models/uav-arrow.glb ../../static/models/radar/ff-uav.glb   # FF peers = generic arrow
```

The scale factor 0.66 puts the generated size into the class the renderers expect (`MODEL_RADIUS` in
`uavTopDown.ts`, `minimumPixelSize`/`scale` in `Map3D.svelte`). Frame convention and the loader's
file-format rules: `static/models/README.md`.

## Check and preview

```sh
python glb_info.py ../../static/models/uav-plane.glb              # extents, red/green centroids
python render_topdown.py . preview.png ../../static/models/uav-*.glb
uv run --with pillow --with numpy python overview.py . uav.png "Title" "Fixed-wing=../../static/models/uav-plane.glb=caption" …
```

`overview.py` produced `docs/user/assets/guides/map-3d/uav_models.png` and
`docs/user/assets/guides/radar/adsb_models.png`.
