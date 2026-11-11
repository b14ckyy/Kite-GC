# Changelog

What's new in each Kite Ground Control release — the big features up top, the full list of changes
below. The newest release is expanded; click an older version to unfold it. This page is shared by
every released version of the documentation; the **Dev** docs also list what is still being built.

Each version heading ends with the line's support status — **Live**, **Maintenance** or **EOL** — as
defined on the [release support](release-support.md) page. Patch releases (1.0.1, 1.0.2, …) are listed
inside the box of the feature release they belong to, so the notes for one release line stay together.

??? note "1.0 — Initial release · Live"

    The first stable release of **Kite Ground Control**: a cross-platform ground station for
    **INAV**, **ArduPilot** and **PX4** — live telemetry over serial, Bluetooth and network links,
    mission planning on 2D and 3D maps, safety subsystems (safe homes, geozones, geofence), a full
    flight logbook with replay, FPV video, radar / airspace awareness and telemetry relaying.

    Everything this version contains is covered by the regular documentation — start with the
    [quick tour](getting-started/quick-tour.md) or the [GitHub release](https://github.com/b14ckyy/Kite-GC/releases).

    ---

    **1.0.2**{ .kite-patch } *unreleased*{ .kite-badge }

    **Fixed**

    - **ArduPilot / PX4 mission editor: waypoint popup under the side panel.** Selecting a waypoint
      near the left edge of the map moved it under the mission panel instead of into view, and the
      map kept shifting while you edited values. The editor now centres the waypoint in the visible
      map area, the way the INAV tab already did.
    - **MAVLink (ArduPilot / PX4): losing the connection could leave the app stuck on "connected".**
      When the link was really gone — a serial cable unplugged, a socket closed — the status bar
      stayed on "connected" until you clicked Disconnect, and a flight that was being recorded
      never offered the "Device connection lost" recovery prompt (Discard / Save / Continue on
      reconnect). Both now work the way they already do on INAV/MSP.
    - **MAVLink (ArduPilot / PX4): a silent vehicle still showed as a healthy connection.** If the
      aircraft stopped sending data while the link itself stayed open — a radio dropout, for example —
      the status bar kept showing a normal connection instead of "Reconnecting…". It now switches to
      "Reconnecting…" a few seconds after the vehicle goes quiet, and back once data resumes.
    - **Network (UDP) links on Windows could drop unexpectedly.** If the other side of a UDP
      connection closed its socket, Windows reported this as a connection error on the next receive
      and Kite disconnected the whole link, even though the other side might come back on its own.
      The link now stays up and shows "Reconnecting…" instead.
    - **Terrain radar and Live AGL: a blank strip along every terrain-tile edge.** The elevation
      sampler refused the last row and column of each 1° Copernicus tile, so a roughly 30 m wide
      strip along every full degree of latitude and longitude reported no terrain. In the terrain
      radar that strip stayed unpainted, which reads as "terrain far below" rather than "unknown";
      Live AGL and the terrain analysis showed a gap. The sampler now covers the whole tile.
    - **"Unfinished recording found": Save Incomplete and Continue on Reconnect always failed.** After a
      crash or a forced close mid-flight the prompt appeared on the next start, but both ways of keeping
      the recording ended in an error ("no such table: flights") and the temp log stayed where it was;
      only Discard worked. Both actions now recover the flight again. Not affected: the "Device
      connection lost" prompt shown while the app is still running.
    - **A failed geofence, rally or geozone save is now reported.** When writing the fence, the rally
      points (ArduPilot / PX4) or the geozones (INAV) to the flight controller failed — a link hiccup,
      a timeout, an item the FC refused — nothing was shown and the Save button simply came back,
      while what the vehicle actually held might not have matched the panel. The panel now shows the
      error under the Save button.
    - **Two Cancel buttons in a confirmation dialog.** Deleting a vehicle and the survey generator's
      "waypoint limit exceeded" question showed *Cancel* twice. Each dialog now has one.
    - **Relay and GPS fix state: one fix scale for every link.** Passive links (LTM, CRSF, SmartPort)
      reported the fix on a different scale than MSP and MAVLink, so the Relay re-encoded an INAV or
      ArduPilot 3D fix as 2D on its LTM and MAVLink outputs and as "no fix" on SmartPort, the map's
      automatic jump to the aircraft on its first fix never happened on an INAV link (and only with an
      RTK fix on ArduPilot/PX4), and the GPS widget coloured a plain 3D fix amber and labelled a passive
      2D fix "3D". All links now use 0 none / 1 2D / 2 3D / 3 DGPS-RTK, and the Relay maps it correctly
      for every output. One visible consequence on passive links: a 2D fix now counts as "no 3D fix"
      everywhere, the same as on MSP and MAVLink — no home is set for that flight until the fix is 3D,
      and ADS-B conflict alerts and the radar distance reference wait for it. Also fixed on the way: the
      2D map no longer jumps to the aircraft when you framed a mission in the meantime, and the
      FormationFlight radar answer reports the fix on INAV's scale.
    - **Your location is no longer taken from the aircraft.** The first 3D fix of a connection was
      stored as your own position and, in the GCS modes without an OS fix, moved the GCS marker to the
      aircraft — on a reconnect in flight, to an airborne aircraft. Now, while your own position is
      unknown, the aircraft's home position stands in for the GCS location; a manually set GCS
      position and a live OS position keep precedence.
    - **ArduPilot flight-log import with two GPS receivers: zig-zag track.** Logs (.bin) from an
      aircraft with a second GPS receiver were imported with the positions of both receivers mixed
      into one track, so the track was jagged and the HDOP, satellite count and raw GPS altitude
      jumped back and forth during replay. The import now follows the receiver the flight controller
      was actually using, and switches with it if the autopilot changed receivers mid-flight.
    - **`.tlog` import: a log that carries several vehicles is no longer mixed into one flight.** When
      a MAVLink log held frames of more than one aircraft on the same link, their positions and
      attitudes ended up interleaved in a single track. The import now follows the first autopilot it
      sees and skips the others.
    ---

    **1.0.1**{ .kite-patch } *2026-09-13*{ .kite-badge }

    **Added**

    - **Community link in About.** The About dialog links to the Kite Discord
      (https://discord.gg/3FM7EWhkg9) next to the source repository. [#163]
    - **Privacy policy in About.** The About dialog links to the published privacy policy, so it
      is reachable from inside the app. [#160]

    **Fixed**

    - **The 3D live view stuttered more the longer a flight went on.** The live trail was rebuilt
      on every frame, so its cost grew with the length of the track, and the hidden 2D map kept
      re-centring and drawing the aircraft underneath. Both now cost the same on every frame. [#140]
    - **"Incomplete recording found" came back on every start.** When unfinished temp logs had piled
      up (the app killed mid-recording, for example by the operating system), the prompt offered them
      one per launch and Discard removed only that one. Discard now clears every leftover, and a
      recording in progress is never touched. [#145]
    - **HDOP on INAV showed the wrong figure.** The GPS tile read the position-error field instead
      of HDOP, because the first field of INAV's GPS statistics message is 16 bits and Kite decoded
      it as 32. The recorder stored the same two values one field out. [#143]
    - **"Fly Here" opened with an empty radius field** on fixed wing, which reads as "no radius"
      while the vehicle would in fact use its configured one. The field now shows the aircraft's own
      loiter radius. Leave it alone and the aircraft keeps using that setting, turn direction
      included; type a value and yours wins. [#143]
    - **The radius stepper was clipped** by the edge of the "Fly Here" popup, so its "+" button
      could not be reached. [#143]
    - **A saved Telemetry connection came back as MSP.** Restoring the last-used protocol mapped
      everything that was not MAVLink onto MSP, so the passive Telemetry choice was silently
      rewritten. [#143]
    - **Bulgarian translation corrected** — 26 strings, contributed by teodoryantcheff. [#141]
    - **Markers and pop-ups turned with the map in heading-up mode.** Waypoint markers, labels,
      the "Fly Here" pop-up and the waypoint editor rotated together with the map tiles, so they
      were unreadable while the map followed the aircraft's heading. They now stay bound to their
      position but upright; the aircraft symbol and radar contacts keep pointing along their
      track. [#168]
    - **The 3D view was stuck in daylight.** Since 1.0.0 the globe and the sky ignored the real
      sun: the terrain stayed bright and the sky blue wherever you looked and at any time of day,
      and a replay's flight time changed nothing either. The lighting that keeps the aircraft model
      readable had taken over the whole scene's light. The model keeps its own lighting; the globe,
      the sky and the day/night line follow the real sun again. [#175]

[#140]: https://github.com/b14ckyy/Kite-GC/pull/140
[#141]: https://github.com/b14ckyy/Kite-GC/pull/141
[#143]: https://github.com/b14ckyy/Kite-GC/pull/143
[#145]: https://github.com/b14ckyy/Kite-GC/pull/145
[#160]: https://github.com/b14ckyy/Kite-GC/pull/160
[#163]: https://github.com/b14ckyy/Kite-GC/pull/163
[#168]: https://github.com/b14ckyy/Kite-GC/issues/168
[#175]: https://github.com/b14ckyy/Kite-GC/pull/175
