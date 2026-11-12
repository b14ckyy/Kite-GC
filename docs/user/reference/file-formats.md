# File formats

The files Kite reads and writes, grouped by what they're for. **R** = Kite can open/import it, **W** =
Kite can save/export it.

## Missions

| Format | Extension | R/W | Notes |
|---|---|---|---|
| **INAV mission** | `.mission` | R / W | MultiWii XML, compatible with the INAV Configurator and MWPTools. |
| **ArduPilot / PX4 mission** | `.waypoints` | R / W | QGroundControl-compatible plain text. |
| **QGroundControl plan** | `.plan` | R | QGC's JSON plan — the format QGC saves (PX4 workflows). Mission waypoints only: QGC pattern items (Survey, Corridor Scan, …) are refused with a clear message, and the plan's geofence/rally sections are ignored — manage those in Kite's own Airspace tools. |

You can also drag a mission file straight onto the map. See **[Missions](../guides/missions.md)**.

## Flights & logs

### Import

| Format | Extension | Notes |
|---|---|---|
| **INAV Blackbox** | `.bbl`, `.bfl`, `.txt` | Onboard-flash (`.bbl` / `.bfl`) or SD-card (`.txt`) blackbox. INAV logs only — a Blackbox log from another firmware (e.g. Betaflight) is refused. |
| **ArduPilot Dataflash** | `.bin` | ArduPilot (4.1 or newer) onboard logs. |
| **PX4 ULog** | `.ulg` | PX4 onboard flash / SD-card logs. PX4 logs no throttle output, so replayed throttle shows the thrust setpoint. |
| **MAVLink telemetry** | `.tlog` | A MAVLink ground-station recording. |
| **MWPTools raw-MSP** | `.rawmsp` | mwp's raw telemetry capture. |
| **Kite flight** | `.kflight` | A flight exported from another Kite install. |

Imported **telemetry** logs (`.tlog` / `.rawmsp`) are split into separate flights on the arm / disarm
markers, the same as a live recording.

### Export

| Format | Extension | Notes |
|---|---|---|
| **Kite flight** | `.kflight` | Kite's portable flight file — export selected flights to move them to another install. |
| **Original log file** | `.bbl` / `.txt` / `.bin` / `.ulg` | Re-export the onboard log stored with a flight (Blackbox, Dataflash or ULog). Imported `.tlog` / `.rawmsp` is parsed straight in and **not** kept as a file, so there's nothing to re-export there. |
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
- **Next to the database** — `flights.pre-migration-backup.db`, the full backup Kite writes before an
  update upgrades the database format (only the latest is kept), and a `hires-cache/` folder that holds
  the temporary decode of a flight in **HI-RES** replay and is emptied when you close the flight. See
  **[Flight logbook](../guides/logbook.md#the-database-updates-and-the-automatic-backup)**.
- **Caches** — 2D map tiles and terrain are cached on disk (under Settings → Map → Cache); these are the
  files that make the 2D map usable offline.
- **Custom maps** (desktop) — your own map tiles in the custom map location, one `Z/X/Y` folder per
  map, plus a small `.kite-coverage.json` index Kite writes into each map folder. See
  **[Custom maps from a folder](../guides/custom-maps.md#custom-maps-from-a-folder)**.
