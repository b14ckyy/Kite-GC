# Changelog

What's new in each Kite Ground Control release — the big features up top, the full list of changes
below. The release you are reading the docs for is expanded; click an older version to unfold it.

???+ note "1.1.0 — in development"

    **Highlights**

    ??? info "Kite goes mobile — Android, iPhone and iPad"
        Kite now runs natively on **Android** tablets and phones (USB serial, Bluetooth LE and Wi-Fi
        links, touch-friendly layout, shared storage folders) and on **iPhone / iPad** (native build
        with phone and tablet layouts, touch RC, BLE and Wi-Fi MAVLink — contributed by Sebastian
        Kumor). Features a device can't provide are hidden by capability, so the mobile app stays
        clean instead of half-working. [#79] · [#16]

        **Phones got their own interface.** In landscape the map fills the whole screen: no bars, a
        chain-link button opens the connection pop-out, and the instruments live in a frosted-glass
        **widget column** on the right edge with two swipeable pages — the map keeps running
        underneath it. Long-press a widget to rearrange or resize it; the others shift live to show
        where they will land. Pinch to zoom, 2D/3D and follow next to the column. [#102]

        **Video on the phone** sits in a docked window at the bottom-right of the map — a camera
        button parks it off-screen and brings it back instantly — or in the Video widget of the
        column; double-tap swaps map and video. [#105] The 3D view centres on the visible map area
        next to the column. [#103] · [#111]

        **The link keeps running in the background.** While connected, Android shows a notification
        with the aircraft, battery, flight mode and distance to home; minimising Kite or switching
        apps no longer stops telemetry or the recording, and the trail catches up when you come
        back. [#111]

    ??? info "Native RTSP video client with hardware decode"
        RTSP video no longer needs the external video engine: Kite ships its **own RTSP client** and
        decodes H.264/H.265 with the **operating system's hardware decoder** on Windows, Android and
        Linux — lower latency, a fraction of the CPU load, and rock-solid reconnects. [#85] · [#88] · [#89]

    ??? info "Telemetry API — live telemetry for other programs"
        Kite can now **serve its live telemetry as JSON** to anything that can read it: an NDJSON
        stream over TCP (port 27300), an HTTP snapshot (port 27301) and UDP to a target of your choice.
        Read-only, everything the Raw telemetry popup shows — position, attitude, battery, link, flight
        mode, sensors, home and the ground-station position — the same for INAV, ArduPilot, PX4 and
        passive links, and it keeps running in the background, also on Android. Loopback by default,
        one switch for the network. The full contract with every field and copy-paste examples is in
        the **[Telemetry API](reference/telemetry-api.md)** reference. [#123]

    ??? info "Unobstructed fullscreen video"
        Fullscreen video now keeps its exact aspect ratio in a clean box, with a blurred, slowly
        following map as the backdrop instead of black bars — and the floating panels stay on top
        where they belong. [#90]

    ??? info "High-resolution replay and instant log preview"
        The replay player gained a **HI-RES** switch: the stored onboard log is re-decoded at its
        **full rate** and drives the horizon, instruments and stick overlay at your screen's refresh
        rate, with new **slow-motion speeds** (0.25× / 0.5×) to study a manoeuvre frame by frame.
        And you no longer need to import a log to look at it — **drop it on the map** (or use the
        Logbook's new **Open** button) while disconnected, replay it exactly like a logbook flight,
        and import it for real only if you want to keep it. [#97] · [#99]

    ??? info "Resizable widgets and a raw telemetry view"
        Every flight widget now has **two or three sizes**: in the dock's edit mode a corner button
        steps it from small to large (and, for the wide Live AGL and Video tiles, to a square).
        The docks shrink to their largest widget, so a row of small tiles hands the freed space back
        to the map or the fullscreen video. The compass and horizon picked up the frosted-glass look
        of the other widgets, and the old Raw Telemetry widget gave way to a **≡ Raw** button in the
        top bar: one popup with **every** value the telemetry link delivers, in its raw unit. [#101]

    **Added**

    - **Android support** — native app with USB serial, Bluetooth LE and Wi-Fi links, touch layout
      and scoped storage folders. [#79]
    - **iPhone & iPad support** — native iOS/iPadOS build: phone/tablet layout, touch RC, BLE,
      Wi-Fi MAVLink. Contributed by Sebastian Kumor. [#16]
    - **Native RTSP video client** — Kite's own RTSP client with OS hardware decode (H.264/H.265)
      on Windows [#85], Android [#88] and Linux [#89].
    - **Unobstructed fullscreen video** — aspect-exact video box with a blurred follow-map
      backdrop. [#90]
    - **PX4 ULog import** — the logbook imports `.ulg` flash/SD logs, split into flights like any
      other log. [#82]
    - **3D buildings** — optional OpenStreetMap building extrusions on the 3D map. [#48]
    - **Automatic database backup** — before a Kite update upgrades the flight database, a full
      backup is written next to it; Settings shows it with size and a delete button. [#96]
    - **High-resolution replay** — HI-RES switch in the player re-decodes the stored log at full
      rate for fluid instruments; slow-motion speeds 0.25× / 0.5×. [#97]
    - **Open a log without importing it** — drop an onboard log on the map or use the Logbook's
      Open button to replay it straight from the file; import it afterwards if you like. [#99]
    - **Resizable widgets** — small/large for square widgets, wide/large/small for Live AGL and
      Video, switched with a corner button in edit mode; docks adapt to their largest widget. [#101]
    - **Raw telemetry view** — the **≡ Raw** button next to Relay lists every telemetry value with
      its raw unit; replaces the Raw Telemetry widget. [#101]
    - **Phone layout** — landscape interface for phones: full-screen map, connection pop-out,
      widget column with two pages and long-press editing. [#102]
    - **Phone video** — docked window with a park button, the Video widget in the column, map ⇄
      video swap with a locked mini map; off-screen video is not rendered but keeps its stream. [#105]
    - **Background telemetry on Android** — a foreground service with a live notification keeps
      the link, the recording and the track running while Kite is minimised. [#111]
    - **High-Resolution 3D** — the globe at native pixel density (sharp on phones, tablets and
      high-DPI screens) or at half resolution for weak GPUs; Settings → Interface → Map. [#111]

    **Improved**

    - **Replay player folds away while playing** — a slim strip (craft, time, progress) replaces
      the panel; hover or tap the area to unfold it, paused = always open. Clicking anywhere
      outside the Logbook collapses it to its info card. [#100]
    - **Live AGL projected flight line is smoother** — the glide path is low-pass filtered instead
      of following every vario wobble; compass and horizon now share the widgets' blurred glass. [#101]
    - **Park a panel with a second click** — clicking the active tool's rail icon again slides the
      panel off the map without losing its state (a mission stays in edit mode); click once more to
      bring it back. [#102]
    - **Replay player is narrower** — the times moved under the buttons, which now share the full
      width. [#102]
    - **Android launcher icon** fills its circle instead of floating small in it. [#102]
    - **3D mission markers** match the 2D marker size and are sharp on high-DPI screens. [#111]
    - **Raspberry Pi 5 HEVC** — the native RTSP client no longer aborts on streams whose picture is
      padded to the encoder's block size (NVENC 720p, every 1080p): the padding is decoded zero-copy
      and hidden under the interface. Start-up no longer drops the frames right after the keyframe,
      and after any packet loss the video pauses until the next keyframe instead of freezing the
      Pi's hardware decoder. Kernel-side report: raspberrypi/linux#7609. [#112]

??? note "1.0.0 — Initial release"

    The first stable release of **Kite Ground Control**: a cross-platform ground station for
    **INAV**, **ArduPilot** and **PX4** — live telemetry over serial, Bluetooth and network links,
    mission planning on 2D and 3D maps, safety subsystems (safe homes, geozones, geofence), a full
    flight logbook with replay, FPV video, radar / airspace awareness and telemetry relaying.

    Everything this version contains is covered by the regular documentation — start with the
    [quick tour](getting-started/quick-tour.md) or the [GitHub release](https://github.com/b14ckyy/Kite-GC/releases).

[#16]: https://github.com/b14ckyy/Kite-GC/pull/16
[#48]: https://github.com/b14ckyy/Kite-GC/pull/48
[#79]: https://github.com/b14ckyy/Kite-GC/pull/79
[#82]: https://github.com/b14ckyy/Kite-GC/pull/82
[#85]: https://github.com/b14ckyy/Kite-GC/pull/85
[#88]: https://github.com/b14ckyy/Kite-GC/pull/88
[#89]: https://github.com/b14ckyy/Kite-GC/pull/89
[#90]: https://github.com/b14ckyy/Kite-GC/pull/90
[#96]: https://github.com/b14ckyy/Kite-GC/pull/96
[#97]: https://github.com/b14ckyy/Kite-GC/pull/97
[#99]: https://github.com/b14ckyy/Kite-GC/pull/99
[#100]: https://github.com/b14ckyy/Kite-GC/pull/100
[#101]: https://github.com/b14ckyy/Kite-GC/pull/101
[#102]: https://github.com/b14ckyy/Kite-GC/pull/102
[#103]: https://github.com/b14ckyy/Kite-GC/pull/103
[#105]: https://github.com/b14ckyy/Kite-GC/pull/105
[#111]: https://github.com/b14ckyy/Kite-GC/pull/111
[#112]: https://github.com/b14ckyy/Kite-GC/pull/112
[#123]: https://github.com/b14ckyy/Kite-GC/pull/123
