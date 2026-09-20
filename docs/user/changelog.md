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
    - **Terrain radar and Live AGL: a blank strip along every terrain-tile edge.** The elevation
      sampler refused the last row and column of each 1° Copernicus tile, so a roughly 30 m wide
      strip along every full degree of latitude and longitude reported no terrain. In the terrain
      radar that strip stayed unpainted, which reads as "terrain far below" rather than "unknown";
      Live AGL and the terrain analysis showed a gap. The sampler now covers the whole tile.
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
