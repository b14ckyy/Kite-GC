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
  modes that need live stick input are folded away under **RC modes** and stay **disabled** until
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
- The **RC modes** bar below the grid folds out the **stick-flown** modes — selectable **only with an
  RC source** (a connected transmitter, or active RC control — see the interlocks above). While there
  is none, the bar itself says **No RC link**.

Mode colours match the rest of the app (see the [quick tour](../getting-started/quick-tour.md) legend).

**ArduPilot 4.7 and newer tell Kite which modes they have.** After connecting, Kite asks the flight
controller for its mode list and offers only modes from it: a mode that is not built into your
firmware, or that the firmware has switched off (Follow without its parameter enabled, for example),
is not shown. Kite keeps its own mode names and the RC lock. A mode Kite does not know — one added by a
Lua script, for example — appears under **RC modes** with the name the flight controller gives
it, and needs an RC source. Older ArduPilot versions and PX4 get Kite's built-in list.

Not every mode of the list becomes a mode button: Guided, Takeoff, RTL, Land, ArduPlane's QRTL,
QLand, Loiter QLand and AutoLand and ArduSub's Surface have their own controls further up (see [Takeoff, return and
landing](#takeoff-return-and-landing)), and modes that only the firmware itself enters (Avoid ADSB,
Initialising) or that are for diagnostics (ArduSub's Motor Detection) are never offered. The mode
buttons are ordered: ordinary flight modes first, then — as hold-to-confirm buttons — the remaining
ones that bring the vehicle back: **SmartRTL** (Copter, Rover) and **Auto RTL** (Copter).
If the flight controller's list has no Guided mode, the **Guided** toggle is greyed out.

## Takeoff, return and landing

- **Takeoff** (with a target altitude) is offered for **multirotor, VTOL and PX4** vehicles. On
  ArduPilot, Kite switches to Guided first (when the vehicle offers that mode) so the FC accepts
  the takeoff.
- **Takeoff on an ArduPilot fixed-wing plane** switches the plane to its **TAKEOFF mode**. There is no
  altitude field: the plane climbs to the altitude of its `TKOFF_ALT` parameter and then loiters.
  Pressed while the plane is disarmed, the mode waits for you to arm. It is meant for the launch:
  pressed in established flight the plane loiters (after climbing to that altitude if it is below
  it), but in the first seconds after launch, or with almost no ground speed, it starts a full
  takeoff climb from where it is.

!!! warning "In TAKEOFF mode the motor starts when you arm"
    ArduPlane does not wait for the throttle stick in TAKEOFF mode. With the firmware's default
    parameters the motor runs up about 0.2 seconds after arming, without waiting for a throw. While
    the plane sits disarmed in TAKEOFF mode, the panel shows a warning line under the arm slider.
    Hold the plane ready for launch before you arm.

- **RTL** returns to the launch/home point. Not on ArduSub, which has no such mode.
- **Land** lands in place (Copter, PX4). A rover or boat has no Land button.
- **ArduSub** — instead of RTL and Land there is **Surface**: the vehicle rises to the surface.
- **ArduPilot QuadPlane** — the return and landing modes sit here in two pairs:
    - **RTL** and **QRTL**: RTL is the fixed-wing return; QRTL returns to home (or a rally point)
      and lands vertically.
    - **Loiter QLand** and **QLand**: QLand lands vertically right where the aircraft is. Loiter
      QLand is for arriving high in forward flight: the aircraft circles down to the altitude of
      `Q_RTL_ALT` and then lands vertically; in hover it lands at once.
- **ArduPilot fixed-wing plane** — next to RTL there is **AutoLand** when the firmware has it
  (ArduPilot 4.7 and newer): an automatic landing at home in the takeoff direction. It works only in
  flight, and the flight controller must have captured the takeoff direction (it does so at takeoff,
  once it has enough ground speed); otherwise it refuses and the panel shows its answer. Older firmware has no
  land-now command for a plane (it lands through an AUTO sequence or RTL), so the place stays empty.
- **Abort Landing** (go-around) is offered for **fixed-wing** to wave off an approach and climb out.
- **VTOL transition** (QuadPlane / VTOL only) switches between **hover** and **forward** flight.

## Guided flight ("Fly Here")

Toggle **Guided** on, then **click the map** to send the vehicle to that point at your chosen altitude
and speed. The toggle turns itself off automatically if the FC leaves the guided mode (for example you
pick another mode here or on the transmitter), so the map interaction always matches the FC.

You don't have to press the toggle first if the vehicle is **already** in its guided mode — after a
mid-flight connect, or when another ground station put it there. Clicking the map works straight away
in that case, so Kite never has to re-send a mode change just to let you command a point. (While a
mission editor or the survey generator is open, those own the map click instead.)

**What you see on the 2D map:** the target is marked where the flight controller says it is, not merely
where you clicked — Kite reads it back from the FC once per second, so a target set or moved by another
ground station shows up too. For a **fixed-wing** vehicle a dashed green circle around it draws the
loiter track the aircraft will actually fly: the radius you set for this Fly Here, or the firmware's
configured default (`WP_LOITER_RAD` / `NAV_LOITER_RAD`) when you didn't set one. A copter simply holds
the point, so it gets no circle. Both disappear when the FC leaves guided or the link drops.

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
| Takeoff | ✓ | ✓ (TAKEOFF mode) | ✓ | ✓ |
| Land | ✓ | AutoLand (ArduPilot 4.7+) | QLand / Loiter QLand | ✓ |
| Abort Landing (go-around) | — | ✓ | ✓ | ✓ |
| VTOL transition | — | — | ✓ | ✓ |
| Set Heading | ✓ (Guided) | — | — | — |
| Set Loiter Radius | — | ✓ | ✓ | ✓ |

## Where to go next

- Plan and upload the mission you'll command here: **[Missions](missions.md)**.
- Fly manually from a gamepad/joystick: **[RC control](rc-control.md)**.
- Keep-in/keep-out areas and return points: **[Safety](safety.md)**.
- Set up the link first: **[Connecting](connecting.md)**.
