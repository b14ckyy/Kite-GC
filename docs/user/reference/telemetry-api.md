# Telemetry API

Kite can serve its **live telemetry to other programs** — a dashboard on the same computer, a script
that logs to a database, a web service that shows the flight on a map, or anything else that can read
JSON. The API is **read-only**: nothing can be sent to the aircraft through it, and it never changes
what Kite does.

Everything in the frames below is what Kite itself holds in its unified telemetry model — the same
values the **Raw telemetry** popup (toolbar, next to Relay) lists — regardless of whether the aircraft
speaks **MSP** (INAV), **MAVLink** (ArduPilot, PX4) or a **passive** link (CRSF, S.Port, LTM). A field
a protocol does not provide is `null`, never a made-up zero.

!!! note "Where it runs"
    The API is served by Kite's backend, not by the user interface: it keeps running while Kite is
    minimised, and on Android while the app is in the background with the link alive.

## Enabling it

**Settings → Data → Telemetry → Telemetry API.** The master toggle starts the server with the
transports selected underneath:

| Setting | What it does | Default |
| --- | --- | --- |
| **TCP stream (port 27300)** | Kite listens on TCP 27300; each client that connects receives a continuous stream, one JSON object per line. | On |
| **HTTP snapshot (port 27301)** | Kite answers `GET` requests on TCP 27301 with the newest frame, or a health document. For `curl`, scripts and browser dashboards. | On |
| **UDP to a target** | Kite *sends* one datagram per frame to the **UDP host / port** you enter. Unicast or a broadcast address. | Off |
| **Reachable on the network** | Off: the TCP listeners bind to `127.0.0.1`, so only programs on this computer can connect. On: they bind to all interfaces and any device on the network can read the telemetry. | Off |
| **Update rate** | Frames per second — **1, 2, 5 or 10 Hz**. This is the API's own clock: a link that updates faster is sampled, a slower one is repeated (the frame's `seq` still increments, `telemetry.lastUpdate` does not). | 5 Hz |

The status line under the settings shows the live endpoints, the number of connected TCP clients, and
the reason if the server could not start (typically another program holding one of the ports).

!!! warning "Keep it on loopback unless you need the network"
    The feed carries the aircraft's position, home and the ground station's position. On a shared
    field Wi-Fi, *Reachable on the network* means everyone on that network can read it. There is no
    authentication in this version.

The ports are **fixed** on purpose — a consumer never has to guess or be configured — and were chosen
outside the ranges common tools use (MAVLink 14550, MediaMTX 8554, SITL 5760, …).

## Transports

### TCP stream — port 27300

Connect, and Kite first sends one **hello** line, then a **frame** line at the configured rate.
Every line is one complete JSON object terminated by `\n` (NDJSON). Read line by line; do not assume
a fixed size.

```json
{"schema":1,"hello":true,"groups":["gps","attitude","altitude","altRef","wind","battery","batteries","link","status","sensors","flightMode","nav","ekf","misc","vehicle","passiveProtocol","lastUpdate"],"rateHz":5}
```

A client that stops reading is disconnected after 200 ms of back-pressure, so one stuck consumer
cannot delay the others. Reconnect at any time; the stream has no state to resume.

### HTTP — port 27301

| Route | Response |
| --- | --- |
| `GET /api/v1/telemetry` | `200` with the newest frame as `application/json`. `503` until the first frame exists (a few hundred milliseconds after enabling). |
| `GET /api/v1/health` | `200` with `{ "ok": true, "schema": 1, "connected": …, "protocol": …, "clients": …, "rateHz": …, "seq": … }`. |

Responses carry `Cache-Control: no-store`, `Access-Control-Allow-Origin: *` (a browser page on the
same machine may poll it directly) and `Connection: close`. Only `GET` is served; anything else
returns `405`, unknown paths `404`.

### UDP — your target

One datagram per frame, same JSON as a TCP line without the trailing newline, no hello. The target
may be a unicast address or a broadcast address (`255.255.255.255`, or your subnet's). UDP is
fire-and-forget: Kite does not know whether anyone listens, and a datagram is dropped rather than
delayed.

## The frame

```json
{
  "schema": 1,
  "ts": 1788700000000,
  "seq": 4711,
  "connected": true,
  "protocol": "msp",
  "fcVariant": "INAV",
  "telemetry": {
    "gps": { "lat": 51.4923, "lon": 11.9263, "altMsl": 143.2, "groundSpeed": 18.4, "course": 271.5, "numSat": 14, "fixType": 3, "hdop": 0.9 },
    "attitude": { "roll": -2.1, "pitch": 3.4, "yaw": 271.0 },
    "altitude": { "altitude": 120.5, "vario": 0.8, "airspeed": 19.2 },
    "altRef": { "msl": true },
    "wind": { "directionFromDeg": 250.0, "speedMs": 4.2 },
    "battery": { "voltage": 15.8, "current": 6.4, "power": 101.1, "mahDrawn": 1210, "percentage": 62, "cellCount": 4, "throttle": 48, "rssi": 987 },
    "batteries": [ { "id": 0, "voltage": 15.8, "current": 6.4, "mahDrawn": 1210, "percentage": 62, "cellCount": 4, "temperature": null } ],
    "link": { "rssiPercent": 96.0, "rssiDbm": -58, "lq": 100, "snrDb": 12 },
    "status": { "armed": true, "armingFlags": 4, "flightModeFlags": 1026, "cpuLoad": 12, "sensorStatus": 63, "mspRcOverride": false, "fcAlive": true },
    "sensors": { "gyro": 1, "acc": 1, "mag": 1, "baro": 1, "gps": 1, "rangefinder": 0, "pitot": 1, "opflow": 0, "prearm": 0, "rcReceiver": 0 },
    "flightMode": { "primary": "cruise", "modifiers": ["althold"] },
    "nav": { "navState": 0, "activeWp": 0 },
    "ekf": null,
    "misc": { "autoThrottle": false, "uptimeS": 812, "flightTimeS": 415 },
    "vehicle": null,
    "passiveProtocol": null,
    "lastUpdate": 1788699999870
  },
  "home": { "lat": 51.4901, "lon": 11.9200, "altMsl": 141.0, "set": true },
  "gcs": { "lat": 51.4902, "lon": 11.9199, "altMsl": 141.0, "accuracyM": 12.5 }
}
```

### Top level

| Field | Type | Meaning |
| --- | --- | --- |
| `schema` | int | Contract version of this document. Bumped only when an existing field changes meaning or disappears; new fields may appear without a bump. |
| `ts` | int | Time the frame was built, Unix epoch **milliseconds** (Kite's clock). |
| `seq` | int | Frame counter since the server started. A gap means you missed frames. |
| `connected` | bool | A vehicle link is active. When `false`, `telemetry` holds the last known values (or all `null` after start) and `protocol` is `null`. |
| `protocol` | `"msp"` · `"mavlink"` · `"passive"` · null | Which input path the link uses. |
| `fcVariant` | string · null | Firmware identifier: `"INAV"` (MSP), `"ArduPlane"` / `"ArduCopter"` / `"ArduRover"` / `"ArduSub"` / `"PX4"` / `"Generic"` (MAVLink, from the autopilot + vehicle type). |
| `telemetry` | object | The groups below. Each group is `null` until the link has delivered it once. |
| `home` | object · null | The flight controller's home position — INAV `MSP_WP 0`, MAVLink `HOME_POSITION`. Changes rarely and is kept between updates. `altMsl` in metres. `set` is always `true` when present. |
| `gcs` | object · null | The ground-station marker as resolved in Kite (see *Radar & ADS-B*): `lat`, `lon`, `altMsl` (terrain height at that point, may be `null`), `accuracyM` (the OS fix's accuracy radius, may be `null`). `null` when the GCS marker is **Off**. |

### `telemetry` groups

Units are Kite's canonical raw units: **degrees, metres, m/s, V, A, W, mAh, %**. Bitfields are plain
integers. **Sources** lists which links fill the group; a field the source does not carry is `null`.

#### `gps` — sources: MSP, MAVLink, passive (LTM / CRSF / S.Port)

| Field | Type | Unit / meaning |
| --- | --- | --- |
| `lat`, `lon` | float | Decimal degrees, WGS-84. |
| `altMsl` | float | Metres above mean sea level as the GPS reports it. |
| `groundSpeed` | float | m/s. |
| `course` | float | Course over ground, degrees 0–360. |
| `numSat` | int | Satellites used. |
| `fixType` | int | 0 none · 1 dead reckoning · 2 = 2D · 3 = 3D (INAV/MAVLink `GPS_FIX_TYPE` values). |
| `hdop` | float · null | Horizontal dilution of precision. MSP (INAV) and MAVLink; `null` on passive links. |

#### `attitude` — sources: all

| Field | Type | Unit |
| --- | --- | --- |
| `roll` | float | Degrees, ±180, right wing down positive. |
| `pitch` | float | Degrees, ±90, nose up positive. |
| `yaw` | float | Heading, degrees 0–360. |

#### `altitude` — sources: all

| Field | Type | Unit |
| --- | --- | --- |
| `altitude` | float | Metres. Baro/fused altitude; **whether it is MSL or relative to home depends on the link** — see `altRef`. |
| `vario` | float | Vertical speed, m/s, positive up. |
| `airspeed` | float · null | m/s from a pitot sensor, when the FC reports it and *Airspeed* is enabled in Settings. |

#### `altRef` — sources: passive only

| Field | Type | Meaning |
| --- | --- | --- |
| `msl` | bool | `true` when the passive link's `altitude` is above sea level, `false` when it is relative to the ground reference. MSP and MAVLink links leave this group `null` (their altitude is defined by the protocol). |

#### `wind` — sources: MAVLink (ArduPilot, PX4), MSP (INAV 10.0+); needs *Wind* enabled in Settings

| Field | Type | Unit |
| --- | --- | --- |
| `directionFromDeg` | float | Bearing the wind blows **from**, degrees. |
| `speedMs` | float | Horizontal wind speed, m/s (0 = no estimate). |

#### `battery` — sources: all (fields vary)

| Field | Type | Unit / meaning |
| --- | --- | --- |
| `voltage` | float | V, pack voltage. |
| `current` | float | A. |
| `power` | float | W. Reported by the FC where available, otherwise `voltage × current`. |
| `mahDrawn` | int | mAh consumed since power-on. |
| `percentage` | int | 0–100, the FC's own estimate. |
| `cellCount` | int | Detected cells (0 = unknown). |
| `throttle` | int · null | 0–100 %, from INAV `MISC2` / MAVLink `VFR_HUD`; `null` when the link does not carry it. |
| `rssi` | int | Raw RSSI as the source protocol reports it (INAV: 0–1023). Use `link` for a normalised value. |

#### `batteries` — sources: MAVLink (`BATTERY_STATUS`)

Array, one object per battery instance: `id`, `voltage` (V), `current` (A), `mahDrawn`, `percentage`,
`cellCount`, `temperature` (°C or `null`). Single-battery setups still get one entry. `null` on INAV
and passive links — use `battery` there.

#### `link` — sources: CRSF, S.Port, LTM, INAV 9.1+, MAVLink `RADIO_STATUS`

| Field | Type | Unit |
| --- | --- | --- |
| `rssiPercent` | float · null | 0–100, normalised across protocols. |
| `rssiDbm` | int · null | Raw dBm (negative), where the protocol reports it. |
| `lq` | int · null | Link quality, 0–100. |
| `snrDb` | int · null | Signal-to-noise ratio, dB. |

#### `status` — sources: MSP, MAVLink (passive links fill `armed` / flags coarsely)

| Field | Type | Meaning |
| --- | --- | --- |
| `armed` | bool | Derived from `armingFlags` bit 2 — the one field almost every consumer wants. |
| `armingFlags` | int | INAV `armingFlag_e` bitfield; MAVLink links map the heartbeat's armed bit onto bit 2. Other bits are arming blockers (INAV). |
| `flightModeFlags` | int | Raw mode bitfield (INAV boxes) / raw custom mode (MAVLink). Prefer `flightMode`. |
| `cpuLoad` | int | %, INAV. |
| `sensorStatus` | int | INAV sensor-present bitfield. Prefer `sensors`. |
| `mspRcOverride` | bool | INAV's *MSP RC Override* box is active (Kite RC control engaged). |
| `fcAlive` | bool · null | The flight controller itself is talking; `false` while the link carrier is up but the FC went quiet. |

#### `sensors` — sources: MSP (INAV), MAVLink (`SYS_STATUS`)

Per-sensor health, INAV `hardwareSensorStatus_e` values: **0** none/absent · **1** OK · **2**
unavailable · **3** unhealthy. Fields: `gyro`, `acc`, `mag`, `baro`, `gps`, `rangefinder`, `pitot`,
`opflow`. MAVLink-only extras: `prearm` (0 unknown · 1 ready · 2 blocked) and `rcReceiver` (0 absent ·
1 OK · 3 unhealthy); INAV leaves both 0.

#### `flightMode` — sources: all

| Field | Type | Meaning |
| --- | --- | --- |
| `primary` | string | Kite's canonical mode id — the same ids the flight-mode widget shows (INAV `"manual"`, `"acro"`, `"angle"`, `"althold"`, `"poshold"`, `"cruise"`, `"rth"`, `"mission"`, `"failsafe"`; ArduPilot `"stabilize"`, `"loiter"`, `"ardu_auto"`, `"rtl"`, `"qloiter"`; PX4 `"px4_position"`, `"px4_return"`, …). Protocol-agnostic: an INAV box set and an ArduPilot custom mode both arrive as one id. |
| `modifiers` | string[] | Active modifier modes stacked on the primary (INAV style: `"althold"`, `"turnassist"`, …). Empty for single-mode protocols. |

#### `nav` — sources: MSP (INAV), MAVLink (`MISSION_CURRENT`)

| Field | Type | Meaning |
| --- | --- | --- |
| `navState` | int | INAV `navState` (0 when not navigating; ArduPilot always 0). |
| `activeWp` | int | The FC's current target waypoint, 0 = not flying a mission. |

#### `ekf` — sources: MAVLink (ArduPilot `EKF_STATUS_REPORT`, `AHRS_EKF_TYPE`)

`status` (Kite's collapsed 0–3 health level), `maxVariance`, `flags` (raw `EKF_STATUS_FLAGS`),
`ekfType` (2 = EKF2, 3 = EKF3). `null` on INAV and passive links.

#### `misc` — sources: MSP (INAV `MISC2`)

`autoThrottle` (navigation controls the throttle), `uptimeS` (seconds since boot), `flightTimeS`
(seconds since first arm).

#### `vehicle` — sources: MAVLink (ArduPilot)

`quadplane`: `true` when ArduPlane reports `Q_ENABLE = 1`. `null` otherwise.

#### `passiveProtocol` — sources: passive only

`primary`: the carrier Kite locked onto (`"CRSF"`, `"SmartPort"`, `"LTM"`, …); `secondary`: a protocol
tunneled inside it (`"MAVLink"` for ArduPilot passthrough), or `null`.

#### `lastUpdate`

Unix epoch milliseconds of the last telemetry event of any group. Compare with `ts` to see how stale
the vehicle data is: on a healthy link the difference stays below the link's slowest update interval.

## Examples

**Watch the stream** (any OS with netcat; on Windows `ncat` from Nmap works):

```bash
nc 127.0.0.1 27300
```

**One snapshot**:

```bash
curl -s http://127.0.0.1:27301/api/v1/telemetry | jq '.telemetry.gps, .telemetry.status.armed'
```

**Python, read the stream and print position + battery** (standard library only):

```python
import json, socket

with socket.create_connection(("127.0.0.1", 27300)) as s:
    f = s.makefile("r", encoding="utf-8")
    hello = json.loads(f.readline())
    assert hello["schema"] == 1, "unexpected API schema"
    for line in f:
        frame = json.loads(line)
        t = frame["telemetry"]
        if not frame["connected"] or t["gps"] is None:
            continue
        g, b = t["gps"], t["battery"]
        print(f'{g["lat"]:.6f} {g["lon"]:.6f} {g["altMsl"]:.0f} m  '
              f'{b["voltage"]:.1f} V  armed={t["status"]["armed"]}')
```

**Forward each frame to a web service** — a 20-line bridge is all an external platform needs; Kite
stays the program that *serves* telemetry and never holds anyone's credentials:

```python
import json, socket, urllib.request

URL = "https://example.com/ingest"   # your endpoint
with socket.create_connection(("127.0.0.1", 27300)) as s:
    f = s.makefile("r", encoding="utf-8")
    f.readline()                       # hello
    for line in f:
        req = urllib.request.Request(URL, data=line.encode(), method="PUT",
                                     headers={"Content-Type": "application/json"})
        try:
            urllib.request.urlopen(req, timeout=2)
        except Exception as e:          # keep reading; the stream does not wait for you
            print("upload failed:", e)
```

**Browser dashboard on the same machine** — the HTTP route allows cross-origin reads:

```js
setInterval(async () => {
  const f = await (await fetch('http://127.0.0.1:27301/api/v1/telemetry')).json();
  document.title = f.connected ? `${f.telemetry.altitude?.altitude ?? '–'} m` : 'no link';
}, 500);
```

## Good to know

- **Rate and pacing.** The frame rate is Kite's, not the aircraft's. Two frames can carry the same
  vehicle data (same `telemetry.lastUpdate`); a 10 Hz MAVLink attitude stream is sampled at the API
  rate. If you need every sample, record a `.tlog` or Blackbox log instead.
- **Reconnects.** When a new vehicle link comes up, the telemetry groups start from `null` again —
  no values from the previous aircraft leak into the new session. `home` follows the FC; `gcs` is
  Kite's own and stays.
- **Replay is not served.** The API reflects the live link only; playing back a logbook flight does
  not feed it.
- **Versioning.** Check `schema` in the hello line or the health route. Within schema 1, fields are only
  ever added.
- **Relay vs. API.** The **Relay** re-encodes telemetry into flight-controller protocols (LTM, MAVLink,
  CRSF, S.Port) for antenna trackers and other ground stations. The API is for software that just
  wants the numbers as JSON. Both can run at the same time.
