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
- **ESRI Dark Gray** — a dark, low-contrast basemap.

All of them work **offline** once the tiles have been cached — see below.

You can also bring your own maps (see **[Your own maps](guides/custom-maps.md)**):

- **Custom online providers** — any online tile service you add by its URL template, with your own API key in
  the URL if the service needs one. On every platform; the tiles are cached like those of the
  built-in maps.
- **Custom maps** — map tiles from a folder on disk, on Windows, Linux and macOS.

### Can I use the maps offline?

Yes, within limits. Kite caches every map tile it downloads (**[Settings → Data → Map →
Tile Cache](reference/settings.md)**). Pan and zoom over your flying area **while you still have an
internet connection**, and those tiles stay available offline afterwards. There is **no bulk
"pre-download this whole region" button** — the cache fills from what you actually view. The default
cache is 200 MB and can be raised up to 5 GB. The same holds for your
**[custom online providers](guides/custom-maps.md#custom-online-providers)**.

On the desktop there is a second way: map tiles you already have can be put into a folder and shown as
a **[custom map](guides/custom-maps.md#custom-maps-from-a-folder)**, straight from disk and without any
connection. Kite does not download those tiles for you.

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
worth it for a map style you can already get, key-free, from the built-in providers. A service you
have a key for can be added yourself as a **[custom online provider](guides/custom-maps.md#custom-online-providers)**,
by its tile URL template — whether its terms allow that use, including Kite's tile cache, is yours to
check.

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
set **[Settings → Alerts → System messages](reference/settings.md)** to *Warning* (the default) —
debug-severity texts such as this one only appear at *Debug*.

## Flight controller setup

### Can Kite edit flight controller parameters?

Only the ones that belong to a mission or to its safety net. Kite is a **mission planner, mission
manager and flight monitor** — not a configurator. Setting up the aircraft itself (ports, receiver,
modes, tuning, sensors, outputs) stays with the tool made for your firmware: **INAV Configurator** for
INAV, **Mission Planner** or **QGroundControl** for ArduPilot and PX4.

What Kite does write to the flight controller is everything you plan on the map:

- **Missions** — waypoints and their actions, on INAV, ArduPilot and PX4 ([Missions](guides/missions.md)).
- **INAV geozones** — each zone with its altitude band and breach action
  ([Safety → Geozones](guides/safety.md#inav-geozones)).
- **INAV safe homes and autoland** — the safe-home slots, their approach settings and the global
  autoland configuration ([Safety → Safe Home & Autoland](guides/safety.md#automatic-landing-safe-home-autoland-inav)).
- **ArduPilot / PX4 geofence** — the fence geometry plus the core fence parameters such as enable,
  breach action and altitude / distance limits ([Safety → Geofence](guides/safety.md#ardupilot-px4-geofence)).
- **ArduPilot / PX4 rally points** — the points and their parameters
  ([Safety → Rally points](guides/safety.md#rally-points-ardupilot-px4)).
- A few single settings where they belong to a workflow: the **loiter radius** from
  [Vehicle control](guides/vehicle-control.md) on ArduPilot / PX4, and the INAV **craft name** from the
  [Vehicle library](guides/vehicles.md), so that future flights link to the right vehicle.

Nothing is written in the background: a change reaches the flight controller only when you save it or
press the matching button, and only the settings that editor shows.

A general **parameter editor** — the full list of every firmware setting — is not ruled out for the
future, but it sits far down the priority list. Each firmware already has a dedicated configurator that
does this well, and a second, half-complete one inside a ground station adds risk without adding much.
