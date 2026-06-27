# File formats

The files Kite reads and writes, grouped by what they're for. **R** = Kite can open/import it, **W** =
Kite can save/export it.

## Missions

| Format | Extension | R/W | Notes |
|---|---|---|---|
| **INAV mission** | `.mission` | R / W | MultiWii XML, compatible with the INAV Configurator and MWPTools. |
| **ArduPilot / PX4 mission** | `.waypoints` | R / W | QGroundControl-compatible plain text. |

You can also drag a mission file straight onto the map. See **[Missions](../guides/missions.md)**.

## Flights & logs

### Import

| Format | Extension | Notes |
|---|---|---|
| **INAV Blackbox** | `.bbl`, `.txt` | Onboard-flash (`.bbl`) or SD-card (`.txt`) blackbox. Needs the `blackbox_decode` helper, which Kite fetches automatically on first use. |
| **ArduPilot Dataflash** | `.bin` | ArduPilot / PX4 onboard logs. |
| **MAVLink telemetry** | `.tlog` | A MAVLink ground-station recording. |
| **MWPTools raw-MSP** | `.rawmsp` | mwp's raw telemetry capture. |
| **Kite flight** | `.kflight` | A flight exported from another Kite install. |

Imported **telemetry** logs (`.tlog` / `.rawmsp`) are split into separate flights on the arm / disarm
markers, the same as a live recording.

### Export

| Format | Extension | Notes |
|---|---|---|
| **Kite flight** | `.kflight` | Kite's portable flight file — export selected flights to move them to another install. |
| **Original log file** | `.bbl` / `.txt` / `.bin` | Re-export the onboard log stored with a flight (Blackbox or Dataflash). Imported `.tlog` / `.rawmsp` is parsed straight in and **not** kept as a file, so there's nothing to re-export there. |
| **Track** | `.kmz` / `.kml` / `.gpx` / `.csv` | A flight's path for Google Earth, GPS tools or a spreadsheet. |

See **[Flight logbook](../guides/logbook.md)**.

## Libraries

| Format | Extension | R/W | Notes |
|---|---|---|---|
| **Vehicle** | `.kvehicle` | R / W | One aircraft's build sheet (with its lifetime baseline). See **[Vehicles](../guides/vehicles.md)**. |
| **Battery** | `.kbatt` | R / W | One pack's record (optionally with its consolidated usage). See **[Batteries](../guides/batteries.md)**. |

## Controller profiles

| Format | Location | Notes |
|---|---|---|
| **RC / HID profile** | `Documents/KiteGC/HID-Profiles/<name>.json` | A controller mapping. Plain shareable JSON files — copy them between installs. See **[RC control](../guides/rc-control.md)**. |

## Where your data lives

- **Flight database** — your logbook (flights, vehicles, batteries, saved missions) is one local SQLite
  database; its location is configurable in **[Settings → Data](settings.md)**.
- **Caches** — 2D map tiles and terrain are cached on disk (also under Settings → Data); these are the
  files that make the 2D map usable offline.
