"""Minimal LTM telemetry simulator — a fake aircraft for Kite's passive "Telemetry" protocol.

Streams LightTelemetry frames (`$T<type><payload><xor>`) the way an INAV LTM output would: A
(attitude) / G (gps) / S (status) at --rate, O (home) at 1 Hz. The aircraft arms after --arm-after
seconds, then flies a circle of --radius m around --home at a climbing altitude in CRUISE (LTM mode
18) with a slowly sagging battery. No hardware, no SITL — enough to exercise the link, the recorder,
the foreground-service notification and the track backfill on a phone
(Dev-Docs active/BACKGROUND_TELEMETRY.md).

Kite side: Protocol = Telemetry, Transport = UDP, host = this machine, port = --port. Kite binds
that port locally and learns the peer from the first datagram, so send TO the phone:

    python tools/ltm_sim.py --udp 192.168.1.87:14551

TCP variant (Kite connects to us — pairs with `adb reverse tcp:14551 tcp:14551` when the phone has
no Wi-Fi): `python tools/ltm_sim.py --tcp 14551`, Kite host = 127.0.0.1, port 14551.
"""
import argparse
import math
import socket
import struct
import time

LTM_MODE_CRUISE = 18


def frame(ty: bytes, payload: bytes) -> bytes:
    crc = 0
    for b in payload:
        crc ^= b
    return b"$T" + ty + payload + bytes([crc])


def a_frame(pitch: float, roll: float, yaw: float) -> bytes:
    return frame(b"A", struct.pack("<hhh", int(pitch), int(roll), int(yaw)))


def g_frame(lat: float, lon: float, gs_ms: float, alt_m: float, sats: int, fix: int) -> bytes:
    return frame(b"G", struct.pack("<iiBiB", int(lat * 1e7), int(lon * 1e7), int(gs_ms), int(alt_m * 100), (sats << 2) | fix))


def s_frame(volts: float, mah: int, rssi: int, airspeed_ms: float, armed: bool, failsafe: bool, mode: int) -> bytes:
    statemode = (mode << 2) | (0x02 if failsafe else 0) | (0x01 if armed else 0)
    return frame(b"S", struct.pack("<HHBBB", int(volts * 1000), mah, rssi, int(airspeed_ms), statemode))


def o_frame(lat: float, lon: float, alt_m: float, fix_home: int) -> bytes:
    return frame(b"O", struct.pack("<iiiBB", int(lat * 1e7), int(lon * 1e7), int(alt_m * 100), 1, fix_home))


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--udp", metavar="HOST:PORT", help="send datagrams to the phone (Kite: Telemetry · UDP)")
    ap.add_argument("--tcp", metavar="PORT", type=int, help="serve one TCP client (Kite: Telemetry · TCP)")
    ap.add_argument("--rate", type=float, default=5.0, help="A/G/S frames per second (default 5)")
    ap.add_argument("--home", default="48.1000,11.5000", help="home lat,lon")
    ap.add_argument("--radius", type=float, default=300.0, help="circle radius in m")
    ap.add_argument("--speed", type=float, default=15.0, help="ground speed in m/s")
    ap.add_argument("--arm-after", type=float, default=3.0, help="seconds on the ground before arming")
    ap.add_argument("--seconds", type=float, default=0, help="stop after N seconds (0 = run until Ctrl-C)")
    args = ap.parse_args()
    if not args.udp and not args.tcp:
        ap.error("one of --udp HOST:PORT or --tcp PORT is required")

    home_lat, home_lon = (float(x) for x in args.home.split(","))
    home_alt = 480.0  # m MSL at home

    if args.udp:
        host, port = args.udp.rsplit(":", 1)
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        target = (host, int(port))
        send = lambda b: sock.sendto(b, target)
        print(f"LTM -> udp {host}:{port} at {args.rate:g} Hz; Kite: Telemetry - UDP - host = this machine, port {port}")
    else:
        srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        srv.bind(("0.0.0.0", args.tcp))
        srv.listen(1)
        print(f"LTM tcp server on :{args.tcp} - waiting for Kite (Telemetry - TCP)...")
        conn, peer = srv.accept()
        print(f"client {peer}")
        send = conn.sendall

    t0 = time.monotonic()
    period = 1.0 / args.rate
    next_tick = t0
    last_o = 0.0
    mah = 0
    n = 0
    try:
        while True:
            now = time.monotonic()
            if now < next_tick:
                time.sleep(next_tick - now)
                continue
            next_tick += period
            t = now - t0
            if args.seconds and t > args.seconds:
                break
            armed = t >= args.arm_after
            ft = max(0.0, t - args.arm_after)  # flight time
            if armed:
                # Circle: angular speed = v / r; climb to 120 m AGL over the first two minutes.
                ang = (args.speed / args.radius) * ft
                dlat = (args.radius * math.cos(ang)) / 111_320.0
                dlon = (args.radius * math.sin(ang)) / (111_320.0 * math.cos(math.radians(home_lat)))
                lat, lon = home_lat + dlat, home_lon + dlon
                agl = min(120.0, ft * 1.0)
                gs = args.speed
                yaw = (math.degrees(ang) + 90.0) % 360.0
                roll = 20.0
            else:
                lat, lon, agl, gs, yaw, roll = home_lat, home_lon, 0.0, 0.0, 90.0, 0.0
            volts = 12.6 - min(2.0, ft / 300.0)  # 12.6 V → 10.6 V over 10 minutes
            if armed:
                mah += int(8_000 / 3_600 * period)  # ~8 A
            send(a_frame(pitch=2.0, roll=roll, yaw=yaw))
            send(g_frame(lat, lon, gs, home_alt + agl, sats=12, fix=3))
            send(s_frame(volts, mah, rssi=200, airspeed_ms=gs, armed=armed, failsafe=False, mode=LTM_MODE_CRUISE))
            if now - last_o >= 1.0:
                send(o_frame(home_lat, home_lon, home_alt, fix_home=1))
                last_o = now
            n += 1
            if n % int(args.rate * 10) == 0:
                print(f"t={t:6.0f}s armed={armed} pos={lat:.5f},{lon:.5f} agl={agl:5.1f} m {volts:.2f} V", flush=True)
    except KeyboardInterrupt:
        pass
    print("stopped")


if __name__ == "__main__":
    main()
