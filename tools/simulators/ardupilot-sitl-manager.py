#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marc Hoffmann (b14ckyy)
"""ArduPilot SITL manager for Kite development — a small Tk desk for simulated vehicles and swarms, so
Mission Planner is not needed to fly SITL. Python 3.10+ with Tkinter (bundled), nothing to install.

    python tools/simulators/ardupilot-sitl-manager.py                # the window
    python tools/simulators/ardupilot-sitl-manager.py --auto-start   # window, Start pressed for you
    python tools/simulators/ardupilot-sitl-manager.py --headless --count 3 --frame quad --layout udp --seconds 600

What it does
- Single vehicle or a swarm (up to 32), the common ArduPilot frames (plane, quadplane, copter frames,
  heli, rover, boat, sub) with the right default parameter files.
- Start position from named presets, swarm formation (line / grid, spacing in metres), speedup, sysid
  per vehicle, extra parameters applied over MAVLink after boot (PARAM_SET, confirmed by the PARAM_VALUE
  echo — so they stick without wiping the EEPROM, and an unknown name is reported instead of ignored).
- Kite link layouts: one TCP link per vehicle (SERIAL0 = tcp 5760+10·instance), one shared UDP port every
  vehicle pushes to (fan-in, target host configurable — a Kite on another machine works), or Mission
  Planner's swarm chain (ArduPilot routing carries every sysid on vehicle 1's TCP link).
- Live status per vehicle (mode, armed, alt, speed, heading, GPS, battery, last STATUSTEXT) on the
  manager's own UDP monitor channel (each instance: --serial5 udpclient → the manager). The manager
  announces itself as sysid 254, not 255, so its heartbeats do not mask a lost Kite link for the GCS
  failsafe.
- Watchdog: an instance that exits is restarted (on Windows the Cygwin binaries die the moment a TCP
  client disconnects — a Kite disconnect would otherwise leave a dead port).
- Binaries: Windows uses Mission Planner's sitl folder or downloads a channel (latest / Stable / Beta / …)
  from firmware.ardupilot.org; Linux downloads the native SITL builds; macOS points at a waf build
  (`./waf configure --board sitl && ./waf plane`, no prebuilt SITL exists there). Parameter files come
  from the ArduPilot GitHub tree.

Why not Mission Planner's own Swarm button: its Cygwin binaries reject the `-P NAME=VALUE` options MP
passes (usage + exit → "connection refused"), the SYSID_THISMAV it writes per instance no longer exists
on current builds (renamed MAV_SYSID — `--sysid` works on every version), and an instance dies the moment
any TCP client disconnects from it. Parameter names change between ArduPilot versions (master 2026:
SYSID_THISMAV → MAV_SYSID, SYSID_MYGCS → MAV_GCS_SYSID, ARMING_CHECK → ARMING_SKIPCHK); an extra
parameter the vehicle does not know is never acknowledged — the status column then says so.

Files: settings + presets in <data dir>/settings.json, instance state
(eeprom.bin, logs, stdout) under <data dir>/instances/<n>, downloads under bin/<channel>, parameter files
under params/. Data dir: %LOCALAPPDATA%\\kite-sitl (Windows), ~/Library/Application Support/kite-sitl (macOS),
~/.local/share/kite-sitl (Linux).
"""
from __future__ import annotations

import argparse
import ctypes
import json
import math
import os
import platform
import re
import socket
import struct
import subprocess
import sys
import threading
import time
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path

IS_WINDOWS = platform.system() == "Windows"
IS_MACOS = platform.system() == "Darwin"
IS_LINUX = platform.system() == "Linux"

# ── Paths ─────────────────────────────────────────────────────────────────────────────────────────
if IS_WINDOWS:
    ROOT = Path(os.environ.get("LOCALAPPDATA", Path.home() / "AppData" / "Local")) / "kite-sitl"
elif IS_MACOS:
    ROOT = Path.home() / "Library" / "Application Support" / "kite-sitl"
else:
    ROOT = Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local" / "share")) / "kite-sitl"
SETTINGS_PATH = ROOT / "settings.json"
INSTANCES_DIR = ROOT / "instances"
BIN_ROOT = ROOT / "bin"
PARAMS_DIR = ROOT / "params"
for _d in (ROOT, INSTANCES_DIR, BIN_ROOT, PARAMS_DIR):
    _d.mkdir(parents=True, exist_ok=True)


def documents_dir() -> Path:
    """The user's Documents folder — on Windows via the shell (honours a OneDrive redirect)."""
    if IS_WINDOWS:
        buf = ctypes.create_unicode_buffer(1024)
        if ctypes.windll.shell32.SHGetFolderPathW(None, 5, None, 0, buf) == 0:  # CSIDL_PERSONAL
            return Path(buf.value)
    return Path.home() / "Documents"


MP_SITL_DIR = documents_dir() / "Mission Planner" / "sitl"

# ── Frames (vehicle → binary, frame → default parameter files; from ArduPilot's vehicleinfo.py) ──
FRAMES = [
    ("Plane", "plane", ["models/plane.parm"]),
    ("Plane", "plane-elevon", ["models/plane.parm", "default_params/plane-elevons.parm"]),
    ("Plane", "plane-vtail", ["models/plane.parm", "default_params/plane-vtail.parm"]),
    ("Plane", "plane-dspoilers", ["models/plane.parm", "default_params/plane-dspoilers.parm"]),
    ("Plane", "plane-jet", ["models/plane.parm", "default_params/plane-jet.parm"]),
    ("Plane", "plane-soaring", ["models/plane.parm", "default_params/plane-soaring.parm"]),
    ("Plane", "glider", ["default_params/glider.parm"]),
    ("Plane", "quadplane", ["default_params/quadplane.parm"]),
    ("Plane", "quadplane-tilt", ["default_params/quadplane.parm", "default_params/quadplane-tilt.parm"]),
    ("Plane", "quadplane-tri", ["default_params/quadplane.parm", "default_params/quadplane-tri.parm"]),
    ("Plane", "quadplane-tilttri", ["default_params/quadplane.parm", "default_params/quadplane-tilttri.parm"]),
    ("Plane", "plane-tailsitter", ["default_params/plane-tailsitter.parm"]),
    ("Plane", "quadplane-copter_tailsitter", ["default_params/quadplane.parm", "default_params/quadplane-copter_tailsitter.parm"]),
    ("Copter", "quad", ["default_params/copter.parm"]),
    ("Copter", "X", ["default_params/copter.parm", "default_params/copter-X.parm"]),
    ("Copter", "hexa", ["default_params/copter.parm", "default_params/copter-hexa.parm"]),
    ("Copter", "octa", ["default_params/copter.parm", "default_params/copter-octa.parm"]),
    ("Copter", "octa-quad", ["default_params/copter.parm", "default_params/copter-octaquad.parm"]),
    ("Copter", "tri", ["default_params/copter.parm", "default_params/copter-tri.parm"]),
    ("Copter", "y6", ["default_params/copter.parm", "default_params/copter-y6.parm"]),
    ("Copter", "singlecopter", ["default_params/copter-single.parm"]),
    ("Copter", "coaxcopter", ["default_params/copter-single.parm", "default_params/copter-coax.parm"]),
    ("Heli", "heli", ["default_params/copter-heli.parm"]),
    ("Heli", "heli-dual", ["default_params/copter-heli.parm", "default_params/copter-heli-dual.parm"]),
    ("Rover", "rover", ["default_params/rover.parm"]),
    ("Rover", "rover-skid", ["default_params/rover.parm", "default_params/rover-skid.parm"]),
    ("Rover", "balancebot", ["default_params/rover.parm", "default_params/rover-skid.parm", "default_params/balancebot.parm"]),
    ("Rover", "motorboat", ["default_params/rover.parm", "default_params/motorboat.parm"]),
    ("Rover", "sailboat", ["default_params/rover.parm", "default_params/sailboat.parm"]),
    ("Sub", "vectored", ["default_params/sub.parm"]),
    ("Sub", "vectored_6dof", ["default_params/sub-6dof.parm"]),
]
VEHICLES = ["Plane", "Copter", "Heli", "Rover", "Sub"]
# Binary names: the Cygwin builds MP downloads (Windows) / the native waf and firmware-server builds.
BINARY_WIN = {"Plane": "ArduPlane", "Copter": "ArduCopter", "Heli": "ArduHeli", "Rover": "ArduRover", "Sub": "ArduSub"}
BINARY_NATIVE = {"Plane": "arduplane", "Copter": "arducopter", "Heli": "arducopter-heli", "Rover": "ardurover", "Sub": "ardusub"}
FIRMWARE_DIR = {"Plane": "Plane", "Copter": "Copter", "Heli": "Copter", "Rover": "Rover", "Sub": "Sub"}
CHANNELS_WIN = {  # firmware.ardupilot.org/Tools/MissionPlanner/sitl/<sub>/
    "latest (daily build)": "", "Stable": "Stable", "Beta": "Beta",
    "PlaneStable": "PlaneStable", "CopterStable": "CopterStable", "RoverStable": "RoverStable",
}
CHANNELS_LINUX = {"latest (daily build)": "latest", "stable": "stable", "beta": "beta"}
CHANNELS = CHANNELS_WIN if IS_WINDOWS else CHANNELS_LINUX
FIRMWARE_MP_URL = "https://firmware.ardupilot.org/Tools/MissionPlanner/sitl/"
FIRMWARE_URL = "https://firmware.ardupilot.org/"
CYGWIN_DLLS = ["cygwin1.dll", "cygstdc++-6.dll", "cyggcc_s-seh-1.dll", "cyggcc_s-1.dll", "cygatomic-1.dll",
               "cyggomp-1.dll", "cygquadmath-0.dll", "cygssp-0.dll", "cygiconv-2.dll", "cygintl-8.dll"]
PARAM_URL = "https://raw.githubusercontent.com/ArduPilot/ardupilot/master/Tools/autotest/"

MODE_NAMES = {
    "plane": {0: "MANUAL", 1: "CIRCLE", 2: "STABILIZE", 3: "TRAINING", 4: "ACRO", 5: "FBWA", 6: "FBWB", 7: "CRUISE",
              8: "AUTOTUNE", 10: "AUTO", 11: "RTL", 12: "LOITER", 13: "TAKEOFF", 14: "AVOID_ADSB", 15: "GUIDED",
              16: "INITIALISING", 17: "QSTABILIZE", 18: "QHOVER", 19: "QLOITER", 20: "QLAND", 21: "QRTL",
              22: "QAUTOTUNE", 23: "QACRO", 24: "THERMAL", 25: "LOITER_ALT_QLAND", 26: "AUTOLAND"},
    "copter": {0: "STABILIZE", 1: "ACRO", 2: "ALT_HOLD", 3: "AUTO", 4: "GUIDED", 5: "LOITER", 6: "RTL", 7: "CIRCLE",
               9: "LAND", 11: "DRIFT", 13: "SPORT", 14: "FLIP", 15: "AUTOTUNE", 16: "POSHOLD", 17: "BRAKE", 18: "THROW",
               19: "AVOID_ADSB", 20: "GUIDED_NOGPS", 21: "SMART_RTL", 22: "FLOWHOLD", 23: "FOLLOW", 24: "ZIGZAG",
               25: "SYSTEMID", 26: "AUTOROTATE", 27: "AUTO_RTL", 28: "TURTLE"},
    "rover": {0: "MANUAL", 1: "ACRO", 3: "STEERING", 4: "HOLD", 5: "LOITER", 6: "FOLLOW", 7: "SIMPLE", 8: "DOCK",
              9: "CIRCLE", 10: "AUTO", 11: "RTL", 12: "SMART_RTL", 15: "GUIDED", 16: "INITIALISING"},
    "sub": {0: "STABILIZE", 1: "ACRO", 2: "ALT_HOLD", 3: "AUTO", 4: "GUIDED", 7: "CIRCLE", 9: "SURFACE",
            16: "POSHOLD", 19: "MANUAL", 20: "MOTORDETECT"},
}


def mode_class(mav_type: int) -> str:
    if mav_type in (1, 19, 20, 21, 22, 23, 24, 25):
        return "plane"
    if mav_type in (10, 11):
        return "rover"
    if mav_type == 12:
        return "sub"
    return "copter"


# ── Settings ──────────────────────────────────────────────────────────────────────────────────────
DEFAULTS = {
    "vehicle": "Plane", "frame": "plane", "count": 1, "sysidBase": 1, "speedup": 1.0, "wipe": False,
    "preset": "CMAC (ArduPilot default)", "formation": "line", "spacingM": 12,
    "layout": "tcp", "udpPort": 14550, "udpHost": "127.0.0.1", "monitorPort": 14650, "autoRestart": True,
    "extraParams": "",
    "binSource": "auto", "channel": next(iter(CHANNELS)), "buildDir": "",
    "presets": [{"name": "CMAC (ArduPilot default)", "lat": -35.363261, "lon": 149.165230, "alt": 584, "hdg": 353}],
}


def load_settings() -> dict:
    s = json.loads(json.dumps(DEFAULTS))
    if SETTINGS_PATH.exists():
        try:
            for k, v in json.loads(SETTINGS_PATH.read_text(encoding="utf-8-sig")).items():
                if v is not None:
                    s[k] = v
        except (OSError, ValueError):
            pass
    if not s["presets"]:
        s["presets"] = json.loads(json.dumps(DEFAULTS["presets"]))
    return s


def save_settings(s: dict) -> None:
    SETTINGS_PATH.write_text(json.dumps(s, indent=2), encoding="utf-8")


SETTINGS = load_settings()


# ── Binaries + parameter files ────────────────────────────────────────────────────────────────────
def channel_dir(name: str) -> Path:
    return BIN_ROOT / re.sub(r"[^A-Za-z]", "_", name)


def fit_window(root, width: int, height: int, min_width: int, min_height: int) -> None:
    """Open at `width`×`height`, but never narrower than the widgets need: Tk's widget sizes follow the
    platform font and theme (Windows: Segoe UI 9 + vista; Linux: DejaVu Sans 10 + default at 1.33 scaling),
    and a fixed geometry that fits one platform clips the right-hand group on the other (measured here:
    the ArduPilot window wants 1424 px on Debian, 1200 was set). The natural width also floors the
    minimum size, so the window cannot be shrunk into clipping; both capped to the screen."""
    root.update_idletasks()
    need_w = min(root.winfo_reqwidth(), root.winfo_screenwidth() - 40)
    need_h = min(root.winfo_reqheight(), root.winfo_screenheight() - 80)
    root.geometry(f"{max(width, need_w)}x{max(height, need_h)}")
    root.minsize(max(min_width, need_w), min_height)


def binary_name(vehicle: str) -> str:
    return BINARY_WIN[vehicle] + ".exe" if IS_WINDOWS else BINARY_NATIVE[vehicle]


def bin_dir() -> Path:
    """Where the vehicle binaries come from: the downloaded channel, MP's folder (Windows) or a waf build."""
    src = SETTINGS["binSource"]
    if src == "build" and SETTINGS["buildDir"]:
        return Path(SETTINGS["buildDir"])
    dl = channel_dir(SETTINGS["channel"])
    if src == "download":
        return dl
    if src == "mp":
        return MP_SITL_DIR
    # auto: a downloaded channel wins, then MP's folder (Windows), then a build dir if given
    if any(dl.glob("*")):
        return dl
    if IS_WINDOWS and (MP_SITL_DIR / "cygwin1.dll").exists():
        return MP_SITL_DIR
    if SETTINGS["buildDir"]:
        return Path(SETTINGS["buildDir"])
    return dl


def download_file(url: str, dest: Path) -> None:
    dest.parent.mkdir(parents=True, exist_ok=True)
    tmp = dest.with_suffix(dest.suffix + ".part")
    with urllib.request.urlopen(url, timeout=60) as r, open(tmp, "wb") as f:
        while chunk := r.read(1 << 16):
            f.write(chunk)
    tmp.replace(dest)


def download_channel(name: str, progress) -> Path:
    """Fetch every vehicle binary of a channel (+ the cygwin DLLs on Windows)."""
    d = channel_dir(name)
    files: list[tuple[str, Path]] = []
    if IS_WINDOWS:
        base = FIRMWARE_MP_URL + (CHANNELS_WIN[name] + "/" if CHANNELS_WIN[name] else "")
        for v in VEHICLES:
            files.append((f"{base}{BINARY_WIN[v]}.elf", d / f"{BINARY_WIN[v]}.exe"))
        files += [(base + dll, d / dll) for dll in CYGWIN_DLLS]
    elif IS_LINUX:
        arch = "SITL_x86_64_linux_gnu" if platform.machine() in ("x86_64", "AMD64") else "SITL_arm_linux_gnueabihf"
        for v in VEHICLES:
            # The heli build lives in its own arch folder (`…_linux_gnu-heli/arducopter-heli`), not next to
            # arducopter — unlike the flat Mission Planner folder Windows downloads from (checked on the
            # server for stable/beta/latest, x86_64 and armhf).
            arch_dir = arch + ("-heli" if v == "Heli" else "")
            files.append((f"{FIRMWARE_URL}{FIRMWARE_DIR[v]}/{CHANNELS_LINUX[name]}/{arch_dir}/{BINARY_NATIVE[v]}", d / BINARY_NATIVE[v]))
    else:
        raise RuntimeError("No prebuilt SITL for macOS — build ArduPilot with waf (--board sitl) and point Source at build/sitl/bin")
    for i, (url, dest) in enumerate(files, 1):
        progress(f"Downloading {i}/{len(files)}: {dest.name}")
        download_file(url, dest)
        if not IS_WINDOWS:
            dest.chmod(0o755)
    progress(f"Channel '{name}' ready in {d}")
    return d


def resolve_param_file(rel: str) -> Path:
    """MP's folder first (it has the common ones), then our cache, then the ArduPilot tree."""
    for base in (MP_SITL_DIR, PARAMS_DIR):
        p = base / rel
        if p.exists():
            return p
    dest = PARAMS_DIR / rel
    download_file(PARAM_URL + rel, dest)
    return dest


# ── MAVLink (v2, the little we need) ─────────────────────────────────────────────────────────────
MGR_SYSID, MGR_COMPID = 254, 190
_seq = 0


def mav_crc(data: bytes, extra: int) -> int:
    c = 0xFFFF
    for b in data + bytes([extra]):
        t = (b ^ (c & 0xFF)) & 0xFF
        t = (t ^ ((t << 4) & 0xFF)) & 0xFF
        c = ((c >> 8) ^ (t << 8) ^ (t << 3) ^ (t >> 4)) & 0xFFFF
    return c


def mav_frame(msgid: int, payload: bytes, crc_extra: int) -> bytes:
    global _seq
    hdr = bytes([0xFD, len(payload), 0, 0, _seq & 0xFF, MGR_SYSID, MGR_COMPID, msgid & 0xFF, (msgid >> 8) & 0xFF, (msgid >> 16) & 0xFF])
    _seq += 1
    body = hdr[1:] + payload
    return hdr + payload + struct.pack("<H", mav_crc(body, crc_extra))


def mav_heartbeat() -> bytes:
    # custom_mode u32, type (6 = GCS), autopilot (8 = invalid), base_mode, system_status, mavlink_version
    return mav_frame(0, struct.pack("<IBBBBB", 0, 6, 8, 0, 0, 3), 50)


def mav_param_set(target: int, name: str, value: float) -> bytes:
    # MAV_PARAM_TYPE_REAL32 (9) — ArduPilot converts to the parameter's own type
    return mav_frame(23, struct.pack("<fBB16sB", value, target, 1, name.encode("ascii")[:16].ljust(16, b"\0"), 9), 168)


def mav_message_interval(target: int, msgid: int, hz: float) -> bytes:
    # COMMAND_LONG: param1..7, command (511 SET_MESSAGE_INTERVAL), target_system, target_component, confirmation
    return mav_frame(76, struct.pack("<7fHBBB", msgid, 1e6 / hz, 0, 0, 0, 0, 0, 511, target, 1, 0), 152)


def pad(p: bytes, n: int) -> bytes:
    """MAVLink 2 trims trailing zero bytes; restore the declared length before reading fields."""
    return p if len(p) >= n else p + bytes(n - len(p))


def iter_frames(d: bytes):
    """Yield (sysid, msgid, payload) for every MAVLink 2 frame in a datagram / stream chunk."""
    i, n = 0, len(d)
    while i + 12 <= n:
        if d[i] != 0xFD:
            i += 1
            continue
        ln = d[i + 1]
        total = 12 + ln + (13 if d[i + 2] & 1 else 0)
        if i + total > n:
            break
        yield d[i + 5], d[i + 7] | (d[i + 8] << 8) | (d[i + 9] << 16), d[i + 10:i + 10 + ln]
        i += total


# ── Instances ─────────────────────────────────────────────────────────────────────────────────────
@dataclass
class Instance:
    index: int
    instance: int
    sysid: int
    frame: str
    vehicle: str
    dir: Path
    proc: subprocess.Popen | None = None
    tree: ProcessTree | None = None
    args: list[str] = field(default_factory=list)
    restarts: int = 0
    started_at: float = 0.0
    state: str = "stopped"
    # live state from the monitor channel
    endpoint: tuple[str, int] | None = None
    mav_type: int = 0
    mode: str = ""
    armed: bool = False
    last_hb: float = 0.0
    lat: float = 0.0
    lon: float = 0.0
    rel_alt: float = 0.0
    hdg: float = 0.0
    ground_speed: float = 0.0
    airspeed: float = 0.0
    throttle: int = 0
    fix_type: int = 0
    sats: int = 0
    voltage: float = 0.0
    current: float = 0.0
    batt_pct: int = -1
    status_text: str = ""
    param_note: str = ""  # sticky: extra params the vehicle never acknowledged
    intervals_requested: bool = False
    params_applied: bool = False
    pending_params: dict[str, float] = field(default_factory=dict)
    param_tries: int = 0
    last_param_send: float = 0.0

    @property
    def alive(self) -> bool:
        return self.proc is not None and self.proc.poll() is None

    @property
    def serial0_port(self) -> int:
        return 5760 + 10 * self.instance


class ProcessTree:
    """One SITL instance's process tree. Windows: a job object with kill-on-close — ArduPilot's SITL re-executes
    itself on a reboot command, so the process we started becomes a stub and the real one is its child; terminating the JOB ends both, and closing the manager does too. POSIX: a
    process group, killed as a whole."""

    def __init__(self):
        self.job = None
        if IS_WINDOWS:
            k = ctypes.windll.kernel32

            class BasicLimit(ctypes.Structure):
                _fields_ = [("PerProcessUserTimeLimit", ctypes.c_int64), ("PerJobUserTimeLimit", ctypes.c_int64),
                            ("LimitFlags", ctypes.c_uint32), ("MinimumWorkingSetSize", ctypes.c_size_t),
                            ("MaximumWorkingSetSize", ctypes.c_size_t), ("ActiveProcessLimit", ctypes.c_uint32),
                            ("Affinity", ctypes.c_size_t), ("PriorityClass", ctypes.c_uint32), ("SchedulingClass", ctypes.c_uint32)]

            class IoCounters(ctypes.Structure):
                _fields_ = [(n, ctypes.c_uint64) for n in ("ReadOperationCount", "WriteOperationCount", "OtherOperationCount",
                                                             "ReadTransferCount", "WriteTransferCount", "OtherTransferCount")]

            class ExtendedLimit(ctypes.Structure):
                _fields_ = [("BasicLimitInformation", BasicLimit), ("IoInfo", IoCounters), ("ProcessMemoryLimit", ctypes.c_size_t),
                            ("JobMemoryLimit", ctypes.c_size_t), ("PeakProcessMemoryUsed", ctypes.c_size_t), ("PeakJobMemoryUsed", ctypes.c_size_t)]

            self.job = k.CreateJobObjectW(None, None)
            info = ExtendedLimit()
            info.BasicLimitInformation.LimitFlags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            k.SetInformationJobObject(self.job, 9, ctypes.byref(info), ctypes.sizeof(info))

    @staticmethod
    def popen_kwargs() -> dict:
        if IS_WINDOWS:
            return {"creationflags": subprocess.CREATE_NO_WINDOW}
        return {"start_new_session": True}

    def adopt(self, proc: subprocess.Popen) -> None:
        if self.job:
            ctypes.windll.kernel32.AssignProcessToJobObject(self.job, int(proc._handle))  # noqa: SLF001

    def terminate(self, proc: subprocess.Popen | None) -> None:
        if IS_WINDOWS:
            if self.job:
                ctypes.windll.kernel32.TerminateJobObject(self.job, 1)
                ctypes.windll.kernel32.CloseHandle(self.job)
                self.job = None
        elif proc is not None:
            import signal
            try:
                os.killpg(proc.pid, signal.SIGTERM)
                time.sleep(0.5)
                os.killpg(proc.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        if proc is not None:
            try:
                proc.wait(timeout=3)
            except subprocess.TimeoutExpired:
                pass


class Manager:
    """The vehicles, their monitor channel and the housekeeping — UI-agnostic."""

    def __init__(self):
        self.instances: list[Instance] = []
        self.udp: socket.socket | None = None
        self.endpoint_sysid: dict[tuple[str, int], int] = {}
        self.last_mgr_hb = 0.0

    # -- configuration → command line ------------------------------------------------------------
    def preset(self) -> dict:
        for p in SETTINGS["presets"]:
            if p["name"] == SETTINGS["preset"]:
                return p
        return SETTINGS["presets"][0]

    def home(self, k: int) -> str:
        pr = self.preset()
        d_lat = SETTINGS["spacingM"] / 111320.0
        d_lon = SETTINGS["spacingM"] / (111320.0 * math.cos(math.radians(pr["lat"])))
        if SETTINGS["formation"] == "grid":
            cols = max(1, math.ceil(math.sqrt(SETTINGS["count"])))
            lat, lon = pr["lat"] - (k // cols) * d_lat, pr["lon"] + (k % cols) * d_lon
        else:
            lat, lon = pr["lat"], pr["lon"] + k * d_lon
        return f"{lat:.7f},{lon:.7f},{pr['alt']:.1f},{pr['hdg']:.0f}"

    @staticmethod
    def extra_params() -> list[tuple[str, float]]:
        out = []
        for line in str(SETTINGS["extraParams"]).splitlines():
            m = re.match(r"^([A-Za-z0-9_]{1,16})\s*[=,]\s*(-?[0-9.]+)$", line.strip())
            if m:
                out.append((m.group(1).upper(), float(m.group(2))))
        return out

    def build_args(self, inst: Instance) -> list[str]:
        files = next(p for v, f, p in FRAMES if f == SETTINGS["frame"])
        defaults = ",".join(str(resolve_param_file(f)) for f in files) + ",identity.parm"
        a = [f"-M{SETTINGS['frame']}", f"-O{self.home(inst.index)}", f"-s{SETTINGS['speedup']}",
             "--instance", str(inst.instance), "--sysid", str(inst.sysid),
             "--serial0", "tcp:0",
             "--serial5", f"udpclient:127.0.0.1:{SETTINGS['monitorPort']}",
             "--defaults", defaults]
        if SETTINGS["wipe"]:
            a.append("-w")
        if SETTINGS["layout"] == "chain" and inst.index < SETTINGS["count"] - 1:
            # Vehicle k's SERIAL2 dials vehicle k+1's SERIAL1 server (5762 + 10·instance) — MP's swarm chain.
            a += ["--serial2", f"tcpclient:127.0.0.1:{5762 + 10 * (inst.instance + 1)}"]
        elif SETTINGS["layout"] == "udp":
            a += ["--serial6", f"udpclient:{SETTINGS['udpHost']}:{SETTINGS['udpPort']}"]
        return a

    def write_identity(self, inst: Instance) -> None:
        # Parameter DEFAULTS at boot (a value already stored in the EEPROM wins — which is why the extra
        # params are also pushed over MAVLink after boot). SERIAL5/6 default to "no protocol".
        lines = ["SERIAL0_PROTOCOL=2", "SERIAL1_PROTOCOL=2", "SERIAL2_PROTOCOL=2", "SERIAL5_PROTOCOL=2", "SERIAL6_PROTOCOL=2",
                 "SIM_TERRAIN=0", "TERRAIN_ENABLE=0", "SIM_DRIFT_SPEED=0", "SIM_DRIFT_TIME=0"]
        lines += [f"{n}={v:g}" for n, v in self.extra_params()]
        (inst.dir / "identity.parm").write_text("\n".join(lines) + "\n", encoding="ascii")

    # -- process control --------------------------------------------------------------------------
    def start_instance(self, inst: Instance) -> None:
        inst.dir.mkdir(parents=True, exist_ok=True)
        if SETTINGS["wipe"] and inst.restarts == 0:
            (inst.dir / "eeprom.bin").unlink(missing_ok=True)
        self.write_identity(inst)
        exe = bin_dir() / binary_name(SETTINGS["vehicle"])
        if not exe.exists():
            raise FileNotFoundError(f"{exe} not found — download the channel first" + (", or start that vehicle once in Mission Planner" if IS_WINDOWS else ""))
        inst.args = self.build_args(inst)
        out = open(inst.dir / "stdout.txt", "wb")
        inst.tree = ProcessTree()
        inst.proc = subprocess.Popen([str(exe), *inst.args], cwd=inst.dir, stdout=out, stderr=subprocess.STDOUT,
                                     **ProcessTree.popen_kwargs())
        out.close()
        inst.tree.adopt(inst.proc)
        inst.started_at = time.time()
        inst.state = "starting"
        inst.last_hb = 0.0
        inst.endpoint = None
        inst.intervals_requested = inst.params_applied = False
        inst.pending_params, inst.param_tries, inst.last_param_send = {}, 0, 0.0
        inst.param_note = ""

    @staticmethod
    def stop_instance(inst: Instance) -> None:
        if inst.tree is not None:
            inst.tree.terminate(inst.proc)  # the whole tree — a re-exec'd child included
        inst.tree = None
        inst.proc = None
        inst.state, inst.mode, inst.armed = "stopped", "", False

    def ports_busy(self) -> list[str]:
        """SERIAL0 ports we are about to use that something else already listens on."""
        mine = {i.serial0_port for i in self.instances if i.alive}
        hits = []
        for k in range(SETTINGS["count"]):
            port = 5760 + 10 * k
            if port in mine:
                continue
            with socket.socket() as s:
                s.settimeout(0.2)
                if s.connect_ex(("127.0.0.1", port)) == 0:
                    hits.append(str(port))
        return hits

    def start_all(self) -> None:
        self.stop_all()
        busy = self.ports_busy()
        if busy:
            raise RuntimeError(f"SITL ports already in use: {', '.join(busy)}. Stop Mission Planner's simulation (or another SITL) first.")
        self.udp = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.udp.bind(("127.0.0.1", int(SETTINGS["monitorPort"])))
        self.udp.setblocking(False)
        self.endpoint_sysid = {}
        order = list(range(SETTINGS["count"]))
        if SETTINGS["layout"] == "chain":
            order.reverse()  # the chain dials "up": start the last vehicle first
        for k in order:
            inst = Instance(index=k, instance=k, sysid=SETTINGS["sysidBase"] + k, frame=SETTINGS["frame"],
                            vehicle=SETTINGS["vehicle"], dir=INSTANCES_DIR / str(k + 1))
            self.start_instance(inst)
            self.instances.append(inst)
            time.sleep(0.4)
        self.instances.sort(key=lambda i: i.index)

    def stop_all(self) -> None:
        for inst in self.instances:
            self.stop_instance(inst)
        self.instances.clear()
        if self.udp:
            self.udp.close()
            self.udp = None

    # -- monitor ------------------------------------------------------------------------------------
    def send_to(self, inst: Instance, frame: bytes) -> None:
        if self.udp and inst.endpoint:
            try:
                self.udp.sendto(frame, inst.endpoint)
            except OSError:
                pass

    def poll(self) -> None:
        if not self.udp:
            return
        for _ in range(1000):  # ~32 vehicles × 30 msg/s × 250 ms tick, with headroom
            try:
                d, addr = self.udp.recvfrom(65535)
            except (BlockingIOError, ConnectionResetError, OSError):
                break
            key = (addr[0], addr[1])
            for sysid, msgid, payload in iter_frames(d):
                # Bind the endpoint to the first sysid heard on it: in a chain, ArduPilot forwards the
                # OTHER vehicles' broadcasts here once it knows us — duplicates of their own pushes.
                bound = self.endpoint_sysid.setdefault(key, sysid)
                if bound != sysid:
                    continue
                inst = next((i for i in self.instances if i.sysid == sysid), None)
                if inst is None:
                    continue
                if inst.endpoint is None:
                    inst.endpoint = key
                self.decode(inst, msgid, payload)
        now = time.time()
        hb_due = now - self.last_mgr_hb >= 1
        for inst in self.instances:
            if inst.endpoint is None:
                continue
            if hb_due:
                self.send_to(inst, mav_heartbeat())
            if inst.last_hb and not inst.intervals_requested:
                for msgid, hz in ((33, 3), (74, 3), (24, 1), (1, 1)):
                    self.send_to(inst, mav_message_interval(inst.sysid, msgid, hz))
                inst.intervals_requested = True
            # Extra params: PARAM_SET each, confirmed by the PARAM_VALUE echo; unanswered ones go again
            # every second (ArduPilot drops the odd request right after boot), five tries, then say so.
            if inst.last_hb and not inst.params_applied and now - inst.started_at >= 4:
                if inst.param_tries == 0:
                    inst.pending_params = dict(self.extra_params())
                if not inst.pending_params:
                    inst.params_applied = True
                    inst.param_note = ""
                elif now - inst.last_param_send >= 1:
                    if inst.param_tries >= 5:
                        inst.param_note = "params NOT confirmed: " + ", ".join(inst.pending_params)
                        inst.params_applied = True
                    else:
                        for name, value in inst.pending_params.items():
                            self.send_to(inst, mav_param_set(inst.sysid, name, value))
                        inst.param_tries += 1
                        inst.last_param_send = now
        if hb_due:
            self.last_mgr_hb = now

    @staticmethod
    def decode(inst: Instance, msgid: int, pl: bytes) -> None:
        if msgid == 0:  # HEARTBEAT
            p = pad(pl, 9)
            mode, mav_type, _ap, base_mode, _st, _v = struct.unpack("<IBBBBB", p[:9])
            inst.mav_type = mav_type
            inst.mode = MODE_NAMES[mode_class(mav_type)].get(mode, f"mode {mode}")
            inst.armed = bool(base_mode & 128)
            inst.last_hb = time.time()
            inst.state = "running"
        elif msgid == 1:  # SYS_STATUS: voltage_battery u16 @14, current_battery i16 @16, battery_remaining i8 @30
            p = pad(pl, 31)
            inst.voltage = struct.unpack_from("<H", p, 14)[0] / 1000.0
            inst.current = struct.unpack_from("<h", p, 16)[0] / 100.0
            inst.batt_pct = struct.unpack_from("<b", p, 30)[0]
        elif msgid == 22:  # PARAM_VALUE: value f32 @0, id char[16] @8 — the echo that confirms a PARAM_SET
            p = pad(pl, 25)
            name = p[8:24].rstrip(b"\0").decode("ascii", "replace")
            if name in inst.pending_params and abs(struct.unpack_from("<f", p, 0)[0] - inst.pending_params[name]) < 1e-3:
                del inst.pending_params[name]
        elif msgid == 24:  # GPS_RAW_INT: fix_type u8 @28, satellites_visible u8 @29
            p = pad(pl, 30)
            inst.fix_type, inst.sats = p[28], p[29]
        elif msgid == 33:  # GLOBAL_POSITION_INT: lat i32 @4, lon i32 @8, relative_alt i32 @16, hdg u16 @26
            p = pad(pl, 28)
            inst.lat = struct.unpack_from("<i", p, 4)[0] / 1e7
            inst.lon = struct.unpack_from("<i", p, 8)[0] / 1e7
            inst.rel_alt = struct.unpack_from("<i", p, 16)[0] / 1000.0
            h = struct.unpack_from("<H", p, 26)[0]
            if h != 65535:
                inst.hdg = h / 100.0
        elif msgid == 74:  # VFR_HUD: airspeed f32 @0, groundspeed f32 @4, throttle u16 @18
            p = pad(pl, 20)
            inst.airspeed, inst.ground_speed = struct.unpack_from("<ff", p, 0)
            inst.throttle = struct.unpack_from("<H", p, 18)[0]
        elif msgid == 253:  # STATUSTEXT: severity u8 @0, text char[50] @1
            p = pad(pl, 51)
            sev, txt = p[0], p[1:51].rstrip(b"\0").decode("ascii", "replace")
            inst.status_text = f"ERROR: {txt}" if sev <= 3 else (f"warn: {txt}" if sev == 4 else txt)

    def watchdog(self) -> None:
        for inst in self.instances:
            if inst.state == "stopped":
                continue
            if inst.proc is not None and inst.proc.poll() is not None:
                if SETTINGS["autoRestart"]:
                    inst.restarts += 1
                    inst.state = "restarting"
                    try:
                        self.start_instance(inst)
                    except Exception as e:  # noqa: BLE001 — shown in the table
                        inst.state = f"failed: {e}"
                else:
                    inst.state, inst.mode, inst.armed = "died", "", False
            elif inst.last_hb and time.time() - inst.last_hb > 5:
                inst.state = "silent"

    def connect_hint(self) -> str:
        if not self.instances:
            return ""
        n = SETTINGS["count"]
        if SETTINGS["layout"] == "udp":
            return f"Kite: MAVLink / UDP  {SETTINGS['udpHost']}:{SETTINGS['udpPort']}   (all {n} vehicle(s) on that one link)"
        if SETTINGS["layout"] == "chain":
            return f"Kite: MAVLink / TCP  127.0.0.1:5760   (all {n} vehicle(s) on that one link via the chain)"
        return "Kite: MAVLink / TCP  " + ", ".join(f"127.0.0.1:{i.serial0_port}" for i in self.instances)


FIX_NAMES = ["none", "none", "2D", "3D", "DGPS", "RTKf", "RTKx"]


def row_values(inst: Instance) -> tuple:
    layout = SETTINGS["layout"]
    link = f"udp:{SETTINGS['udpPort']}" if layout == "udp" else ("tcp:5760 (head)" if layout == "chain" and inst.index == 0 else ("chained" if layout == "chain" else f"tcp:{inst.serial0_port}"))
    state = inst.state + (f" (×{inst.restarts})" if inst.restarts else "")
    live = bool(inst.last_hb)
    gps = f"{FIX_NAMES[min(inst.fix_type, 6)]} {inst.sats}" if live else ""
    batt = f"{inst.voltage:.1f} V" + (f" {inst.batt_pct} %" if inst.batt_pct >= 0 else "") if live and inst.voltage > 0 else ""
    return (inst.sysid, inst.frame, link, inst.proc.pid if inst.alive else "", state, inst.mode,
            ("ARMED" if inst.armed else "disarmed") if live else "",
            str(int(round(inst.rel_alt))) if live else "", f"{inst.ground_speed:.1f}" if live else "", f"{inst.hdg:.0f}" if live else "",
            gps, batt, (f"{inst.param_note} | {inst.status_text}" if inst.param_note else inst.status_text))


# ── Headless mode ─────────────────────────────────────────────────────────────────────────────────
def run_headless(mgr: Manager, seconds: int) -> None:
    mgr.start_all()
    print(f"Started {SETTINGS['count']} x {SETTINGS['frame']} ({SETTINGS['layout']}); monitor on UDP {SETTINGS['monitorPort']}. Ctrl+C stops everything.")
    print(mgr.connect_hint())
    t0 = last_print = time.time()
    try:
        while True:
            mgr.poll()
            mgr.watchdog()
            if time.time() - last_print >= 2:
                last_print = time.time()
                for i in mgr.instances:
                    state = i.state + (f" (x{i.restarts})" if i.restarts else "")
                    print(f"{i.sysid:3}  {i.frame:<10} {state:<14} {i.proc.pid if i.alive else '-':<8} {i.mode:<12} "
                          f"{'ARMED' if i.armed else 'disarmed':<8} {int(round(i.rel_alt)):6d} m {i.ground_speed:5.1f} m/s {i.hdg:4.0f}deg  "
                          f"gps {i.fix_type}/{i.sats:<2} {i.voltage:5.1f} V  {(i.param_note + ' | ') if i.param_note else ''}{i.status_text}")
                print()
            if seconds and time.time() - t0 >= seconds:
                break
            time.sleep(0.1)
    except KeyboardInterrupt:
        pass
    finally:
        mgr.stop_all()
        print("stopped")


# ── UI ────────────────────────────────────────────────────────────────────────────────────────────
def run_ui(mgr: Manager, auto_start: bool) -> None:
    import tkinter as tk
    from tkinter import filedialog, messagebox, ttk

    if IS_WINDOWS:
        try:
            ctypes.windll.shcore.SetProcessDpiAwareness(1)
        except (AttributeError, OSError):
            pass

    root = tk.Tk()
    root.title("Kite SITL manager")
    pad_ = {"padx": 4, "pady": 2}

    top = ttk.Frame(root, padding=6)
    top.pack(fill="x")

    # Vehicle
    g_veh = ttk.LabelFrame(top, text="Vehicle", padding=6)
    g_veh.grid(row=0, column=0, sticky="nsew", padx=4, pady=2)
    v_vehicle, v_frame = tk.StringVar(), tk.StringVar()
    v_count, v_sysid, v_speed, v_wipe = tk.IntVar(), tk.IntVar(), tk.DoubleVar(), tk.BooleanVar()
    ttk.Label(g_veh, text="Type").grid(row=0, column=0, sticky="w", **pad_)
    cb_vehicle = ttk.Combobox(g_veh, textvariable=v_vehicle, values=VEHICLES, state="readonly", width=10)
    cb_vehicle.grid(row=0, column=1, **pad_)
    ttk.Label(g_veh, text="Frame").grid(row=0, column=2, sticky="w", **pad_)
    cb_frame = ttk.Combobox(g_veh, textvariable=v_frame, state="readonly", width=22)
    cb_frame.grid(row=0, column=3, **pad_)
    ttk.Label(g_veh, text="Vehicles").grid(row=1, column=0, sticky="w", **pad_)
    ttk.Spinbox(g_veh, from_=1, to=32, textvariable=v_count, width=6).grid(row=1, column=1, sticky="w", **pad_)
    ttk.Label(g_veh, text="first sysid").grid(row=1, column=2, sticky="w", **pad_)
    ttk.Spinbox(g_veh, from_=1, to=250, textvariable=v_sysid, width=6).grid(row=1, column=3, sticky="w", **pad_)
    ttk.Label(g_veh, text="Speedup").grid(row=2, column=0, sticky="w", **pad_)
    ttk.Spinbox(g_veh, from_=1, to=20, increment=0.5, textvariable=v_speed, width=6).grid(row=2, column=1, sticky="w", **pad_)
    ttk.Checkbutton(g_veh, text="Wipe EEPROM (fresh params)", variable=v_wipe).grid(row=2, column=2, columnspan=2, sticky="w", **pad_)

    # Start position
    g_pos = ttk.LabelFrame(top, text="Start position", padding=6)
    g_pos.grid(row=0, column=1, sticky="nsew", padx=4, pady=2)
    v_preset, v_lat, v_lon, v_alt, v_hdg = (tk.StringVar() for _ in range(5))
    v_formation, v_spacing = tk.StringVar(), tk.IntVar()
    ttk.Label(g_pos, text="Preset").grid(row=0, column=0, sticky="w", **pad_)
    cb_preset = ttk.Combobox(g_pos, textvariable=v_preset, width=26)
    cb_preset.grid(row=0, column=1, columnspan=3, sticky="w", **pad_)
    ttk.Button(g_pos, text="Save", width=7, command=lambda: save_preset()).grid(row=0, column=4, **pad_)
    ttk.Button(g_pos, text="Delete", width=7, command=lambda: delete_preset()).grid(row=0, column=5, **pad_)
    ttk.Label(g_pos, text="Lat").grid(row=1, column=0, sticky="w", **pad_)
    ttk.Entry(g_pos, textvariable=v_lat, width=13).grid(row=1, column=1, sticky="w", **pad_)
    ttk.Label(g_pos, text="Lon").grid(row=1, column=2, sticky="w", **pad_)
    ttk.Entry(g_pos, textvariable=v_lon, width=13).grid(row=1, column=3, sticky="w", **pad_)
    ttk.Label(g_pos, text="Alt m").grid(row=1, column=4, sticky="w", **pad_)
    ttk.Entry(g_pos, textvariable=v_alt, width=7).grid(row=1, column=5, sticky="w", **pad_)
    ttk.Label(g_pos, text="Hdg°").grid(row=2, column=0, sticky="w", **pad_)
    ttk.Entry(g_pos, textvariable=v_hdg, width=6).grid(row=2, column=1, sticky="w", **pad_)
    ttk.Label(g_pos, text="Formation").grid(row=2, column=2, sticky="w", **pad_)
    ttk.Combobox(g_pos, textvariable=v_formation, values=["line", "grid"], state="readonly", width=6).grid(row=2, column=3, sticky="w", **pad_)
    ttk.Label(g_pos, text="spacing m").grid(row=2, column=4, sticky="w", **pad_)
    ttk.Spinbox(g_pos, from_=2, to=500, textvariable=v_spacing, width=6).grid(row=2, column=5, sticky="w", **pad_)

    # Kite link layout
    g_link = ttk.LabelFrame(top, text="Kite connects via", padding=6)
    g_link.grid(row=0, column=2, sticky="nsew", padx=4, pady=2)
    v_layout, v_udp_host, v_udp_port, v_restart = tk.StringVar(), tk.StringVar(), tk.IntVar(), tk.BooleanVar()
    ttk.Radiobutton(g_link, text="TCP link per vehicle (5760, 5770, …)", variable=v_layout, value="tcp").grid(row=0, column=0, columnspan=3, sticky="w", **pad_)
    ttk.Radiobutton(g_link, text="one UDP port, all vehicles push to it (fan-in)", variable=v_layout, value="udp").grid(row=1, column=0, columnspan=3, sticky="w", **pad_)
    udp_row = ttk.Frame(g_link)
    udp_row.grid(row=2, column=0, columnspan=3, sticky="w", padx=(24, 4))
    ttk.Label(udp_row, text="host").pack(side="left")
    ttk.Entry(udp_row, textvariable=v_udp_host, width=14).pack(side="left", padx=(4, 10))
    ttk.Label(udp_row, text="port").pack(side="left")
    ttk.Spinbox(udp_row, from_=1024, to=65535, textvariable=v_udp_port, width=7).pack(side="left", padx=4)
    ttk.Radiobutton(g_link, text="one TCP link 5760, vehicles chained (MP swarm)", variable=v_layout, value="chain").grid(row=3, column=0, columnspan=3, sticky="w", **pad_)
    ttk.Checkbutton(g_link, text="Restart vehicles that exit (TCP disconnect kills them)", variable=v_restart).grid(row=4, column=0, columnspan=3, sticky="w", **pad_)

    # Extra params + binaries
    mid = ttk.Frame(root, padding=(6, 0))
    mid.pack(fill="x")
    g_params = ttk.LabelFrame(mid, text="Extra parameters (NAME=VALUE per line)", padding=6)
    g_params.grid(row=0, column=0, sticky="nsew", padx=4, pady=2)
    txt_params = tk.Text(g_params, width=44, height=5, font=("Consolas" if IS_WINDOWS else "Menlo", 10))
    txt_params.pack(fill="both", expand=True)

    g_bin = ttk.LabelFrame(mid, text="Binaries", padding=6)
    g_bin.grid(row=0, column=1, sticky="nsew", padx=4, pady=2)
    mid.columnconfigure(1, weight=1)
    v_source, v_channel, v_build = tk.StringVar(), tk.StringVar(), tk.StringVar()
    sources = ["auto", "download", "build"] + (["mp"] if IS_WINDOWS else [])
    source_labels = {"auto": "auto (downloaded channel, else MP folder / build)", "download": "downloaded channel",
                     "build": "waf build folder (build/sitl/bin)", "mp": "Mission Planner folder"}
    ttk.Label(g_bin, text="Source").grid(row=0, column=0, sticky="w", **pad_)
    cb_source = ttk.Combobox(g_bin, textvariable=v_source, values=[source_labels[s] for s in sources], state="readonly", width=44)
    cb_source.grid(row=0, column=1, columnspan=2, sticky="w", **pad_)
    ttk.Label(g_bin, text="Channel").grid(row=0, column=3, sticky="w", **pad_)
    ttk.Combobox(g_bin, textvariable=v_channel, values=list(CHANNELS), state="readonly", width=20).grid(row=0, column=4, sticky="w", **pad_)
    btn_download = ttk.Button(g_bin, text="Download / update channel", command=lambda: start_download())
    btn_download.grid(row=0, column=5, sticky="w", **pad_)
    ttk.Label(g_bin, text="Build dir").grid(row=1, column=0, sticky="w", **pad_)
    ttk.Entry(g_bin, textvariable=v_build, width=46).grid(row=1, column=1, columnspan=2, sticky="w", **pad_)
    ttk.Button(g_bin, text="…", width=3, command=lambda: pick_build_dir()).grid(row=1, column=3, sticky="w", **pad_)
    lbl_bin = ttk.Label(g_bin, text="", justify="left")
    lbl_bin.grid(row=2, column=0, columnspan=6, sticky="w", **pad_)

    # Actions
    act = ttk.Frame(root, padding=(10, 4))
    act.pack(fill="x")
    btn_start = ttk.Button(act, text="Start", width=14, command=lambda: on_start())
    btn_start.pack(side="left", padx=4)
    ttk.Button(act, text="Stop all", width=14, command=lambda: on_stop()).pack(side="left", padx=4)
    ttk.Button(act, text="Restart selected", width=16, command=lambda: on_restart_one()).pack(side="left", padx=4)
    lbl_connect = ttk.Label(act, text="", font=("Consolas" if IS_WINDOWS else "Menlo", 10))
    lbl_connect.pack(side="left", padx=16)

    # Grid
    cols = [("Sysid", 50), ("Frame", 90), ("Link", 110), ("PID", 60), ("State", 90), ("Mode", 90), ("Armed", 65),
            ("Alt m", 55), ("GS m/s", 60), ("Hdg", 45), ("GPS", 70), ("Batt", 90), ("Last status text", 320)]
    grid_box = ttk.Frame(root)
    grid_box.pack(fill="both", expand=True, padx=10, pady=(2, 4))
    grid = ttk.Treeview(grid_box, columns=[c for c, _ in cols], show="headings", height=8, selectmode="browse")
    for c, w in cols:
        grid.heading(c, text=c)
        grid.column(c, width=w, minwidth=40, stretch=(c == "Last status text"), anchor="w")
    grid.tag_configure("running", foreground="black")
    grid.tag_configure("starting", foreground="#b8860b")
    grid.tag_configure("bad", foreground="#b22222")
    sb = ttk.Scrollbar(grid_box, orient="vertical", command=grid.yview)
    grid.configure(yscrollcommand=sb.set)
    sb.pack(side="right", fill="y")
    grid.pack(side="left", fill="both", expand=True)

    # Log tail (the selected vehicle's stdout)
    txt_log = tk.Text(root, height=9, font=("Consolas" if IS_WINDOWS else "Menlo", 9), state="disabled")
    txt_log.pack(fill="x", padx=10, pady=(0, 4))
    v_status = tk.StringVar(value="idle")
    ttk.Label(root, textvariable=v_status, anchor="w", relief="sunken", padding=(6, 2)).pack(fill="x", side="bottom")

    # -- UI ↔ settings ---------------------------------------------------------------------------
    loading = {"on": False}

    def fill_frames(*_):
        frames = [f for v, f, _p in FRAMES if v == v_vehicle.get()]
        cb_frame["values"] = frames
        v_frame.set(SETTINGS["frame"] if SETTINGS["frame"] in frames else frames[0])

    def fill_presets():
        names = [p["name"] for p in SETTINGS["presets"]]
        cb_preset["values"] = names
        v_preset.set(SETTINGS["preset"] if SETTINGS["preset"] in names else names[0])

    def show_preset(*_):
        for p in SETTINGS["presets"]:
            if p["name"] == v_preset.get():
                v_lat.set(f"{p['lat']:.7f}".rstrip("0").rstrip("."))
                v_lon.set(f"{p['lon']:.7f}".rstrip("0").rstrip("."))
                v_alt.set(f"{p['alt']:g}")
                v_hdg.set(f"{p['hdg']:g}")

    def parse_preset() -> dict:
        try:
            return {"name": v_preset.get().strip(), "lat": float(v_lat.get()), "lon": float(v_lon.get()),
                    "alt": float(v_alt.get()), "hdg": float(v_hdg.get())}
        except ValueError:
            raise ValueError("Lat / Lon / Alt / Hdg must be numbers (decimal point, e.g. -35.363261)") from None

    def ui_to_settings():
        SETTINGS.update(vehicle=v_vehicle.get(), frame=v_frame.get(), count=int(v_count.get()), sysidBase=int(v_sysid.get()),
                        speedup=float(v_speed.get()), wipe=bool(v_wipe.get()), preset=v_preset.get(),
                        formation=v_formation.get(), spacingM=int(v_spacing.get()), layout=v_layout.get(),
                        udpHost=v_udp_host.get().strip() or "127.0.0.1", udpPort=int(v_udp_port.get()), autoRestart=bool(v_restart.get()),
                        extraParams=txt_params.get("1.0", "end").strip(),
                        binSource=next((s for s in sources if source_labels[s] == v_source.get()), "auto"),
                        channel=v_channel.get(), buildDir=v_build.get().strip())
        save_settings(SETTINGS)

    def settings_to_ui():
        loading["on"] = True
        v_vehicle.set(SETTINGS["vehicle"])
        fill_frames()
        v_count.set(int(SETTINGS["count"]))
        v_sysid.set(int(SETTINGS["sysidBase"]))
        v_speed.set(float(SETTINGS["speedup"]))
        v_wipe.set(bool(SETTINGS["wipe"]))
        fill_presets()
        show_preset()
        v_formation.set(SETTINGS["formation"])
        v_spacing.set(int(SETTINGS["spacingM"]))
        v_layout.set(SETTINGS["layout"] if SETTINGS["layout"] in ("tcp", "udp", "chain") else "tcp")
        v_udp_host.set(SETTINGS.get("udpHost") or "127.0.0.1")
        v_udp_port.set(int(SETTINGS["udpPort"]))
        v_restart.set(bool(SETTINGS["autoRestart"]))
        txt_params.delete("1.0", "end")
        txt_params.insert("1.0", str(SETTINGS["extraParams"]))
        v_source.set(source_labels.get(SETTINGS["binSource"] if SETTINGS["binSource"] in sources else "auto"))
        v_channel.set(SETTINGS["channel"] if SETTINGS["channel"] in CHANNELS else next(iter(CHANNELS)))
        v_build.set(SETTINGS.get("buildDir") or "")
        loading["on"] = False

    def refresh_bin_label():
        d = bin_dir()
        have = [v for v in VEHICLES if (d / binary_name(v)).exists()]
        missing = [v for v in VEHICLES if v not in have]
        extra = "   (cygwin DLLs missing!)" if IS_WINDOWS and not (d / "cygwin1.dll").exists() else ""
        lbl_bin["text"] = f"Using: {d}\nVehicles present: {', '.join(have) or 'none'}" + (f"   missing: {', '.join(missing)}" if missing else "") + extra

    def refresh_grid():
        rows = grid.get_children()
        if len(rows) != len(mgr.instances):
            grid.delete(*rows)
            rows = [grid.insert("", "end") for _ in mgr.instances]
            if rows:
                grid.selection_set(rows[0])
        for iid, inst in zip(rows, mgr.instances):
            vals = row_values(inst)
            if tuple(grid.item(iid, "values")) != tuple(str(v) for v in vals):
                grid.item(iid, values=vals)
            tag = "running" if inst.state == "running" else ("starting" if inst.state in ("starting", "restarting") else "bad")
            if grid.item(iid, "tags") != (tag,):
                grid.item(iid, tags=(tag,))

    def selected_instance() -> Instance | None:
        sel = grid.selection()
        if not sel or not mgr.instances:
            return None
        idx = grid.get_children().index(sel[0])
        return mgr.instances[idx] if idx < len(mgr.instances) else None

    def refresh_log():
        inst = selected_instance()
        if inst is None:
            return
        f = inst.dir / "stdout.txt"
        if f.exists():
            try:
                tail = "\n".join(f.read_text(encoding="utf-8", errors="replace").splitlines()[-40:])
            except OSError:
                return
            if txt_log.get("1.0", "end").strip() != tail.strip():
                txt_log["state"] = "normal"
                txt_log.delete("1.0", "end")
                txt_log.insert("1.0", tail)
                txt_log.see("end")
                txt_log["state"] = "disabled"

    # -- actions ------------------------------------------------------------------------------------
    def save_preset():
        try:
            p = parse_preset()
            if not p["name"]:
                raise ValueError("Give the preset a name (type it into the Preset box)")
            SETTINGS["presets"] = [q for q in SETTINGS["presets"] if q["name"] != p["name"]] + [p]
            SETTINGS["preset"] = p["name"]
            fill_presets()
            ui_to_settings()
            v_status.set(f"Preset '{p['name']}' saved")
        except ValueError as e:
            messagebox.showerror("Preset", str(e))

    def delete_preset():
        if len(SETTINGS["presets"]) <= 1:
            return
        SETTINGS["presets"] = [q for q in SETTINGS["presets"] if q["name"] != v_preset.get()]
        SETTINGS["preset"] = SETTINGS["presets"][0]["name"]
        fill_presets()
        show_preset()
        ui_to_settings()

    def pick_build_dir():
        d = filedialog.askdirectory(title="ArduPilot waf build folder (build/sitl/bin)")
        if d:
            v_build.set(d)
            v_source.set(source_labels["build"])
            ui_to_settings()
            refresh_bin_label()

    dl_state = {"msg": None, "done": False, "error": None}

    def start_download():
        ui_to_settings()
        btn_download["state"] = "disabled"
        dl_state.update(msg=None, done=False, error=None)

        def work():
            try:
                download_channel(SETTINGS["channel"], lambda m: dl_state.update(msg=m))
            except Exception as e:  # noqa: BLE001 — reported in the status bar
                dl_state["error"] = str(e)
            dl_state["done"] = True

        threading.Thread(target=work, daemon=True).start()
        poll_download()

    def poll_download():
        if dl_state["msg"]:
            v_status.set(dl_state["msg"])
        if not dl_state["done"]:
            root.after(300, poll_download)
            return
        btn_download["state"] = "normal"
        if dl_state["error"]:
            messagebox.showerror("Download failed", dl_state["error"])
            v_status.set("download failed")
        elif SETTINGS["binSource"] == "mp":
            v_source.set(source_labels["download"])
            ui_to_settings()
        refresh_bin_label()

    def on_start():
        try:
            p = parse_preset()
            # An edited position without a saved preset still flies: keep it as the preset's live values.
            SETTINGS["presets"] = [q for q in SETTINGS["presets"] if q["name"] != p["name"]] + [p]
            SETTINGS["preset"] = p["name"]
            fill_presets()
            ui_to_settings()
            v_status.set("starting…")
            root.update_idletasks()
            mgr.start_all()
            lbl_connect["text"] = mgr.connect_hint()
            refresh_grid()
            v_status.set(f"{SETTINGS['count']} × {SETTINGS['frame']} started — monitor on UDP {SETTINGS['monitorPort']}")
        except Exception as e:  # noqa: BLE001 — every start problem ends in this dialog
            messagebox.showerror("Start failed", str(e))
            v_status.set("start failed")

    def on_stop():
        mgr.stop_all()
        refresh_grid()
        lbl_connect["text"] = ""
        v_status.set("stopped")

    def on_restart_one():
        inst = selected_instance()
        if inst is None:
            return
        mgr.stop_instance(inst)
        inst.restarts += 1
        try:
            mgr.start_instance(inst)
        except Exception as e:  # noqa: BLE001
            messagebox.showerror("Restart failed", str(e))

    def tick():
        try:
            mgr.poll()
            mgr.watchdog()
            refresh_grid()
        except Exception as e:  # noqa: BLE001 — keep the loop alive, show the problem
            v_status.set(f"monitor: {e}")
        root.after(250, tick)

    def log_tick():
        try:
            refresh_log()
        except Exception:  # noqa: BLE001
            pass
        root.after(1500, log_tick)

    def on_close():
        try:
            ui_to_settings()
        except Exception:  # noqa: BLE001
            pass
        mgr.stop_all()
        root.destroy()

    cb_vehicle.bind("<<ComboboxSelected>>", fill_frames)
    cb_preset.bind("<<ComboboxSelected>>", show_preset)
    cb_source.bind("<<ComboboxSelected>>", lambda *_: (None if loading["on"] else (ui_to_settings(), refresh_bin_label())))
    grid.bind("<<TreeviewSelect>>", lambda *_: refresh_log())
    root.protocol("WM_DELETE_WINDOW", on_close)

    settings_to_ui()
    refresh_bin_label()
    fit_window(root, 1200, 820, 1000, 660)
    root.after(250, tick)
    root.after(1500, log_tick)
    if auto_start:
        root.after(100, on_start)
    root.mainloop()


def main() -> None:
    ap = argparse.ArgumentParser(description="ArduPilot SITL manager for Kite development")
    ap.add_argument("--headless", action="store_true", help="console mode: start with the saved settings, print a status table, Ctrl+C stops")
    ap.add_argument("--count", type=int, default=0)
    ap.add_argument("--frame", default="")
    ap.add_argument("--layout", choices=["", "tcp", "udp", "chain"], default="")
    ap.add_argument("--wipe", action="store_true")
    ap.add_argument("--seconds", type=int, default=0, help="headless: stop after this many seconds (0 = until Ctrl+C)")
    ap.add_argument("--auto-start", action="store_true", help="window mode: press Start right after opening")
    a = ap.parse_args()
    if a.count > 0:
        SETTINGS["count"] = min(a.count, 32)
    if a.frame:
        SETTINGS["frame"] = a.frame
        SETTINGS["vehicle"] = next(v for v, f, _p in FRAMES if f == a.frame)
    if a.layout:
        SETTINGS["layout"] = a.layout
    if a.wipe:
        SETTINGS["wipe"] = True
    mgr = Manager()
    if a.headless:
        run_headless(mgr, a.seconds)
    else:
        run_ui(mgr, a.auto_start)


if __name__ == "__main__":
    main()
