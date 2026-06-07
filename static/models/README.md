# UAV 3D models

Low-poly glb models for the **own UAV** on the maps: Cesium glTF in 3D ([Map3D.svelte](../../src/lib/components/Map3D.svelte))
and a software top-down renderer in 2D ([uavTopDown.ts](../../src/lib/helpers/uavTopDown.ts)) — **one
glb is the single source of truth for both.** Resolved by platform/override in
[uavModels.ts](../../src/lib/helpers/uavModels.ts).

| File                  | Kind       | Platform                |
|-----------------------|------------|-------------------------|
| `uav-quad.glb`        | quad       | multirotor              |
| `uav-tricopter.glb`   | tricopter  | tricopter               |
| `uav-plane.glb`       | plane      | fixed-wing              |
| `uav-vtol.glb`        | vtol       | VTOL                    |
| `uav-arrow.glb`       | generic    | fallback / unknown      |

> The **radar / ADS-B** contact models live separately in [`radar/`](radar/) (own colour rules — see
> that folder's README). The guidelines below are for the **own-UAV** models.

## Authoring guidelines

**Orientation / frame** (authored glb axes):
- **Nose = +Z** (forward / flight direction) · **up = +Y** · **port (left) = +X**, **starboard = −X**.
- Cesium's Y-up→Z-up load + the orientation math are handled in code — model in the frame above and
  heading/pitch/roll come out right in both 2D and 3D.

**Spike in flight direction:** put a clear pointed nose / spike toward **+Z** so the heading reads
unambiguously from straight above (top-down 2D view).

**Red–green rule (aviation nav lights):** **red on the port (left, +X)** side, **green on the starboard
(right, −X)** side. This makes left/right — and therefore orientation — readable at a glance.

**Body colour & colour mixing:** the 3D model is tinted by the **flight-mode colour** via
`colorBlendMode = MIX` with `colorBlendAmount = 0.2` — i.e. ~20 % mode tint over ~80 % of the model's own
colours. So:
- Give the body a **neutral, recognisable base colour** (the mode tint only nudges it).
- Keep the **red/green nav markers and the nose spike** as distinct colours so they stay readable through
  the tint.
- The 2D top-down renderer uses the glb's `baseColorFactor` colours directly (flat-shaded), so the same
  colours serve both views.

**Size:** drawn with `minimumPixelSize` 73 + `scale` 5.2 on the map — model at a small real-world size;
keep it centred on the origin.
