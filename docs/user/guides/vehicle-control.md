# Vehicle control

The **Control** tool (navigation rail) commands the aircraft **directly from the ground station** —
arm and disarm, switch flight modes, take off, return or land, fly to a point, adjust the active
flight, and run the loaded mission. It talks to the flight controller over MAVLink and shows you the
FC's own reply to every command.

!!! warning "Read this first — it commands a real aircraft"
    This panel sends **live commands** to a connected vehicle. Buttons that can move or disarm the
    aircraft are deliberately **guarded** (slide- or hold-to-confirm, see [Safety
    interlocks](#safety-interlocks)) — but they still do exactly what they say. Know your aircraft's
    state and have your transmitter ready to take over before you use them.

## Availability

| Stack | Vehicle control |
|---|---|
| **ArduPilot** (Copter, Plane, QuadPlane/VTOL, Rover) | ✓ |
| **PX4** (multirotor, fixed-wing, VTOL) | ✓ |
| **INAV** | not yet — INAV mode changes are done on the transmitter; GCS control is a later phase |

The panel only works over a **MAVLink** connection. Until a MAVLink vehicle is connected it shows
*"Connect via MAVLink (ArduPilot / PX4) to command the vehicle."* What you see inside the panel also
**adapts to the vehicle** — commands that don't apply to your airframe are hidden (see [What each
vehicle gets](#what-each-vehicle-gets)).

## Safety interlocks

These are the heart of the panel — every action that can affect the aircraft is gated so it can't fire
on a stray click.

- **Arm is a slide, not a tap.** Arming uses a **slide-to-ARM** control; disarm is a **hold-to-confirm**.
- **RTL, Land, Abort Landing, Takeoff and VTOL transitions all hold-to-confirm.** A momentary press
  does nothing — you must hold the button until it completes.
- **Force Arm / Force Disarm only appear *after* a refusal.** If the FC rejects a normal arm/disarm
  (for example a failed pre-arm check), a separate **Force** button appears with a longer hold and
  danger styling. It **bypasses the FC's pre-arm checks** — use it only when you understand *why* the
  normal command was refused.
- **Stick-flown modes are locked without an RC source.** Manual / Acro / Stabilize / FBWA and similar
  modes that need live stick input are hidden behind **"Show all modes"** and stay **disabled** until
  there's a usable RC source — either a **physical transmitter** (the FC reports a valid receiver
  signal) **or active [RC control](rc-control.md)** (Kite streaming the sticks to the FC). With
  neither, the panel shows **"No RC link"** so you can't switch into a mode you couldn't actually fly.
- **Set active WP only unlocks when the mission is in sync.** Choosing a waypoint to jump to is
  disabled until the mission on the map matches the FC's (see [Mission control](#mission-control)),
  so the number you pick always maps to the FC's real item.
- **Guided-only adjustments are gated to Guided.** "Change Alt" only appears while the vehicle is in
  the guided / reposition-ready mode; otherwise the altitude is mission- or pilot-controlled.
- **Every command shows the FC's answer.** The footer reports each command as **accepted** or shows the
  FC's **rejection reason** — there's no silent failure.

## Arm and disarm

The top of the panel reflects the live armed state:

- **Disarmed** → a **Slide to ARM** control. If the FC refuses (pre-arm checks), a **Force Arm**
  hold-button appears.
- **Armed** → a **Hold to DISARM** button (plus **Force Disarm** if a normal disarm is refused).

## Flight mode

The current mode is shown large and centred. Below it:

- A grid of **quick modes** for the common GCS-friendly modes of your vehicle.
- **Show all modes** reveals the remaining **stick-flown** modes — selectable **only with an RC source**
  (a connected transmitter, or active RC control — see the interlocks above).

Mode colours match the rest of the app (see the [quick tour](../getting-started/quick-tour.md) legend).

## Takeoff, return and landing

- **Takeoff** (with a target altitude) is offered for **multirotor, VTOL and PX4** vehicles. A
  fixed-wing plane launches via an AUTO mission takeoff or by hand, so the command is hidden there. On
  ArduPilot, Kite switches to Guided first so the FC accepts the takeoff.
- **RTL** returns to the launch/home point.
- **Land** lands in place — hidden for plain ArduPlane fixed-wing (which lands via an AUTO sequence or
  RTL); QuadPlane uses its vertical QLAND path.
- **Abort Landing** (go-around) is offered for **fixed-wing** to wave off an approach and climb out.
- **VTOL transition** (QuadPlane / VTOL only) switches between **hover** and **forward** flight.

## Guided flight ("Fly Here")

Toggle **Guided** on, then **click the map** to send the vehicle to that point at your chosen altitude
and speed. The toggle turns itself off automatically if the FC leaves the guided mode (for example you
pick another mode here or on the transmitter), so the map interaction always matches the FC.

!!! note "Why there's no 'set heading' for planes"
    Set Heading is offered for **ArduCopter in Guided only** (it simply yaws the nose). It is
    **intentionally not exposed for fixed-wing / Cruise**: ArduPlane's guided heading change sets a
    *sticky* heading that bypasses waypoint navigation, isn't cleared by a reposition, and can even
    refuse to descend — which is unsafe to drive from the GCS. To steer a plane from the ground, use
    **Guided "Fly Here"** instead; in **Cruise** the aircraft holds the course you set on the
    transmitter.

## Active-flight adjustments

While flying you can fine-tune the current flight:

- **Change Alt** — repositions to the current position at a new altitude (Guided only; needs a GPS fix).
- **Set Heading** — ArduCopter in Guided only (yaws the nose).
- **Change Speed** — sets the target ground/air speed.
- **Set Loiter Radius** — fixed-wing only (writes the firmware's loiter-radius parameter).
- **Set Home Here** — sets the home position to the vehicle's current location.

## Mission control

Commands the mission already loaded on the flight controller:

- **Download Mission** — pulls the FC's current mission onto the map. This also brings the mission
  **in sync** with the FC, which is what unlocks **Set WP** below. Use it when you've just connected
  and want to command the mission that's already on the aircraft.
- **Start** — begins (or resumes) the mission from the current item.
- **Restart** — rewinds the mission to the first item.
- **Set WP** — jumps the FC to a chosen waypoint. It's **only enabled when the mission is in sync**
  with the FC (just downloaded or uploaded, with no edits since) so the waypoint number maps to the
  FC's actual item. If you've edited the mission on the map, download or upload it again first.

## What each vehicle gets

The panel hides commands that don't apply to the connected airframe:

| Command | Copter | Fixed-wing (Plane) | QuadPlane / VTOL | PX4 |
|---|---|---|---|---|
| Takeoff | ✓ | — (AUTO/manual) | ✓ | ✓ |
| Land | ✓ | — | ✓ (QLAND) | ✓ |
| Abort Landing (go-around) | — | ✓ | ✓ | ✓ |
| VTOL transition | — | — | ✓ | ✓ |
| Set Heading | ✓ (Guided) | — | — | — |
| Set Loiter Radius | — | ✓ | ✓ | ✓ |

## Where to go next

- Plan and upload the mission you'll command here: **[Missions](missions.md)**.
- Fly manually from a gamepad/joystick: **[RC control](rc-control.md)**.
- Keep-in/keep-out areas and return points: **[Safety](safety.md)**.
- Set up the link first: **[Connecting](connecting.md)**.
