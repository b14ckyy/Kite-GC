# Frequently asked questions

## Maps

### Which map providers can I use?

Kite ships with several free map sources — no account or API key required. Pick one under
**[Settings → Interface → Map → Tile Provider](reference/settings.md)**; the choice applies to **both
the 2D map and the 3D globe**:

- **OpenStreetMap** — the standard street map.
- **ESRI World Street Map** — an alternative street map.
- **ESRI World Imagery** — satellite / aerial imagery.
- **ESRI Hybrid** — satellite imagery with street and place labels on top (the default).
- **OpenTopoMap** — topographic map with contour lines.
- **CartoDB Dark Matter** — a dark, low-contrast basemap.

All of them work **offline** once the tiles have been cached — see below.

### Can I use the maps offline?

Yes, within limits. Kite caches every map tile it downloads (**[Settings → Data → Map →
Tile Cache](reference/settings.md)**). Pan and zoom over your flying area **while you still have an
internet connection**, and those tiles stay available offline afterwards. There is **no bulk
"pre-download this whole region" button** — the cache fills from what you actually view. The default
cache is 200 MB and can be raised up to 5 GB.

### Why isn't Google Maps or HERE Maps available?

This comes up because other ground stations (Mission Planner, QGroundControl) show Google satellite
imagery. We looked at both Google and HERE carefully and deliberately chose **not** to integrate them.
The short version: for a ground control station the downsides outweigh a marginal difference in map
style, and the satellite imagery is **not actually better** than what Kite already offers.

**Google Maps.** The way other apps show Google imagery relies on **undocumented tile endpoints** that
Google's Terms of Service explicitly forbid accessing outside the official Maps Platform. It works only
because Google doesn't bother enforcing it against small projects — it can be blocked at any time, and
it is not something we want to build into an open-source application. The **official** Google Maps Tiles
API, by contrast, requires every user to create a billing-enabled API key, forbids caching imagery for
offline use, and mandates Google branding that can't be removed. Neither path fits an offline-capable
UAV tool.

**HERE Maps.** HERE requires a per-user developer account and API key, its Acceptable Use Policy
restricts "high-risk" use, and — like Google — it does not permit pre-caching tiles for offline use.
On top of that, a direct comparison showed **no quality benefit**: HERE's satellite tiles are the same
mid-zoom imagery as ESRI, it lacks the highest zoom levels ESRI provides, and its low-zoom imagery is
the older Bing basemap that ESRI already serves in better quality.

In both cases the extra setup (account, API key, billing) and the licensing constraints simply aren't
worth it for a map style you can already get, key-free, from the built-in providers. If a specific,
properly licensed provider ever adds real value, we would add it as an optional *bring-your-own-key*
source rather than route around anyone's terms of service.

## Connections

### Why does Kite keep showing a firmware message that Mission Planner doesn't?

When Kite is connected through **another ground station's MAVLink forward** (Mission Planner's
forward or mirror, for example), it sees everything the aircraft sends — including replies to
requests the *other* ground station made. ArduPilot broadcasts its status texts to every connected
ground station, and Kite shows them as notifications; Mission Planner keeps the low-severity ones in
its Messages tab, so the same line can look like a Kite problem while it is a conversation between
Mission Planner and the firmware.

A typical example: **"Sending unknown message (44)" twice a second**. Mission Planner asks the
aircraft for a camera message (`CAMERA_FOV_STATUS`) at its 2 Hz status rate; on ArduPilot 4.5 and
the early 4.6 betas a board whose firmware build has no camera field-of-view support (small-flash
F405-class boards, typically) accepts the request and then complains at that rate, at its *debug*
severity. Connected directly to the aircraft, without Mission Planner in the loop, the line never
appears.

Such messages are harmless. **Update the aircraft to ArduPilot 4.6 or newer** — the current
firmware refuses a request it cannot serve once instead of repeating the complaint — and keep the
other ground station up to date as well. To keep the notification banner quiet in the meantime,
set **[Settings → Alerts → System messages](reference/settings.md)** to *Warning*.
