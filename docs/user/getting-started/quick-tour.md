# Quick tour

A two-minute look at the Kite interface, so you know where everything lives. Don't worry about the
details here — each area has its own guide; this is just the map of the cockpit.

![The Kite interface with its main regions numbered](../assets/ui_overview.png)
/// caption
The main interface, with the numbered regions described below.
///

## The layout at a glance

The window is built from a few fixed regions around a full-screen map:

| # | Region | What it's for |
|---|---|---|
| **1** | **Top bar** | Connecting, plus aircraft / sensor / battery status |
| **2** | **Navigation rail & panel** | The tool rail (left edge) and the panel it opens |
| **3** | **Right widget dock** | Flight instruments down the side |
| **4** | **Bottom widget dock** | Flight instruments along the bottom |
| **5** | **Map controls** | Switch 2D/3D, follow mode, zoom |
| — | **Map** | Fills the background: your aircraft, track, home and mission |
| — | **Status bar** | The thin strip at the very bottom: connection & arming state |

## 1 · Top bar

The command centre. From left to right:

- **Brand & version** — the Kite logo and the running version.
- **Aircraft status (centre)** — the **arming indicator**, a row of **sensor-health tiles** (gyro, acc,
  mag, baro, GPS, and rangefinder/airspeed when fitted — green = OK, amber = warning, red = fault), and
  the **battery** readout. Tiles only appear for sensors your craft actually reports, so the row adapts
  to the airframe.
- **Connection controls (right)** — protocol, transport and port while disconnected; once connected
  they're replaced by the live link status and a **Disconnect** button. See the
  **[Connecting guide](../guides/connecting.md)**.
- **⇅ Relay** — opens the telemetry **[relay/forwarding](../guides/relay-and-forwarding.md)** dropdown.
- **Window controls** — minimise / maximise / close.

## 2 · Navigation rail & panels

The strip down the **left edge** is the navigation rail. Click an icon to open its **panel**; click it
again (or the rail's toggle) to close. Only one panel is open at a time, and the map stays live behind
it. Some tools appear **only when relevant** (e.g. *Control* only on an ArduPilot/PX4 link), so the rail
stays uncluttered.

Each tool, at a glance — expand for a quick description and the link to its full guide:

??? note "UAV Info"
    Flight-controller identity (firmware variant, version, board) and live vehicle status — a quick
    read-out of what you're connected to and how it's doing.

    ![The UAV Info panel](../assets/uav_info.png)

    *Details: [Telemetry & display](../guides/telemetry-and-display.md).*

??? note "Mission"
    Plan a waypoint mission: add, edit and reorder waypoints, set altitudes (incl. AGL /
    terrain-following), generate survey patterns, undo/redo, and upload / download to the aircraft.
    Includes the reusable mission library.

    <!-- SCREENSHOT: ../assets/getting-started/panels/mission.png -->

    *Details: [Missions](../guides/missions.md).*

??? note "Control (ArduPilot / PX4)"
    Send vehicle commands over MAVLink — arm / disarm, change flight mode, take off, RTL, loiter and
    more. Only shown when connected to an ArduPilot or PX4 vehicle.

    <!-- SCREENSHOT: ../assets/getting-started/panels/control.png -->

??? note "RC"
    Fly from the GCS with a gamepad or joystick: map your controller's axes/buttons to RC channels and
    manage control profiles. Only shown when RC control is enabled (and not on a passive telemetry link).

    <!-- SCREENSHOT: ../assets/getting-started/panels/rc-control.png -->

    *Details: [RC control](../guides/rc-control.md).*

??? note "Terrain"
    A terrain-profile analysis of your planned mission or a recorded track — see the ground elevation
    beneath the route and check your above-ground clearance.

    <!-- SCREENSHOT: ../assets/getting-started/panels/terrain.png -->

    *Details: [Missions](../guides/missions.md).*

??? note "Logbook"
    Your flight history: automatic recordings with replay, plus import of INAV blackbox, ArduPilot
    Dataflash, MAVLink `.tlog` and MWPTools raw-MSP logs. Add notes and weather, and reach the **Vehicle**
    and **Battery** managers from here.

    <!-- SCREENSHOT: ../assets/getting-started/panels/logbook.png -->

    *Details: [Logbook](../guides/logbook.md) · [Vehicles](../guides/vehicles.md) · [Batteries](../guides/batteries.md).*

??? note "Radar"
    Foreign-vehicle radar — show nearby ADS-B (and other) traffic on the map, with proximity and
    conflict alerts. Only shown when radar is switched on.

    <!-- SCREENSHOT: ../assets/getting-started/panels/radar.png -->

    *Details: [Radar & ADS-B](../guides/radar-and-adsb.md).*

??? note "Airspace"
    Aeronautical overlays (airports, controlled airspace and obstacles), plus the editors for INAV
    **geozones** and ArduPilot/PX4 **geofences**. Shown when the overlay is on, or when a connected FC
    supports geozones/geofences.

    <!-- SCREENSHOT: ../assets/getting-started/panels/airspace.png -->

    *Details: [Safety](../guides/safety.md).*

??? note "Video"
    Set up and watch a live RTSP video feed alongside (or behind) the map, with one-click map ⇄ video
    swapping.

    <!-- SCREENSHOT: ../assets/getting-started/panels/video.png -->

    *Details: [Video](../guides/video.md).*

??? note "Settings"
    Everything configurable — units, map provider, telemetry rates, flight logging, language, widget
    selection and more.

    <!-- SCREENSHOT: ../assets/getting-started/panels/settings.png -->

    *Details: [Settings reference](../reference/settings.md).*

## 3 & 4 · Widget docks

Two docks hold your **flight widgets** (attitude, altitude, speed, compass, and more): the **right
dock** (3) down the side, and the **bottom dock** (4) along the bottom.

Click the **✎ (edit) button** by a dock to enter **edit mode**, then drag widgets to rearrange them or
move them between docks. Choose which widgets appear in **Settings**. Your layout is remembered between
sessions.

## 5 · Map & its controls

The map fills the whole background and is always interactive — pan and zoom around it even with a panel
open. A small cluster of buttons sits in one **corner of the map** (5):

- **2D / 3D** — switch between the flat moving map and the full 3D globe. (The button shows the mode
  you'll switch *to*.)
- **Follow mode** — cycles **Free → Follow → Heading-up**: free panning, keep the aircraft centred, or
  centre *and* rotate the map to the aircraft's heading.
- **Zoom + / −** — zoom the map (the mouse wheel works too).

The 3D view has more of its own controls — see the **[3D map guide](../guides/map-3d.md)**.

## Status bar

The thin strip along the very **bottom**:

- **Left** — a connection dot (green = connected, red = not) and, once connected, the firmware variant,
  version and port (e.g. *INAV 8.0.0 on COM7*).
- **Right** — the **arming state** (ARMED / DISARMED) while connected.

## Where to go next

- Get linked up: **[Connecting](../guides/connecting.md)**.
- Make sense of the instruments: **[Telemetry & display](../guides/telemetry-and-display.md)**.
- Plan a flight: **[Missions](../guides/missions.md)**.
