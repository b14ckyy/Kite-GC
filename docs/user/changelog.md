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

    ??? info "Native RTSP video client with hardware decode"
        RTSP video no longer needs the external video engine: Kite ships its **own RTSP client** and
        decodes H.264/H.265 with the **operating system's hardware decoder** on Windows, Android and
        Linux — lower latency, a fraction of the CPU load, and rock-solid reconnects. [#85] · [#88] · [#89]

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
