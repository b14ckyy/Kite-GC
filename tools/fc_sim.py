"""Flight controller simulator for testing Kite Ground Control without hardware.

One process, three combinations. `--firmware ardupilot` is an ArduPilot board speaking MAVLink over
UDP; `--firmware inav` is an INAV board speaking MSP over TCP or UDP; `--firmware inav --protocol
mavlink` is the same INAV board on its own MAVLink port, which supports far less than ArduPilot's.
Everything Kite consumes is covered end to end: the handshake, live telemetry, arming, flight modes,
guided reposition, missions, parameters or settings, and direct RC stick control. No third-party
packages.

Both stacks are modelled on their current master, not on any released version: ArduPlane 4.8.0-dev
and INAV 9.1.0. Parameter names, defaults, message sets and command handling all come from those
trees, so where a behaviour changed between releases only the current one is reproduced.

MAVLink message layouts, CRC_EXTRA seeds and enum values are all derived at runtime from the
MAVLink dialect XML, so nothing here can drift from the dialect the app links. MSP layouts follow
INAV's own fc_msp.c field order, including the ones whose width is easy to get wrong.

  Kite side, ArduPilot: connection type UDP, host 127.0.0.1, port 14550.
    The sim binds 14550 first, so Kite's own bind falls back to an ephemeral port and its GCS
    HEARTBEAT teaches the sim where to stream (transport/udp.rs peer learning).
  Kite side, INAV: protocol MSP, transport TCP, host 127.0.0.1, port 5761.

Usage: python3 tools/fc_sim.py [--firmware ardupilot|inav] [--protocol mavlink|msp]
       [--vehicle plane|copter] [--transport udp|tcp] [--port N] [--lat LAT --lon LON]
       [--radius M] [--speed MS] [--alt M] [--no-fix] [--disarmed] [--no-gcs-nav]
       [--chatter] [--verbose] [--defs PATH] [--version V]

  --firmware   which flight stack to imitate, and with it the default protocol, transport and
               port: ardupilot means MAVLink over UDP 14550, inav means MSP over TCP 5761
               (default ardupilot)
  --protocol   overrides that pairing. `--firmware inav --protocol mavlink` gives INAV's MAVLink
               port; ArduPilot never speaks MSP, so that pairing is rejected
  --vehicle    airframe to imitate; sets MAV_TYPE or the INAV platform type, the mode numbering,
               orbit geometry, cruise speed and battery cell count (default plane)
  --no-fix     report no GPS fix with 0 satellites, and drop GPS + prearm out of the health mask
               (exercises the prearm / GPS warning UI)
  --disarmed   start disarmed and parked at home
  --no-gcs-nav INAV over MAVLink: no GCS NAV box on a switch, which is what makes a real board
               refuse every guided target
  --chatter    ArduPilot only: emit a periodic STATUSTEXT nag (exercises the toast de-dup)
  --defs       MAVLink only: path to ardupilotmega.xml (default: the vendored mavlink crate)
  --version    INAV only: the firmware version to report (default 9.1.0, INAV master)
  --verbose    log every command, mission exchange and stream re-rate

The airframe is a point mass with turn-rate, climb-rate and acceleration limits rather than a
scripted path, so the telemetry follows from the same state the commands mutate. A plane cannot
hover, so every "hold position" becomes an orbit; a copter stops and hovers.

ArduPilot commands honoured, each changing real vehicle state rather than being acknowledged and
ignored: COMPONENT_ARM_DISARM (refused without a 3D fix, and in flight, unless forced with 21196),
DO_SET_MODE, NAV_TAKEOFF, NAV_LAND, NAV_RETURN_TO_LAUNCH, MISSION_START, DO_REPOSITION,
DO_CHANGE_SPEED, CONDITION_YAW, GUIDED_CHANGE_HEADING, DO_SET_HOME, DO_PAUSE_CONTINUE,
DO_SET_MISSION_CURRENT, DO_GO_AROUND, REQUEST_MESSAGE and SET_MESSAGE_INTERVAL (which really does
re-rate the stream). Missions upload, download, clear and then fly. Parameters read, list and set.
RC_CHANNELS_OVERRIDE and MANUAL_CONTROL fly the aircraft in the stick modes and are echoed back in
RC_CHANNELS. DO_VTOL_TRANSITION is DENIED because Q_ENABLE is 0, and anything not listed is
answered MAV_RESULT_UNSUPPORTED rather than a fake success.

INAV over MAVLink (--firmware inav --protocol mavlink) is deliberately poorer than ArduPilot, and
all of this is what telemetry/mavlink.c does on master rather than a simplification:
  - COMMAND_LONG is an unfinished TODO in the firmware, so every command sent that way (arm,
    disarm, mode, takeoff, land, RTL, change speed, set home) is dropped with no ACK at all.
  - COMMAND_INT handles exactly one command, DO_REPOSITION, and only with frame MAV_FRAME_GLOBAL.
    The relative-alt and terrain frames are commented out, so they come back
    MAV_RESULT_UNSUPPORTED. The altitude in that frame is nevertheless used as metres above home,
    not as the AMSL the frame means: see "the altitude trap" below, and do not validate a GCS's
    altitude conversion against the spec here.
  - Guided targets also need isGCSValid(): armed, a trusted position estimate, a valid GPS origin,
    POSHOLD as the nav state, and the GCS NAV box active on a switch. Anything else is DENIED.
    Over MAVLink the only way into POSHOLD is the mode channel in RC_CHANNELS_OVERRIDE, exactly as
    on a real aircraft.
  - Missions use MISSION_ITEM and MISSION_REQUEST, not the _INT variants, which are ignored.
  - PARAM_REQUEST_LIST answers with a single empty PARAM_VALUE and a count of 0; there is no
    parameter read or write, and no AUTOPILOT_VERSION, so a GCS cannot even learn the firmware
    version over this port.
  - Nothing ArduPilot-specific is sent: no HOME_POSITION, POSITION_TARGET_GLOBAL_INT,
    NAV_CONTROLLER_OUTPUT, MISSION_CURRENT, WIND or EKF_STATUS_REPORT.
INAV advertises MAV_AUTOPILOT_ARDUPILOTMEGA here, which is `mavlink_autopilot_type = ARDUPILOT`
on a real board and what INAV users are told to set for GCS use. The firmware can also advertise
GENERIC (its default), but then a GCS has no mode table to apply, while INAV keeps sending
ArduPlane mode numbers anyway because inavToArduPlaneMap runs regardless of that setting.

INAV has no mode-change or arm command: the real firmware takes both from RC channels, so the sim
does too, and Kite's RC page drives them through MSP_SET_RAW_RC:
  CH5  arm switch:  < 1300 disarmed, > 1700 armed
  CH6  mode switch: < 1300 MANUAL, 1300..1700 POSHOLD, > 1700 WAYPOINT MISSION
  CH1..4 roll / pitch / throttle / yaw in the stick modes
Missions upload, download and fly; settings read and write by name, and a written radius or cruise
speed changes how the aircraft actually flies. Any request the sim does not implement is logged
with --verbose and answered with an MSP error frame rather than a plausible-looking empty payload.
"""
import argparse
import glob
import math
import os
import socket
import struct
import sys
import time
import xml.etree.ElementTree as ET

# ── MAVLink wire types ───────────────────────────────────────────────────────

# struct format + byte size per MAVLink field type. `char` travels as a byte; the
# `uint8_t_mavlink_version` pseudo-type is a uint8_t that must still hash as "uint8_t".
TYPES = {
    'float': ('f', 4), 'double': ('d', 8),
    'int8_t': ('b', 1), 'uint8_t': ('B', 1), 'char': ('B', 1),
    'int16_t': ('h', 2), 'uint16_t': ('H', 2),
    'int32_t': ('i', 4), 'uint32_t': ('I', 4),
    'int64_t': ('q', 8), 'uint64_t': ('Q', 8),
}


def crc16_mcrf4xx(data, crc=0xFFFF):
    """CRC-16/MCRF4XX: the MAVLink frame checksum and the CRC_EXTRA hash."""
    for byte in data:
        tmp = (byte ^ crc) & 0xFF
        tmp = (tmp ^ (tmp << 4)) & 0xFF
        crc = ((crc >> 8) ^ (tmp << 8) ^ (tmp << 3) ^ (tmp >> 4)) & 0xFFFF
    return crc


class Field:
    __slots__ = ('name', 'type', 'count', 'fmt', 'size')

    def __init__(self, name, decl):
        self.name = name
        base, _, arr = decl.partition('[')
        # uint8_t_mavlink_version hashes and packs exactly as uint8_t.
        self.type = 'uint8_t' if base == 'uint8_t_mavlink_version' else base
        self.count = int(arr.rstrip(']')) if arr else 1
        self.fmt, self.size = TYPES[self.type]


class Dialect:
    """Message layouts + enum values parsed from the MAVLink XML (follows <include> chains)."""

    def __init__(self, path):
        self.messages = {}   # name -> (msgid, base_fields_sorted, ext_fields, crc_extra)
        self.enums = {}      # enum name -> {entry name: value}
        seen = set()
        self._load(path, seen)

    def _load(self, path, seen):
        path = os.path.realpath(path)
        if path in seen or not os.path.exists(path):
            return
        seen.add(path)
        root = ET.parse(path).getroot()
        # Includes first, so same-named local definitions win over inherited ones.
        for inc in root.findall('include'):
            self._load(os.path.join(os.path.dirname(path), inc.text.strip()), seen)
        for en in root.findall('./enums/enum'):
            entries = self.enums.setdefault(en.get('name'), {})
            for e in en.findall('entry'):
                if e.get('value') is not None:
                    entries[e.get('name')] = int(str(e.get('value')), 0)
        for msg in root.findall('./messages/message'):
            self._add_message(msg)

    def _add_message(self, msg):
        base, ext, in_ext = [], [], False
        for child in msg:
            if child.tag == 'extensions':
                in_ext = True
            elif child.tag == 'field':
                (ext if in_ext else base).append(Field(child.get('name'), child.get('type')))
        # Wire order: base fields sorted by descending type size (stable, so declaration order
        # breaks ties), then extension fields in declaration order. Only base fields are hashed.
        base.sort(key=lambda f: -f.size)
        name = msg.get('name')
        self.messages[name] = (int(msg.get('id')), base, ext, self._crc_extra(name, base))

    @staticmethod
    def _crc_extra(name, fields):
        crc = crc16_mcrf4xx(f'{name} '.encode())
        for f in fields:
            crc = crc16_mcrf4xx(f'{f.type} {f.name} '.encode(), crc)
            if f.count > 1:
                crc = crc16_mcrf4xx(bytes([f.count]), crc)
        return (crc & 0xFF) ^ (crc >> 8)

    def enum(self, enum_name, entry):
        """Resolve an enum entry, so no magic number in this file is recalled from memory."""
        try:
            return self.enums[enum_name][entry]
        except KeyError:
            sys.exit(f'{enum_name}.{entry} missing from the dialect XML')

    def pack(self, name, values):
        """Build a payload for `name` from a {field_name: value} dict (missing fields = 0)."""
        msgid, base, ext, _ = self.messages[name]
        out = bytearray()
        for f in base + ext:
            v = values.get(f.name, 0)
            if f.count > 1:
                if isinstance(v, str):
                    v = v.encode()
                if isinstance(v, (bytes, bytearray)):
                    v = list(v[:f.count]) + [0] * max(0, f.count - len(v))
                elif not isinstance(v, (list, tuple)):
                    v = [v] + [0] * (f.count - 1)
                out += struct.pack('<' + f.fmt * f.count, *v[:f.count])
            else:
                out += struct.pack('<' + f.fmt, int(v) if f.fmt not in 'fd' else v)
        return msgid, bytes(out)

    def unpack_name(self, msgid):
        for name, (mid, *_rest) in self.messages.items():
            if mid == msgid:
                return name
        return None

    def unpack(self, name, payload):
        """Decode a payload into a dict. Tolerates v2 truncation and unknown trailing bytes.

        A v2 sender trims trailing zero bytes, which can cut through the MIDDLE of the last field:
        PARAM_REQUEST_READ for "WP_LOITER_RAD" arrives with 13 of its 16 param_id bytes. Zero-pad
        the payload to the full declared size and decode normally, so a partially delivered array
        still yields its bytes. Treating a partial field as absent (the obvious reading of "treat
        missing as zero") silently emptied every truncated string, which looked like the GCS asking
        for a nameless parameter.
        """
        _msgid, base, ext, _crc = self.messages[name]
        want = sum(f.size * f.count for f in base + ext)
        buf = bytes(payload) + b'\x00' * max(0, want - len(payload))
        out, off = {}, 0
        for f in base + ext:
            chunk = struct.unpack_from('<' + f.fmt * f.count, buf, off)
            out[f.name] = chunk[0] if f.count == 1 else chunk
            off += f.size * f.count
        return out


def find_dialect():
    """Locate ardupilotmega.xml inside the vendored `mavlink` crate (the dialect Kite links)."""
    pattern = os.path.expanduser(
        '~/.cargo/registry/src/*/mavlink-*/mavlink/message_definitions/v1.0/ardupilotmega.xml')
    hits = sorted(glob.glob(pattern))
    return hits[-1] if hits else None


# ── Framing ──────────────────────────────────────────────────────────────────

class Link:
    """MAVLink v2 framing over a UDP socket, with peer learning like the app's own transport."""

    def __init__(self, dialect, sock, sysid=1, compid=1):
        self.d, self.sock, self.sysid, self.compid = dialect, sock, sysid, compid
        self.seq = 0
        self.peer = None
        self.rx = bytearray()
        # When set, only these message names go out. Used to hold the INAV flavour down to the
        # messages INAV's telemetry/mavlink.c actually packs, so a GCS relying on an ArduPilot-only
        # message (HOME_POSITION, POSITION_TARGET_GLOBAL_INT, AUTOPILOT_VERSION) sees the same
        # silence it would get from a real INAV board.
        self.allow = None
        self.suppressed = set()

    def send(self, name, **values):
        if self.peer is None:
            return
        if self.allow is not None and name not in self.allow:
            self.suppressed.add(name)
            return
        msgid, payload = self.d.pack(name, values)
        crc_extra = self.d.messages[name][3]
        header = struct.pack('<BBBBBB', len(payload), 0, 0, self.seq, self.sysid, self.compid)
        header += struct.pack('<I', msgid)[:3]
        crc = crc16_mcrf4xx(header + payload)
        crc = crc16_mcrf4xx(bytes([crc_extra]), crc)
        self.sock.sendto(b'\xfd' + header + payload + struct.pack('<H', crc), self.peer)
        self.seq = (self.seq + 1) & 0xFF

    def poll(self):
        """Drain the socket and yield (name, fields) for every complete frame (v1 and v2)."""
        while True:
            try:
                data, addr = self.sock.recvfrom(4096)
            except BlockingIOError:
                return
            except OSError:
                return
            if self.peer != addr:
                self.peer = addr
                print(f'[sim] GCS at {addr[0]}:{addr[1]}, streaming')
            self.rx += data
            yield from self._frames()

    def _frames(self):
        while self.rx:
            magic = self.rx[0]
            if magic == 0xFD:
                hdr, idlen = 10, 3
            elif magic == 0xFE:
                hdr, idlen = 6, 1
            else:
                del self.rx[0]
                continue
            if len(self.rx) < hdr:
                return
            plen = self.rx[1]
            total = hdr + plen + 2
            if magic == 0xFD and (self.rx[2] & 0x01):
                total += 13                                   # signed frame: skip the signature
            if len(self.rx) < total:
                return
            frame, self.rx = bytes(self.rx[:total]), self.rx[total:]
            msgid = int.from_bytes(frame[hdr - idlen:hdr], 'little')
            name = self.d.unpack_name(msgid)
            if name:
                yield name, self.d.unpack(name, frame[hdr:hdr + plen])


# ── Geometry helpers ─────────────────────────────────────────────────────────

R_EARTH = 6378137.0
G = 9.80665


def clamp(x, lo, hi):
    return lo if x < lo else hi if x > hi else x


def wrap_pi(a):
    """Fold an angle into (-pi, pi], so heading errors take the short way round."""
    return (a + math.pi) % (2 * math.pi) - math.pi


def dist_m(lat1, lon1, lat2, lon2):
    """Flat-earth distance. Good to a few cm over the km-scale legs a GCS test flies."""
    dn = math.radians(lat2 - lat1) * R_EARTH
    de = math.radians(lon2 - lon1) * R_EARTH * math.cos(math.radians((lat1 + lat2) / 2))
    return math.hypot(dn, de)


def bearing_to(lat1, lon1, lat2, lon2):
    dn = math.radians(lat2 - lat1)
    de = math.radians(lon2 - lon1) * math.cos(math.radians((lat1 + lat2) / 2))
    return math.atan2(de, dn)


def offset_m(lat, lon, north, east):
    return (lat + math.degrees(north / R_EARTH),
            lon + math.degrees(east / (R_EARTH * math.cos(math.radians(lat)))))


# ── Airframe profiles ────────────────────────────────────────────────────────

# ArduPilot custom_mode numbering is per-firmware and Kite maps the label from MAV_TYPE +
# custom_mode (flightmode/mod.rs), so these tables have to match that file or the UI names the
# wrong mode. Only the modes this sim can actually fly are listed; a DO_SET_MODE for anything
# else is answered DENIED rather than silently accepted.
PLANE_MODES = {0: 'manual', 1: 'circle', 2: 'stabilize', 5: 'fbwa', 6: 'fbwb', 7: 'cruise',
               10: 'auto', 11: 'rtl', 12: 'loiter', 13: 'takeoff', 15: 'guided'}
# The rest of ArduPlane's Mode::Number (ArduPlane/mode.h). Named but not flown: a DO_SET_MODE for
# one of these is a mode a real board would accept, so answering DENIED is the honest reply and
# quietly renumbering it would be worse. Kite's own plane table is missing AUTOLAND = 26.
PLANE_MODES_UNSIMULATED = {3: 'training', 4: 'acro', 8: 'autotune',
                           14: 'avoid_adsb', 16: 'initialising', 17: 'qstabilize', 18: 'qhover',
                           19: 'qloiter', 20: 'qland', 21: 'qrtl', 22: 'qautotune', 23: 'qacro',
                           24: 'thermal', 25: 'loiter_alt_qland', 26: 'autoland'}
COPTER_MODES = {0: 'stabilize', 2: 'althold', 3: 'auto', 4: 'guided', 5: 'loiter', 6: 'rtl',
                7: 'circle', 9: 'land'}

# Modes flown directly from the RC sticks. Everything else is autopilot-guided.
MANUAL_MODES = {'manual', 'stabilize', 'fbwa', 'fbwb', 'cruise', 'acro', 'althold'}

PROFILES = {
    'copter': dict(mav_type='MAV_TYPE_QUADROTOR', modes=COPTER_MODES,
                   auto_mode=3, loiter_mode=5, rtl_mode=6, land_mode=9, guided_mode=4,
                   radius=200.0, speed=10.0, alt=80.0, cells=3, nominal_v=12.6, throttle=48,
                   wind=0.0, wind_from=math.radians(225), fw='ArduCopter V4.8.0-dev',
                   fw_ver=(4, 8, 0),
                   # A copter can stop and hover, turns on the spot and climbs briskly. Its climb
                   # and sink are rates rather than pitch angles (ArduCopter PILOT_SPEED_UP 250 cm/s
                   # and PILOT_SPEED_DN, RTL_ALT 1500 cm), and it brakes into a waypoint instead of
                   # overflying it, so no miss angle applies.
                   bank_max=math.radians(35), yaw_rate=math.radians(90), climb=2.5, sink=1.5,
                   accel=3.0, wp_radius=6.0, can_hover=True, stall=0.0,
                   speed_max=10.0, rtl_alt=15.0, miss_angle=None,
                   pitch_up=math.radians(15), pitch_dn=math.radians(15)),
    'plane': dict(mav_type='MAV_TYPE_FIXED_WING', modes=PLANE_MODES,
                  auto_mode=10, loiter_mode=12, rtl_mode=11, land_mode=None, guided_mode=15,
                  radius=60.0, speed=12.0, alt=80.0, cells=4, nominal_v=16.8, throttle=62,
                  wind=2.5, wind_from=math.radians(225), fw='ArduPlane V4.8.0-dev',
                  fw_ver=(4, 8, 0),
                  # A plane cannot hover: below stall it stops being a simulation worth trusting,
                  # so speed is floored and every "hold position" becomes an orbit. The limits are
                  # ArduPlane's shipped defaults (ArduPlane/config.h): ROLL_LIMIT_DEG 45,
                  # PITCH_MAX 20, PITCH_MIN -25, AIRSPEED_CRUISE 12, AIRSPEED_FBW_MIN 9,
                  # AIRSPEED_FBW_MAX 22, LOITER_RADIUS_DEFAULT 60, WP_RADIUS_DEFAULT 90,
                  # ALT_HOLD_HOME (RTL_ALTITUDE) 100.
                  bank_max=math.radians(45), yaw_rate=math.radians(25),
                  pitch_up=math.radians(20), pitch_dn=math.radians(25),
                  accel=2.0, wp_radius=90.0, can_hover=False, stall=9.0,
                  speed_max=22.0, rtl_alt=100.0, miss_angle=math.radians(90)),
}

# Where INAV's fixed-wing defaults differ from ArduPlane's, applied on top of the plane profile for
# `--firmware inav` so each firmware flies to its own numbers. From src/main/fc/settings.yaml:
# nav_fw_bank_angle 35, nav_fw_climb_angle 20, nav_fw_dive_angle 15, nav_fw_loiter_radius 7500 cm,
# nav_wp_radius 100 cm, nav_rth_altitude 1000 cm, nav_min_ground_speed 7.
#
# The 1 m waypoint radius is not a typo and not unflyable: INAV also sequences a waypoint once the
# bearing to it swings more than 100 degrees off the original (`isWaypointReached`, navigation.c),
# which is what actually ends a leg for a fixed wing. ArduPlane does the same thing geometrically,
# by testing whether the aircraft has crossed the finish line through the waypoint
# (`past_interval_finish_line`, verify_nav_wp), so both need the overshoot rule and not just a
# radius.
INAV_FW_DEFAULTS = dict(
    bank_max=math.radians(35), pitch_up=math.radians(20), pitch_dn=math.radians(15),
    radius=75.0, wp_radius=1.0, rtl_alt=10.0, stall=7.0,
    miss_angle=math.radians(100),
)

# Default orbit centre: a real open test area, so the map has something sensible underneath it.
HOME_LAT = 61.48647335002389
HOME_LON = 23.804132386602753

# MAV_CMD numbers used by the mission items this sim executes. Resolved from the dialect at
# startup (see CMDS in main) rather than written as literals.
MISSION_CMD_NAMES = [
    'MAV_CMD_NAV_WAYPOINT', 'MAV_CMD_NAV_LOITER_UNLIM', 'MAV_CMD_NAV_LOITER_TURNS',
    'MAV_CMD_NAV_LOITER_TIME', 'MAV_CMD_NAV_RETURN_TO_LAUNCH', 'MAV_CMD_NAV_LAND',
    'MAV_CMD_NAV_TAKEOFF', 'MAV_CMD_DO_CHANGE_SPEED', 'MAV_CMD_DO_SET_HOME',
    'MAV_CMD_DO_JUMP', 'MAV_CMD_CONDITION_DELAY', 'MAV_CMD_CONDITION_YAW',
]


class Mission:
    """The vehicle's stored mission plus the upload/download state machine.

    Slot 0 is ArduPilot's home placeholder: the GCS uploads it and expects it back, but the
    vehicle never navigates to it, so execution starts at seq 1 (mission.rs `reserve_home`).
    """

    def __init__(self):
        self.items = []          # list of field dicts, index == seq
        self.current = 0         # seq the vehicle is navigating to (0 = not running)
        self.up_expect = None    # next seq we are waiting for during an upload
        self.up_count = 0
        self.up_buf = []

    def nav_items(self):
        return len(self.items)

    def item(self, seq):
        return self.items[seq] if 0 <= seq < len(self.items) else None


class Vehicle:
    """Point-mass airframe with turn-rate, climb-rate and acceleration limits.

    Deliberately not a fixed circular path any more: the GCS can arm it, change mode, upload and
    run a mission, reposition it in GUIDED, send it home, or fly it directly on the RC sticks, and
    the reported telemetry has to follow from the same state the commands mutate. Anything the
    sim cannot really do is refused at the command layer instead of being faked here.
    """

    def __init__(self, args, profile, mission):
        self.p = profile
        self.mission = mission
        self.notify = lambda text, sev='info': None   # replaced by main() once the link exists
        self.home_lat, self.home_lon = args.lat, args.lon
        self.home_alt = 30.0
        self.radius = args.radius if args.radius else profile['radius']
        self.cruise_alt = args.alt if args.alt else profile['alt']
        self.cruise_speed = args.speed if args.speed else profile['speed']
        self.speed = 0.0
        self.armed = not args.disarmed
        self.roll = self.pitch = 0.0
        self.yaw = math.pi / 2
        self.vz = 0.0
        self.t0 = time.time()
        self.t_last = self.t0
        self.voltage = profile['nominal_v']
        self.rc = {}              # 1-based channel -> microseconds, from RC_CHANNELS_OVERRIDE
        self.rc_t = 0.0           # last override arrival, for the failsafe/level-out below
        self.guided = None        # (lat, lon, alt) target while in GUIDED
        self.guided_yaw = None    # CONDITION_YAW / GUIDED_CHANGE_HEADING hold
        self.leg_seq = None       # mission leg whose bearing is captured in `leg_brg`
        self.leg_brg = 0.0
        # Sequence of the waypoint just reached, consumed by the protocol loop for
        # MISSION_ITEM_REACHED. Initialised here rather than only in update() so the model can be
        # driven step by step (offline checks) without hitting an undefined attribute.
        self.reached = None
        self.loiter = None        # (lat, lon, alt) centre while loitering
        self.takeoff_alt = None
        self.paused = False
        self.on_ground = not self.armed

        # Armed means already airborne on the orbit, which is what a GCS sees on first connect to a
        # flying aircraft. Disarmed means parked at home.
        if self.armed:
            self.lat, self.lon = offset_m(args.lat, args.lon, self.radius, 0.0)
            self.rel_alt = self.cruise_alt
            self.speed = self.cruise_speed
            self.mode = profile['modes'][profile['auto_mode']]
            self.loiter = (self.home_lat, self.home_lon, self.cruise_alt)
        else:
            self.lat, self.lon = args.lat, args.lon
            self.rel_alt = 0.0
            self.mode = profile['modes'][0]

        # --mode overrides the start mode. It exists mainly for INAV over MAVLink, where a GCS has
        # no way to change mode at all: without this the only route into POSHOLD would be the RC
        # mode channel.
        want = getattr(args, 'mode', None)
        if want:
            if want not in profile['modes'].values():
                sys.exit(f'--mode {want} is not one of: '
                         f'{", ".join(sorted(set(profile["modes"].values())))}')
            self.mode = want
            if want in ('loiter', 'circle'):
                self.loiter = (self.lat, self.lon, self.rel_alt)

    # ── mode plumbing ───────────────────────────────────────────────────────

    def mode_num(self):
        for num, name in self.p['modes'].items():
            if name == self.mode:
                return num
        return 0

    def set_mode(self, name):
        if name == self.mode:
            return True
        if name not in self.p['modes'].values():
            return False
        self.mode = name
        self.paused = False
        # Entering a hold mode pins the centre where the vehicle is now, which is what ArduPilot
        # does: LOITER captures the current position rather than returning to some earlier one.
        if name in ('loiter', 'circle'):
            self.loiter = (self.lat, self.lon, self.rel_alt)
        if name == 'auto' and self.mission.nav_items() > 1:
            self.mission.current = max(1, self.mission.current)
        self.notify(f'Kite SIM: mode {name.upper()}')
        return True

    # ── inputs ──────────────────────────────────────────────────────────────

    def rc_override(self, fields):
        """Ingest RC_CHANNELS_OVERRIDE. Sentinels differ per band (rc_tx.rs documents both):
        CH1-8 use 0 to release and UINT16_MAX to ignore, CH9-18 the other way round."""
        got = False
        for i in range(1, 19):
            us = int(fields.get(f'chan{i}_raw', 0) or 0)
            ignore = (us == 0xFFFF) if i <= 8 else (us == 0)
            release = (us == 0) if i <= 8 else (us == 65534)
            if ignore:
                continue
            if release:
                self.rc.pop(i, None)
                continue
            self.rc[i] = us
            got = True
        if got:
            self.rc_t = time.time()

    def manual_control(self, f):
        """MANUAL_CONTROL is the normalised form of the same thing: x/y/r span -1000..1000 and z
        spans 0..1000, so translate it into the microsecond channels the model already reads."""
        def us(v, lo=-1000.0):
            return int(1500 + (float(v) / 1000.0) * 500) if lo < 0 else int(1000 + float(v) / 2.0)
        self.rc[1] = us(f.get('y', 0))          # roll
        self.rc[2] = us(f.get('x', 0))          # pitch
        self.rc[3] = us(f.get('z', 0), lo=0)    # throttle
        self.rc[4] = us(f.get('r', 0))          # yaw
        self.rc_t = time.time()

    def _stick(self, ch, default=0.0):
        us = self.rc.get(ch)
        if us is None:
            return default
        return clamp((us - 1500) / 500.0, -1.0, 1.0)

    def _throttle_stick(self):
        us = self.rc.get(3)
        if us is None:
            return None
        return clamp((us - 1000) / 1000.0, 0.0, 1.0)

    # ── flight ──────────────────────────────────────────────────────────────

    def _turn_to(self, dt, target_yaw):
        """Turn toward a heading, rate-limited by the bank the airframe can hold."""
        err = wrap_pi(target_yaw - self.yaw)
        if self.p['can_hover']:
            rate_max = self.p['yaw_rate']
        else:
            # Coordinated turn: omega = g*tan(bank)/V. Slower flight turns tighter in radians.
            rate_max = G * math.tan(self.p['bank_max']) / max(self.speed, self.p['stall'])
        rate = clamp(err / max(dt, 1e-3), -rate_max, rate_max)
        self.yaw = (self.yaw + rate * dt) % (2 * math.pi)
        self._bank_for(rate)

    def _bank_for(self, rate):
        if self.p['can_hover']:
            self.roll = clamp(rate / max(self.p['yaw_rate'], 1e-6) * math.radians(12), -0.6, 0.6)
        else:
            self.roll = clamp(math.atan(rate * max(self.speed, 1.0) / G),
                              -self.p['bank_max'], self.p['bank_max'])

    def climb_limits(self):
        """Climb and sink rates the airframe can hold, in m/s.

        A copter's limits are rates in their own right. A plane's are angles: both firmwares cap the
        climb and the descent by pitch (ArduPlane PTCH_LIM_MAX_DEG / PTCH_LIM_MIN_DEG, INAV
        nav_fw_climb_angle / nav_fw_dive_angle), so the rate has to follow from the airspeed. That
        is why a slow plane climbs slowly and the same aircraft climbs faster with the throttle up,
        instead of every plane climbing at one hardcoded figure.
        """
        if self.p['can_hover']:
            return self.p['climb'], self.p['sink']
        v = max(self.speed, self.p['stall'])
        return v * math.sin(self.p['pitch_up']), v * math.sin(self.p['pitch_dn'])

    def _hold_alt(self, dt, target_alt):
        err = target_alt - self.rel_alt
        climb, sink = self.climb_limits()
        want = clamp(err, -sink, climb)
        self.vz = -want                                  # MAVLink vz is positive DOWN
        self.rel_alt = max(0.0, self.rel_alt + want * dt)
        self.pitch = clamp(math.asin(clamp(want / max(self.speed, 1.0), -0.9, 0.9)),
                           -self.p['pitch_dn'], self.p['pitch_up'])

    def _hold_speed(self, dt, target):
        target = max(target, self.p['stall'] if not self.on_ground else 0.0)
        step = self.p['accel'] * dt
        self.speed += clamp(target - self.speed, -step, step)
        self.speed = max(0.0, self.speed)

    def ground_vector(self):
        """North/east ground velocity: the air vector plus the wind vector, in m/s.

        `wind` is the speed the air moves and `wind_from` the direction it comes FROM, which is the
        convention MAVLink's WIND message uses. A wind from 225 degrees pushes the aircraft toward
        45 degrees, hence the reversal here.
        """
        vn = math.cos(self.yaw) * self.speed
        ve = math.sin(self.yaw) * self.speed
        if self.speed <= 0.5 or self.on_ground:
            return vn, ve                       # parked or rolling: no drift to add
        wind = self.p['wind']
        toward = self.p['wind_from'] + math.pi
        return vn + math.cos(toward) * wind, ve + math.sin(toward) * wind

    def _advance(self, dt):
        vn, ve = self.ground_vector()
        self.lat, self.lon = offset_m(self.lat, self.lon, vn * dt, ve * dt)

    def _goto(self, dt, lat, lon, alt, arrive, leg_bearing=None):
        """Fly toward a point; True once the leg is complete.

        A leg ends on distance *or* on overshoot, which is how both firmwares really do it: a plane
        that misses the acceptance radius must still move on rather than circle the waypoint for
        ever. ArduPlane tests whether the aircraft has crossed the finish line through the waypoint
        (`past_interval_finish_line`) and INAV whether the bearing has swung past its limit off the
        original leg bearing (`isWaypointReached`); both reduce to "the waypoint is now behind us",
        which is what `leg_bearing` plus the miss angle expresses here. Without it, INAV's 1 m
        default acceptance radius would never be met by a fixed wing.
        """
        d = dist_m(self.lat, self.lon, lat, lon)
        brg = bearing_to(self.lat, self.lon, lat, lon)
        self._turn_to(dt, brg)
        self._hold_alt(dt, alt)
        # A hovering airframe brakes into the point, otherwise it sails past at cruise speed and
        # circles the target for ever. A plane cannot slow below stall, so it flies through.
        want = min(self.cruise_speed, max(1.0, d * 0.6)) if self.p['can_hover'] else self.cruise_speed
        self._hold_speed(dt, want)
        if d <= arrive:
            return True
        miss = self.p['miss_angle']
        if miss is not None and leg_bearing is not None:
            # Only past a point where overshooting is even possible: within one turn diameter the
            # bearing swings wildly while the aircraft is still legitimately closing in.
            turn_d = max(self.radius, 2.0 * arrive)
            if d < turn_d and abs(wrap_pi(brg - leg_bearing)) > miss:
                return True
        return False

    def _orbit(self, dt, lat, lon, alt):
        """Hold a point: a copter hovers on it, a plane circles it at `radius`."""
        if self.p['can_hover']:
            d = dist_m(self.lat, self.lon, lat, lon)
            if d > 2.0:
                self._goto(dt, lat, lon, alt, 2.0)
            else:
                self._hold_alt(dt, alt)
                self._hold_speed(dt, 0.0)
                self.roll = self.pitch = 0.0
            return
        d = dist_m(self.lat, self.lon, lat, lon)
        brg = bearing_to(self.lat, self.lon, lat, lon)
        # Tangent, biased toward the centre when outside the circle and away when inside, so the
        # track converges on the radius instead of spiralling. Clockwise, like the old orbit.
        bias = clamp((d - self.radius) / max(self.radius, 1.0), -1.0, 1.0) * math.radians(60)
        self._turn_to(dt, brg - math.pi / 2 + bias)
        self._hold_alt(dt, alt)
        self._hold_speed(dt, self.cruise_speed)

    def _fly_rc(self, dt):
        """Stick flying. With no override for RC_DEADMAN the model levels out and holds, which is
        what a real FC does on RC failsafe rather than continuing the last input for ever."""
        stale = (time.time() - self.rc_t) > 1.5
        if stale or not self.rc:
            self._hold_alt(dt, self.rel_alt)
            self._hold_speed(dt, self.cruise_speed if not self.on_ground else 0.0)
            self.roll = 0.0
            return
        roll_cmd = self._stick(1) * self.p['bank_max']
        pitch_cmd = -self._stick(2)                       # stick forward (low us) = nose down
        yaw_cmd = self._stick(4)
        thr = self._throttle_stick()

        if self.p['can_hover']:
            # Copter: sticks are velocity demands, so neutral means hover in place.
            self.yaw = (self.yaw + yaw_cmd * self.p['yaw_rate'] * dt) % (2 * math.pi)
            self._hold_speed(dt, abs(pitch_cmd) * self.cruise_speed)
            self.roll = roll_cmd
            self.pitch = math.radians(-12) * abs(pitch_cmd)
            climb = ((thr - 0.5) * 2.0 * self.climb_limits()[0]) if thr is not None else 0.0
            self._hold_alt(dt, self.rel_alt + climb * dt * 4)
        else:
            # Plane: roll turns, elevator climbs, throttle sets speed.
            rate = G * math.tan(roll_cmd) / max(self.speed, self.p['stall'])
            self.yaw = (self.yaw + rate * dt) % (2 * math.pi)
            self.roll = roll_cmd
            target_speed = self.cruise_speed * (0.5 + thr) if thr is not None else self.cruise_speed
            self._hold_speed(dt, target_speed)
            self._hold_alt(dt, self.rel_alt + pitch_cmd * self.climb_limits()[0] * dt * 4)

    def _fly_mission(self, dt):
        """Execute stored items in sequence. Slot 0 is home and is never navigated to."""
        m = self.mission
        if m.nav_items() <= 1:
            self._orbit(dt, self.home_lat, self.home_lon, self.cruise_alt)   # nothing loaded
            return
        if m.current <= 0:
            m.current = 1
        it = m.item(m.current)
        if it is None:
            self.notify('Kite SIM: mission complete', 'notice')
            self.set_mode(self.p['modes'][self.p['loiter_mode']])
            return

        cmd = int(it.get('command', 0))
        lat, lon = it.get('x', 0) / 1e7, it.get('y', 0) / 1e7
        alt = float(it.get('z', 0.0)) or self.cruise_alt
        C = self.CMDS

        if cmd == C['MAV_CMD_NAV_WAYPOINT']:
            if self._goto(dt, lat, lon, alt, self.p['wp_radius'],
                          self._leg_bearing(m.current, lat, lon)):
                d = dist_m(self.lat, self.lon, lat, lon)
                # ArduPlane announces both outcomes by name (verify_nav_wp), and the distance in the
                # text is how a pilot tells a clean capture from an overshoot.
                verb = 'Reached' if d <= self.p['wp_radius'] else 'Passed'
                self.notify(f'{verb} waypoint #{m.current} dist {int(d)}m')
                self._reach(m.current)
        elif cmd in (C['MAV_CMD_NAV_LOITER_UNLIM'], C['MAV_CMD_NAV_LOITER_TURNS'],
                     C['MAV_CMD_NAV_LOITER_TIME']):
            # Turns/time are not counted: the item holds until the GCS moves the vehicle on, and
            # the STATUSTEXT says so rather than pretending a lap counter ran.
            self._orbit(dt, lat, lon, alt)
        elif cmd == C['MAV_CMD_NAV_TAKEOFF']:
            self.on_ground = False
            self._hold_speed(dt, self.cruise_speed)
            self._hold_alt(dt, alt)
            if self.rel_alt >= alt - 2.0:
                self._reach(m.current)
            return
        elif cmd == C['MAV_CMD_NAV_RETURN_TO_LAUNCH']:
            self.set_mode(self.p['modes'][self.p['rtl_mode']])
            return
        elif cmd == C['MAV_CMD_NAV_LAND']:
            land = self.p['land_mode']
            if land is not None:
                self.set_mode(self.p['modes'][land])
            else:
                # ArduPlane has no LAND mode; a real one flies the landing sequence in AUTO.
                self._land(dt, lat or self.home_lat, lon or self.home_lon)
            return
        elif cmd == C['MAV_CMD_DO_CHANGE_SPEED']:
            spd = float(it.get('param2', 0.0))
            if spd > 0:
                self.cruise_speed = spd
            self._reach(m.current)
        elif cmd == C['MAV_CMD_DO_SET_HOME']:
            if int(it.get('param1', 0)) != 1:
                self.home_lat, self.home_lon = lat, lon
            self._reach(m.current)
        elif cmd == C['MAV_CMD_DO_JUMP']:
            nxt = int(it.get('param1', 0))
            m.current = nxt if 1 <= nxt < m.nav_items() else m.current + 1
        else:
            # Unhandled item: skip it and say so, rather than stalling on it silently.
            self.notify(f'Kite SIM: skipping unsupported mission cmd {cmd} at seq {m.current}',
                        'warning')
            m.current += 1

    def _reach(self, seq):
        self.reached = seq            # picked up by the main loop for MISSION_ITEM_REACHED
        self.mission.current = seq + 1
        self.leg_seq = None           # the next leg captures its own bearing on first pass

    def _leg_bearing(self, seq, lat, lon):
        """Bearing to the waypoint as the leg began, captured once per leg.

        Both firmwares compare against the bearing at the *start* of the leg rather than a live one:
        ArduPlane stores prev_WP_loc and INAV `posControl.activeWaypoint.bearing`. Recomputing it
        every tick would make the comparison meaningless, since the difference would always be 0.
        """
        if self.leg_seq != seq:
            self.leg_seq = seq
            self.leg_brg = bearing_to(self.lat, self.lon, lat, lon)
        return self.leg_brg

    def _land(self, dt, lat, lon):
        self._goto(dt, lat, lon, 0.0, max(self.p['wp_radius'], 15.0))
        self._hold_alt(dt, 0.0)
        if self.rel_alt <= 0.3 and not self.on_ground:
            self.on_ground = True
            self.armed = False
            self.speed = 0.0
            self.notify('Kite SIM: landed and disarmed', 'notice')

    def update(self):
        now = time.time()
        dt = clamp(now - self.t_last, 0.0, 0.5)   # clamped so a stalled process cannot teleport it
        self.t_last = now
        t = now - self.t0
        self.reached = None

        if not self.armed:
            # A disarm in the air is a crash, not a teleport: keep sinking until the ground
            # arrives, then stay put. Snapping rel_alt to 0 would make the altitude trace lie.
            self.roll = self.pitch = 0.0
            if self.rel_alt > 0.0:
                sink = self.climb_limits()[1]
                self.rel_alt = max(0.0, self.rel_alt - sink * dt)
                self.vz = sink
                self._hold_speed(dt, 0.0)
            else:
                self.on_ground = True
                self.speed = self.vz = 0.0
        elif self.paused:
            self._orbit(dt, *(self.loiter or (self.home_lat, self.home_lon, self.rel_alt)))
        elif self.mode in MANUAL_MODES:
            self._fly_rc(dt)
        elif self.mode == 'takeoff':
            self.on_ground = False
            target = self.takeoff_alt or self.cruise_alt
            self._hold_alt(dt, target)
            self._hold_speed(dt, self.cruise_speed)
            if self.rel_alt >= target - 2.0:
                self.notify('Kite SIM: takeoff complete')
                nxt = 'auto' if self.mission.nav_items() > 1 else \
                    self.p['modes'][self.p['loiter_mode']]
                self.set_mode(nxt)
        elif self.mode == 'land':
            self._land(dt, self.lat, self.lon)
        elif self.mode == 'rtl':
            # RTL climbs to the firmware's return altitude (ArduPlane RTL_ALTITUDE / INAV
            # nav_rth_altitude), which is not the cruise altitude the vehicle happened to be at.
            rtl_alt = self.p['rtl_alt']
            if self._goto(dt, self.home_lat, self.home_lon, rtl_alt, self.p['wp_radius']):
                self._orbit(dt, self.home_lat, self.home_lon, rtl_alt)
        elif self.mode == 'guided':
            tgt = self.guided or (self.lat, self.lon, self.rel_alt)
            if self._goto(dt, tgt[0], tgt[1], tgt[2], self.p['wp_radius']):
                if self.guided_yaw is not None:
                    self._turn_to(dt, self.guided_yaw)
                    self._hold_alt(dt, tgt[2])
                    self._hold_speed(dt, self.cruise_speed)
                else:
                    self._orbit(dt, *tgt)
        elif self.mode == 'auto':
            self._fly_mission(dt)
        else:   # loiter, circle, althold and anything else that just holds
            centre = self.loiter or (self.home_lat, self.home_lon, self.cruise_alt)
            self._orbit(dt, *centre)

        # ONE integration step, after whichever branch above set heading, speed and altitude.
        # It used to live inside each branch, and the "AUTO with no mission loaded" path returned
        # early without it: the vehicle then reported a healthy 22 m/s while its position never
        # changed, which is the default state a GCS sees on first connect. Keeping it here means no
        # future mode can forget it. The unarmed case still moves, because a crashing aircraft
        # carries its momentum into the descent.
        self._advance(dt)

        self.omega = 0.0 if not self.armed else \
            (G * math.tan(self.roll) / max(self.speed, 1.0) if not self.p['can_hover']
             else self.roll / math.radians(12) * self.p['yaw_rate'])
        self.alt = self.home_alt + self.rel_alt
        # Ground track is the air vector plus the wind vector, so `speed` is the airspeed the model
        # controls and ground speed falls out of the geometry: upwind legs are slow over the ground
        # and downwind legs fast, and a loiter circle cycles between the two. Treating wind as a
        # constant added to ground speed instead would report an airspeed that no heading justifies
        # and would leave the aircraft tracking as if the air were still.
        vn, ve = self.ground_vector()
        self.vn, self.ve = vn, ve
        self.groundspeed = math.hypot(vn, ve)
        self.airspeed = self.speed
        # ~1 V of sag across a 12 min flight, so the battery widget visibly moves.
        self.voltage = max(self.p['nominal_v'] * 0.76, self.p['nominal_v'] - t / 720.0)
        return t


# ── Parameters ───────────────────────────────────────────────────────────────

# Kite reads AHRS_EKF_TYPE and Q_ENABLE by name (params.rs). The rest exist so PARAM_REQUEST_LIST
# returns something realistic to page through, and so a PARAM_SET round-trip can be tested against
# values that plausibly affect flight. Q_ENABLE stays 0: this sim is not a VTOL and refuses
# MAV_CMD_DO_VTOL_TRANSITION accordingly.
def profile_for(args):
    """The airframe profile, with INAV's fixed-wing defaults applied when that is the firmware.

    Both stacks fly the same model; what differs is the numbers they ship with, so the firmware
    selects the limits rather than every caller remembering to override them.
    """
    profile = dict(PROFILES[args.vehicle])
    if args.firmware == 'inav' and not profile['can_hover']:
        profile.update(INAV_FW_DEFAULTS)
        profile['fw'] = f'INAV {args.version}'
    return profile


def default_params(profile, v=None):
    p = {
        'AHRS_EKF_TYPE': 3.0, 'Q_ENABLE': 0.0,
        'SYSID_THISMAV': 1.0, 'SYSID_MYGCS': 255.0,
        'ARMING_CHECK': 1.0, 'ARMING_REQUIRE': 1.0,
        'BATT_MONITOR': 4.0, 'BATT_CAPACITY': 5000.0,
        'BATT_LOW_VOLT': round(profile['nominal_v'] * 0.85, 2),
        'BATT_CRT_VOLT': round(profile['nominal_v'] * 0.80, 2),
        'BATT_ARM_VOLT': round(profile['nominal_v'] * 0.90, 2),
        'FENCE_ENABLE': 0.0, 'FENCE_ACTION': 1.0, 'FENCE_ALT_MAX': 120.0, 'FENCE_RADIUS': 300.0,
        'RTL_ALTITUDE': profile['rtl_alt'], 'GPS_TYPE': 1.0, 'COMPASS_USE': 1.0,
        'SERIAL1_BAUD': 57.0, 'SERIAL1_PROTOCOL': 2.0,
        'SR1_POSITION': 5.0, 'SR1_EXTRA1': 10.0, 'SR1_EXTRA2': 5.0, 'SR1_EXT_STAT': 2.0,
        'RC1_MIN': 1000.0, 'RC1_MAX': 2000.0, 'RC1_TRIM': 1500.0,
        'RC3_MIN': 1000.0, 'RC3_MAX': 2000.0, 'RC3_TRIM': 1000.0,
        'FLTMODE1': 0.0, 'FLTMODE6': 11.0,
    }
    # Report what the vehicle is actually flying, not the airframe default: a GCS reads these to
    # seed its own fields (Kite pulls WP_LOITER_RAD for the Fly-Here radius and the loiter ring), so
    # a profile default here would put 300 in the box while the aircraft circles at 250.
    radius = v.radius if v else profile['radius']
    speed = v.cruise_speed if v else profile['speed']
    if profile['can_hover']:
        p.update({'WPNAV_SPEED': speed * 100, 'WPNAV_RADIUS': profile['wp_radius'] * 100,
                  'PILOT_SPEED_UP': profile['climb'] * 100, 'LAND_SPEED': 50.0})
    else:
        # Climb and sink are reported from the same pitch limits the model flies to, at the
        # airspeed it is flying, rather than as independent constants that could disagree with it.
        climb, sink = (v.climb_limits() if v else
                       (speed * math.sin(profile['pitch_up']), speed * math.sin(profile['pitch_dn'])))
        p.update({'AIRSPEED_CRUISE': speed, 'AIRSPEED_MIN': profile['stall'],
                  'AIRSPEED_MAX': profile['speed_max'],
                  'WP_RADIUS': profile['wp_radius'], 'WP_LOITER_RAD': radius,
                  'TRIM_THROTTLE': float(profile['throttle']),
                  # ROLL_LIMIT_DEG in degrees. The old centidegree LIM_ROLL_CD is gone from
                  # ArduPlane master, so serving it would be inventing a parameter.
                  'ROLL_LIMIT_DEG': round(math.degrees(profile['bank_max']), 1),
                  'PTCH_LIM_MAX_DEG': round(math.degrees(profile['pitch_up']), 1),
                  'PTCH_LIM_MIN_DEG': -round(math.degrees(profile['pitch_dn']), 1),
                  'TECS_CLMB_MAX': round(climb, 1), 'TECS_SINK_MAX': round(sink, 1)})
    return p


# ── INAV's MAVLink surface (telemetry/mavlink.c, 9.1) ───────────────────────
# INAV speaks a deliberately small slice of MAVLink, and `--firmware inav --protocol mavlink`
# reproduces exactly that slice so a GCS cannot pass here by relying on ArduPilot extras.
#
# Everything INAV packs. Note what is missing: no HOME_POSITION, no POSITION_TARGET_GLOBAL_INT,
# no NAV_CONTROLLER_OUTPUT, no MISSION_CURRENT, no WIND, no EKF_STATUS_REPORT, no
# AUTOPILOT_VERSION (so a GCS cannot read the firmware version over MAVLink at all), and
# MISSION_ITEM rather than MISSION_ITEM_INT.
INAV_MAVLINK_TX = {
    'HEARTBEAT', 'SYS_STATUS', 'SYSTEM_TIME', 'GPS_RAW_INT', 'GPS_GLOBAL_ORIGIN',
    'GLOBAL_POSITION_INT', 'ATTITUDE', 'VFR_HUD', 'SCALED_PRESSURE', 'RC_CHANNELS',
    'RC_CHANNELS_RAW', 'BATTERY_STATUS', 'STATUSTEXT', 'PARAM_VALUE', 'COMMAND_ACK',
    'MISSION_ACK', 'MISSION_COUNT', 'MISSION_ITEM', 'MISSION_REQUEST',
}

# What INAV's receive switch dispatches on. Anything else falls through `default: return false`,
# which means silently dropped with no ACK of any kind. COMMAND_LONG is in the source as a
# commented-out TODO, so it is on this list of things a real board ignores, not of things it
# handles: every Kite command that travels as COMMAND_LONG (arm, disarm, mode, takeoff, land, RTL,
# change speed, set home) vanishes without a trace.
INAV_MAVLINK_RX = {
    'HEARTBEAT', 'PARAM_REQUEST_LIST', 'MISSION_CLEAR_ALL', 'MISSION_COUNT', 'MISSION_ITEM',
    'MISSION_REQUEST_LIST', 'MISSION_REQUEST', 'COMMAND_INT', 'RC_CHANNELS_OVERRIDE',
    'ADSB_VEHICLE', 'RADIO_STATUS',
}


def run_mavlink(args):
    """Serve MAVLink: an ArduPilot flight controller, or INAV's smaller MAVLink slice, over UDP."""

    inav = args.firmware == 'inav'
    defs = args.defs or find_dialect()
    if not defs:
        sys.exit('ardupilotmega.xml not found. Run `cargo fetch` in src-tauri, or pass --defs PATH')
    d = Dialect(defs)
    # HEARTBEAT's CRC_EXTRA is 50 in every MAVLink dialect; if that fails the parser is wrong and
    # every other seed would be wrong too, so refuse to emit garbage onto the wire.
    if d.messages['HEARTBEAT'][3] != 50:
        sys.exit(f'CRC_EXTRA self-test failed (HEARTBEAT={d.messages["HEARTBEAT"][3]}, expected 50)')
    print(f'[sim] dialect {os.path.basename(defs)}: {len(d.messages)} messages, CRC self-test ok')

    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    sock.bind(('0.0.0.0', args.port))
    sock.setblocking(False)
    link = Link(d, sock)
    if inav:
        link.allow = INAV_MAVLINK_TX
    profile = profile_for(args)
    mission = Mission()
    v = Vehicle(args, profile, mission)
    params = default_params(profile, v)
    param_names = list(params)

    # Enum values resolved from the XML rather than hardcoded.
    MAV_TYPE = d.enum('MAV_TYPE', profile['mav_type'])
    # Always ARDUPILOTMEGA, for both firmwares. INAV's own `mavlink_autopilot_type` setting can
    # also advertise GENERIC (its firmware default), but a GCS then has no mode table and shows the
    # aircraft as an unidentified vehicle, which is not a configuration worth testing against:
    # ARDUPILOT is the value INAV users are told to set for GCS use, and it is the only one under
    # which the mode numbers INAV sends (always ArduPlane's) actually mean anything to the receiver.
    AUTOPILOT = d.enum('MAV_AUTOPILOT', 'MAV_AUTOPILOT_ARDUPILOTMEGA')
    # The custom_mode mapping is NOT conditional on that setting: INAV always packs ArduPlane /
    # ArduCopter mode numbers (inavToArduPlaneMap), even while advertising GENERIC. So a GCS gets
    # ArduPilot mode numbers from an autopilot that says it is not ArduPilot.
    ARMED = d.enum('MAV_MODE_FLAG', 'MAV_MODE_FLAG_SAFETY_ARMED')
    CUSTOM_ON = d.enum('MAV_MODE_FLAG', 'MAV_MODE_FLAG_CUSTOM_MODE_ENABLED')
    STABILIZE_FLAG = d.enum('MAV_MODE_FLAG', 'MAV_MODE_FLAG_STABILIZE_ENABLED')
    GUIDED_FLAG = d.enum('MAV_MODE_FLAG', 'MAV_MODE_FLAG_GUIDED_ENABLED')
    STATE_ACTIVE = d.enum('MAV_STATE', 'MAV_STATE_ACTIVE')
    STATE_STANDBY = d.enum('MAV_STATE', 'MAV_STATE_STANDBY')
    SEV_INFO = d.enum('MAV_SEVERITY', 'MAV_SEVERITY_INFO')
    SEV_NOTICE = d.enum('MAV_SEVERITY', 'MAV_SEVERITY_NOTICE')
    SEV_WARNING = d.enum('MAV_SEVERITY', 'MAV_SEVERITY_WARNING')
    SEVERITY = {'info': SEV_INFO, 'notice': SEV_NOTICE, 'warning': SEV_WARNING}
    ACCEPTED = d.enum('MAV_RESULT', 'MAV_RESULT_ACCEPTED')
    UNSUPPORTED = d.enum('MAV_RESULT', 'MAV_RESULT_UNSUPPORTED')
    DENIED = d.enum('MAV_RESULT', 'MAV_RESULT_DENIED')
    FAILED = d.enum('MAV_RESULT', 'MAV_RESULT_FAILED')
    TEMP_REJECT = d.enum('MAV_RESULT', 'MAV_RESULT_TEMPORARILY_REJECTED')
    PARAM_REAL32 = d.enum('MAV_PARAM_TYPE', 'MAV_PARAM_TYPE_REAL32')
    MISSION_ACCEPTED = d.enum('MAV_MISSION_RESULT', 'MAV_MISSION_ACCEPTED')
    MISSION_ERROR = d.enum('MAV_MISSION_RESULT', 'MAV_MISSION_ERROR')
    MISSION_INVALID_SEQ = d.enum('MAV_MISSION_RESULT', 'MAV_MISSION_INVALID_SEQUENCE')
    FRAME_GLOBAL = d.enum('MAV_FRAME', 'MAV_FRAME_GLOBAL')
    fix = d.enum('GPS_FIX_TYPE', 'GPS_FIX_TYPE_NO_FIX' if args.no_fix else 'GPS_FIX_TYPE_3D_FIX')
    sats = 0 if args.no_fix else 14
    TYPE_MISSION = d.enum('MAV_MISSION_TYPE', 'MAV_MISSION_TYPE_MISSION')

    CMD = {n: d.enum('MAV_CMD', n) for n in (
        'MAV_CMD_REQUEST_MESSAGE', 'MAV_CMD_SET_MESSAGE_INTERVAL', 'MAV_CMD_COMPONENT_ARM_DISARM',
        'MAV_CMD_DO_SET_MODE', 'MAV_CMD_NAV_TAKEOFF', 'MAV_CMD_NAV_LAND',
        'MAV_CMD_NAV_RETURN_TO_LAUNCH', 'MAV_CMD_MISSION_START', 'MAV_CMD_DO_REPOSITION',
        'MAV_CMD_DO_CHANGE_SPEED', 'MAV_CMD_CONDITION_YAW', 'MAV_CMD_DO_SET_HOME',
        'MAV_CMD_DO_PAUSE_CONTINUE', 'MAV_CMD_DO_SET_MISSION_CURRENT', 'MAV_CMD_DO_GO_AROUND',
        'MAV_CMD_DO_VTOL_TRANSITION', 'MAV_CMD_GUIDED_CHANGE_HEADING',
        'MAV_CMD_PREFLIGHT_CALIBRATION', 'MAV_CMD_DO_FLIGHTTERMINATION')}
    v.CMDS = {n: d.enum('MAV_CMD', n) for n in MISSION_CMD_NAMES}
    # Number -> name over the whole MAV_CMD enum, so a dropped or unsupported command can be named
    # in the log even when it is one the sim has no branch for.
    CMD_NAMES = {num: n.replace('MAV_CMD_', '') for n, num in d.enums['MAV_CMD'].items()}

    def say(text, sev='info'):
        link.send('STATUSTEXT', severity=SEVERITY.get(sev, SEV_INFO), text=text[:50])
    v.notify = say

    def log(*a):
        if args.verbose:
            print('[sim]', *a)

    # Note the two odd ones out: AHRS and the prearm check drop the SENSOR_ infix in the dialect
    # (MAV_SYS_STATUS_AHRS, MAV_SYS_STATUS_PREARM_CHECK), so they cannot be named by pattern.
    sensor_names = ['MAV_SYS_STATUS_SENSOR_3D_GYRO', 'MAV_SYS_STATUS_SENSOR_3D_ACCEL',
                    'MAV_SYS_STATUS_SENSOR_3D_MAG', 'MAV_SYS_STATUS_SENSOR_ABSOLUTE_PRESSURE',
                    'MAV_SYS_STATUS_SENSOR_GPS', 'MAV_SYS_STATUS_SENSOR_RC_RECEIVER',
                    'MAV_SYS_STATUS_AHRS', 'MAV_SYS_STATUS_SENSOR_BATTERY',
                    'MAV_SYS_STATUS_PREARM_CHECK']

    def sensor_bits(*names):
        bits = 0
        for n in names:
            bits |= d.enum('MAV_SYS_STATUS_SENSOR', n)
        return bits

    sensors_present = sensor_bits(*sensor_names)
    # With no fix the GPS is present but unhealthy, and so is the prearm check, which is what
    # drives Kite's prearm warning. Present stays full: the hardware exists, it just is not happy.
    unhealthy = ('MAV_SYS_STATUS_SENSOR_GPS', 'MAV_SYS_STATUS_PREARM_CHECK') if args.no_fix else ()
    sensors_healthy = sensor_bits(*[n for n in sensor_names if n not in unhealthy])

    def send_home():
        link.send('HOME_POSITION', latitude=int(v.home_lat * 1e7),
                  longitude=int(v.home_lon * 1e7), altitude=int(v.home_alt * 1000),
                  x=0.0, y=0.0, z=0.0, q=[1.0, 0.0, 0.0, 0.0],
                  approach_x=0.0, approach_y=0.0, approach_z=0.0, time_usec=int(time.time() * 1e6))

    def send_autopilot_version():
        # flight_sw_version packs as major<<24 | minor<<16 | patch<<8 | type, and Kite decodes exactly
        # that (handshake.rs); type 255 means "release", anything lower renders as -rc/-beta/-dev.
        major, minor, patch = profile['fw_ver']
        sw = (major << 24) | (minor << 16) | (patch << 8) | 255
        link.send('AUTOPILOT_VERSION', capabilities=0, flight_sw_version=sw,
                  middleware_sw_version=sw, os_sw_version=sw, board_version=0,
                  vendor_id=0, product_id=0, uid=0)

    # Messages Kite pulls one-shot with MAV_CMD_REQUEST_MESSAGE instead of waiting for the stream.
    # AUTOPILOT_VERSION matters most: the handshake requests it and then BLOCKS for up to 3 s, so
    # without a reply every connection stalls and the FC reports its firmware as "unknown".
    ON_DEMAND = {
        d.messages['AUTOPILOT_VERSION'][0]: send_autopilot_version,
        d.messages['HOME_POSITION'][0]: send_home,
    }

    def send_param(name, index=None):
        if name not in params:
            return False
        link.send('PARAM_VALUE', param_id=name, param_value=float(params[name]),
                  param_type=PARAM_REAL32, param_count=len(params),
                  param_index=param_names.index(name) if index is None else index)
        return True

    def mission_item(seq):
        it = mission.item(seq)
        if it is None:
            return False
        # Items are stored with 1e7 ints. INAV only packs MISSION_ITEM, whose x/y are degrees, so
        # the same item goes out scaled differently depending on which firmware we are being.
        x, y = int(it.get('x', 0)), int(it.get('y', 0))
        link.send('MISSION_ITEM' if inav else 'MISSION_ITEM_INT',
                  target_system=0, target_component=0, seq=seq,
                  frame=it.get('frame', 3), command=it.get('command', 16),
                  current=1 if seq == mission.current else 0,
                  autocontinue=it.get('autocontinue', 1),
                  param1=it.get('param1', 0.0), param2=it.get('param2', 0.0),
                  param3=it.get('param3', 0.0), param4=it.get('param4', 0.0),
                  x=x / 1e7 if inav else x, y=y / 1e7 if inav else y,
                  z=float(it.get('z', 0.0)), mission_type=TYPE_MISSION)
        return True

    def mission_ack(result):
        link.send('MISSION_ACK', target_system=0, target_component=0, type=result,
                  mission_type=TYPE_MISSION)

    def gcs_valid():
        """INAV's isGCSValid() (navigation.c): what guided control really requires.

        All five conditions, none of them optional: armed, a trusted position estimate, a valid GPS
        origin, the GCS NAV box active on a switch (`isGCSAssistedNavigationEnabled`), and the nav
        state actually being POSHOLD 3D. That last one is why "Fly Here" on INAV only works from
        POSHOLD: in any other mode the FC answers DENIED however well-formed the command is.

        'guided' counts as POSHOLD here. On a real board the nav state never leaves POSHOLD 3D for
        a GCS waypoint; only the telemetry mapping renames it to GUIDED while isGCSValid() holds.
        The model has no separate hold-position state, so it uses its guided mode for that and both
        names mean the same nav state.
        """
        return (v.armed and not args.no_fix and args.gcs_nav
                and v.mode in ('loiter', 'guided') and not v.on_ground)

    def handle_command_inav(cmd, f, is_int):
        """INAV's handleIncoming_COMMAND_INT (telemetry/mavlink.c, 9.1).

        One command exists: DO_REPOSITION, and only with frame MAV_FRAME_GLOBAL. The relative-alt
        and terrain-alt frames are commented out in the firmware, so Kite's own reposition (which
        sends MAV_FRAME_GLOBAL_RELATIVE_ALT) is answered UNSUPPORTED here, exactly as a real board
        answers it. Everything else is UNSUPPORTED too.
        """
        if cmd != CMD['MAV_CMD_DO_REPOSITION']:
            return UNSUPPORTED
        frame = int(f.get('frame', 0))
        if frame != FRAME_GLOBAL:
            log(f'DO_REPOSITION frame {frame} is not MAV_FRAME_GLOBAL: UNSUPPORTED')
            return UNSUPPORTED
        if not gcs_valid():
            log('DO_REPOSITION rejected: isGCSValid() false '
                f'(armed={v.armed}, mode={v.mode}, gcs_nav={args.gcs_nav})')
            return DENIED
        tlat, tlon = int(f.get('x', 0)) / 1e7, int(f.get('y', 0)) / 1e7
        if not (-90 <= tlat <= 90 and -180 <= tlon <= 180) or (tlat == 0 and tlon == 0):
            return DENIED
        # setWaypoint(255): XY always, Z only when the altitude is non-zero, heading only when
        # param4 is in 0..360.
        #
        # The altitude is taken as METRES ABOVE HOME, not as the AMSL the frame promises. INAV
        # requires MAV_FRAME_GLOBAL, whose spec meaning is AMSL, and then converts with
        # geoConvertGeodeticToLocal(..., GEO_ALT_RELATIVE), which is `pos->z = llh->alt` verbatim
        # (navigation_geo.c: "llh altitude is already relative to origin"). So the frame check and
        # the conversion disagree inside the firmware, and the firmware behaviour is what this
        # models: sending an AMSL altitude here makes a real board climb to that number above home,
        # which on a 300 m field turns an 80 m target into 380 m AGL.
        alt = float(f.get('z', 0.0))
        v.guided = (tlat, tlon, alt if alt else v.rel_alt)
        yaw = float(f.get('param4', 0.0) or 0.0)
        v.guided_yaw = math.radians(yaw) if 0.0 < yaw < 360.0 else None
        # No mode change: INAV stays in POSHOLD and just moves the hold position, which is why the
        # heartbeat then reports PLANE_MODE_GUIDED (inavToArduPlaneMap with isGCSValid() true).
        v.set_mode(profile['modes'][profile['guided_mode']])
        log(f'GCS waypoint -> {tlat:.6f},{tlon:.6f} @ {v.guided[2]:.0f}m')
        say('Kite SIM: GCS waypoint set')
        return ACCEPTED

    def handle_command(cmd, f, is_int):
        """Shared COMMAND_LONG / COMMAND_INT handling.

        Every branch either changes vehicle state or returns a refusal. A blanket ACCEPTED would
        make an unimplemented action look like it worked while the vehicle ignored it, so a
        control test would pass against a sim that did nothing.
        """
        def p(n, default=0.0):
            return float(f.get(f'param{n}', default) or 0.0)

        # COMMAND_INT carries the position in its own x/y/z fields (x/y as 1e7 ints, z in metres);
        # COMMAND_LONG has no such fields and puts the same three in param5/6/7. Reading param7 for
        # the COMMAND_INT altitude leaves it at 0, which every caller here reads as "keep the
        # current altitude", so a reposition would be ACCEPTED and then flown at the wrong height.
        if is_int:
            tlat, tlon, talt = (int(f.get('x', 0)) / 1e7, int(f.get('y', 0)) / 1e7,
                                float(f.get('z', 0.0) or 0.0))
        else:
            tlat, tlon, talt = p(5), p(6), p(7)

        if cmd == CMD['MAV_CMD_SET_MESSAGE_INTERVAL']:
            msgid, interval = int(p(1)), p(2)
            name = d.unpack_name(msgid)
            if name is None:
                return FAILED
            if interval < 0:
                sched.pop(name, None)
                due.pop(name, None)
            else:
                # 0 means "the default rate"; anything else is honoured as given, in microseconds.
                sched[name] = DEFAULT_RATES.get(name, 1.0) if interval == 0 else interval / 1e6
                due[name] = 0.0
            log(f'stream {name} -> {sched.get(name, "off")}')
            return ACCEPTED
        if cmd == CMD['MAV_CMD_REQUEST_MESSAGE']:
            fn = ON_DEMAND.get(int(p(1)))
            if fn:
                fn()
                return ACCEPTED
            return UNSUPPORTED
        if cmd == CMD['MAV_CMD_COMPONENT_ARM_DISARM']:
            want = bool(int(p(1)))
            force = int(p(2)) == 21196          # ArduPilot's documented force-arm magic number
            if want and args.no_fix and not force:
                say('PreArm: Need 3D Fix', 'warning')
                return DENIED
            if not want and not v.on_ground and not force:
                say('Disarm: vehicle is flying', 'warning')
                return DENIED
            v.armed = want
            if want:
                v.on_ground = v.rel_alt <= 0.5      # arming on the ground still needs a takeoff
            say(f'Kite SIM: {"ARMED" if want else "DISARMED"} by GCS')
            return ACCEPTED
        if cmd == CMD['MAV_CMD_DO_SET_MODE']:
            num = int(p(2))                     # ArduPilot puts the custom mode in param2
            name = profile['modes'].get(num)
            if name is None or not v.set_mode(name):
                # Name the mode when it is one the firmware really has: "AUTOLAND not simulated"
                # says the sim is the limit, while a bare number leaves it ambiguous whether the
                # mode exists at all. Both answers are DENIED either way.
                known = (PLANE_MODES_UNSIMULATED.get(num) if not profile['can_hover'] else None)
                say(f'Kite SIM: mode {known.upper() if known else num} not simulated', 'warning')
                return DENIED
            return ACCEPTED
        if cmd == CMD['MAV_CMD_NAV_TAKEOFF']:
            if not v.armed:
                say('Takeoff: vehicle is disarmed', 'warning')
                return TEMP_REJECT
            v.takeoff_alt = talt if talt > 0 else v.cruise_alt
            v.set_mode('takeoff' if 'takeoff' in profile['modes'].values() else
                       profile['modes'][profile['guided_mode']])
            if v.mode != 'takeoff':
                v.guided = (v.lat, v.lon, v.takeoff_alt)   # copter: climb in GUIDED
            v.on_ground = False
            return ACCEPTED
        if cmd == CMD['MAV_CMD_NAV_LAND']:
            land = profile['land_mode']
            if land is not None:
                v.set_mode(profile['modes'][land])
            else:
                v.mode = 'land'                 # ArduPlane has no LAND mode; fly it down anyway
                say('Kite SIM: landing (simulated approach)')
            return ACCEPTED
        if cmd == CMD['MAV_CMD_NAV_RETURN_TO_LAUNCH']:
            v.set_mode(profile['modes'][profile['rtl_mode']])
            return ACCEPTED
        if cmd == CMD['MAV_CMD_MISSION_START']:
            if mission.nav_items() <= 1:
                say('Mission start: no mission loaded', 'warning')
                return DENIED
            mission.current = max(1, int(p(1)) or 1)
            v.set_mode('auto')
            return ACCEPTED
        if cmd == CMD['MAV_CMD_DO_REPOSITION']:
            if not (-90 <= tlat <= 90 and -180 <= tlon <= 180) or (tlat == 0 and tlon == 0):
                return DENIED
            v.guided = (tlat, tlon, talt if talt > 0 else v.rel_alt)
            v.guided_yaw = None
            v.set_mode(profile['modes'][profile['guided_mode']])
            log(f'reposition -> {tlat:.6f},{tlon:.6f} @ {v.guided[2]:.0f}m')
            return ACCEPTED
        if cmd == CMD['MAV_CMD_DO_CHANGE_SPEED']:
            spd = p(2)
            if spd <= 0:
                return DENIED
            v.cruise_speed = spd
            say(f'Kite SIM: speed {spd:.0f} m/s')
            return ACCEPTED
        if cmd in (CMD['MAV_CMD_CONDITION_YAW'], CMD['MAV_CMD_GUIDED_CHANGE_HEADING']):
            # CONDITION_YAW takes degrees in param1 (param4 != 0 means relative);
            # GUIDED_CHANGE_HEADING puts the heading in param2.
            if cmd == CMD['MAV_CMD_CONDITION_YAW']:
                ang = math.radians(p(1))
                v.guided_yaw = wrap_pi(v.yaw + ang) if int(p(4)) else ang
            else:
                v.guided_yaw = math.radians(p(2))
            return ACCEPTED
        if cmd == CMD['MAV_CMD_DO_SET_HOME']:
            if int(p(1)) == 1:
                v.home_lat, v.home_lon = v.lat, v.lon
            elif -90 <= tlat <= 90 and -180 <= tlon <= 180 and not (tlat == 0 and tlon == 0):
                v.home_lat, v.home_lon = tlat, tlon
            else:
                return DENIED
            send_home()
            say('Kite SIM: home set')
            return ACCEPTED
        if cmd == CMD['MAV_CMD_DO_PAUSE_CONTINUE']:
            v.paused = int(p(1)) == 0
            if v.paused:
                v.loiter = (v.lat, v.lon, v.rel_alt)
            say(f'Kite SIM: {"paused" if v.paused else "resumed"}')
            return ACCEPTED
        if cmd == CMD['MAV_CMD_DO_SET_MISSION_CURRENT']:
            seq = int(p(1))
            if not 0 <= seq < max(mission.nav_items(), 1):
                return DENIED
            mission.current = seq
            link.send('MISSION_CURRENT', seq=seq)
            return ACCEPTED
        if cmd == CMD['MAV_CMD_DO_GO_AROUND']:
            if v.mode != 'land':
                return TEMP_REJECT
            v.takeoff_alt = talt if talt > 0 else v.cruise_alt
            v.set_mode('takeoff' if 'takeoff' in profile['modes'].values() else 'auto')
            say('Kite SIM: go around', 'notice')
            return ACCEPTED
        if cmd == CMD['MAV_CMD_DO_VTOL_TRANSITION']:
            # Q_ENABLE is 0: this airframe is not a quadplane, and a real one answers DENIED.
            say('VTOL transition: Q_ENABLE is 0', 'warning')
            return DENIED
        return UNSUPPORTED

    # msg name -> period seconds. Rates mirror what Kite asks for via SET_MESSAGE_INTERVAL, and
    # that command now really does re-rate them (see handle_command), so the sim honours the
    # stream request instead of merely acknowledging it.
    DEFAULT_RATES = {'HEARTBEAT': 1.0, 'SYS_STATUS': 0.5, 'GPS_RAW_INT': 0.2,
                     'GLOBAL_POSITION_INT': 0.2, 'ATTITUDE': 0.1, 'VFR_HUD': 0.2,
                     'BATTERY_STATUS': 1.0, 'EKF_STATUS_REPORT': 1.0, 'HOME_POSITION': 2.0,
                     'RC_CHANNELS': 0.5, 'MISSION_CURRENT': 1.0, 'WIND': 1.0,
                     'POSITION_TARGET_GLOBAL_INT': 0.5, 'NAV_CONTROLLER_OUTPUT': 0.5}
    if inav:
        # INAV paces its own stream and takes no SET_MESSAGE_INTERVAL, so the schedule is the fixed
        # set it packs. SYSTEM_TIME, SCALED_PRESSURE, GPS_GLOBAL_ORIGIN and RC_CHANNELS_RAW are here
        # because a real board sends them; the ArduPilot-only messages are gone.
        DEFAULT_RATES = {'HEARTBEAT': 1.0, 'SYS_STATUS': 0.5, 'SYSTEM_TIME': 1.0,
                         'GPS_RAW_INT': 0.2, 'GPS_GLOBAL_ORIGIN': 2.0,
                         'GLOBAL_POSITION_INT': 0.2, 'ATTITUDE': 0.1, 'VFR_HUD': 0.2,
                         'SCALED_PRESSURE': 1.0, 'BATTERY_STATUS': 1.0,
                         'RC_CHANNELS': 0.5, 'RC_CHANNELS_RAW': 0.5}
    sched = dict(DEFAULT_RATES)
    needed = list(sched) + ['MISSION_ITEM_REACHED', 'MISSION_ITEM_INT', 'MISSION_COUNT',
                            'MISSION_ACK', 'MISSION_REQUEST_INT', 'PARAM_VALUE', 'STATUSTEXT',
                            'COMMAND_ACK', 'AUTOPILOT_VERSION']
    missing = [m for m in needed if m not in d.messages]
    if missing:
        sys.exit(f'dialect is missing {missing}. Wrong XML?')
    due = {m: 0.0 for m in sched}

    stack = f'INAV {args.version} over MAVLink' if inav else profile['fw']
    print(f'[sim] {stack} ({args.vehicle}): {v.mode.upper()}, r={v.radius:.0f}m @ '
          f'{v.cruise_speed:.0f}m/s, alt {v.cruise_alt:.0f}m around {args.lat:.5f},{args.lon:.5f}')
    print(f'[sim] UDP {args.port}, point Kite at 127.0.0.1:{args.port} '
          f'({"NO GPS FIX" if args.no_fix else "3D fix"}, {"disarmed" if not v.armed else "armed"})')
    if inav:
        print('[sim] INAV MAVLink surface only: COMMAND_INT DO_REPOSITION (frame GLOBAL, and only '
              'while isGCSValid()), missions via MISSION_ITEM, RC override. No arm, no mode change, '
              'no params, and COMMAND_LONG is dropped without an ACK.')
        print(f'[sim] GCS NAV switch is {"ON" if args.gcs_nav else "OFF"} '
              f'(--no-gcs-nav to see what a pilot without the box set gets)')
    else:
        print('[sim] accepting arm/disarm, mode changes, takeoff/land/RTL, guided reposition, '
              'missions, params and RC override')
    greeted = False
    next_chatter = 0.0

    while True:
        for name, f in link.poll():
            if inav and name not in INAV_MAVLINK_RX:
                # INAV's receive switch ends in `default: return false` — no ACK, no error, the
                # frame is simply gone. Logged rather than answered so the gap is visible here
                # instead of looking like a link fault in the app. The command name is part of the
                # line because "COMMAND_LONG dropped" on its own does not say which feature broke.
                what = name
                if name == 'COMMAND_LONG':
                    num = int(f.get('command', 0))
                    what = f'{name} {CMD_NAMES.get(num, num)}'
                elif name == 'PARAM_REQUEST_READ':
                    pid = bytes(f.get('param_id', b'')).split(b'\x00')[0].decode('ascii', 'replace')
                    what = f'{name} {pid or f.get("param_index", "?")}'
                log(f'{what} dropped: INAV has no handler for it')
                continue

            if name == 'COMMAND_LONG' or name == 'COMMAND_INT':
                cmd = int(f.get('command', 0))
                result = (handle_command_inav(cmd, f, name == 'COMMAND_INT') if inav
                          else handle_command(cmd, f, name == 'COMMAND_INT'))
                log(f'{name} {cmd} -> {result}')
                link.send('COMMAND_ACK', command=cmd, result=result,
                          target_system=f.get('target_system', 255), target_component=0)

            elif inav and name == 'PARAM_REQUEST_LIST':
                # handleIncoming_PARAM_REQUEST_LIST: one empty PARAM_VALUE with count 0, whose
                # comment in the firmware is "force Mission Planner to give up quickly". A GCS
                # cannot read or write a single INAV parameter over MAVLink.
                link.send('PARAM_VALUE', param_id='', param_value=0.0, param_type=0,
                          param_count=0, param_index=0)
                log('PARAM_REQUEST_LIST -> empty list (INAV serves no params over MAVLink)')

            elif name == 'PARAM_REQUEST_READ':
                idx = int(f.get('param_index', -1))
                if 0 <= idx < len(param_names):
                    send_param(param_names[idx], idx)
                else:
                    pid = bytes(f.get('param_id', b'')).split(b'\x00')[0].decode('ascii', 'replace')
                    if not send_param(pid):
                        log(f'unknown param read {pid!r}')
            elif name == 'PARAM_REQUEST_LIST':
                for i, pid in enumerate(param_names):
                    send_param(pid, i)
            elif name == 'PARAM_SET':
                pid = bytes(f.get('param_id', b'')).split(b'\x00')[0].decode('ascii', 'replace')
                if pid in params:
                    params[pid] = float(f.get('param_value', 0.0))
                    send_param(pid)                     # echo, which is how a GCS confirms the set
                    log(f'param {pid} = {params[pid]}')
                else:
                    # A real FC ignores unknown params silently; log it so a typo is visible here.
                    log(f'PARAM_SET for unknown {pid!r} ignored')

            elif name == 'MISSION_REQUEST_LIST':
                # Fence and rally points travel over the same protocol with a different
                # mission_type. We only store a mission, so the others are honestly empty.
                mtype = int(f.get('mission_type', 0))
                count = mission.nav_items() if mtype == TYPE_MISSION else 0
                link.send('MISSION_COUNT', target_system=0, target_component=0,
                          count=count, mission_type=mtype)
                log(f'download: {count} items (type {mtype})')
            elif name in ('MISSION_REQUEST_INT', 'MISSION_REQUEST'):
                seq = int(f.get('seq', 0))
                if not mission_item(seq):
                    mission_ack(MISSION_INVALID_SEQ)
                    log(f'download: bad seq {seq}')
            elif name == 'MISSION_COUNT':
                # Upload starting. Ask for item 0 and walk up; MISSION_ACK closes it out.
                mtype = int(f.get('mission_type', 0))
                if mtype != TYPE_MISSION:
                    mission_ack(MISSION_ACCEPTED)       # accept and discard fence/rally
                    continue
                mission.up_count = int(f.get('count', 0))
                mission.up_buf = []
                if mission.up_count == 0:
                    mission.items, mission.current, mission.up_expect = [], 0, None
                    mission_ack(MISSION_ACCEPTED)
                    say('Kite SIM: mission cleared')
                    continue
                mission.up_expect = 0
                # INAV asks with MISSION_REQUEST (the float variant); ArduPilot uses the _INT one.
                link.send('MISSION_REQUEST' if inav else 'MISSION_REQUEST_INT',
                          target_system=0, target_component=0, seq=0, mission_type=TYPE_MISSION)
                log(f'upload: expecting {mission.up_count} items')
            elif name in ('MISSION_ITEM_INT', 'MISSION_ITEM'):
                seq = int(f.get('seq', 0))
                if inav and int(f.get('current', 0)) == 2:
                    # "Legacy Mission Planner BS for GUIDED" in INAV's own words: a NAV_WAYPOINT
                    # item with current == 2 is a guided target rather than a mission item, and it
                    # takes the same frame and isGCSValid() gates as DO_REPOSITION.
                    if int(f.get('command', 0)) != v.CMDS['MAV_CMD_NAV_WAYPOINT']:
                        mission_ack(MISSION_ERROR)
                    elif int(f.get('frame', 0)) != FRAME_GLOBAL:
                        mission_ack(d.enum('MAV_MISSION_RESULT', 'MAV_MISSION_UNSUPPORTED_FRAME'))
                        log('guided MISSION_ITEM rejected: frame is not MAV_FRAME_GLOBAL')
                    elif not gcs_valid():
                        mission_ack(MISSION_ERROR)
                        log('guided MISSION_ITEM rejected: isGCSValid() false')
                    else:
                        # Same setWaypoint(255) path, so the altitude is relative to home here too.
                        alt = float(f.get('z', 0.0))
                        v.guided = (float(f.get('x', 0.0)), float(f.get('y', 0.0)),
                                    alt if alt else v.rel_alt)
                        v.guided_yaw = None
                        v.set_mode(profile['modes'][profile['guided_mode']])
                        mission_ack(MISSION_ACCEPTED)
                        log(f'guided MISSION_ITEM -> {v.guided[0]:.6f},{v.guided[1]:.6f}')
                    continue
                if mission.up_expect is None:
                    log(f'stray mission item seq {seq} outside an upload')
                    continue
                if seq != mission.up_expect:
                    mission_ack(MISSION_INVALID_SEQ)
                    log(f'upload: expected {mission.up_expect}, got {seq}')
                    continue
                item = dict(f)
                if name == 'MISSION_ITEM':
                    # The float variant carries degrees, so scale to the 1e7 ints we store.
                    item['x'] = int(float(f.get('x', 0.0)) * 1e7)
                    item['y'] = int(float(f.get('y', 0.0)) * 1e7)
                mission.up_buf.append(item)
                mission.up_expect += 1
                if mission.up_expect < mission.up_count:
                    link.send('MISSION_REQUEST' if inav else 'MISSION_REQUEST_INT',
                              target_system=0, target_component=0,
                              seq=mission.up_expect, mission_type=TYPE_MISSION)
                else:
                    mission.items = mission.up_buf
                    mission.up_buf, mission.up_expect = [], None
                    mission.current = 1 if len(mission.items) > 1 else 0
                    mission_ack(MISSION_ACCEPTED)
                    say(f'Kite SIM: mission received, {len(mission.items)} items', 'notice')
                    log(f'upload complete: {len(mission.items)} items')
            elif name == 'MISSION_CLEAR_ALL':
                mission.items, mission.current = [], 0
                mission.up_buf, mission.up_expect = [], None
                mission_ack(MISSION_ACCEPTED)
                say('Kite SIM: mission cleared')
            elif name == 'MISSION_SET_CURRENT':
                seq = int(f.get('seq', 0))
                if 0 <= seq < mission.nav_items():
                    mission.current = seq
                    link.send('MISSION_CURRENT', seq=seq)
                else:
                    mission_ack(MISSION_INVALID_SEQ)
            elif name == 'MISSION_ACK':
                log(f'GCS acked mission: type {f.get("type", 0)}')

            elif name == 'RC_CHANNELS_OVERRIDE':
                v.rc_override(f)
                if inav:
                    # On INAV these channels are the radio, so the arm and mode switches in them
                    # take effect. It is the only route to POSHOLD over MAVLink, and POSHOLD is
                    # what isGCSValid() demands before a guided target is accepted.
                    inav_apply_switches(v, args.no_fix, log=lambda m: log(m.replace('[sim] ', '')))
            elif name == 'MANUAL_CONTROL':
                v.manual_control(f)

        t = v.update()
        now = time.time()
        boot_ms = int(t * 1000)
        usec = int(now * 1e6)
        # MAV_BATTERY percentages are int8_t (-1 = unknown), so the drain has to be clamped: an
        # unclamped linear ramp underflows the field about 27 min in and kills the encoder.
        remaining = max(0, min(100, int(100 - t / 7.2)))

        if v.reached is not None:
            link.send('MISSION_ITEM_REACHED', seq=v.reached)
            link.send('MISSION_CURRENT', seq=mission.current)
            log(f'reached seq {v.reached}')

        if link.peer and not greeted:
            greeted = True
            say(f'Kite SIM: {profile["fw"]} (simulated)')
            say('Kite SIM: EKF3 IMU0 is using GPS')
            if args.no_fix:
                say('PreArm: Need 3D Fix', 'warning')

        for name in list(sched):
            period = sched.get(name)
            if period is None or now < due.get(name, 0.0):
                continue
            due[name] = now + period
            if name == 'HEARTBEAT':
                # base_mode is rebuilt every beat, not cached: a GCS arm/disarm flips SAFETY_ARMED.
                base_mode = (CUSTOM_ON | STABILIZE_FLAG) | (ARMED if v.armed else 0)
                mode_num = v.mode_num()
                if inav and v.mode == 'loiter' and gcs_valid():
                    # inavToArduPlaneMap: POSHOLD reports as GUIDED, not LOITER, while isGCSValid()
                    # holds. That is how a GCS learns it may send a guided target at all, so
                    # reporting plain LOITER here made the sim look like it refused guided control
                    # when a real board would have advertised it.
                    mode_num = next(n for n, nm in profile['modes'].items()
                                    if nm == profile['modes'][profile['guided_mode']])
                # INAV sets GUIDED_ENABLED for POSHOLD, RTH and MISSION (mavlink.c:798).
                if inav and v.mode in ('loiter', 'guided', 'rtl', 'auto'):
                    base_mode |= GUIDED_FLAG
                link.send('HEARTBEAT', type=MAV_TYPE, autopilot=AUTOPILOT, base_mode=base_mode,
                          custom_mode=mode_num,
                          system_status=STATE_ACTIVE if v.armed else STATE_STANDBY, mavlink_version=3)
            elif name == 'SYS_STATUS':
                link.send('SYS_STATUS', onboard_control_sensors_present=sensors_present,
                          onboard_control_sensors_enabled=sensors_present,
                          onboard_control_sensors_health=sensors_healthy,
                          load=350, voltage_battery=int(v.voltage * 1000),
                          current_battery=1850, battery_remaining=remaining,
                          drop_rate_comm=0, errors_comm=0)
            elif name == 'GPS_RAW_INT':
                link.send('GPS_RAW_INT', time_usec=usec, fix_type=fix,
                          lat=int(v.lat * 1e7), lon=int(v.lon * 1e7), alt=int(v.alt * 1000),
                          eph=110 if not args.no_fix else 9999,
                          epv=180 if not args.no_fix else 9999,
                          vel=int(v.groundspeed * 100), cog=int(math.degrees(v.yaw) * 100),
                          satellites_visible=sats)
            elif name == 'GLOBAL_POSITION_INT':
                link.send('GLOBAL_POSITION_INT', time_boot_ms=boot_ms,
                          lat=int(v.lat * 1e7), lon=int(v.lon * 1e7),
                          alt=int(v.alt * 1000), relative_alt=int(v.rel_alt * 1000),
                          vx=int(v.vn * 100), vy=int(v.ve * 100), vz=int(v.vz * 100),
                          hdg=int(math.degrees(v.yaw) * 100))
            elif name == 'ATTITUDE':
                link.send('ATTITUDE', time_boot_ms=boot_ms, roll=v.roll, pitch=v.pitch,
                          yaw=v.yaw - 2 * math.pi if v.yaw > math.pi else v.yaw,
                          rollspeed=0.0, pitchspeed=0.0, yawspeed=v.omega)
            elif name == 'VFR_HUD':
                thr = profile['throttle'] if v.speed > 0.5 else 0
                link.send('VFR_HUD', airspeed=v.airspeed, groundspeed=v.groundspeed,
                          heading=int(math.degrees(v.yaw)) % 360,
                          throttle=thr, alt=v.alt, climb=-v.vz)
            elif name == 'BATTERY_STATUS':
                n = profile['cells']
                cells = [int(v.voltage * 1000 / n)] * n + [0xFFFF] * (10 - n)
                link.send('BATTERY_STATUS', id=0, battery_function=0, type=0,
                          temperature=2300, voltages=cells, current_battery=1850,
                          current_consumed=int(t * 0.5), energy_consumed=-1,
                          battery_remaining=remaining)
            elif name == 'EKF_STATUS_REPORT':
                # Healthy-estimator bitmask; velocity/pos flags drop out with no GPS.
                flags = 0x01 | 0x02 | 0x100 | (0 if args.no_fix else 0x04 | 0x08 | 0x10)
                link.send('EKF_STATUS_REPORT', flags=flags, velocity_variance=0.06,
                          pos_horiz_variance=0.11, pos_vert_variance=0.04,
                          compass_variance=0.08, terrain_alt_variance=0.0)
            elif name == 'HOME_POSITION':
                send_home()   # same payload Kite can also pull on demand, so keep it in one place
            elif name == 'RC_CHANNELS':
                # Echo the override the GCS is sending, like a real FC: the RC page then shows what
                # actually reached the vehicle rather than a static frame that never moves.
                chans = {f'chan{i}_raw': int(v.rc.get(i, 1500 if i <= 4 else 1000))
                         for i in range(1, 9)}
                chans.update({f'chan{i}_raw': 0xFFFF for i in range(9, 19)})
                link.send('RC_CHANNELS', time_boot_ms=boot_ms, chancount=8, rssi=180, **chans)
            elif name == 'SYSTEM_TIME':
                link.send('SYSTEM_TIME', time_unix_usec=usec, time_boot_ms=boot_ms)
            elif name == 'SCALED_PRESSURE':
                # ISA pressure at the current altitude, so the baro reading tracks the climb.
                press = 1013.25 * (1.0 - 2.25577e-5 * v.alt) ** 5.25588
                link.send('SCALED_PRESSURE', time_boot_ms=boot_ms, press_abs=press,
                          press_diff=0.0, temperature=2300)
            elif name == 'GPS_GLOBAL_ORIGIN':
                link.send('GPS_GLOBAL_ORIGIN', latitude=int(v.home_lat * 1e7),
                          longitude=int(v.home_lon * 1e7), altitude=int(v.home_alt * 1000))
            elif name == 'RC_CHANNELS_RAW':
                chans = {f'chan{i}_raw': int(v.rc.get(i, 1500 if i <= 4 else 1000))
                         for i in range(1, 9)}
                link.send('RC_CHANNELS_RAW', time_boot_ms=boot_ms, port=0, rssi=180, **chans)
            elif name == 'MISSION_CURRENT':
                link.send('MISSION_CURRENT', seq=mission.current)
            elif name == 'WIND':
                # WIND.direction is where the wind blows FROM, the same convention the model
                # drifts by, so the widget and the ground track cannot disagree.
                link.send('WIND', direction=math.degrees(profile['wind_from']),
                          speed=profile['wind'], speed_z=0.2)
            elif name == 'POSITION_TARGET_GLOBAL_INT':
                tgt = v.guided if v.mode == 'guided' and v.guided else None
                if tgt is None and v.mode == 'auto':
                    it = mission.item(mission.current)
                    if it:
                        tgt = (it.get('x', 0) / 1e7, it.get('y', 0) / 1e7, float(it.get('z', 0.0)))
                if tgt:
                    link.send('POSITION_TARGET_GLOBAL_INT', time_boot_ms=boot_ms, coordinate_frame=6,
                              type_mask=0, lat_int=int(tgt[0] * 1e7), lon_int=int(tgt[1] * 1e7),
                              alt=float(tgt[2]), vx=0.0, vy=0.0, vz=0.0, afx=0.0, afy=0.0, afz=0.0,
                              yaw=0.0, yaw_rate=0.0)
            elif name == 'NAV_CONTROLLER_OUTPUT':
                tgt = None
                if v.mode == 'guided' and v.guided:
                    tgt = v.guided
                elif v.mode == 'auto':
                    it = mission.item(mission.current)
                    if it:
                        tgt = (it.get('x', 0) / 1e7, it.get('y', 0) / 1e7, float(it.get('z', 0.0)))
                elif v.mode == 'rtl':
                    tgt = (v.home_lat, v.home_lon, v.cruise_alt)
                dist = dist_m(v.lat, v.lon, tgt[0], tgt[1]) if tgt else 0.0
                brg = math.degrees(bearing_to(v.lat, v.lon, tgt[0], tgt[1])) % 360 if tgt else 0.0
                link.send('NAV_CONTROLLER_OUTPUT', nav_roll=math.degrees(v.roll),
                          nav_pitch=math.degrees(v.pitch), nav_bearing=int(brg),
                          target_bearing=int(brg), wp_dist=int(min(dist, 65535)),
                          alt_error=0.0, aspd_error=0.0, xtrack_error=0.0)

        if args.chatter and now >= next_chatter:
            next_chatter = now + 3.0
            say('PreArm: Waiting for GPS fix', 'warning')
            say('Unable to arm: check RC', 'warning')

        time.sleep(0.01)

# ── INAV over MAVLink: the altitude trap ─────────────────────────────────────
# Read this before validating any guided-altitude behaviour against this file.
#
# INAV's DO_REPOSITION handler accepts MAV_FRAME_GLOBAL and nothing else. In MAVLink that frame
# means AMSL. The firmware then passes the altitude to setWaypoint(255, ...), which converts with
# geoConvertGeodeticToLocal(..., GEO_ALT_RELATIVE), and that branch is `pos->z = llh->alt` verbatim:
# "llh altitude is already relative to origin" (navigation_geo.c). The frame check and the
# conversion therefore disagree inside the firmware, and the altitude is used as METRES ABOVE HOME.
#
# This model follows the firmware, not the spec. A GCS that converts its relative altitude to AMSL
# before sending will be accepted here and will fly too high by the field elevation, which is what a
# real board does: an 80 m target on a 300 m field ends up at 380 m above the ground.
#
# Reported for the record rather than smoothed over, because a simulator that implemented the spec
# here would confirm a GCS's own assumption instead of catching it.


# ══ INAV / MSP ═══════════════════════════════════════════════════════════════
# Everything below serves the same Vehicle over MSP instead of MAVLink.

# MAV_CMD numbers, which the shared mission executor above speaks. Hardcoded rather than read from
# the dialect so that --firmware inav never needs ardupilotmega.xml: these are frozen protocol
# constants, and an INAV run must not fail because the MAVLink XML is missing.
MAV_CMDS = {
    'MAV_CMD_NAV_WAYPOINT': 16, 'MAV_CMD_NAV_LOITER_UNLIM': 17,
    'MAV_CMD_NAV_LOITER_TURNS': 18, 'MAV_CMD_NAV_LOITER_TIME': 19,
    'MAV_CMD_NAV_RETURN_TO_LAUNCH': 20, 'MAV_CMD_NAV_LAND': 21, 'MAV_CMD_NAV_TAKEOFF': 22,
    'MAV_CMD_DO_CHANGE_SPEED': 178, 'MAV_CMD_DO_SET_HOME': 179, 'MAV_CMD_DO_JUMP': 177,
    'MAV_CMD_CONDITION_DELAY': 112, 'MAV_CMD_CONDITION_YAW': 115,
}

# ── MSP command codes (src-tauri/src/msp/types.rs) ───────────────────────────

MSP_API_VERSION = 1
MSP_FC_VARIANT = 2
MSP_FC_VERSION = 3
MSP_BOARD_INFO = 4
MSP_BUILD_INFO = 5
MSP_NAME = 10
MSP_WP_GETINFO = 20
MSP_STATUS = 101
MSP_RAW_IMU = 102
MSP_RC = 105
MSP_RAW_GPS = 106
MSP_COMP_GPS = 107
MSP_ATTITUDE = 108
MSP_ALTITUDE = 109
MSP_ANALOG = 110
MSP_ACTIVEBOXES = 113
MSP_WP = 118
MSP_BOXIDS = 119
MSP_NAV_STATUS = 121
MSP_STATUS_EX = 150
MSP_SENSOR_STATUS = 151
MSP_UID = 160
MSP_GPSSTATISTICS = 166
MSP_SET_RAW_RC = 200
MSP_SET_WP = 209
MSP_EEPROM_WRITE = 250

MSP2_COMMON_SETTING = 0x1003
MSP2_COMMON_SET_SETTING = 0x1004
MSP2_INAV_STATUS = 0x2000
MSP2_INAV_ANALOG = 0x2002
MSP2_INAV_AIR_SPEED = 0x2009
MSP2_INAV_MIXER = 0x2010
MSP2_INAV_MISC2 = 0x203A
MSP2_INAV_GET_LINK_STATS = 0x2103
MSP2_INAV_SET_AUX_RC = 0x2230
MSP2_INAV_WIND = 0x2231

CODE_NAMES = {v: k for k, v in list(globals().items()) if k.startswith(('MSP_', 'MSP2_'))}

# INAV armingFlags: ARMED is bit 2 (armingFlag_e starts there; bits 0/1 are unused by real MSP).
ARMING_FLAG_ARMED = 1 << 2

# Permanent box IDs in the order the sim reports them from MSP_BOXIDS. The active-modes bitmask in
# MSP2_INAV_STATUS is indexed into THIS list, not by permanent ID, which is exactly what Kite's
# parse_active_modes expects (telemetry.rs documents the distinction).
BOXES = [
    0,   # BOXARM        (a real INAV lists the arm box first, so it takes bit 0)
    1,   # BOXANGLE
    2,   # BOXHORIZON
    12,  # BOXMANUAL
    3,   # BOXNAVALTHOLD
    11,  # BOXNAVPOSHOLD
    10,  # BOXNAVRTH
    28,  # BOXNAVWP
    45,  # BOXNAVCOURSEHOLD
    53,  # BOXNAVCRUISE
    36,  # BOXNAVLAUNCH
    50,  # BOXMSPRCOVERRIDE
    27,  # BOXFAILSAFE
]

# Which boxes are lit for each flight-model mode. ANGLE rides along with the nav modes because the
# firmware forces it (processRcModes), and Kite mirrors that, so leaving it out would show a mode
# combination a real INAV never reports.
# The arm box is lit whenever the vehicle is armed, on top of the mode boxes below.
BOX_ARM = 0
MODE_BOXES = {
    'manual':    [12],
    'stabilize': [1],
    'fbwa':      [1],
    'fbwb':      [1, 3],
    'cruise':    [1, 53],
    'althold':   [1, 3],
    'loiter':    [1, 11],
    'circle':    [1, 11],
    'guided':    [1, 11],
    'auto':      [1, 28],
    'rtl':       [1, 10],
    'takeoff':   [1, 36],
    'land':      [1, 10],
}

# INAV waypoint actions (navigation/navigation.h `navWaypointActions_e`) mapped onto the MAVLink
# commands the shared mission executor understands.
WP_ACTION_WAYPOINT = 1
WP_ACTION_HOLD_TIME = 3
WP_ACTION_RTH = 4
WP_ACTION_SET_POI = 5
WP_ACTION_JUMP = 6
WP_ACTION_SET_HEAD = 7
WP_ACTION_LAND = 8
WP_ACTION_TO_MAV = {
    WP_ACTION_WAYPOINT: MAV_CMDS['MAV_CMD_NAV_WAYPOINT'],
    WP_ACTION_HOLD_TIME: MAV_CMDS['MAV_CMD_NAV_LOITER_TIME'],
    WP_ACTION_RTH: MAV_CMDS['MAV_CMD_NAV_RETURN_TO_LAUNCH'],
    WP_ACTION_LAND: MAV_CMDS['MAV_CMD_NAV_LAND'],
    WP_ACTION_JUMP: MAV_CMDS['MAV_CMD_DO_JUMP'],
    WP_ACTION_SET_HEAD: MAV_CMDS['MAV_CMD_CONDITION_YAW'],
}

MAX_WAYPOINTS = 60

# INAV settings the sim serves by name over MSP2_COMMON_SETTING. Kite reads nav_fw_loiter_radius
# for the Fly-Here radius and the loiter ring; the rest exist so a settings round-trip is testable.
# Values are (struct format, value) and the radius is in centimetres, as INAV stores it.
SETTINGS_FMT = {
    'nav_fw_loiter_radius': '<H',
    'nav_fw_cruise_speed': '<H',
    'nav_wp_radius': '<H',
    'nav_fw_climb_angle': '<b',
    'nav_fw_dive_angle': '<b',
    'nav_rth_altitude': '<I',
    'nav_rth_climb_first': '<B',
    'nav_max_auto_speed': '<H',
    'battery_capacity': '<I',
    'vbat_min_cell_voltage': '<H',
    'vbat_warning_cell_voltage': '<H',
    'platform_type': '<B',
}


def crc8_dvb_s2(data, crc=0):
    """MSP v2 checksum, poly 0xD5 (msp/parser.rs crc8_dvb_s2_byte)."""
    for b in data:
        crc ^= b
        for _ in range(8):
            crc = ((crc << 1) ^ 0xD5) & 0xFF if crc & 0x80 else (crc << 1) & 0xFF
    return crc


class MspLink:
    """MSP v1 + v2 framing over a stream (TCP) or datagram (UDP) socket.

    Kite is the client in both cases: it connects out to host:port, so the sim listens. Requests
    arrive as `$M<` / `$X<` frames and every reply carries the same code back, which is how the
    request/response pairing works (MSP has no sequence numbers).
    """

    def __init__(self, sock, udp=False, verbose=False):
        self.sock, self.udp, self.verbose = sock, udp, verbose
        self.conn = None          # TCP: the accepted client
        self.peer = None          # UDP: whoever last spoke to us
        self.rx = bytearray()

    # ── framing ─────────────────────────────────────────────────────────────

    @staticmethod
    def frame_v1(code, payload, err=False):
        # $M> <len> <code> <payload> <xor of len^code^payload>
        head = bytes([len(payload), code & 0xFF])
        body = head + bytes(payload)
        crc = 0
        for b in body:
            crc ^= b
        return b'$M' + (b'!' if err else b'>') + body + bytes([crc])

    @staticmethod
    def frame_v2(code, payload, err=False):
        # $X> <flag> <code lo> <code hi> <len lo> <len hi> <payload> <crc8 dvb-s2 over flag..payload>
        body = struct.pack('<BHH', 0, code, len(payload)) + bytes(payload)
        return b'$X' + (b'!' if err else b'>') + body + bytes([crc8_dvb_s2(body)])

    def send(self, code, payload=b'', err=False):
        # v1 cannot express a code above 255 or a payload over 254 bytes, so those go out as v2.
        # Kite's parser handles both, and a real INAV picks the same way.
        if code > 0xFF or len(payload) > 254:
            frame = self.frame_v2(code, payload, err)
        else:
            frame = self.frame_v1(code, payload, err)
        try:
            if self.udp:
                if self.peer:
                    self.sock.sendto(frame, self.peer)
            elif self.conn:
                self.conn.sendall(frame)
        except OSError as e:
            if self.verbose:
                print(f'[sim] send failed: {e}')

    # ── receive ─────────────────────────────────────────────────────────────

    def poll(self):
        """Yield (code, payload) for every complete request frame available."""
        if self.udp:
            while True:
                try:
                    data, addr = self.sock.recvfrom(4096)
                except (BlockingIOError, OSError):
                    break
                if self.peer != addr:
                    self.peer = addr
                    print(f'[sim] GCS at {addr[0]}:{addr[1]}')
                self.rx += data
        else:
            if self.conn is None:
                try:
                    self.conn, addr = self.sock.accept()
                    self.conn.setblocking(False)
                    print(f'[sim] GCS connected from {addr[0]}:{addr[1]}')
                except (BlockingIOError, OSError):
                    return
            try:
                data = self.conn.recv(4096)
                if data == b'':                      # orderly close
                    print('[sim] GCS disconnected')
                    self.conn.close()
                    self.conn = None
                    self.rx.clear()
                    return
                self.rx += data
            except (BlockingIOError, OSError):
                pass
        yield from self._frames()

    def _frames(self):
        while self.rx:
            start = self.rx.find(b'$')
            if start < 0:
                self.rx.clear()
                return
            if start:
                del self.rx[:start]
            if len(self.rx) < 3:
                return
            kind = self.rx[1:2]
            if kind == b'M':
                if len(self.rx) < 6:
                    return
                plen, code = self.rx[3], self.rx[4]
                total = 6 + plen
                if len(self.rx) < total:
                    return
                payload = bytes(self.rx[5:5 + plen])
                del self.rx[:total]
                yield code, payload
            elif kind == b'X':
                if len(self.rx) < 9:
                    return
                code, plen = struct.unpack_from('<HH', self.rx, 4)
                total = 9 + plen
                if len(self.rx) < total:
                    return
                payload = bytes(self.rx[8:8 + plen])
                del self.rx[:total]
                yield code, payload
            else:
                del self.rx[:1]                       # not a frame start after all


def inav_apply_switches(v, no_fix, log=print):
    """Arm and mode selection from RC channels, which is the only way INAV offers either.

    Shared by both protocols: over MSP the sticks arrive as MSP_SET_RAW_RC, over MAVLink as
    RC_CHANNELS_OVERRIDE, and a real board treats them the same because they both feed the RX
    layer. CH5 is the arm switch, CH6 a normal 3-position mode switch.
    """
    rc = v.rc
    # Both switches are read every time, and the arm result is returned rather than returned early:
    # one RC frame can move both channels at once, and skipping the mode switch in that frame left
    # the vehicle armed in the wrong mode.
    just_armed = None
    arm = rc.get(INAV_ARM_CH)
    if arm is not None:
        want = arm > 1700
        if want != v.armed:
            if want and no_fix:
                log('[sim] arm refused: no GPS fix')
            else:
                v.armed = want
                if want:
                    v.on_ground = v.rel_alt <= 0.5
                log(f'[sim] {"ARMED" if want else "DISARMED"} by RC switch (CH{INAV_ARM_CH})')
                just_armed = want
    mode_us = rc.get(INAV_MODE_CH)
    if mode_us is not None:
        want = 'manual' if mode_us < 1300 else 'loiter' if mode_us <= 1700 else 'auto'
        # A GCS waypoint leaves the switch in the POSHOLD position while the nav state holds the
        # commanded position, so 'guided' must not be dragged back to a plain hold every frame.
        if want != v.mode and not (want == 'loiter' and v.mode == 'guided'):
            v.set_mode(want)
    return just_armed


INAV_ARM_CH, INAV_MODE_CH = 5, 6


class InavVehicle:
    """Wraps the shared flight model with the parts that are INAV's rather than ArduPilot's:
    RC-switch driven arming and mode selection, and an INAV waypoint list."""

    # RC switch geometry, matching a normal 3-position mode switch setup.
    ARM_CH, MODE_CH = INAV_ARM_CH, INAV_MODE_CH

    def __init__(self, args):
        profile = profile_for(args)
        self.mission = Mission()
        self.v = Vehicle(args, profile, self.mission)
        self.v.CMDS = MAV_CMDS
        self.profile = profile
        self.wps = []              # INAV waypoints as dicts, index 0 = WP1
        self.t_start = time.time()
        self.flight_start = None
        self.statustexts = []
        self.v.notify = self._notify

    def _notify(self, text, sev='info'):
        # MSP has no STATUSTEXT equivalent, so these go to the console instead of the GCS.
        print(f'[sim] {text}')

    # ── RC-driven arming and mode, the way INAV works ───────────────────────

    def apply_rc(self):
        armed = inav_apply_switches(self.v, self.no_fix)
        if armed:
            self.flight_start = time.time()

    # ── INAV waypoint list <-> the shared mission executor ──────────────────

    def adopt_wps(self):
        """Translate the stored INAV list into the mission the flight model executes.

        Slot 0 of that structure is ArduPilot's home placeholder and is never navigated to, so a
        filler item goes in front and INAV's WP1 becomes seq 1.
        """
        items = [{'command': MAV_CMDS['MAV_CMD_NAV_WAYPOINT'], 'x': int(self.v.home_lat * 1e7),
                  'y': int(self.v.home_lon * 1e7), 'z': 0.0}]
        for wp in self.wps:
            cmd = WP_ACTION_TO_MAV.get(wp['action'])
            if cmd is None:
                continue
            items.append({'command': cmd, 'x': wp['lat'], 'y': wp['lon'],
                          'z': wp['alt'] / 100.0,          # INAV stores altitude in cm
                          'param1': float(wp['p1']), 'param2': float(wp['p2']),
                          'param3': float(wp['p3'])})
        self.mission.items = items
        self.mission.current = 1 if len(items) > 1 else 0

    def wp_payload(self, number):
        """MSP_WP response: 21 bytes (mission/codec.rs decode_wp)."""
        if number == 0:
            # WP0 is the home/reference point in INAV, not part of the mission list.
            return struct.pack('<BBiiihhhB', 0, WP_ACTION_WAYPOINT,
                               int(self.v.home_lat * 1e7), int(self.v.home_lon * 1e7),
                               0, 0, 0, 0, 0xA5)
        idx = number - 1
        if idx >= len(self.wps):
            return None
        wp = self.wps[idx]
        last = 0xA5 if idx == len(self.wps) - 1 else 0
        return struct.pack('<BBiiihhhB', number, wp['action'], wp['lat'], wp['lon'],
                           wp['alt'], wp['p1'], wp['p2'], wp['p3'], last)

    def set_wp(self, payload):
        if len(payload) < 21:
            return False
        number, action, lat, lon, alt, p1, p2, p3, flag = struct.unpack('<BBiiihhhB', payload[:21])
        wp = dict(action=action, lat=lat, lon=lon, alt=alt, p1=p1, p2=p2, p3=p3, flag=flag)
        if number == 0:
            self.v.home_lat, self.v.home_lon = lat / 1e7, lon / 1e7
            return True
        idx = number - 1
        while len(self.wps) <= idx:
            self.wps.append(dict(action=WP_ACTION_WAYPOINT, lat=0, lon=0, alt=0,
                                 p1=0, p2=0, p3=0, flag=0))
        self.wps[idx] = wp
        # INAV marks the final waypoint with flag 0xA5; that is the upload's end, so adopt it then.
        if flag == 0xA5:
            del self.wps[idx + 1:]
            self.adopt_wps()
            print(f'[sim] mission received: {len(self.wps)} waypoints')
        return True

    # ── payload builders ────────────────────────────────────────────────────

    def active_modes_bytes(self):
        boxes = MODE_BOXES.get(self.v.mode, [1])
        if self.v.armed:
            boxes = boxes + [BOX_ARM]
        if self.v.rc:
            boxes = boxes + [50]           # MSP RC override is live while the GCS sends channels
        bits = 0
        for perm in boxes:
            if perm in BOXES:
                bits |= 1 << BOXES.index(perm)
        return bits.to_bytes((len(BOXES) + 7) // 8, 'little')

    def status_payload(self):
        """MSP2_INAV_STATUS: 13-byte header, then the active-modes bitmask, then mixerProfile."""
        arming = ARMING_FLAG_ARMED if self.v.armed else 0
        # packSensorStatus(): ACC<<0, BARO<<1, MAG<<2, GPS<<3, RANGEFINDER<<4, OPFLOW<<5,
        # PITOT<<6, TEMP<<7. A fixed wing with an airspeed estimate carries the pitot bit.
        sensors = 0x01 | 0x02 | 0x04 | (0x40 if not self.profile['can_hover'] else 0)
        if not self.no_fix:
            sensors |= 0x08
        head = struct.pack('<HHHHBI', 2500, 0, sensors, 12, 0, arming)
        return head + self.active_modes_bytes() + b'\x00'

    def analog_payload(self):
        v = self.v
        cells = self.profile['cells']
        pct_now = max(0, min(100, int(100 - (time.time() - self.t_start) / 7.2)))
        # batteryState: 0 OK, 1 WARNING, 2 CRITICAL (batteryState_e), which is what drives Kite's
        # battery colour, so it has to follow the same drain the voltage does.
        state = 0 if pct_now > 30 else 1 if pct_now > 15 else 2
        flags = 0x01 | ((state & 0x03) << 2) | ((cells & 0x0F) << 4)
        pct = max(0, min(100, int(100 - (time.time() - self.t_start) / 7.2)))
        mah = int((time.time() - self.t_start) * 0.5)
        return struct.pack('<BHhIIIIBH', flags, int(v.voltage * 100), 1850,
                           int(v.voltage * 18.5 * 100), mah, mah * 15, 5000 - mah, pct, 1023)

    def gps_payload(self):
        # fixType u8, numSat u8, lat i32, lon i32, alt i16 (m), groundSpeed u16 (cm/s),
        # groundCourse u16 (decidegrees), hdop u16. The trailing hdop is easy to miss: INAV has
        # sent it since well before 7.x and a 16-byte reply is simply short.
        v = self.v
        return struct.pack('<BBiihHHH', 0 if self.no_fix else 3, 0 if self.no_fix else 14,
                           int(v.lat * 1e7), int(v.lon * 1e7), int(v.alt),
                           int(v.groundspeed * 100), int(math.degrees(v.yaw) % 360 * 10),
                           9999 if self.no_fix else 110)

    def nav_status_payload(self):
        v = self.v
        # nav_state: 0 none, 1 RTH start, 2 RTH enroute, 3 hold infinite, 4 hold timed, 5 WP enroute
        # navSystemStatus_State_e: 0 NONE, 1 RTH_START, 2 RTH_ENROUTE, 3 HOLD_INFINIT,
        # 4 HOLD_TIMED, 5 WP_ENROUTE, 6 PROCESS_NEXT, 7 DO_JUMP, 8 LAND_START,
        # 9 LAND_IN_PROGRESS, 10 LANDED.
        state = {'rtl': 2, 'loiter': 3, 'circle': 3, 'guided': 3, 'auto': 5,
                 'land': 9, 'takeoff': 1}.get(v.mode, 0)
        if v.on_ground and not v.armed and v.mode == 'land':
            state = 10
        wp_no = self.mission.current if v.mode == 'auto' else 0
        return struct.pack('<BBBBBh', 0 if self.no_fix else 4, state, WP_ACTION_WAYPOINT,
                           max(0, wp_no), 0, int(math.degrees(v.yaw)) % 360)

    def rc_payload(self):
        chans = [int(self.v.rc.get(i, 1500 if i <= 4 else 1000)) for i in range(1, 9)]
        return struct.pack('<8H', *chans)


def build_settings(iv, args):
    p = iv.profile
    return {
        'nav_fw_loiter_radius': int(iv.v.radius * 100),   # cm, and the live radius not the default
        'nav_fw_cruise_speed': int(iv.v.cruise_speed * 100),
        'nav_wp_radius': int(p['wp_radius'] * 100),
        # Reported from the same limits the model pitches to, so a written angle and the flown
        # climb cannot disagree.
        'nav_fw_climb_angle': round(math.degrees(p['pitch_up'])),
        'nav_fw_dive_angle': round(math.degrees(p['pitch_dn'])),
        # The altitude RTH really climbs to in the model, not the cruise altitude: a GCS that reads
        # this setting and then triggers RTH has to see the two agree.
        'nav_rth_altitude': int(p['rtl_alt'] * 100),
        'nav_rth_climb_first': 1,
        'nav_max_auto_speed': int(iv.v.cruise_speed * 100),
        'battery_capacity': 5000,
        'vbat_min_cell_voltage': 320,
        'vbat_warning_cell_voltage': 350,
        'platform_type': 1 if not p['can_hover'] else 0,
    }


def run_msp(args):
    """Serve MSP: an INAV flight controller over TCP or UDP."""

    iv = InavVehicle(args)
    iv.no_fix = args.no_fix
    v = iv.v
    settings = build_settings(iv, args)

    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM if args.transport == 'udp'
                         else socket.SOCK_STREAM)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    sock.bind(('0.0.0.0', args.port))
    if args.transport == 'tcp':
        sock.listen(1)
    sock.setblocking(False)
    link = MspLink(sock, udp=(args.transport == 'udp'), verbose=args.verbose)

    ver = [int(x) for x in args.version.split('.')] + [0, 0]

    print(f'[sim] INAV {args.version} ({args.vehicle}): {v.mode.upper()}, r={v.radius:.0f}m @ '
          f'{v.cruise_speed:.0f}m/s, alt {v.cruise_alt:.0f}m around {args.lat:.5f},{args.lon:.5f}')
    print(f'[sim] MSP over {args.transport.upper()} on port {args.port}: in Kite pick protocol MSP, '
          f'transport {args.transport.upper()}, host 127.0.0.1, port {args.port}')
    print(f'[sim] {"NO GPS FIX" if args.no_fix else "3D fix"}, '
          f'{"disarmed" if not v.armed else "armed"}. '
          f'RC: CH5 arms, CH6 selects MANUAL / POSHOLD / MISSION')

    def handle(code, payload):
        """Return the response payload, or None to answer with an MSP error frame."""
        if code == MSP_API_VERSION:
            return bytes([2, 2, 5])                       # MSP protocol 2, API 2.5
        if code == MSP_FC_VARIANT:
            return b'INAV'
        if code == MSP_FC_VERSION:
            return bytes(ver[:3])
        if code == MSP_BOARD_INFO:
            return b'SITL' + struct.pack('<H', 0)
        if code == MSP_BUILD_INFO:
            return b'Jan 01 2026' + b'00:00:00' + b'kitesim'
        if code == MSP_NAME:
            return b'Kite SIM'
        if code == MSP_UID:
            return struct.pack('<III', 0x4B495445, 0x53494D554C, 0x41544F52)
        if code == MSP2_INAV_MIXER:
            # platform_type at byte 3, mixer preset (i16) at 5..7 (connection.rs step 5)
            # motorDirectionInverted, reserved, motorstopOnLow, platformType, hasFlaps,
            # appliedMixerPreset u16, maxMotors, maxServos. PLATFORM_AIRPLANE = 1 (mixer.h).
            plane = not iv.profile['can_hover']
            return struct.pack('<BBBBBhBB', 0, 0, 0, 1 if plane else 0, 1 if plane else 0,
                               0, 8, 8)
        if code == MSP2_INAV_STATUS:
            return iv.status_payload()
        if code in (MSP_STATUS, MSP_STATUS_EX):
            # Legacy status: cycle time, i2c errors, sensors, mode bitmask, profile.
            sensors = 0x01 | 0x02 | 0x04 | (0 if iv.no_fix else 0x08)
            return struct.pack('<HHHIB', 2500, 0, sensors,
                               int.from_bytes(iv.active_modes_bytes(), 'little'), 0)
        if code == MSP_ATTITUDE:
            return struct.pack('<hhh', int(math.degrees(v.roll) * 10),
                               int(math.degrees(v.pitch) * 10),
                               int(math.degrees(v.yaw) % 360))
        if code == MSP_RAW_GPS:
            return iv.gps_payload()
        if code == MSP_ALTITUDE:
            # estimated alt (cm), vario (cm/s), baro alt (cm). Kite reads the first two; the third
            # is there because a real FC sends it and a short reply would be a lie about the format.
            return struct.pack('<ihi', int(v.rel_alt * 100), int(-v.vz * 100),
                               int(v.rel_alt * 100))
        if code == MSP2_INAV_ANALOG:
            return iv.analog_payload()
        if code == MSP_ANALOG:
            return struct.pack('<BHHh', int(v.voltage * 10), 0, 1023, 1850)
        if code == MSP2_INAV_AIR_SPEED:
            return struct.pack('<i', int(v.airspeed * 100))
        if code == MSP_SENSOR_STATUS:
            gps = 3 if iv.no_fix else 1                    # 1 = OK, 3 = UNHEALTHY
            return bytes([0 if iv.no_fix else 1, 1, 1, 1, 1, gps, 0, 1, 0])
        if code == MSP_GPSSTATISTICS:
            # lastMessageDt u16, errors u32, timeouts u32, packetCount u32, hdop u16, eph u16,
            # epv u16, hwVersion u8. Note the FIRST field is 16-bit: that puts hdop at offset 14,
            # and anything assuming a u32 there reads eph as the HDOP.
            return struct.pack('<HIIIHHHB', 100, 0, 0, 12345,
                               9999 if iv.no_fix else 110, 0 if iv.no_fix else 150,
                               0 if iv.no_fix else 220, 6)
        if code == MSP2_INAV_WIND:
            # speed cm/s, bearing the air moves TOWARD, flags bit0 = estimate valid. INAV reports
            # the direction the air moves toward, the opposite of MAVLink's WIND.
            toward = int(math.degrees(iv.profile['wind_from']) + 180) % 360
            return struct.pack('<HHB', int(iv.profile['wind'] * 100), toward, 0x01)
        if code == MSP2_INAV_MISC2:
            up = int(time.time() - iv.t_start)
            flight = int(time.time() - iv.flight_start) if iv.flight_start else 0
            thr = iv.profile['throttle'] if v.speed > 0.5 else 0
            return struct.pack('<IIBB', up, flight, thr, 1 if v.mode != 'manual' else 0)
        if code == MSP2_INAV_GET_LINK_STATS:
            return struct.pack('<BBb', 62, 100, 9)         # -62 dBm, LQ 100 %, SNR 9 dB
        if code == MSP_NAV_STATUS:
            return iv.nav_status_payload()
        if code == MSP_BOXIDS:
            return bytes(BOXES)
        if code == MSP_ACTIVEBOXES:
            return iv.active_modes_bytes()
        if code == MSP_RC:
            return iv.rc_payload()
        if code == MSP_RAW_IMU:
            return struct.pack('<9h', 0, 0, 1024, 0, 0, 0, 200, 0, 0)
        if code == MSP_COMP_GPS:
            d = dist_m(v.lat, v.lon, v.home_lat, v.home_lon)
            b = math.degrees(bearing_to(v.lat, v.lon, v.home_lat, v.home_lon)) % 360
            return struct.pack('<HHB', int(d), int(b), 1)
        if code == MSP_WP_GETINFO:
            return bytes([0, MAX_WAYPOINTS, 1 if iv.wps else 0, len(iv.wps)])
        if code == MSP_WP:
            if not payload:
                return None
            return iv.wp_payload(payload[0])
        if code == MSP_SET_WP:
            return b'' if iv.set_wp(payload) else None
        if code == MSP_SET_RAW_RC:
            chans = struct.unpack_from(f'<{len(payload) // 2}H', payload, 0)
            v.rc_override({f'chan{i + 1}_raw': u for i, u in enumerate(chans)})
            iv.apply_rc()
            return b''
        if code == MSP2_INAV_SET_AUX_RC:
            return b''                                     # latched overlay; acked so Kite stops resending
        if code == MSP2_COMMON_SETTING:
            name = payload.split(b'\x00')[0].decode('ascii', 'replace')
            if name not in settings:
                return None
            return struct.pack(SETTINGS_FMT[name], settings[name])
        if code == MSP2_COMMON_SET_SETTING:
            name = payload.split(b'\x00')[0].decode('ascii', 'replace')
            if name not in settings:
                return None
            raw = payload[len(name) + 1:]
            fmt = SETTINGS_FMT[name]
            size = struct.calcsize(fmt)
            if len(raw) < size:
                return None
            settings[name] = struct.unpack(fmt, raw[:size])[0]
            # Keep the flight model honest: a written radius has to change how it actually flies.
            if name == 'nav_fw_loiter_radius':
                v.radius = max(10.0, settings[name] / 100.0)
            elif name == 'nav_fw_cruise_speed':
                v.cruise_speed = max(1.0, settings[name] / 100.0)
            print(f'[sim] setting {name} = {settings[name]}')
            return b''
        if code == MSP_EEPROM_WRITE:
            return b''
        return None

    while True:
        for code, payload in link.poll():
            resp = handle(code, payload)
            name = CODE_NAMES.get(code, f'0x{code:04X}')
            if resp is None:
                link.send(code, b'', err=True)
                print(f'[sim] unsupported request {name} ({code}), answered MSP error')
            else:
                link.send(code, resp)
                if args.verbose:
                    print(f'[sim] {name} -> {len(resp)} B')
        v.update()
        time.sleep(0.002)


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--firmware', choices=['ardupilot', 'inav'], default='ardupilot',
                    help='flight stack to imitate (default ardupilot)')
    ap.add_argument('--protocol', choices=['mavlink', 'msp'],
                    help='wire protocol (default mavlink for ardupilot, msp for inav; '
                         'inav also speaks mavlink, ardupilot never speaks msp)')
    ap.add_argument('--gcs-nav', dest='gcs_nav', action='store_true', default=True,
                    help='inav over mavlink: the GCS NAV box is active, which INAV requires before '
                         'it will accept a guided target at all (default on)')
    ap.add_argument('--no-gcs-nav', dest='gcs_nav', action='store_false',
                    help='inav over mavlink: no GCS NAV box, so guided targets come back DENIED')
    ap.add_argument('--vehicle', choices=sorted(PROFILES), default='plane',
                    help='airframe to imitate (default plane)')
    # Transport and port default per protocol rather than to a constant, because the right value
    # differs, and each matches what Kite's own connection bar defaults to for that transport:
    # MAVLink is UDP 14550, MSP is TCP 5761 (ConnectionControls.svelte). Resolved after parsing so
    # an explicit flag still wins.
    ap.add_argument('--transport', choices=['udp', 'tcp'],
                    help='how Kite reaches the sim (default udp for mavlink, tcp for msp; '
                         'MAVLink here is UDP only)')
    ap.add_argument('--port', type=int, help='port to bind (default 14550 mavlink, 5761 msp)')
    ap.add_argument('--lat', type=float, default=HOME_LAT, help='home / orbit centre latitude')
    ap.add_argument('--lon', type=float, default=HOME_LON, help='home / orbit centre longitude')
    ap.add_argument('--radius', type=float, help='orbit / loiter radius in m (default per airframe)')
    ap.add_argument('--speed', type=float, help='cruise speed in m/s (default per airframe)')
    ap.add_argument('--alt', type=float, help='cruise altitude above home in m (default per airframe)')
    ap.add_argument('--mode', help='flight mode to start in, e.g. loiter (INAV POSHOLD), auto, '
                                   'manual, rtl (default auto when armed, manual when disarmed)')
    ap.add_argument('--no-fix', action='store_true', help='report no GPS fix')
    ap.add_argument('--disarmed', action='store_true', help='stay disarmed and parked at home')
    ap.add_argument('--chatter', action='store_true',
                    help='ardupilot only: emit a repeating STATUSTEXT nag')
    ap.add_argument('--verbose', action='store_true', help='log every command and mission exchange')
    ap.add_argument('--defs', help='mavlink only: path to ardupilotmega.xml '
                                   '(default: the vendored mavlink crate)')
    ap.add_argument('--version', default='9.1.0',
                    help='inav only: version to report (default 9.1.0, INAV master)')
    args = ap.parse_args()

    inav = args.firmware == 'inav'
    if args.protocol is None:
        args.protocol = 'msp' if inav else 'mavlink'
    if args.protocol == 'msp' and not inav:
        ap.error('ArduPilot does not speak MSP; use --firmware inav')
    msp = args.protocol == 'msp'
    if args.transport is None:
        args.transport = 'tcp' if msp else 'udp'
    if args.port is None:
        args.port = 5761 if msp else 14550
    if not msp and args.transport != 'udp':
        ap.error('MAVLink here is UDP only; use --protocol msp for TCP')

    run_msp(args) if msp else run_mavlink(args)


if __name__ == '__main__':
    try:
        main()
    except KeyboardInterrupt:
        print('\n[sim] stopped')
