# Changelog

What's new in each Kite Ground Control release — the big features up top, the full list of changes
below. The newest release is expanded; click an older version to unfold it. This page is shared by
every released version of the documentation; the **Dev** docs also list what is still being built.

Each version heading ends with the line's support status — **Live**, **Maintenance** or **EOL** — as
defined on the [release support](release-support.md) page. Patch releases (1.0.1, 1.0.2, …) are listed
inside the box of the feature release they belong to, so the notes for one release line stay together.

???+ note "2.0 — in development"

    **Why 2.0, not 1.1:** the flight database changes its schema with this release. Kite
    writes one backup next to it before migrating (`flights.pre-migration-backup.db`) — but the
    migrated database cannot be opened by Kite 1.0 any more, so there is no way back except that
    backup. A step that closes the door behind you is a major version.

    **Highlights**

    ??? info "Kite goes mobile — Android, iPhone and iPad"
        Kite runs natively on **Android** tablets and phones, with USB serial, Bluetooth LE and
        Wi-Fi links. It comes as an APK download; the Play Store follows later. [#79]

        **iPhone and iPad** get a native app as well, contributed by Sebastian Kumor: the phone
        interface on the iPhone, the tablet interface on the iPad, connected over Bluetooth LE or
        the network (TCP / UDP) — these devices have no serial port. The iPad plays RTSP video
        with hardware decoding. The app comes through the App Store only, which follows later.
        [#16] · [#221]

        On every mobile device, features the device cannot provide are hidden.

        **Phones have their own interface.** In landscape the map fills the whole screen. A
        chain-link button opens the connection pop-out, and the instruments sit in a **widget
        column** on the right edge with two swipeable pages, plus three slots on the map's bottom
        edge. Long-press a widget to rearrange or resize it. [#102] · [#173]

        **Video on an Android phone** sits in a docked window at the bottom-right of the map; a
        camera button parks it and brings it back. It can also run in the Video widget of the
        column, and a double-tap swaps map and video. Video on the iPhone is being looked into
        but is not available yet. [#105]

        **Android only: the link keeps running in the background.** While connected, Android shows
        a notification with the aircraft, battery, flight mode and distance to home. Minimising
        Kite or switching apps does not stop telemetry or the recording. iOS does not offer the
        interfaces for this. [#111]

        **RC control** from Kite is not available on phones and tablets for now. We will look at
        it again for a later update.

    ??? info "Native RTSP video — hardware decode, two surfaces, a window of its own"
        Kite has its **own RTSP client** and decodes H.264/H.265 with the **operating system's
        hardware decoder** on Windows, Android, Linux, macOS and iPad. RTSP no longer needs the
        external video engine, which lowers latency and CPU load; on the desktop the classic engine
        can still be selected in the Video panel. [#85] · [#88] · [#89] · [#126] · [#221]

        **Two places at once (desktop).** The picture runs in the Video widget and in the floating
        window or the fullscreen swap at the same time. [#129] · [#132] · [#135]

        **A window of its own (desktop).** A button on the floating window moves the picture into
        its own always-on-top window — beside Kite or on a second monitor, where it opens again at
        the next start, in fullscreen if you left it that way. A Wayland desktop cannot keep it
        above other applications. [#130] · [#132] · [#135]

        **Unobstructed fullscreen.** A new switch in the Video panel keeps the fullscreen video
        clear of the interface: the picture no longer runs underneath the widgets but takes the
        free area beside them, and grows into the space of a dock that is hidden or empty. It
        keeps the stream's exact aspect ratio, and instead of black bars a blurred map that follows
        the aircraft fills the rest of the screen. Off by default. [#90]

        Details in the [Video](guides/video.md) guide.

    ??? info "INAV over ELRS, mLRS, SiK or Wi-Fi — the full planner on a MAVLink-only link"
        ELRS and mLRS in MAVLink mode, SiK radios and MAVLink Wi-Fi bridges carry no MSP, and MSP
        is what an INAV flight controller needs for missions, safe homes and geozones. **INAV 10.0**
        can carry MSP inside the MAVLink stream, and Kite uses that by itself: connect with the
        **MAVLink** protocol, and when the flight controller answers on the tunnel the status shows
        **MSP/MAV**. Mission upload and download (EEPROM and multi-mission included), safe homes,
        geozones, craft name and flight statistics then work as over a direct MSP link. Telemetry
        stays MAVLink; nothing is polled through the tunnel.

        Needs INAV 10.0 or newer. RC control from Kite and the flight controller's own ADS-B
        receiver are not available on this path. INAV versions before 10.0 connect over MAVLink on a
        best-effort basis and are not officially supported. Setup and limits:
        [Connecting → INAV over MAVLink](guides/connecting.md#inav-over-mavlink-mspmav). [#202]

    ??? info "Your own maps — online with your own key, offline from a folder"
        **Custom online providers.** Add any online tile service by its URL template under
        Settings → Map → Providers; an API key goes into the URL. **Test** checks the service and
        detects its tile size. The provider then appears in the map selector, on the 2D map and the
        3D globe, on every platform, and its tiles go into the tile cache. [#249]

        **Offline tiles from a folder (desktop).** Put tiles in the usual `Z/X/Y` layout into a
        subfolder of the custom map location and the folder appears in the map selector, on the 2D
        map and the 3D globe, without internet. Gaps are filled from lower zoom levels, and a frame
        on the 2D map shows where the detailed tiles are. Not on Android and iOS; the 3D terrain
        still needs the internet. [#246] · [#253]

        Both are described in [Your own maps](guides/custom-maps.md).

    ??? info "Logbook and replay — native Blackbox import, HI-RES replay, a smaller database"
        **INAV Blackbox import without a helper program.** Kite decodes INAV Blackbox logs itself:
        no `blackbox_decode` download, faster imports, and the stretches the old tool lost after a
        flight-mode change are in the track. Only INAV logs are accepted; a Blackbox log from
        another firmware is refused with a message that names it. If an import goes wrong, starting
        Kite with the environment variable `KITE_BLACKBOX_LEGACY=1` brings back the old decoder for
        now — please report the log that failed. [#234]

        **High-resolution replay.** A **HI-RES** switch in the player re-decodes the stored onboard
        log at its full rate for the horizon, the instruments and the stick overlay, with
        slow-motion speeds of 0.25× and 0.5×. [#97]

        **Open a log without importing it.** Drop a log on the map, or use the Logbook's **Open**
        button, while disconnected. It replays like a logbook flight, and you import it only if you
        want to keep it. [#99]

        **A smaller flight database.** The upgrade removes an unused text copy from imported INAV
        Blackbox flights; a logbook that consists of such flights can shrink by more than 30 %.
        Flights from ArduPilot and PX4 logs keep their size. For every flight, space freed by
        deleting flights or stored logs is now handed back to the disk. [#232]

    ??? info "Telemetry API — live telemetry for other programs"
        Kite can **serve its live telemetry as JSON** to other programs: an NDJSON stream over TCP
        (port 27300), an HTTP snapshot (port 27301) and UDP subscribers. It is read-only and holds
        everything the Raw telemetry view shows — position, attitude, battery, link, flight mode,
        sensors, home and the ground-station position — the same for INAV, ArduPilot, PX4 and
        passive links. It keeps running in the background, also on Android. Loopback by default,
        one switch for the network. Every field and examples are in the
        **[Telemetry API](reference/telemetry-api.md)** reference. [#123]

    ??? info "Widgets in several sizes, and a raw telemetry view"
        Every flight widget has **two or three sizes**: in edit mode a corner button steps it from
        small to large (and, for the wide Live AGL and Video tiles, to a square). The docks shrink
        to their largest widget, which hands the freed space back to the map. Widgets are
        rearranged by **long-pressing** them, with the mouse or a finger. [#101] · [#127]

        The **≡ Raw** button in the top bar opens one view with every value the telemetry link
        delivers, in its raw unit. It replaces the Raw Telemetry widget. [#101]

    ??? info "Kite updates itself"
        The update notice shows the **release notes** in the dialog, and next to *Open Release Page*
        there is **Update and Restart**: Kite downloads the new version, installs it and starts
        again — on Windows and Linux through the installer of the package you have (a `.deb`/`.rpm`
        asks for your password), on macOS by replacing the app bundle, and a **portable** copy
        replaces its own executable. Nothing is downloaded until you press the button. Android has
        no in-app install and offers the release page instead; on iPhone and iPad the update check
        is off.

        **Patch releases only.** A new choice under Settings → Updates keeps you on the bug-fix
        line of the version you run: 2.0.0 is told about 2.0.1, never about 2.1.0. The default
        stays *Stable releases*. [#162]

    **Added**

    *Mobile*

    - **Android support** — native app for tablets and phones with USB serial, Bluetooth LE and
      Wi-Fi links, a touch layout and shared storage folders (APK download; the Play Store
      follows later). [#79]
    - **iPhone & iPad support** — native iOS/iPadOS build with phone and tablet layouts, Bluetooth
      LE and Wi-Fi MAVLink (App Store only, follows later). Contributed by Sebastian Kumor. [#16]
    - **Phone layout** — landscape interface with a full-screen map, a connection pop-out, a widget
      column with two pages, three widget slots on the map's bottom edge and long-press editing.
      [#102] · [#173]
    - **Phone video** — docked window in two sizes with a park button, the Video widget in the
      column and a map ⇄ video swap. [#105] · [#152]
    - **Background telemetry on Android** — a notification keeps the link, the recording and the
      track running while Kite is minimised. [#111]

    *Video*

    - **Native RTSP video client** — Kite's own RTSP client with the operating system's hardware
      decoder (H.264/H.265) on Windows [#85], Android [#88], Linux [#89], macOS [#126] and iPad
      [#221]. No video on iPhone yet.
    - **Video in two places at once** — with the native RTSP client the picture runs in the Video
      widget and in the floating window or the fullscreen swap at the same time. Desktop only.
      [#129] · [#132] · [#135]
    - **Detached video window** — the floating video frame moves into its own always-on-top window,
      beside Kite or on a second monitor, and returns there at the next start. Needs the native
      RTSP client. Desktop only (Windows, macOS and Linux), not on tablets.
      [#130] · [#132] · [#135]
    - **Unobstructed fullscreen video** — a switch in the Video panel keeps the fullscreen video
      clear of the widgets instead of running underneath them, with a blurred map that follows the
      aircraft as the backdrop instead of black bars. [#90]
    - **Mini-map gestures** — when the map is swapped into the Video widget or into the phone's
      docked video window, a tap on it toggles heading-up ⇄ north-up and sliding one finger up or
      down zooms. The map in the floating window and in the detached window is operated as usual.
      [#152]

    *Map*

    - **Map search** — a magnifier button jumps the 2D or 3D map to a place or to coordinates, and
      a location button next to it to your own position. See
      [Map search](guides/telemetry-and-display.md#map-search). [#263]
    - **Custom online map providers** — add any online tile service by its URL template, with your
      own API key if it needs one. See [Your own maps](guides/custom-maps.md). [#249]
    - **Custom offline maps (desktop)** — a folder of tiles in the `Z/X/Y` layout shows up in the
      map selector and works without internet. See [Your own maps](guides/custom-maps.md).
      [#246] · [#253]
    - **3D buildings** — optional OpenStreetMap building extrusions on the 3D map. [#48]
    - **High-Resolution 3D** — the globe at the screen's native pixel density, or at half
      resolution for weak GPUs; Settings → Map → 3D View. [#111]
    - **Guided "Fly Here" from the 3D map** — a click or tap on the terrain opens the same target
      form as on the 2D map, in every camera mode except FPV. See
      [Vehicle control](guides/vehicle-control.md#guided-flight-fly-here). [#265]

    *Logbook and replay*

    - **INAV Blackbox import without a helper program** — Kite decodes INAV Blackbox logs itself;
      imports are faster and no longer lose the frames after a flight-mode change. [#234]
    - **PX4 ULog import** — the logbook imports `.ulg` logs, split into flights like any other
      log. [#82]
    - **High-resolution replay** — a HI-RES switch in the player re-decodes the stored log at its
      full rate; new slow-motion speeds 0.25× / 0.5×. [#97]
    - **Open a log without importing it** — drop an onboard log on the map or use the Logbook's
      Open button to replay it straight from the file. [#99]
    - **Automatic database backup** — before an update upgrades the flight database, a full backup
      is written next to it; Settings shows it with its size and a delete button. [#96]

    *Vehicles and control*

    - **INAV over a MAVLink-only link (MSP/MAV)** — with INAV 10.0, missions, safe homes, geozones,
      craft name and flight statistics work over ELRS/mLRS in MAVLink mode, SiK radios and MAVLink
      Wi-Fi bridges. INAV versions before 10.0 connect over MAVLink on a best-effort basis and
      are not officially supported. See
      [Connecting → INAV over MAVLink](guides/connecting.md#inav-over-mavlink-mspmav). [#202]
    - **ArduPlane: Takeoff button for fixed-wing planes** — switches the plane to TAKEOFF mode
      (hold to confirm). In that mode the motor starts as soon as you arm. [#242]
    - **Vehicle type override, FC ID and "Save to vehicle library" in the UAV Info panel** — the
      detected type can be changed for the session, the flight controller's hardware ID is shown,
      and the connected craft can be saved as a library vehicle. [#157]
    - **Local radar devices over the network** — an ADS-B receiver and the FormationFlight module
      connect over TCP or UDP as well as over a serial port. See
      [Radar & ADS-B](guides/radar-and-adsb.md#connecting-a-local-device). [#261]

    *Connection and data*

    - **Telemetry API** — Kite serves its live telemetry as read-only JSON over TCP, HTTP and UDP
      for other programs. See the [Telemetry API](reference/telemetry-api.md) reference. [#123]
    - **Default protocol** — Settings → Interface → UI pins the protocol Kite opens with to MSP,
      MAVLink or Telemetry; the default stays *Last used*. [#151]

    *Interface*

    - **Resizable widgets** — every flight widget has two or three sizes, switched with a corner
      button in edit mode; the docks adapt to their largest widget. [#101]
    - **Raw telemetry view** — the **≡ Raw** button in the top bar lists every telemetry value in
      its raw unit. [#101]
    - **Update and Restart** — the update notice shows the release notes and installs the new
      version in place. [#162]
    - **Patch releases only** — an update channel that offers bug-fix releases of the running
      version only (2.0.0 → 2.0.1, never 2.1.0). [#162]
    - **Slovak interface language**, contributed by Sebastian Kumor. [#222]
    - **Spanish interface language.** [#225]

    **Changed**

    *Video*

    - **RTSP uses Kite's own client by default** on Windows, macOS and Linux; the classic engine
      (MediaMTX / ffmpeg) can still be selected in the Video panel. **Disable hardware
      acceleration** appears only where it has an effect: on Linux, on the classic path, when Kite
      found hardware for the stream conversion. [#259]
    - **Floating video window** — starting a source slides the window in, and a camera button next
      to it parks it off-screen and brings it back while the stream keeps running. The Video panel
      shows the source's state, resolution, frame rate, codec and bitrate instead of a preview.
      [#127]
    - **The floating video window moves by a handle** in its bottom-left corner; dragging the
      picture, the right mouse button and two fingers go to the map in the frame. [#152]
    - **The map inside the floating window or a widget has no + / − zoom buttons.** Zoom it with
      the mouse wheel, a pinch or the mini-map's slide gesture. [#152]

    *Map*

    - **Bigger icons on the map buttons** — the 2D / 3D, follow, camera-mode and video buttons use
      one icon size that fills the button. [#263]
    - **3D mission markers** match the 2D marker size and are sharp on high-DPI screens. [#111]
    - **Many radar contacts load the map less** — a contact's symbol is redrawn only when it
      visibly changes, not on every position update. [#174]
    - **Location names for the logbook** — requests to OpenStreetMap's Nominatim are at least a
      second apart, and flights from the same field reuse the name instead of asking again. [#255]

    *Logbook and replay*

    - **A smaller flight database** — imported INAV Blackbox flights no longer carry an unused
      text copy of their rows, and space freed by deleting flights or stored logs is handed back
      to the disk. [#232]
    - **Flights belong to their vehicle by FC ID** — every flight stores the library vehicle it
      belongs to, recognised by the flight controller's hardware ID first and by the craft name
      second; when unsure, the flight stays unassigned. Existing flights are linked once during
      the database upgrade. The post-flight summary and the flight detail show the vehicle and
      let you change it. [#228]
    - **`.kflight` files of INAV Blackbox flights are smaller** — a file written by 2.0 still
      imports into Kite 1.0. [#232]
    - **Compact Database runs in the background** behind a notice, is refused while a flight is
      being recorded or an import is running, and reports how much space was freed. [#243]

    *Vehicles and control*

    - **ArduPilot: the control panel offers the modes your flight controller really has.** From
      ArduPilot 4.7 on, Kite reads the mode list from the flight controller. Older ArduPilot
      versions and PX4 use the built-in list, which is complete for Copter, Plane and Rover now.
      [#242]
    - **Stick-flown modes fold out from an "RC modes" bar** instead of the *Show all modes*
      checkbox; the bar shows *No RC link* while those modes are locked. [#242]
    - **ArduPilot: returning and landing are hold-to-confirm buttons in one place.** A QuadPlane
      has **RTL / QRTL** and **Loiter QLand / QLand**, a fixed-wing plane on ArduPilot 4.7 gets
      **AutoLand** next to RTL, a rover or boat shows only RTL, and a Sub shows **Surface**.
      SmartRTL and Auto RTL need a hold too. [#242] · [#258]
    - **System messages default to *Warning*, and a new *Debug* level** shows the texts a flight
      controller sends at its debug severity; they no longer appear as info messages. [#186]

    *Connection and data*

    - **Network links: an empty host means this device.** The host field starts empty with a
      `localhost` hint; left empty, Kite connects to `127.0.0.1`. [#237]
    - **The network port follows the protocol.** MAVLink offers UDP 14550 or TCP 5760 and MSP
      offers TCP 5761. A port you typed yourself is never touched. [#151]
    - **The diagnostics log has a size limit** — a day's log file stops at 50 MB and is kept; the
      log continues in a second file for that day, which starts over each time it reaches 50 MB.
      [#271]
    - **Errors of the user interface are written to the diagnostics log file** as well, so a
      report from a release build is complete. [#193]

    *Interface*

    - **Settings has three tabs: Interface, Map and Data.** The new Map tab gathers the 3D view,
      the mission defaults, the providers and the caches. *Tile Provider* is now called **Map
      Provider**, and the 3D **Altitude Curtain** is off by default for new installations. See
      [Settings](reference/settings.md). [#250]
    - **Widgets are rearranged by long-pressing them** — mouse or finger; the ✎ button is gone.
      Click outside the docks or press Escape to finish. [#127]
    - **The replay player folds away while playing** — a slim strip with craft, time and progress
      replaces the panel; hover or tap it to unfold. The player is also narrower. [#100] · [#102]
    - **Park a panel with a second click** — clicking the active tool's icon again slides the
      panel off the map without losing its state; click once more to bring it back. [#102]
    - **The mouse wheel steps any + / − number field** that has the focus; a field without focus
      never reacts, so scrolling a panel stays safe. [#189] · [#191]
    - **Power Saving also drops the glass blur** of panels and widgets. [#174]
    - **Live AGL: the projected flight line is smoother**, and the compass and horizon have the
      frosted-glass look of the other widgets. [#101]
    - **The second battery widget is labelled "BAT 2".** [#237]

    **Fixed**

    Every fix of 1.0.1 and 1.0.2 is part of 2.0. This list holds what is fixed in 2.0 only.

    - **3D map: leaving FPV left the camera in the cockpit** — banked with the aircraft and
      looking at the horizon, and on a touch screen it could hardly be tilted or turned. The free
      camera now starts behind and above the aircraft, looking down at it. [#262]
    - **Floating video window at a UI scale above 100 %** — the snapped window landed past the
      bottom edge of the screen. [#127]
    - **INAV flash download: one unreadable log ended the import.** If one log of a `.bbl` was
      damaged, the following flights were never imported. Kite now imports the rest and shows
      which logs failed and why. [#243]
    - **UDP links: a busy local port, and several senders on one port.** When another program
      already listens on the port you target, Kite binds the same port plus 10000 instead of a
      random one, so a relay finds it again after a reconnect. Commands go to every source heard
      in the last ten seconds instead of only the last one. Contributed by dldnjsxo95.
      [#194] · [#204]
    - **PX4: take-off and land headed for 0°N 0°E, missions with unset yaw or action items did
      not upload, and integer parameters were read and written wrongly.** The desktop RC panel
      also shows `COM_RC_IN_MODE` with a one-click fix when it blocks joystick input, and a
      geofence or rally save lists the parameters Kite could not write as a warning instead of
      failing. For ArduPilot too: mission, fence and rally transfers pass through a routing relay
      such as Mission Planner's MAVLink mirror. Contributed by dldnjsxo95. [#195] · [#205]
    - **Windows: an Xbox-class gamepad that was listed but delivered no input** is now read
      through XInput. If the sticks feel inverted after this, re-learn the mapping in the RC
      panel. [#196]
    - **macOS 13 / 14: panels, widgets and map buttons had no background blur**; they were only
      semi-transparent. macOS 15 was not affected. [#274]
    - **Chinese and Bulgarian** — the connection-status and toolbar strings that fell back to
      English are translated. [#154] · [#156]

    **Removed**

    - **Video window (detach)** — the old detached window was the browser's Picture-in-Picture and
      only existed on Windows. The detached video window replaces it. [#129]
    - **Raw Telemetry widget** — replaced by the **≡ Raw** view. [#101]
    - **`blackbox_decode` download** — no longer needed for INAV Blackbox imports. [#234]

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
    - **ArduPilot / PX4 mission editor: edit mode stayed on after you left the Mission tab.**
      Switching to another tab or closing the side panel left the editor active — the thick edit
      lines stayed, and every click on the map kept adding waypoints until you pressed *Edit*
      again. Leaving the Mission tab now ends edit mode, as it always did for INAV missions.
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
    - **A flight that could not be saved was dropped without a word.** If saving a recorded flight
      failed — the database busy with a long import, a full disk — the post-flight summary simply
      closed: no error, no logbook entry, and the notes and battery serials you had entered were
      gone. The recording itself was still on disk, but the next *Discard Recording* in the same
      session deleted it. Now Kite shows the error and offers **Retry**; if you cancel, the recording
      is kept and offered again at the next start. The same holds for *Save* in the "connection lost"
      and "unfinished recording" prompts. The automatic save when you arm again keeps the recording
      the same way, without a message, and offers it at the next start.
    - **ArduSub: the control panel sent ArduRover's mode numbers.** On a Sub the mode buttons meant
      something else to the vehicle: *Steering* switched to **Auto and started the mission**, *Manual*
      switched to Stabilize, and Auto, Loiter, SmartRTL and the Guided toggle were refused. The
      panel now offers ArduSub's own modes: Guided, Auto, PosHold, Circle and Surface, and with an
      RC source Stabilize, Acro, Depth Hold, Manual and Surftrak. Clicking a target on the map
      ("Fly Here") stays unavailable on a Sub.
    - **ArduRover, Boat and ArduSub: wrong flight-mode names.** Their modes were read with ArduCopter's
      table, so a rover in Hold showed as "Guided", one in Guided as "AutoTune", a Sub in Manual as
      "Avoid ADSB" — in the mode widget, the track colours, recordings and imported logs. Each vehicle
      type is now read with its own table, and ArduCopter's Turtle and ArduPlane's Autoland and
      Initialising have names instead of a raw number. Flights recorded or imported before this
      fix keep the names they were stored with.
    - **ArduRover / Boat: Hold was missing from the mode buttons.** It is offered now.
    - **Pressing F5 while connected left Kite connected with no way to disconnect.** F5 (or
      Ctrl+R) reloaded the user interface only. The connection to the flight controller stayed
      open in the background, but the status bar showed "disconnected" and the Connect button
      could no longer reach it — only quitting Kite helped. The reload keys no longer do anything,
      and should the interface ever restart on its own while connected, Connect now offers to
      close the leftover connection and connect again.
    - **The dark map showed "API KEY REQUIRED" instead of the map.** The provider of *CartoDB Dark
      Matter* now demands an API key for every tile. The dark map is **ESRI Dark Gray** instead;
      if you had the CartoDB map selected, Kite switches to it by itself. Its tiles end at zoom
      level 16 — zoomed in further, Kite enlarges them.
    - **QuadPlane: QHover was offered without an RC source.** QHover only holds the altitude; the
      pilot holds the position. It is now behind *Show all modes* and locked without an RC source,
      like the other stick-flown modes.
    - **macOS: a lost network (TCP) connection filled the log file without end.** When the Mac itself
      dropped a TCP connection to an ArduPilot / PX4 vehicle — a network change, for example — Kite
      did not notice the loss: the status bar stayed on "connected", and the diagnostic log grew by
      the same error line many thousand times a second, to several gigabytes within minutes. Kite now
      treats this as the lost connection it is and disconnects, with the "Device connection lost"
      prompt for a running recording. On every system, a read error that keeps repeating is written
      to the log once every few seconds instead of every time.
    - **Linux: bright lines along the map tile edges, and a very slow map.** After a system update
      to WebKitGTK 2.54 the 2D map showed a thin bright grid, one line at every tile border, with
      every map provider and at any display scaling — and moving the map dropped to a few frames
      per second, worst with video running. The tiles are drawn without those lines again, and
      the map moves as smoothly as before. One thing remains on these systems and is not Kite's to
      fix: after each zoom step the map shows the previous zoom level for a single frame. That is a
      fault in WebKitGTK 2.54 itself (WebKit bug 321993), corrected by its developers; it goes away
      with a later system update.
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
[#105]: https://github.com/b14ckyy/Kite-GC/pull/105
[#111]: https://github.com/b14ckyy/Kite-GC/pull/111
[#123]: https://github.com/b14ckyy/Kite-GC/pull/123
[#126]: https://github.com/b14ckyy/Kite-GC/pull/126
[#221]: https://github.com/b14ckyy/Kite-GC/pull/221
[#127]: https://github.com/b14ckyy/Kite-GC/pull/127
[#129]: https://github.com/b14ckyy/Kite-GC/pull/129
[#130]: https://github.com/b14ckyy/Kite-GC/pull/130
[#132]: https://github.com/b14ckyy/Kite-GC/pull/132
[#135]: https://github.com/b14ckyy/Kite-GC/pull/135
[#140]: https://github.com/b14ckyy/Kite-GC/pull/140
[#141]: https://github.com/b14ckyy/Kite-GC/pull/141
[#222]: https://github.com/b14ckyy/Kite-GC/pull/222
[#225]: https://github.com/b14ckyy/Kite-GC/pull/225
[#228]: https://github.com/b14ckyy/Kite-GC/pull/228
[#232]: https://github.com/b14ckyy/Kite-GC/pull/232
[#234]: https://github.com/b14ckyy/Kite-GC/pull/234
[#237]: https://github.com/b14ckyy/Kite-GC/pull/237
[#242]: https://github.com/b14ckyy/Kite-GC/pull/242
[#243]: https://github.com/b14ckyy/Kite-GC/pull/243
[#246]: https://github.com/b14ckyy/Kite-GC/pull/246
[#249]: https://github.com/b14ckyy/Kite-GC/pull/249
[#250]: https://github.com/b14ckyy/Kite-GC/pull/250
[#253]: https://github.com/b14ckyy/Kite-GC/pull/253
[#255]: https://github.com/b14ckyy/Kite-GC/pull/255
[#258]: https://github.com/b14ckyy/Kite-GC/pull/258
[#261]: https://github.com/b14ckyy/Kite-GC/pull/261
[#262]: https://github.com/b14ckyy/Kite-GC/pull/262
[#263]: https://github.com/b14ckyy/Kite-GC/pull/263
[#265]: https://github.com/b14ckyy/Kite-GC/pull/265
[#271]: https://github.com/b14ckyy/Kite-GC/pull/271
[#274]: https://github.com/b14ckyy/Kite-GC/pull/274
[#259]: https://github.com/b14ckyy/Kite-GC/pull/259
[#143]: https://github.com/b14ckyy/Kite-GC/pull/143
[#145]: https://github.com/b14ckyy/Kite-GC/pull/145
[#151]: https://github.com/b14ckyy/Kite-GC/pull/151
[#152]: https://github.com/b14ckyy/Kite-GC/pull/152
[#154]: https://github.com/b14ckyy/Kite-GC/pull/154
[#156]: https://github.com/b14ckyy/Kite-GC/pull/156
[#157]: https://github.com/b14ckyy/Kite-GC/pull/157
[#160]: https://github.com/b14ckyy/Kite-GC/pull/160
[#162]: https://github.com/b14ckyy/Kite-GC/pull/162
[#163]: https://github.com/b14ckyy/Kite-GC/pull/163
[#168]: https://github.com/b14ckyy/Kite-GC/issues/168
[#173]: https://github.com/b14ckyy/Kite-GC/pull/173
[#174]: https://github.com/b14ckyy/Kite-GC/pull/174
[#175]: https://github.com/b14ckyy/Kite-GC/pull/175
[#186]: https://github.com/b14ckyy/Kite-GC/pull/186
[#189]: https://github.com/b14ckyy/Kite-GC/pull/189
[#191]: https://github.com/b14ckyy/Kite-GC/pull/191
[#193]: https://github.com/b14ckyy/Kite-GC/pull/193
[#196]: https://github.com/b14ckyy/Kite-GC/pull/196
[#202]: https://github.com/b14ckyy/Kite-GC/pull/202
[#194]: https://github.com/b14ckyy/Kite-GC/pull/194
[#204]: https://github.com/b14ckyy/Kite-GC/pull/204
[#195]: https://github.com/b14ckyy/Kite-GC/pull/195
[#205]: https://github.com/b14ckyy/Kite-GC/pull/205
