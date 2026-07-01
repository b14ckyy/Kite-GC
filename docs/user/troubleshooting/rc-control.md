# RC control isn't reaching the aircraft

Your device shows up and the channels move in the **RC control** panel, but the aircraft doesn't respond
— or a servo only **twitches** when you move an axis instead of holding the commanded position. Work
through the checks below. For how RC control works in the first place, see the
**[RC control guide](../guides/rc-control.md)**.

!!! warning
    GCS stick control is a safety-critical feature. Test on the bench with props off, and keep a way to
    recover (a physical transmitter, or the FC's failsafe).

## Nothing moves at all

- **Did you long-press "Take control"?** Streaming never starts automatically — it begins only on the
  long-press engage and stops the moment you release it or the app loses focus (the deadman).
- **Is the mapping set?** A channel with no axis/button assigned sends nothing. Check the per-channel
  mapping in the panel.
- **Watch the engage state text** in the panel — it tells you whether your sticks are actually being sent
  (see the firmware-specific notes below).

## ArduPilot: a servo twitches but doesn't hold

This is the classic sign of **a physical RC receiver competing with the GCS override** — for example an
mLRS (or ELRS) module feeding **CRSF into a second UART** while Kite sends `RC_CHANNELS_OVERRIDE` over the
MAVLink link. ArduPilot applies the override **only while it keeps arriving and while it's allowed to**;
the moment it doesn't, the physical RC (sticks centred) reasserts — so you get a twitch on movement rather
than steady control. Kite is sending correctly (continuous stream, standard GCS system ID 255); the fix is
on the ArduPilot side. Check, in order:

1. **`RCx_OPTION = 46` ("RC Override Enable").** If you assigned this to a switch, it can **disable**
   MAVLink overrides depending on its position. This is the most common cause. Remove it (set the option
   back to `0`) or make sure the switch is in the enable position.
2. **`RC_OPTIONS`** — the **"Ignore MAVLink Overrides"** bit must **not** be set.
3. **`SYSID_MYGCS` (newer firmware: `MAV_GCS_SYSID`) = `255`.** ArduPilot only accepts overrides from its
   configured GCS system ID; Kite uses **255** (the GCS standard, and ArduPilot's default). If you changed
   it, set it back or match it. On newer firmware also check `MAV_OPTIONS`.
4. **`RC_OVERRIDE_TIME`** — default **3 s** is fine. As a test, set it to **`-1`** (never time out). If the
   servo now **holds**, the override was arriving intermittently (the link can't carry the RC rate on top
   of telemetry) → **lower the RC rate** in the panel until it's steady, then restore `RC_OVERRIDE_TIME`.
5. **Isolation test:** temporarily **unplug the physical receiver** (the CRSF/RC UART). If Kite then
   controls cleanly, the receiver was the competitor and one of the settings above is the real fix.

## INAV: only AUX works, sticks do nothing

With a normal receiver fitted, INAV ignores your CH1–16 sticks unless **`MSP RC OVERRIDE`** mode is
**active**:

- The panel shows *"Override inactive — AUX only"* vs *"MSP RC OVERRIDE active — controlling CH1–16"*.
  Map an **AUX switch to `MSP RC OVERRIDE`** (or drive it from the GCS) and turn it on.
- INAV also keeps an **override bitmask** of which channels MSP may override. If some of your mapped
  channels aren't included, use the panel's **Set override bitmask** button (applied at runtime).

If the receiver is set to **MSP with no other radio**, the GCS *is* the receiver — no override switch is
needed and it takes over fully.

## PX4: joystick input is ignored

- **`COM_RC_IN_MODE`** must allow a MAVLink/joystick source (not "RC only"). Kite shows a reminder, but
  set the parameter in PX4 / QGroundControl.

## Still stuck

Grab the in-app diagnostics log (**Settings → Diagnostics**) with the RC panel open and engaged, and
include your autopilot, firmware version and receiver setup when
**[reporting the problem](reporting-issues.md)**.
