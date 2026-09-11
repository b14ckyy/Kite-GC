# Privacy policy

*Effective: 2026-09-06 (Kite 1.0.0 and later) · Applies to Kite Ground Control on Windows, macOS, Linux,
Android and iOS. Not every feature named here exists on every platform or version; where a feature is
absent, the connection it would make is absent too.*

Kite Ground Control ("Kite") is a ground-control station for RC aircraft and drones. It is free and
open-source software (GPL-3.0-or-later) and is built to work in the field without depending on us.
This page states what data the app handles, what stays on your device, and which outside services the
app contacts while you use it.

The short version:

- **We collect nothing — as of today.** Kite has no user account, no analytics, no crash reporting, no
  advertising and no server of ours that the app reports to. We do not see who uses Kite or how. Should a
  future version ever add anything of that kind, it will be **opt-in**, off by default, and announced in
  the release notes and on this page before it ships.
- **Your data stays on your device.** Settings, flight logs, missions, vehicle and battery records are
  stored locally and leave the device only when you export or share them.
- **Some features talk to third-party services directly from your device** (map tiles, terrain,
  weather, ADS-B traffic, airspace data, the update check). Those services see the technical details of
  such requests. The list below says exactly which ones and when.

## Who provides Kite

Kite is free software published under the GNU General Public License v3 or later by

**Marc Hoffmann** · [b14ckyy@outlook.com](mailto:b14ckyy@outlook.com)

The complete source code is public in the project's [GitHub repository](https://github.com/b14ckyy/Kite-GC),
so every statement on this page can be checked against the code. As the licence states, the software is
provided **as is, without warranty of any kind** and without any obligation on the authors; this page
describes what the app does, it does not create liability beyond the licence terms. Questions about this
policy go to the address above.

## What Kite stores on your device

Everything Kite keeps is a local file on the device you run it on. Nothing is synchronised to a cloud
service by the app.

| Data | What it contains | Where |
| --- | --- | --- |
| **Settings** | Units, layout, connection settings, map providers, API keys you entered (Cesium ion, OpenAIP), interface options | App data folder of the OS |
| **Flight log database** | Telemetry recorded during flights: GPS track, altitude, speed, battery, flight modes, events; take-off place name and weather; your vehicle and battery records; the flight controller's hardware ID where the firmware reports one | App data folder of the OS |
| **Missions, geozones, safe homes** | Waypoints and areas you plan, with coordinates | Files you save |
| **Terrain cache** | Elevation tiles for areas you planned or flew in, kept so terrain features work offline | App data folder of the OS |
| **Last known position** | A coarse (city-level) position, used to time the automatic night mode on the next start | Settings |
| **Diagnostic log** | Technical events (port names, firmware strings, connection state, errors) — no coordinates and no personal data. Level and location under Settings → Diagnostics; *Off* stops writing it | App log folder of the OS |

The exact locations and file formats are listed under
[Reference → File formats](reference/file-formats.md) and
[Troubleshooting → Connection](troubleshooting/connection.md#getting-a-diagnostic-log).

Deleting the app's data folder, or uninstalling the app on Android and iOS, removes all of it.

## Data that leaves your device only when you act

These transfers happen only because you trigger them, and you choose the recipient every time:

- **Exports** — flight logs (`.kflight`), vehicle (`.kvehicle`) and battery (`.kbatt`) files, missions
  (`.mission`, `.waypoints`, `.plan`). Kite writes the file where you tell it to; on Android and iOS it
  hands the file to the system share sheet.
- **Diagnostic log** — Settings → Diagnostics → *Open Folder* shows you the log file, and on Android
  *Share log file* passes it to the app you pick (mail, messenger, cloud drive). Attaching it to a bug
  report is your step; nothing is uploaded by Kite itself.
- **Reporting a problem** — the About dialog and the docs link to GitHub. Anything you post there is
  governed by [GitHub's privacy statement](https://docs.github.com/site-policy/privacy-policies).
- **Telemetry and video on your network** — telemetry forwarding, the video relay and the Telemetry API
  send live data to other devices *only after you enable them* under Settings, and only to the addresses
  you configure. The Telemetry API listens on the local machine only unless you switch *Reachable on the
  network* on.

## Third-party services the app contacts

Kite is a client: for some features it downloads data from public services straight from your device.
The app sends only what the request needs — usually a map position and a radius — plus what every
internet request carries (your IP address, a user-agent string naming Kite and its version). None of
these requests contain your name, your flight logs or any identifier for your device or your aircraft.
Each provider handles the request under its own privacy policy, which we link below.

### Automatic

| When | Service | What is sent | How to switch it off |
| --- | --- | --- | --- |
| **At every app start** | GitHub Releases API ([GitHub privacy statement](https://docs.github.com/site-policy/privacy-policies)) | A request for the latest release version. No data about you or your installation | Settings → Updates → *Check for updates: Disabled* |
| **When a flight is recorded** (the aircraft arms with a valid GPS fix) | Open-Meteo ([privacy](https://open-meteo.com/en/terms)) and OpenStreetMap Nominatim ([privacy](https://osmfoundation.org/wiki/Privacy_Policy)) | The take-off coordinates, to store the weather and the place name in the logbook entry | *(no setting yet — see the note below)* |
| **While the map is shown** | The map tile provider you selected: OpenStreetMap ([privacy](https://osmfoundation.org/wiki/Privacy_Policy)), OpenTopoMap, CARTO ([privacy](https://carto.com/privacy)), Esri ArcGIS ([privacy](https://www.esri.com/en-us/privacy/overview)) | The map tiles for the area on screen, i.e. which region you are looking at | Use the map without internet — Kite then shows only what the system's web cache still holds |
| **When a terrain feature needs elevation** (AGL planning, terrain clearance, line-of-sight analysis, terrain radar) | Copernicus DEM on AWS Open Data ([AWS privacy notice](https://aws.amazon.com/privacy/)) | The 1°×1° elevation tile covering the area | Downloads only for areas you plan or fly in; tiles are cached |

### Only while the feature is enabled

| Feature | Service | What is sent |
| --- | --- | --- |
| **3D map** | Cesium ion ([privacy](https://cesium.com/legal/privacy-policy/)) with the access token *you* created; imagery and terrain from the providers configured in your Cesium account | Terrain and imagery requests for the area on screen, authenticated with your own token, i.e. under your own Cesium account |
| **Radar / ADS-B traffic** | adsb.lol, adsb.one, adsb.fi (community ADS-B aggregators; each publishes its own terms) | The query centre (the map viewport or your aircraft's position) and a search radius, polled while the radar is on |
| **Airspace Manager** | OpenAIP ([privacy](https://www.openaip.net/privacy)) with the API key *you* registered | A position and radius for the airspace layers you enabled, authenticated with your key |

!!! note "Weather and place-name lookup"
    The logbook enrichment sends the take-off position to Open-Meteo and Nominatim automatically when
    a recorded flight starts. If you fly without internet, the lookup simply fails and the entry stays
    without weather and place name. *(Draft note for the maintainers: if a setting to disable this lookup
    is added, replace this paragraph with the setting's name.)*

## Device permissions

Kite asks for a permission only when the feature that needs it is used, and uses it only for that
feature. Denying a permission disables that feature and nothing else.

| Permission | Used for | Platforms that ask |
| --- | --- | --- |
| **Location** | Placing the ground-station marker on the map, the reference point for the radar, and the sunset timing of the automatic night mode. The position is used on the device only: it is not written to the flight log and not sent anywhere; a coarse last-known value stays in the settings | Android, iOS, macOS, Windows (system dialog); Linux has no permission system |
| **Camera** | The *Camera (device)* video source: showing an FPV feed from a capture device or built-in camera. The image is displayed only; Kite does not record it | Android, iOS, macOS, Windows |
| **Microphone** | Declared together with the camera for the capture source. Kite does not record or transmit audio | Android |
| **Bluetooth** | Connecting to a flight controller or telemetry link over BLE or Bluetooth SPP. On Android 12+ the scan is declared *never for location* — it cannot be used to derive your position | Android, iOS, macOS |
| **Local network** | Telemetry over Wi-Fi (TCP/UDP), RTSP video from an onboard computer, the Telemetry API | iOS, macOS 15+ |
| **USB** | Serial connection to a flight controller. Granted per device by the system dialog | Android |
| **Notifications** | The persistent notification that keeps the telemetry link alive while Kite is in the background | Android 13+ |
| **Network state / Wi-Fi state** | Showing which network the tablet is on, so a telemetry bridge's soft-AP can be recognised | Android (no prompt) |

## Your data, your control

The only personal data involved is what your device sends when it contacts the services above (your IP
address) and, if you grant it, the optional location permission — both used on your device, only to
provide the functions you use. We do not store, see or receive any of it, so there is nothing we could
hand over, correct or delete on your behalf: everything Kite keeps is on your device and under your
control, and every outside connection is listed above so you can decide whether to use the feature that
makes it. Requests to a third-party service are subject to that service's own policy; contact them
directly for anything concerning their processing.

If you do not agree with how Kite works as described here, do not use the app.

## Children

Kite is a tool for operating aircraft and is not directed at children under 16.

## Changes to this policy

When the app gains a feature that changes what leaves your device, this page is updated with the
release that ships it and the change is listed in the [changelog](changelog.md). The effective date
at the top tells you the version you are reading.
