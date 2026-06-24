- better "new waypoint" action for usability instead of hard default values in settings only.

- PERF (only if real problems, e.g. mobile iGPU at its limit): render distant ADS-B 3D contacts as batched billboard sprites instead of glb models, full model only near/selected — flattens the one-time pool warm-up + steady frame time. The entity pool already removed the recurring stalls; this is further headroom, not needed now.

- UDP link via Mavlink not working but TCP fine
- Make the Battery selector in post flight a dropdown (Maybe combined with search field to keep both usage options) 
- User feedback:
> In the serial option, the COM port selection... is it possible to add an addition description to the COM ports which are for bluetooth devices, so I'd know which ones are the bluetooth ones? MP has that and it makes it so much easier, especially in my case where I have like 50 bluetooth COM ports for all the various gear!
- RTSP video input on ToDo to test with DJI PC capture software
- add popup if Cesium key is missing when first entering 3D view with option to enter, ignore or remind later
- make terrain radar altitutde range and radar type persistent
