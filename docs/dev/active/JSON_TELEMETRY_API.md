# JSON Telemetry API (read-only export)

## Why

External services need Kite's live telemetry, but every existing egress path is a binary FC protocol
(LTM / MAVLink / CRSF / SmartPort over serial / BLE / TCP / UDP). A consumer that just wants "where is the
aircraft and is it armed" had to implement a protocol parser first.

This adds a read-only JSON export: a snapshot endpoint and a live stream, over plain HTTP. Each stream is
tagged with a **mission ID** so the consumer can attribute samples to a mission record.

## Shape

It is **not** a new subsystem. It is one `Encoder` (`json`) plus one `OutputSink` (`http`) inside the
existing Telemetry Relay (`src-tauri/src/telemetry_forward/`), so it inherits the relay's persisted config,
settings UI, connect/disconnect lifecycle, throughput stats and reconfigure reconciler. No producer
(MAVLink / MSP / passive) is touched — the relay already taps the app's own `telemetry-*` events.

```
telemetry-* events ─► RelayHub tap ─► TelemetryCache ─► JsonEncoder ─► HttpSink ─► GET /api/v1/…
                                      (6 unified fields)  (rate-limited)  (snapshot + SSE)
```

Because it rides the shared cache, the export works for **any** inbound protocol, not just MAVLink.

## Endpoints

Served on `127.0.0.1:<port>` (default 8080), or `0.0.0.0:<port>` if LAN is opted into.

| Route | Response |
|---|---|
| `GET /api/v1/telemetry` | `200 application/json` — the most recent frame. `503` if no telemetry has arrived yet. |
| `GET /api/v1/stream` | `200 text/event-stream` — SSE, one `data:` record per frame. |
| `GET /api/v1/health` | `200 application/json` — `{ ok, schema, missionId, hasData, streamClients }`. |

All responses carry `Access-Control-Allow-Origin: *` — safe, because the API is read-only and carries no
credentials — so a browser consumer can `fetch()` / `EventSource` it directly.

### Payload

Compact single-line JSON. `schema` is the contract version (`SCHEMA_VERSION` in `encoders/json.rs`); `seq`
is monotonic so a consumer can detect dropped frames. Telemetry blocks are **omitted entirely** when the
source hasn't reported them, rather than being zero-filled.

```json
{
  "schema": 1,
  "missionId": "ollebo-test-1",
  "ts": 1784018754527,
  "seq": 42,
  "attitude": { "roll": -2.1, "pitch": 4.8, "yaw": 271.3 },
  "gps": { "fixType": 3, "numSat": 14, "lat": 59.3293, "lon": 18.0686,
           "altMsl": 132.4, "groundSpeed": 17.2, "course": 271.0 },
  "altitude": { "altitude": 118.0, "vario": -0.4 },
  "battery": { "voltage": 22.1, "current": 8.4, "power": 185.6,
               "mahDrawn": 1420, "percentage": 63, "cellCount": 6, "rssi": 1023 },
  "status": { "armed": true, "armingFlags": 4, "flightModeFlags": 1,
              "cpuLoad": 21, "sensorStatus": 3 },
  "airspeed": { "airspeed": 18.9 }
}
```

`armed` is derived from `armingFlags` bit 2, which is normalized across MSP and MAVLink (mirrors
`ARMED_BIT` in `src/lib/helpers/arming.ts`) — consumers shouldn't have to decode the bitfield.

The DTO is defined explicitly rather than serializing the internal cache structs. Those are snake_case
internals that change with the frontend; a public contract must not be coupled to them.

## Decisions

**The mission ID is a hard gate, enforced backend-side.** `Relay::build` resolves it *before* constructing
the output sink, so with no mission ID the relay is refused and **no port is ever bound** — the feature is
genuinely off, not merely serving untagged data. The frontend flags the empty field, but the backend check
is the authoritative one (hand-editing localStorage does not get you an untagged API).

**HTTP output requires the JSON protocol.** The sink wraps each frame as an SSE record, so pairing it with
a binary encoder would emit garbage. The reverse is allowed: JSON out a serial/TCP/UDP sink is legitimate
newline-delimited JSON.

**SSE, not WebSocket.** The stream is one-way and read-only. A WebSocket handshake needs SHA-1 + base64 and
client-frame unmasking, and `tokio` is compiled without `net`/`rt-multi-thread` in the release build, so
axum/hyper/tungstenite aren't available without a significant dependency change. SSE is plain HTTP/1.1 text
and hand-rolls in ~40 lines on `std::net::TcpListener` — the same approach as `video/mjpeg_server.rs`.

**Loopback by default.** A tracker relay binds `0.0.0.0` because reaching the LAN is its whole purpose. A
telemetry API is different: it should not be silently readable by everyone on a field or public network.
LAN exposure is an explicit per-relay opt-in.

**Rate-limited in the encoder.** The relay paces `frame_set` on the attitude update, which on MAVLink can
run at 10–50 Hz. `JsonEncoder` returns an empty `Vec` when called too soon; `Relay::emit_set` already
early-returns on that, so no frame is written and the byte/frame counters correctly stay put. Default 5 Hz,
configurable 0.1–50.

**Stalled consumers can't stall the relay.** `HttpSink::write` runs on the Tauri event-listener thread that
drives *every* relay's dispatch. So: per-client write timeout (2 s) with dead clients dropped; the snapshot
mutex is never held across a socket write; and each connection is served on its own thread so a client that
connects and goes silent can't block the accept loop.

## Lifecycle (known limitation)

Relays are configured on primary connect and cleared on disconnect, so **the HTTP server binds on connect
and disappears on disconnect** — a consumer gets connection-refused, not a "no vehicle" response, whenever
Kite isn't connected to an aircraft. If a persistent endpoint is needed, the server has to be lifted out of
the relay lifecycle; that's a deliberate follow-up, not an oversight.

## Worked example: streaming a flight into an Ollebo mission

[Ollebo](https://www.ollebo.com) is one concrete consumer — a free service for hosting drone maps and
showing live missions. It's used here purely to show the export doing real work; **no Ollebo-specific code
exists in Kite**, and the same pattern fits any consumer.

Ollebo ingests telemetry as `PUT https://api.ollebo.com/event/<mission-key>` and replays it on its own live
map. So the bridge is a plain client: read Kite's SSE stream, map each frame to an Ollebo event, PUT it.

> **The Ollebo mission key is a credential**, not just a label — anyone holding it can write to that
> mission. Keep it in the bridge (below), and be careful about putting it in Kite's `missionId` field: that
> value is stamped into every frame, so with the LAN checkbox on it would be readable by anyone on the
> network. The `missionId` field is intended as a plain identifier; loopback is the default for a reason.

```python
#!/usr/bin/env python3
"""Feed Kite-GC's JSON telemetry export into an Ollebo mission. Stdlib only.

    python3 ollebo-bridge.py --mission-key <uuid>
"""
import argparse, json, urllib.request


def to_ollebo(frame, device):
    """Kite frame -> Ollebo event. Skips frames with no usable GPS fix."""
    gps = frame.get("gps")
    if not gps or gps.get("fixType", 0) < 2:
        return None
    lon, lat = gps["lon"], gps["lat"]
    alt = (frame.get("altitude") or {}).get("altitude", gps.get("altMsl", 0.0))
    return {
        "type": "telemetry",
        "device": device,
        "geopoint": [lon, lat],
        "x": lon, "y": lat, "z": alt,
        "data": alt,
        "jsonData": {
            "seq": frame.get("seq"),
            "ts": frame.get("ts"),
            "armed": (frame.get("status") or {}).get("armed"),
            "heading": (frame.get("attitude") or {}).get("yaw"),
            "speed": gps.get("groundSpeed"),
            "battery": (frame.get("battery") or {}).get("percentage"),
            "voltage": (frame.get("battery") or {}).get("voltage"),
        },
    }


def put_event(api, key, event):
    req = urllib.request.Request(
        f"{api}/event/{key}",
        data=json.dumps(event).encode("utf-8"),
        method="PUT",
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=10) as r:
        return r.status


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--kite", default="http://127.0.0.1:8080", help="Kite JSON export")
    p.add_argument("--api", default="https://api.ollebo.com")
    p.add_argument("--mission-key", required=True, help="per-mission credential")
    p.add_argument("--device", default="kite-gc")
    a = p.parse_args()

    with urllib.request.urlopen(f"{a.kite}/api/v1/stream") as stream:
        for raw in stream:                       # SSE records: "data: {...}\n\n"
            line = raw.decode("utf-8", "replace").strip()
            if not line.startswith("data: "):
                continue
            event = to_ollebo(json.loads(line[6:]), a.device)
            if event:
                put_event(a.api, a.mission_key, event)


if __name__ == "__main__":
    main()
```

Set the relay to JSON/HTTP with a mission ID, connect, then run the bridge — the flight shows up live on
the Ollebo mission map.

Note the shape of this: Kite stays vendor-neutral and just *serves* telemetry; the consumer does the
pushing. An in-app push sink (Kite PUTs directly, no bridge process) is a plausible follow-up, but it would
put a third-party endpoint and a credential inside the app, which is a much bigger ask of the project.

## Out of scope / follow-ups

- **Richer payload.** The relay cache carries six unified fields; the MAVLink handler emits ~20 event types
  (wind, EKF status, RC channels, per-battery data, nav status, statustext, GPS stats…). Adding them is
  purely additive — a field on `TelemetryCache`, a `tap!` line in `telemetry_forward/mod.rs`, a field on the
  DTO — and touches no producer.
- **Full-fidelity raw MAVLink.** For a consumer that speaks MAVLink natively, the raw frames are already
  captured at `mavlink_proto/handler.rs` (`frame.raw_bytes`, for the `.tlog`). A passthrough sink there
  would be a different, larger feature.
- **Auth.** Not needed while read-only and loopback. Would be required before any write path, or before
  LAN exposure is made the default.

## Files

| File | Role |
|---|---|
| `src-tauri/src/telemetry_forward/encoders/json.rs` | DTO, mission-ID stamping, rate limiter |
| `src-tauri/src/telemetry_forward/output/http.rs` | HTTP server, snapshot + SSE broadcast |
| `src-tauri/src/telemetry_forward/relay.rs` | Config fields + factory, the mission-ID gate |
| `src/lib/stores/relay.ts` | `json` / `http` types, `missionIdMissing()`, defaults |
| `src/lib/components/RelayPanel.svelte` | Protocol/output options, mission-ID field, port-collision guard |
