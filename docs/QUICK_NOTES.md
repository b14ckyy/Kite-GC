> better "new waypoint" action for usability instead of hard default values in settings only.
> BUG: earth can be covered by widgets when zoomed far out and hard to get back into control especially on touchpads
> PERF (only if real problems, e.g. mobile iGPU at its limit): render distant ADS-B 3D contacts as batched billboard sprites instead of glb models, full model only near/selected — flattens the one-time pool warm-up + steady frame time. The entity pool already removed the recurring stalls; this is further headroom, not needed now.
