#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marc Hoffmann (b14ckyy)
"""INAV SITL manager for Kite development — the INAV sibling of ardupilot-sitl-manager.py: a small Tk desk
that fetches INAV's SITL binaries, runs one or several instances and shows their state. Python 3.10+
with Tkinter (bundled), nothing to install.

    python tools/simulators/inav-sitl-manager.py                # the window
    python tools/simulators/inav-sitl-manager.py --auto-start
    python tools/simulators/inav-sitl-manager.py --headless --count 2 --seconds 600

What it does
- Binaries without building: every nightly (github.com/iNavFlight/inav-nightly) ships `sitl-resources.zip`
  with the SITL for Windows (Cygwin), Linux (x86_64 / arm64) and macOS; every Configurator release zip
  carries the SITL of that version under `resources/sitl/` (pulled out of the 150 MB zip with HTTP range
  reads, nothing else is downloaded). Or point at your own build. The binary's own banner
  ("INAV 9.1.0 SITL (a898c03d)") is what the table shows — nightly tags carry the wrong version number.
- One instance or several: instance k listens on TCP base port 5760 + 10·k with its own eeprom.bin; a config
  wipe deletes it before the start. INAV's SITL has eight UARTs (base port + 0…7) but only binds the ones
  that carry a function — the default config is MSP on UART1 and UART2. Assign more in the Configurator
  (Ports tab, e.g. MAVLink on UART3 → port 5762 for a Kite MAVLink link; telemetry functions also need the
  Telemetry feature): the manager only READS the serial config back and shows it per UART, it never writes
  the vehicle's configuration. The manager itself sits on UART2 (MSP).
- Simulator passthrough for the first instance (INAV SITL has no physics of its own: without `--sim` it
  is "configurator only" — the firmware runs, nothing flies): RealFlight or X-Plane, host, port, --useimu,
  --chanmap. A serial receiver / proxy FC can be attached to the first instance as well.
- Live status per instance over the manager's own MSP connection on UART2: firmware version, armed /
  arming blocked, active modes, GPS, altitude, speed, heading, battery. Kite takes UART1.
- Watchdog: an instance that exits is restarted. (INAV's SITL survives client disconnects — unlike the
  ArduPilot Cygwin builds — so this only matters for crashes.)

Kite side: protocol MSP, transport TCP, host 127.0.0.1, port 5760 (5770, 5780 … for the others).

Files: settings in <data dir>/settings-inav.json, instance state under <data dir>/instances-inav/<n>,
binaries under <data dir>/bin/inav/<source>/. Data dir: %LOCALAPPDATA%\\kite-sitl (Windows),
~/Library/Application Support/kite-sitl (macOS), ~/.local/share/kite-sitl (Linux).
"""
from __future__ import annotations

import argparse
import ctypes
import io
import json
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
import zipfile
from dataclasses import dataclass, field
from pathlib import Path

IS_WINDOWS = platform.system() == "Windows"
IS_MACOS = platform.system() == "Darwin"
IS_ARM = platform.machine().lower() in ("arm64", "aarch64")

# ── Paths ─────────────────────────────────────────────────────────────────────────────────────────
if IS_WINDOWS:
    ROOT = Path(os.environ.get("LOCALAPPDATA", Path.home() / "AppData" / "Local")) / "kite-sitl"
elif IS_MACOS:
    ROOT = Path.home() / "Library" / "Application Support" / "kite-sitl"
else:
    ROOT = Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local" / "share")) / "kite-sitl"
SETTINGS_PATH = ROOT / "settings-inav.json"
INSTANCES_DIR = ROOT / "instances-inav"
BIN_ROOT = ROOT / "bin" / "inav"
for _d in (ROOT, INSTANCES_DIR, BIN_ROOT):
    _d.mkdir(parents=True, exist_ok=True)

NIGHTLY_REPO = "iNavFlight/inav-nightly"
CONFIGURATOR_REPO = "iNavFlight/inav-configurator"
BINARY = "inav_SITL.exe" if IS_WINDOWS else "inav_SITL"
# Where the SITL for this machine lives inside sitl-resources.zip / the Configurator zip.
if IS_WINDOWS:
    ZIP_MEMBERS = ["resources/sitl/windows/inav_SITL.exe", "resources/sitl/windows/cygwin1.dll"]
elif IS_MACOS:
    ZIP_MEMBERS = ["resources/sitl/macos/inav_SITL"]
elif IS_ARM:
    ZIP_MEMBERS = ["resources/sitl/linux/arm64/inav_SITL"]
else:
    ZIP_MEMBERS = ["resources/sitl/linux/inav_SITL"]


def configurator_asset(version: str) -> str:
    if IS_WINDOWS:
        return f"INAV-Configurator_Win64_{version}.zip"
    if IS_MACOS:
        return f"INAV-Configurator_MacOS_{'arm64' if IS_ARM else 'x64'}_{version}.zip"
    return f"INAV-Configurator_linux_{'arm64' if IS_ARM else 'x64'}_{version}.zip"


# ── Settings ──────────────────────────────────────────────────────────────────────────────────────
DEFAULTS = {
    "count": 1, "basePort": 5760, "wipe": False,
    "source": "nightly", "nightlyTag": "", "configuratorVersion": "", "localBinary": "",
    "sim": "none", "simIp": "127.0.0.1", "simPort": 0, "useImu": False, "chanmap": "",
    "sdcard": "",
    "rxUart": 0, "rxPort": "", "rxBaud": 115200, "rxStopbits": "One", "rxParity": "None", "fcProxy": False,
    "autoRestart": True,
}


def load_settings() -> dict:
    s = dict(DEFAULTS)
    if SETTINGS_PATH.exists():
        try:
            for k, v in json.loads(SETTINGS_PATH.read_text(encoding="utf-8-sig")).items():
                if v is not None:
                    s[k] = v
        except (OSError, ValueError):
            pass
    return s


def save_settings(s: dict) -> None:
    SETTINGS_PATH.write_text(json.dumps(s, indent=2), encoding="utf-8")


SETTINGS = load_settings()


# ── Binaries ──────────────────────────────────────────────────────────────────────────────────────
def github_json(path: str):
    req = urllib.request.Request(f"https://api.github.com/{path}", headers={"Accept": "application/vnd.github+json", "User-Agent": "kite-sitl-manager"})
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.load(r)


def list_nightlies(n: int = 15) -> list[str]:
    return [r["tag_name"] for r in github_json(f"repos/{NIGHTLY_REPO}/releases?per_page={n}")]


def list_configurator_versions(n: int = 15) -> list[str]:
    return [r["tag_name"] for r in github_json(f"repos/{CONFIGURATOR_REPO}/releases?per_page={n}")]


class RangeFile(io.RawIOBase):
    """A remote file read through HTTP range requests — zipfile pulls single members out of a 150 MB
    Configurator zip without downloading the rest."""

    def __init__(self, url: str):
        r = urllib.request.urlopen(urllib.request.Request(url, method="HEAD"), timeout=30)
        self.url, self.size, self.pos = r.url, int(r.headers["Content-Length"]), 0

    def seekable(self):
        return True

    def readable(self):
        return True

    def tell(self):
        return self.pos

    def seek(self, off, whence=0):
        self.pos = {0: off, 1: self.pos + off, 2: self.size + off}[whence]
        return self.pos

    def read(self, n=-1):
        if n is None or n < 0:
            n = self.size - self.pos
        if n <= 0:
            return b""
        req = urllib.request.Request(self.url, headers={"Range": f"bytes={self.pos}-{self.pos + n - 1}"})
        d = urllib.request.urlopen(req, timeout=60).read()
        self.pos += len(d)
        return d

    def readinto(self, b):
        d = self.read(len(b))
        b[:len(d)] = d
        return len(d)


def extract_members(zf: zipfile.ZipFile, dest: Path, progress) -> None:
    dest.mkdir(parents=True, exist_ok=True)
    names = set(zf.namelist())
    for m in ZIP_MEMBERS:
        if m not in names:
            raise FileNotFoundError(f"{m} is not in this archive (no SITL for this platform in it)")
        progress(f"Extracting {Path(m).name}")
        out = dest / Path(m).name
        with zf.open(m) as src, open(out, "wb") as f:
            while chunk := src.read(1 << 16):
                f.write(chunk)
        if not IS_WINDOWS:
            out.chmod(0o755)


def fetch_nightly(tag: str, progress) -> Path:
    dest = BIN_ROOT / f"nightly-{tag}"
    if (dest / BINARY).exists():
        return dest
    url = f"https://github.com/{NIGHTLY_REPO}/releases/download/{tag}/sitl-resources.zip"
    progress(f"Downloading sitl-resources.zip of {tag}")
    with urllib.request.urlopen(url, timeout=120) as r:
        data = r.read()
    extract_members(zipfile.ZipFile(io.BytesIO(data)), dest, progress)
    progress(f"Nightly {tag} ready")
    return dest


def fetch_configurator(version: str, progress) -> Path:
    dest = BIN_ROOT / f"configurator-{version}"
    if (dest / BINARY).exists():
        return dest
    url = f"https://github.com/{CONFIGURATOR_REPO}/releases/download/{version}/{configurator_asset(version)}"
    progress(f"Reading the Configurator {version} zip (range requests, ~5 MB of it)")
    extract_members(zipfile.ZipFile(io.BufferedReader(RangeFile(url), buffer_size=1 << 20)), dest, progress)
    progress(f"Configurator {version} SITL ready")
    return dest


def binary_path() -> Path:
    src = SETTINGS["source"]
    if src == "local":
        return Path(SETTINGS["localBinary"])
    if src == "configurator":
        return BIN_ROOT / f"configurator-{SETTINGS['configuratorVersion']}" / BINARY
    return BIN_ROOT / f"nightly-{SETTINGS['nightlyTag']}" / BINARY


def binary_banner(exe: Path) -> str:
    """The first line the binary prints — 'INAV 9.1.0 SITL (a898c03d)'."""
    try:
        r = subprocess.run([str(exe), "--help"], capture_output=True, timeout=10,
                           creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        for line in (r.stdout + r.stderr).decode("utf-8", "replace").splitlines():
            if line.strip():
                return line.strip()
        return "?"
    except (OSError, subprocess.SubprocessError):
        return "?"


# ── MSP (v2 framing for everything, replies may come back in v1 or v2) ────────────────────────────
MSP_FC_VARIANT, MSP_FC_VERSION, MSP_BOXNAMES = 2, 3, 116
MSP_RAW_GPS, MSP_ATTITUDE, MSP_ALTITUDE = 106, 108, 109
MSP2_INAV_STATUS, MSP2_INAV_ANALOG = 0x2000, 0x2002
MSP2_COMMON_SERIAL_CONFIG = 0x1009  # read only — the manager never writes the vehicle's configuration
# serialPortFunction_e bits → names, for the UART table.
UART_FUNCTION_NAMES = {
    1 << 0: "MSP", 1 << 1: "GPS", 1 << 2: "FrSky D", 1 << 3: "HoTT", 1 << 4: "LTM", 1 << 5: "SmartPort",
    1 << 6: "RX serial", 1 << 7: "Blackbox", 1 << 8: "MAVLink", 1 << 9: "IBUS", 1 << 10: "RC device",
    1 << 11: "SmartAudio", 1 << 12: "Tramp", 1 << 14: "Optical flow", 1 << 15: "Log", 1 << 16: "Rangefinder",
    1 << 17: "FFPV VTX", 1 << 18: "ESC serial", 1 << 19: "SIM telemetry", 1 << 20: "FrSky OSD", 1 << 21: "DJI HD OSD",
    1 << 22: "Servo serial", 1 << 23: "S.Port master", 1 << 25: "MSP OSD", 1 << 26: "Gimbal", 1 << 27: "Head tracker",
}


def function_names(mask: int) -> str:
    names = [n for bit, n in UART_FUNCTION_NAMES.items() if mask & bit]
    rest = mask & ~sum(UART_FUNCTION_NAMES)
    if rest:
        names.append(f"fn {rest:#x}")
    return " + ".join(names) if names else "—"


ARMED_FLAG = 1 << 2
ARMING_DISABLED_MASK = ~((1 << 7) - 1) & 0xFFFFFFFF  # every armingFlags bit from 7 up is an ARMING_DISABLED_* reason


def crc8_dvb_s2(data: bytes, crc: int = 0) -> int:
    for b in data:
        crc ^= b
        for _ in range(8):
            crc = ((crc << 1) ^ 0xD5) & 0xFF if crc & 0x80 else (crc << 1) & 0xFF
    return crc


def msp2_request(cmd: int, payload: bytes = b"") -> bytes:
    body = struct.pack("<BHH", 0, cmd, len(payload)) + payload
    return b"$X<" + body + bytes([crc8_dvb_s2(body)])


def parse_msp(buf: bytearray):
    """Yield (cmd, payload) for every complete reply in buf; leaves a partial frame in place."""
    while True:
        i = buf.find(b"$")
        if i < 0:
            buf.clear()
            return
        if i:
            del buf[:i]
        if len(buf) < 3:
            return
        if buf[1:3] == b"X>" or buf[1:3] == b"X!":
            if len(buf) < 8:
                return
            _flags, cmd, size = struct.unpack_from("<BHH", buf, 3)
            if len(buf) < 9 + size:
                return
            payload = bytes(buf[8:8 + size])
            ok = buf[2:3] == b">"
            del buf[:9 + size]
            if ok:
                yield cmd, payload
        elif buf[1:3] == b"M>" or buf[1:3] == b"M!":
            if len(buf) < 5:
                return
            size, cmd = buf[3], buf[4]
            if len(buf) < 6 + size:
                return
            payload = bytes(buf[5:5 + size])
            ok = buf[2:3] == b">"
            del buf[:6 + size]
            if ok:
                yield cmd, payload
        else:
            del buf[:1]


# ── Instances ─────────────────────────────────────────────────────────────────────────────────────
@dataclass
class Instance:
    index: int
    base_port: int
    dir: Path
    proc: subprocess.Popen | None = None
    tree: ProcessTree | None = None
    restarts: int = 0
    started_at: float = 0.0
    state: str = "stopped"
    # monitor connection (UART2) and decoded state
    sock: socket.socket | None = None
    rx: bytearray = field(default_factory=bytearray)
    last_poll: float = 0.0
    last_reply: float = 0.0
    version: str = ""
    boxnames: list[str] = field(default_factory=list)
    armed: bool = False
    arming_blocked: bool = False
    modes: str = ""
    fix_type: int = 0
    sats: int = 0
    lat: float = 0.0
    lon: float = 0.0
    alt: float = 0.0
    ground_speed: float = 0.0
    hdg: float = 0.0
    voltage: float = 0.0
    current: float = 0.0
    batt_pct: int = -1
    # the vehicle's serial config, read only (identifier, functionMask, 4 baud indices) per UART
    serial_cfg: list[tuple] | None = None
    serial_cfg_at: float = 0.0

    @property
    def alive(self) -> bool:
        return self.proc is not None and self.proc.poll() is None


class ProcessTree:
    """One SITL instance's process tree. Windows: a job object with kill-on-close — the SITL re-executes
    itself on every reboot (Configurator "Save and reboot"), so the process we started becomes a stub and
    the real one is its child; terminating the JOB ends both, and closing the manager does too. POSIX: a
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
    def __init__(self):
        self.instances: list[Instance] = []

    @staticmethod
    def build_args(inst: Instance) -> list[str]:
        a = [f"--path={inst.dir / 'eeprom.bin'}", f"--tcpbaseport={inst.base_port}"]
        if inst.index == 0:
            # One simulator, one receiver: both belong to the first instance.
            if SETTINGS["sim"] in ("xp", "rf"):
                a.append(f"--sim={SETTINGS['sim']}")
                if SETTINGS["simIp"]:
                    a.append(f"--simip={SETTINGS['simIp']}")
                if int(SETTINGS["simPort"] or 0) > 0:
                    a.append(f"--simport={SETTINGS['simPort']}")
                if SETTINGS["useImu"]:
                    a.append("--useimu")
                if SETTINGS["chanmap"].strip():
                    a.append(f"--chanmap={SETTINGS['chanmap'].strip()}")
            if SETTINGS["sdcard"].strip():
                a.append(f"--sdcard={SETTINGS['sdcard'].strip()}")
            if int(SETTINGS["rxUart"] or 0) > 0 and SETTINGS["rxPort"].strip():
                a += [f"--serialuart={SETTINGS['rxUart']}", f"--serialport={SETTINGS['rxPort'].strip()}",
                      f"--baudrate={SETTINGS['rxBaud']}", f"--stopbits={SETTINGS['rxStopbits']}", f"--parity={SETTINGS['rxParity']}"]
                if SETTINGS["fcProxy"]:
                    a.append("--fcproxy")
        return a

    def start_instance(self, inst: Instance) -> None:
        inst.dir.mkdir(parents=True, exist_ok=True)
        if SETTINGS["wipe"] and inst.restarts == 0:
            (inst.dir / "eeprom.bin").unlink(missing_ok=True)
        exe = binary_path()
        if not exe.exists():
            raise FileNotFoundError(f"{exe} not found — fetch a nightly / Configurator SITL first, or point at a local binary")
        out = open(inst.dir / "stdout.txt", "wb")
        inst.tree = ProcessTree()
        inst.proc = subprocess.Popen([str(exe), *self.build_args(inst)], cwd=inst.dir, stdout=out, stderr=subprocess.STDOUT,
                                     **ProcessTree.popen_kwargs())
        out.close()
        inst.tree.adopt(inst.proc)
        inst.started_at = time.time()
        inst.state = "starting"
        self.close_monitor(inst)
        inst.version, inst.boxnames, inst.modes = "", [], ""
        inst.last_reply = 0.0
        inst.serial_cfg, inst.serial_cfg_at = None, 0.0

    def stop_instance(self, inst: Instance) -> None:
        self.close_monitor(inst)
        if inst.tree is not None:
            inst.tree.terminate(inst.proc)  # the whole tree — the re-exec'd child included
        inst.tree = None
        inst.proc = None
        inst.state, inst.armed, inst.modes = "stopped", False, ""

    def ports_busy(self) -> list[str]:
        mine = {i.base_port for i in self.instances if i.alive}
        hits = []
        for k in range(SETTINGS["count"]):
            port = SETTINGS["basePort"] + 10 * k
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
            raise RuntimeError(f"Ports already in use: {', '.join(busy)}. Stop the other SITL / Configurator session first.")
        for k in range(SETTINGS["count"]):
            inst = Instance(index=k, base_port=SETTINGS["basePort"] + 10 * k, dir=INSTANCES_DIR / str(k + 1))
            self.start_instance(inst)
            self.instances.append(inst)
            time.sleep(0.3)

    def stop_all(self) -> None:
        for inst in self.instances:
            self.stop_instance(inst)
        self.instances.clear()

    # -- monitor (MSP on UART2) --------------------------------------------------------------------
    @staticmethod
    def close_monitor(inst: Instance) -> None:
        if inst.sock:
            try:
                inst.sock.close()
            except OSError:
                pass
        inst.sock = None
        inst.rx.clear()

    def poll(self) -> None:
        now = time.time()
        for inst in self.instances:
            if not inst.alive:
                continue
            if inst.sock is None:
                if now - inst.started_at < 1.5 or now - inst.last_poll < 1:
                    continue
                inst.last_poll = now
                try:
                    s = socket.create_connection(("127.0.0.1", inst.base_port + 1), timeout=0.3)
                    s.setblocking(False)
                    inst.sock = s
                except OSError:
                    continue
            # read what arrived
            try:
                while True:
                    d = inst.sock.recv(65535)
                    if not d:
                        raise ConnectionError("closed")
                    inst.rx += d
            except BlockingIOError:
                pass
            except OSError:
                self.close_monitor(inst)
                continue
            for cmd, payload in parse_msp(inst.rx):
                inst.last_reply = now
                self.decode(inst, cmd, payload)
            # ask again at 2 Hz
            if now - inst.last_poll >= 0.5:
                inst.last_poll = now
                reqs = [MSP2_INAV_STATUS, MSP_RAW_GPS, MSP_ATTITUDE, MSP_ALTITUDE, MSP2_INAV_ANALOG]
                if not inst.version:
                    reqs = [MSP_FC_VARIANT, MSP_FC_VERSION] + reqs
                if not inst.boxnames:
                    reqs.append(MSP_BOXNAMES)
                if now - inst.serial_cfg_at >= 5:  # the Configurator may have changed it — keep the table honest
                    reqs.append(MSP2_COMMON_SERIAL_CONFIG)
                try:
                    inst.sock.sendall(b"".join(msp2_request(c) for c in reqs))
                except OSError:
                    self.close_monitor(inst)

    @staticmethod
    def decode(inst: Instance, cmd: int, p: bytes) -> None:
        if cmd == MSP_FC_VERSION and len(p) >= 3:
            inst.version = f"{p[0]}.{p[1]}.{p[2]}"
            inst.state = "running"
        elif cmd == MSP_BOXNAMES:
            inst.boxnames = [n for n in p.decode("ascii", "replace").split(";") if n]
        elif cmd == MSP2_INAV_STATUS and len(p) >= 14:
            # cycleTime u16, i2cErrors u16, sensorStatus u16, avgLoad u16, profiles u8, armingFlags u32,
            # box bitmask (the rest but the trailing mixer-profile byte)
            flags = struct.unpack_from("<I", p, 9)[0]
            inst.armed = bool(flags & ARMED_FLAG)
            inst.arming_blocked = bool(flags & ARMING_DISABLED_MASK)
            bits = p[13:-1] if len(p) > 14 else p[13:]
            active = [i for i in range(len(bits) * 8) if bits[i // 8] >> (i % 8) & 1]
            inst.modes = " ".join(inst.boxnames[i] if i < len(inst.boxnames) else f"box{i}" for i in active)
            inst.state = "running"
        elif cmd == MSP_RAW_GPS and len(p) >= 16:
            inst.fix_type, inst.sats = p[0], p[1]
            inst.lat = struct.unpack_from("<i", p, 2)[0] / 1e7
            inst.lon = struct.unpack_from("<i", p, 6)[0] / 1e7
            inst.ground_speed = struct.unpack_from("<H", p, 12)[0] / 100.0
        elif cmd == MSP_ATTITUDE and len(p) >= 6:
            inst.hdg = struct.unpack_from("<h", p, 4)[0]
        elif cmd == MSP_ALTITUDE and len(p) >= 4:
            inst.alt = struct.unpack_from("<i", p, 0)[0] / 100.0
        elif cmd == MSP2_COMMON_SERIAL_CONFIG and len(p) % 9 == 0:
            inst.serial_cfg = [struct.unpack_from("<BIBBBB", p, i) for i in range(0, len(p), 9)]
            inst.serial_cfg_at = time.time()
        elif cmd == MSP2_INAV_ANALOG and len(p) >= 24:
            # batteryFlags u8, vbat u16 (0.01 V), amperage i16 (0.01 A), power u32, mAh u32, mWh u32, remaining u32, pct u8
            inst.voltage = struct.unpack_from("<H", p, 1)[0] / 100.0
            inst.current = struct.unpack_from("<h", p, 3)[0] / 100.0
            inst.batt_pct = p[23]

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
                    inst.state, inst.armed, inst.modes = "died", False, ""
            elif inst.last_reply and time.time() - inst.last_reply > 5:
                inst.state = "silent"

    def connect_hint(self) -> str:
        if not self.instances:
            return ""
        cfg = self.instances[0].serial_cfg
        if not cfg:
            return "Kite: MSP / TCP  " + ", ".join(f"127.0.0.1:{i.base_port}" for i in self.instances) + "   (UART1; the manager sits on UART2)"
        parts = []
        for ident, mask, *_ in cfg:
            if ident == 1 or ident >= 8 or not mask:
                continue
            ports = ", ".join(str(i.base_port + ident) for i in self.instances)
            parts.append(f"{function_names(mask)} → tcp {ports}")
        return "Kite: " + "   ".join(parts) + "   (UART2 = manager)"


FIX_NAMES = ["none", "none", "2D", "3D"]


def row_values(inst: Instance) -> tuple:
    live = bool(inst.last_reply)
    state = inst.state + (f" (×{inst.restarts})" if inst.restarts else "")
    armed = ("ARMED" if inst.armed else ("blocked" if inst.arming_blocked else "disarmed")) if live else ""
    gps = f"{FIX_NAMES[min(inst.fix_type, 3)]} {inst.sats}" if live else ""
    batt = (f"{inst.voltage:.2f} V" + (f" {inst.batt_pct} %" if 0 <= inst.batt_pct <= 100 else "")) if live and inst.voltage > 0 else ("no battery" if live else "")
    sim = ""
    if inst.index == 0 and SETTINGS["sim"] in ("xp", "rf"):
        sim = {"xp": "X-Plane", "rf": "RealFlight"}[SETTINGS["sim"]]
    return (inst.index + 1, inst.version, f"tcp:{inst.base_port}", inst.proc.pid if inst.alive else "", state, armed,
            inst.modes, gps, f"{inst.alt:.0f}" if live else "", f"{inst.ground_speed:.1f}" if live else "",
            f"{inst.hdg:.0f}" if live else "", batt, sim)


# ── Headless mode ─────────────────────────────────────────────────────────────────────────────────
def run_headless(mgr: Manager, seconds: int) -> None:
    sys.stdout.reconfigure(line_buffering=True)  # tail -f friendly when redirected
    mgr.start_all()
    print(f"Started {SETTINGS['count']} INAV SITL instance(s) from {binary_path()}. Ctrl+C stops everything.")
    print(mgr.connect_hint())
    t0 = last_print = time.time()
    try:
        while True:
            mgr.poll()
            mgr.watchdog()
            if time.time() - last_print >= 2:
                last_print = time.time()
                for i in mgr.instances:
                    v = row_values(i)
                    print(f"#{v[0]}  {v[1]:<8} {v[2]:<10} {str(v[3]):<7} {v[4]:<14} {v[5]:<9} gps {v[7]:<8} alt {v[8]:>4} m  {v[9]:>5} m/s  {v[10]:>4}°  {v[11]:<14} {v[6]}")
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
    mono = "Consolas" if IS_WINDOWS else "Menlo"

    root = tk.Tk()
    root.title("Kite INAV SITL manager")
    root.geometry("1320x820")
    root.minsize(1060, 660)
    pad_ = {"padx": 4, "pady": 2}

    top = ttk.Frame(root, padding=6)
    top.pack(fill="x")

    # Instances
    g_inst = ttk.LabelFrame(top, text="Instances", padding=6)
    g_inst.grid(row=0, column=0, sticky="nsew", padx=4, pady=2)
    v_count, v_base, v_wipe, v_restart = tk.IntVar(), tk.IntVar(), tk.BooleanVar(), tk.BooleanVar()
    ttk.Label(g_inst, text="Count").grid(row=0, column=0, sticky="w", **pad_)
    ttk.Spinbox(g_inst, from_=1, to=16, textvariable=v_count, width=6).grid(row=0, column=1, sticky="w", **pad_)
    ttk.Label(g_inst, text="first base port").grid(row=0, column=2, sticky="w", **pad_)
    ttk.Spinbox(g_inst, from_=1024, to=65000, increment=10, textvariable=v_base, width=7).grid(row=0, column=3, sticky="w", **pad_)
    ttk.Checkbutton(g_inst, text="Wipe config (delete eeprom.bin) at start", variable=v_wipe).grid(row=1, column=0, columnspan=4, sticky="w", **pad_)
    ttk.Checkbutton(g_inst, text="Restart instances that exit", variable=v_restart).grid(row=2, column=0, columnspan=4, sticky="w", **pad_)
    ttk.Label(g_inst, text="Instance k: UART1 = base + 10·k (Kite), UART2 = +1 (this manager)", foreground="#666").grid(row=3, column=0, columnspan=4, sticky="w", **pad_)

    # Simulator (instance 1)
    g_sim = ttk.LabelFrame(top, text="Simulator (instance 1 — without one the firmware runs but nothing flies)", padding=6)
    g_sim.grid(row=0, column=1, sticky="nsew", padx=4, pady=2)
    v_sim, v_sim_ip, v_sim_port, v_useimu, v_chanmap, v_sdcard = tk.StringVar(), tk.StringVar(), tk.IntVar(), tk.BooleanVar(), tk.StringVar(), tk.StringVar()
    ttk.Radiobutton(g_sim, text="none", variable=v_sim, value="none").grid(row=0, column=0, sticky="w", **pad_)
    ttk.Radiobutton(g_sim, text="X-Plane (--sim=xp)", variable=v_sim, value="xp").grid(row=0, column=1, sticky="w", **pad_)
    ttk.Radiobutton(g_sim, text="RealFlight (--sim=rf)", variable=v_sim, value="rf").grid(row=0, column=2, sticky="w", **pad_)
    ttk.Label(g_sim, text="host").grid(row=1, column=0, sticky="w", **pad_)
    ttk.Entry(g_sim, textvariable=v_sim_ip, width=14).grid(row=1, column=1, sticky="w", **pad_)
    ttk.Label(g_sim, text="port (0 = default)").grid(row=1, column=2, sticky="w", **pad_)
    ttk.Spinbox(g_sim, from_=0, to=65535, textvariable=v_sim_port, width=7).grid(row=1, column=3, sticky="w", **pad_)
    ttk.Checkbutton(g_sim, text="--useimu (raw IMU from the sim)", variable=v_useimu).grid(row=2, column=0, columnspan=2, sticky="w", **pad_)
    ttk.Label(g_sim, text="chanmap").grid(row=2, column=2, sticky="w", **pad_)
    ttk.Entry(g_sim, textvariable=v_chanmap, width=22).grid(row=2, column=3, sticky="w", **pad_)
    ttk.Label(g_sim, text="SD card image").grid(row=3, column=0, sticky="w", **pad_)
    ttk.Entry(g_sim, textvariable=v_sdcard, width=36).grid(row=3, column=1, columnspan=2, sticky="w", **pad_)
    ttk.Button(g_sim, text="…", width=3, command=lambda: pick_file(v_sdcard, "SD card image")).grid(row=3, column=3, sticky="w", **pad_)

    # Serial receiver (instance 1)
    g_rx = ttk.LabelFrame(top, text="Serial receiver / proxy FC (instance 1)", padding=6)
    g_rx.grid(row=0, column=2, sticky="nsew", padx=4, pady=2)
    v_rx_uart, v_rx_port, v_rx_baud, v_rx_stop, v_rx_par, v_fcproxy = tk.IntVar(), tk.StringVar(), tk.IntVar(), tk.StringVar(), tk.StringVar(), tk.BooleanVar()
    ttk.Label(g_rx, text="UART (0 = off)").grid(row=0, column=0, sticky="w", **pad_)
    ttk.Spinbox(g_rx, from_=0, to=8, textvariable=v_rx_uart, width=5).grid(row=0, column=1, sticky="w", **pad_)
    ttk.Label(g_rx, text="host port").grid(row=0, column=2, sticky="w", **pad_)
    ttk.Entry(g_rx, textvariable=v_rx_port, width=10).grid(row=0, column=3, sticky="w", **pad_)
    ttk.Label(g_rx, text="baud").grid(row=1, column=0, sticky="w", **pad_)
    ttk.Combobox(g_rx, textvariable=v_rx_baud, values=[9600, 19200, 38400, 57600, 100000, 115200, 230400, 420000], width=8).grid(row=1, column=1, sticky="w", **pad_)
    ttk.Label(g_rx, text="stop / parity").grid(row=1, column=2, sticky="w", **pad_)
    rp = ttk.Frame(g_rx)
    rp.grid(row=1, column=3, sticky="w")
    ttk.Combobox(rp, textvariable=v_rx_stop, values=["None", "One", "Two"], state="readonly", width=5).pack(side="left")
    ttk.Combobox(rp, textvariable=v_rx_par, values=["None", "Even", "Odd"], state="readonly", width=5).pack(side="left", padx=4)
    ttk.Checkbutton(g_rx, text="--fcproxy (a real FC forwards the receiver)", variable=v_fcproxy).grid(row=2, column=0, columnspan=4, sticky="w", **pad_)

    # UART functions — read from the selected instance, never written
    g_uart = ttk.LabelFrame(root, text="UART functions of the selected instance (read from the vehicle; change them in the Configurator's Ports tab)", padding=6)
    g_uart.pack(fill="x", padx=10, pady=2)
    v_uarts = [tk.StringVar(value="") for _ in range(8)]
    for n in range(8):
        col = (n % 4) * 2
        ttk.Label(g_uart, text=f"UART{n + 1}", width=7).grid(row=n // 4, column=col, sticky="w", padx=(4 if col == 0 else 16, 2), pady=2)
        ttk.Label(g_uart, textvariable=v_uarts[n], font=(mono, 10), width=26, anchor="w").grid(row=n // 4, column=col + 1, sticky="w", pady=2)
    ttk.Label(g_uart, text="Only UARTs with a function get a TCP port (base + n). UART2 carries this manager's MSP channel; telemetry functions also need the Telemetry feature.", foreground="#666").grid(row=2, column=0, columnspan=8, sticky="w", padx=4, pady=(4, 0))

    # Binaries
    g_bin = ttk.LabelFrame(root, text="Binaries", padding=6)
    g_bin.pack(fill="x", padx=10, pady=2)
    v_source, v_nightly, v_cfg, v_local = tk.StringVar(), tk.StringVar(), tk.StringVar(), tk.StringVar()
    ttk.Radiobutton(g_bin, text="Nightly", variable=v_source, value="nightly").grid(row=0, column=0, sticky="w", **pad_)
    cb_nightly = ttk.Combobox(g_bin, textvariable=v_nightly, width=44)
    cb_nightly.grid(row=0, column=1, sticky="w", **pad_)
    ttk.Button(g_bin, text="List", width=6, command=lambda: list_tags("nightly")).grid(row=0, column=2, sticky="w", **pad_)
    ttk.Button(g_bin, text="Fetch", width=7, command=lambda: fetch("nightly")).grid(row=0, column=3, sticky="w", **pad_)
    ttk.Radiobutton(g_bin, text="Configurator release", variable=v_source, value="configurator").grid(row=1, column=0, sticky="w", **pad_)
    cb_cfg = ttk.Combobox(g_bin, textvariable=v_cfg, width=44)
    cb_cfg.grid(row=1, column=1, sticky="w", **pad_)
    ttk.Button(g_bin, text="List", width=6, command=lambda: list_tags("configurator")).grid(row=1, column=2, sticky="w", **pad_)
    ttk.Button(g_bin, text="Fetch", width=7, command=lambda: fetch("configurator")).grid(row=1, column=3, sticky="w", **pad_)
    ttk.Radiobutton(g_bin, text="Local binary", variable=v_source, value="local").grid(row=2, column=0, sticky="w", **pad_)
    ttk.Entry(g_bin, textvariable=v_local, width=47).grid(row=2, column=1, sticky="w", **pad_)
    ttk.Button(g_bin, text="…", width=3, command=lambda: pick_file(v_local, "inav_SITL binary")).grid(row=2, column=2, sticky="w", **pad_)
    lbl_bin = ttk.Label(g_bin, text="", justify="left")
    lbl_bin.grid(row=0, column=4, rowspan=3, sticky="nw", padx=16)

    # Actions
    act = ttk.Frame(root, padding=(10, 4))
    act.pack(fill="x")
    ttk.Button(act, text="Start", width=14, command=lambda: on_start()).pack(side="left", padx=4)
    ttk.Button(act, text="Stop all", width=14, command=lambda: on_stop()).pack(side="left", padx=4)
    ttk.Button(act, text="Restart selected", width=16, command=lambda: on_restart_one()).pack(side="left", padx=4)
    lbl_connect = ttk.Label(act, text="", font=(mono, 10))
    lbl_connect.pack(side="left", padx=16)

    # Grid
    cols = [("#", 35), ("Version", 70), ("Link", 90), ("PID", 60), ("State", 95), ("Armed", 75), ("Modes", 220),
            ("GPS", 70), ("Alt m", 55), ("GS m/s", 60), ("Hdg", 45), ("Batt", 100), ("Sim", 80)]
    grid_box = ttk.Frame(root)
    grid_box.pack(fill="both", expand=True, padx=10, pady=(2, 4))
    grid = ttk.Treeview(grid_box, columns=[c for c, _ in cols], show="headings", height=8, selectmode="browse")
    for c, w in cols:
        grid.heading(c, text=c)
        grid.column(c, width=w, minwidth=35, stretch=(c == "Modes"), anchor="w")
    grid.tag_configure("running", foreground="black")
    grid.tag_configure("starting", foreground="#b8860b")
    grid.tag_configure("bad", foreground="#b22222")
    sb = ttk.Scrollbar(grid_box, orient="vertical", command=grid.yview)
    grid.configure(yscrollcommand=sb.set)
    sb.pack(side="right", fill="y")
    grid.pack(side="left", fill="both", expand=True)

    txt_log = tk.Text(root, height=9, font=(mono, 9), state="disabled")
    txt_log.pack(fill="x", padx=10, pady=(0, 4))
    v_status = tk.StringVar(value="idle")
    ttk.Label(root, textvariable=v_status, anchor="w", relief="sunken", padding=(6, 2)).pack(fill="x", side="bottom")

    # -- UI ↔ settings ---------------------------------------------------------------------------
    def ui_to_settings():
        SETTINGS.update(count=int(v_count.get()), basePort=int(v_base.get()), wipe=bool(v_wipe.get()), autoRestart=bool(v_restart.get()),
                        source=v_source.get(), nightlyTag=v_nightly.get().strip(), configuratorVersion=v_cfg.get().strip(), localBinary=v_local.get().strip(),
                        sim=v_sim.get(), simIp=v_sim_ip.get().strip(), simPort=int(v_sim_port.get() or 0), useImu=bool(v_useimu.get()),
                        chanmap=v_chanmap.get(), sdcard=v_sdcard.get(),
                        rxUart=int(v_rx_uart.get() or 0), rxPort=v_rx_port.get(), rxBaud=int(v_rx_baud.get() or 115200),
                        rxStopbits=v_rx_stop.get(), rxParity=v_rx_par.get(), fcProxy=bool(v_fcproxy.get()))
        save_settings(SETTINGS)

    def settings_to_ui():
        v_count.set(int(SETTINGS["count"]))
        v_base.set(int(SETTINGS["basePort"]))
        v_wipe.set(bool(SETTINGS["wipe"]))
        v_restart.set(bool(SETTINGS["autoRestart"]))
        v_source.set(SETTINGS["source"] if SETTINGS["source"] in ("nightly", "configurator", "local") else "nightly")
        v_nightly.set(SETTINGS["nightlyTag"])
        v_cfg.set(SETTINGS["configuratorVersion"])
        v_local.set(SETTINGS["localBinary"])
        v_sim.set(SETTINGS["sim"] if SETTINGS["sim"] in ("none", "xp", "rf") else "none")
        v_sim_ip.set(SETTINGS["simIp"])
        v_sim_port.set(int(SETTINGS["simPort"] or 0))
        v_useimu.set(bool(SETTINGS["useImu"]))
        v_chanmap.set(SETTINGS["chanmap"])
        v_sdcard.set(SETTINGS["sdcard"])
        v_rx_uart.set(int(SETTINGS["rxUart"] or 0))
        v_rx_port.set(SETTINGS["rxPort"])
        v_rx_baud.set(int(SETTINGS["rxBaud"] or 115200))
        v_rx_stop.set(SETTINGS["rxStopbits"] or "One")
        v_rx_par.set(SETTINGS["rxParity"] or "None")
        v_fcproxy.set(bool(SETTINGS["fcProxy"]))
        # what is already on disk
        cb_nightly["values"] = sorted((p.name[len("nightly-"):] for p in BIN_ROOT.glob("nightly-*")), reverse=True)
        cb_cfg["values"] = sorted((p.name[len("configurator-"):] for p in BIN_ROOT.glob("configurator-*")), reverse=True)

    def refresh_bin_label():
        exe = binary_path()
        if exe.exists():
            lbl_bin["text"] = f"Using: {exe}\n{binary_banner(exe)}"
        else:
            lbl_bin["text"] = f"Not fetched yet: {exe}"

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
        sel = selected_instance()
        cfg = {ident: mask for ident, mask, *_ in (sel.serial_cfg or [])} if sel else {}
        for n, v in enumerate(v_uarts):
            text = ""
            if sel and sel.serial_cfg is not None:
                mask = cfg.get(n, 0)
                text = f"{function_names(mask)}" + (f"  → tcp {sel.base_port + n}" if mask else "")
            if v.get() != text:
                v.set(text)
        if mgr.instances and mgr.instances[0].serial_cfg is not None and lbl_connect["text"] != mgr.connect_hint():
            lbl_connect["text"] = mgr.connect_hint()

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
    def pick_file(var, title):
        f = filedialog.askopenfilename(title=title)
        if f:
            var.set(f)

    bg = {"msg": None, "done": True, "error": None, "result": None}

    def run_bg(work, on_done):
        bg.update(msg=None, done=False, error=None, result=None)

        def wrapper():
            try:
                bg["result"] = work()
            except Exception as e:  # noqa: BLE001 — reported in the status bar
                bg["error"] = str(e)
            bg["done"] = True

        threading.Thread(target=wrapper, daemon=True).start()

        def poll():
            if bg["msg"]:
                v_status.set(bg["msg"])
            if not bg["done"]:
                root.after(300, poll)
                return
            if bg["error"]:
                messagebox.showerror("GitHub", bg["error"])
                v_status.set("failed")
            else:
                on_done(bg["result"])

        poll()

    def list_tags(kind):
        def done(tags):
            if kind == "nightly":
                cb_nightly["values"] = tags
                if tags and not v_nightly.get():
                    v_nightly.set(tags[0])
            else:
                cb_cfg["values"] = tags
                if tags and not v_cfg.get():
                    v_cfg.set(tags[0])
            v_status.set(f"{len(tags)} {kind} releases listed")

        v_status.set("asking GitHub…")
        run_bg(list_nightlies if kind == "nightly" else list_configurator_versions, done)

    def fetch(kind):
        ui_to_settings()
        tag = SETTINGS["nightlyTag"] if kind == "nightly" else SETTINGS["configuratorVersion"]
        if not tag:
            messagebox.showinfo("Fetch", "Pick a release first (List, then choose one)")
            return

        def work():
            return (fetch_nightly if kind == "nightly" else fetch_configurator)(tag, lambda m: bg.update(msg=m))

        def done(_):
            v_source.set(kind)
            ui_to_settings()
            settings_to_ui()
            refresh_bin_label()
            v_status.set(f"{kind} {tag} ready")

        run_bg(work, done)

    def on_start():
        try:
            ui_to_settings()
            v_status.set("starting…")
            root.update_idletasks()
            mgr.start_all()
            lbl_connect["text"] = mgr.connect_hint()
            refresh_grid()
            v_status.set(f"{SETTINGS['count']} INAV SITL instance(s) started")
        except Exception as e:  # noqa: BLE001
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
        except Exception as e:  # noqa: BLE001
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

    for var in (v_source, v_nightly, v_cfg, v_local):
        var.trace_add("write", lambda *_: (ui_to_settings(), refresh_bin_label()) if not loading["on"] else None)
    grid.bind("<<TreeviewSelect>>", lambda *_: refresh_log())
    root.protocol("WM_DELETE_WINDOW", on_close)

    loading = {"on": True}
    settings_to_ui()
    loading["on"] = False
    refresh_bin_label()
    root.after(250, tick)
    root.after(1500, log_tick)
    if auto_start:
        root.after(100, on_start)
    root.mainloop()


def main() -> None:
    ap = argparse.ArgumentParser(description="INAV SITL manager for Kite development")
    ap.add_argument("--headless", action="store_true", help="console mode: start with the saved settings, print a status table, Ctrl+C stops")
    ap.add_argument("--count", type=int, default=0)
    ap.add_argument("--wipe", action="store_true")
    ap.add_argument("--nightly", metavar="TAG", help="use (and fetch if needed) this nightly, or 'latest'")
    ap.add_argument("--configurator", metavar="VERSION", help="use (and fetch if needed) the SITL of this Configurator release, or 'latest'")
    ap.add_argument("--seconds", type=int, default=0, help="headless: stop after this many seconds (0 = until Ctrl+C)")
    ap.add_argument("--auto-start", action="store_true", help="window mode: press Start right after opening")
    a = ap.parse_args()
    if a.count > 0:
        SETTINGS["count"] = min(a.count, 16)
    if a.wipe:
        SETTINGS["wipe"] = True
    if a.nightly:
        tag = list_nightlies(1)[0] if a.nightly == "latest" else a.nightly
        fetch_nightly(tag, print)
        SETTINGS.update(source="nightly", nightlyTag=tag)
    if a.configurator:
        ver = list_configurator_versions(1)[0] if a.configurator == "latest" else a.configurator
        fetch_configurator(ver, print)
        SETTINGS.update(source="configurator", configuratorVersion=ver)
    mgr = Manager()
    if a.headless:
        run_headless(mgr, a.seconds)
    else:
        run_ui(mgr, a.auto_start)


if __name__ == "__main__":
    main()
