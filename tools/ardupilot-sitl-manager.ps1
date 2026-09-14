# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marc Hoffmann (b14ckyy)
#
# ArduPilot SITL manager for Kite development — a small WinForms desk for the SITL binaries Mission
# Planner downloads, so Mission Planner itself is not needed to fly a simulated vehicle (or a swarm).
#
#   .\tools\ardupilot-sitl-manager.ps1
#
# What it does
# - Single vehicle or a swarm (Count), any of the common ArduPilot frames (plane, quadplane, copter
#   frames, heli, rover, boat, sub) with the right default parameter files.
# - Start position from named presets, swarm formation (line / grid, spacing in metres), speedup,
#   sysid per vehicle, extra parameters applied over MAVLink after boot (PARAM_SET, so they stick
#   without wiping the EEPROM).
# - Kite link layouts: one TCP link per vehicle (SERIAL0 = tcp 5760+10·instance), one shared UDP port
#   every vehicle pushes to (fan-in), or Mission Planner's swarm chain (ArduPilot routing carries every
#   sysid on vehicle 1's TCP link).
# - Live status per vehicle (mode, armed, alt, speed, heading, GPS, battery, last STATUSTEXT) on the
#   manager's own UDP monitor channel (each instance: --serial5 udpclient → the manager). UDP, because
#   the Cygwin binaries DIE when a TCP client disconnects (see below); the manager announces itself as
#   sysid 254, not 255, so its heartbeats do not mask a lost Kite link for the GCS failsafe.
# - Watchdog: an instance that exits (typically: Kite disconnected from its TCP port) is restarted.
# - Binaries: uses Mission Planner's sitl folder when present, or downloads a channel (latest / Stable
#   / Beta / …) from firmware.ardupilot.org; parameter files come from the ArduPilot GitHub tree.
#
# Why not Mission Planner's own Swarm button: its Cygwin binaries reject the `-P NAME=VALUE` options
# MP passes (usage + exit → "connection refused"), the SYSID_THISMAV it writes per instance no longer
# exists on current builds (renamed MAV_SYSID — `--sysid` works on every version), and an instance dies
# the moment any TCP client disconnects from it. Each of those is handled here.
#
# Parameter names change between ArduPilot versions (master 2026: SYSID_THISMAV → MAV_SYSID,
# SYSID_MYGCS → MAV_GCS_SYSID, ARMING_CHECK → ARMING_SKIPCHK). An extra parameter the vehicle does not
# know is never acknowledged; the status column then says "params NOT confirmed: …".
#
# Files: settings + presets in %LOCALAPPDATA%\kite-sitl\settings.json, instance state (eeprom.bin,
# logs, stdout) under %LOCALAPPDATA%\kite-sitl\instances\<n>, downloads under ...\bin\<channel>,
# parameter files under ...\params. Windows PowerShell 5.1, no modules.

[CmdletBinding()]
param(
    # Console mode without the window: start with the saved settings (overridable below), print a status
    # table every 2 s, stop everything on Ctrl+C / after -Seconds. Handy for scripts and smoke tests.
    [switch]$Headless,
    [int]$Count = 0,
    [string]$Frame = '',
    [ValidateSet('', 'tcp', 'udp', 'chain')]
    [string]$Layout = '',
    [switch]$Wipe,
    [int]$Seconds = 0,
    # Window mode: press Start right after opening (the saved settings).
    [switch]$AutoStart
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

# A Windows job object with kill-on-close: every SITL process is assigned to it, so closing the manager
# (or the console hosting it) takes the vehicles down too — no orphaned ArduPlane.exe on port 5760.
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class KiteJob {
    [StructLayout(LayoutKind.Sequential)] struct JOBOBJECT_BASIC_LIMIT_INFORMATION { public long PerProcessUserTimeLimit; public long PerJobUserTimeLimit; public uint LimitFlags; public UIntPtr MinimumWorkingSetSize; public UIntPtr MaximumWorkingSetSize; public uint ActiveProcessLimit; public UIntPtr Affinity; public uint PriorityClass; public uint SchedulingClass; }
    [StructLayout(LayoutKind.Sequential)] struct IO_COUNTERS { public ulong ReadOperationCount, WriteOperationCount, OtherOperationCount, ReadTransferCount, WriteTransferCount, OtherTransferCount; }
    [StructLayout(LayoutKind.Sequential)] struct JOBOBJECT_EXTENDED_LIMIT_INFORMATION { public JOBOBJECT_BASIC_LIMIT_INFORMATION BasicLimitInformation; public IO_COUNTERS IoInfo; public UIntPtr ProcessMemoryLimit, JobMemoryLimit, PeakProcessMemoryUsed, PeakJobMemoryUsed; }
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] static extern IntPtr CreateJobObject(IntPtr a, string name);
    [DllImport("kernel32.dll")] static extern bool SetInformationJobObject(IntPtr job, int infoClass, IntPtr info, uint size);
    [DllImport("kernel32.dll")] static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);
    static IntPtr job = IntPtr.Zero;
    public static void Assign(IntPtr processHandle) {
        if (job == IntPtr.Zero) {
            job = CreateJobObject(IntPtr.Zero, null);
            var info = new JOBOBJECT_EXTENDED_LIMIT_INFORMATION();
            info.BasicLimitInformation.LimitFlags = 0x2000; // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            int size = Marshal.SizeOf(typeof(JOBOBJECT_EXTENDED_LIMIT_INFORMATION));
            IntPtr p = Marshal.AllocHGlobal(size);
            Marshal.StructureToPtr(info, p, false);
            SetInformationJobObject(job, 9, p, (uint)size); // JobObjectExtendedLimitInformation
            Marshal.FreeHGlobal(p);
        }
        AssignProcessToJobObject(job, processHandle);
    }
}
'@

# ── Paths ────────────────────────────────────────────────────────────────────────────────────────
$Root = Join-Path $env:LOCALAPPDATA 'kite-sitl'
$SettingsPath = Join-Path $Root 'settings.json'
$InstancesDir = Join-Path $Root 'instances'
$BinRoot = Join-Path $Root 'bin'
$ParamsDir = Join-Path $Root 'params'
$MpSitlDir = Join-Path ([Environment]::GetFolderPath('MyDocuments')) 'Mission Planner\sitl'
foreach ($d in @($Root, $InstancesDir, $BinRoot, $ParamsDir)) { New-Item -ItemType Directory -Force $d | Out-Null }

# ── Frames (vehicle → binary, frame → default parameter files; from ArduPilot's vehicleinfo.py) ──
$Frames = @(
    @{ v = 'Plane';  f = 'plane';                 p = @('models/plane.parm') }
    @{ v = 'Plane';  f = 'plane-elevon';          p = @('models/plane.parm', 'default_params/plane-elevons.parm') }
    @{ v = 'Plane';  f = 'plane-vtail';           p = @('models/plane.parm', 'default_params/plane-vtail.parm') }
    @{ v = 'Plane';  f = 'plane-dspoilers';       p = @('models/plane.parm', 'default_params/plane-dspoilers.parm') }
    @{ v = 'Plane';  f = 'plane-jet';             p = @('models/plane.parm', 'default_params/plane-jet.parm') }
    @{ v = 'Plane';  f = 'plane-soaring';         p = @('models/plane.parm', 'default_params/plane-soaring.parm') }
    @{ v = 'Plane';  f = 'glider';                p = @('default_params/glider.parm') }
    @{ v = 'Plane';  f = 'quadplane';             p = @('default_params/quadplane.parm') }
    @{ v = 'Plane';  f = 'quadplane-tilt';        p = @('default_params/quadplane.parm', 'default_params/quadplane-tilt.parm') }
    @{ v = 'Plane';  f = 'quadplane-tri';         p = @('default_params/quadplane.parm', 'default_params/quadplane-tri.parm') }
    @{ v = 'Plane';  f = 'quadplane-tilttri';     p = @('default_params/quadplane.parm', 'default_params/quadplane-tilttri.parm') }
    @{ v = 'Plane';  f = 'plane-tailsitter';      p = @('default_params/plane-tailsitter.parm') }
    @{ v = 'Plane';  f = 'quadplane-copter_tailsitter'; p = @('default_params/quadplane.parm', 'default_params/quadplane-copter_tailsitter.parm') }
    @{ v = 'Copter'; f = 'quad';                  p = @('default_params/copter.parm') }
    @{ v = 'Copter'; f = 'X';                     p = @('default_params/copter.parm', 'default_params/copter-X.parm') }
    @{ v = 'Copter'; f = 'hexa';                  p = @('default_params/copter.parm', 'default_params/copter-hexa.parm') }
    @{ v = 'Copter'; f = 'octa';                  p = @('default_params/copter.parm', 'default_params/copter-octa.parm') }
    @{ v = 'Copter'; f = 'octa-quad';             p = @('default_params/copter.parm', 'default_params/copter-octaquad.parm') }
    @{ v = 'Copter'; f = 'tri';                   p = @('default_params/copter.parm', 'default_params/copter-tri.parm') }
    @{ v = 'Copter'; f = 'y6';                    p = @('default_params/copter.parm', 'default_params/copter-y6.parm') }
    @{ v = 'Copter'; f = 'singlecopter';          p = @('default_params/copter-single.parm') }
    @{ v = 'Copter'; f = 'coaxcopter';            p = @('default_params/copter-single.parm', 'default_params/copter-coax.parm') }
    @{ v = 'Heli';   f = 'heli';                  p = @('default_params/copter-heli.parm') }
    @{ v = 'Heli';   f = 'heli-dual';             p = @('default_params/copter-heli.parm', 'default_params/copter-heli-dual.parm') }
    @{ v = 'Rover';  f = 'rover';                 p = @('default_params/rover.parm') }
    @{ v = 'Rover';  f = 'rover-skid';            p = @('default_params/rover.parm', 'default_params/rover-skid.parm') }
    @{ v = 'Rover';  f = 'balancebot';            p = @('default_params/rover.parm', 'default_params/rover-skid.parm', 'default_params/balancebot.parm') }
    @{ v = 'Rover';  f = 'motorboat';             p = @('default_params/rover.parm', 'default_params/motorboat.parm') }
    @{ v = 'Rover';  f = 'sailboat';              p = @('default_params/rover.parm', 'default_params/sailboat.parm') }
    @{ v = 'Sub';    f = 'vectored';              p = @('default_params/sub.parm') }
    @{ v = 'Sub';    f = 'vectored_6dof';         p = @('default_params/sub-6dof.parm') }
)
$Binaries = @{ Plane = 'ArduPlane'; Copter = 'ArduCopter'; Heli = 'ArduHeli'; Rover = 'ArduRover'; Sub = 'ArduSub' }
$Channels = [ordered]@{
    'latest (daily build)' = ''
    'Stable'               = 'Stable'
    'Beta'                 = 'Beta'
    'PlaneStable'          = 'PlaneStable'
    'CopterStable'         = 'CopterStable'
    'RoverStable'          = 'RoverStable'
}
$FirmwareUrl = 'https://firmware.ardupilot.org/Tools/MissionPlanner/sitl/'
$CygwinDlls = @('cygwin1.dll', 'cygstdc++-6.dll', 'cyggcc_s-seh-1.dll', 'cyggcc_s-1.dll', 'cygatomic-1.dll', 'cyggomp-1.dll', 'cygquadmath-0.dll', 'cygssp-0.dll', 'cygiconv-2.dll', 'cygintl-8.dll')
$ParamUrl = 'https://raw.githubusercontent.com/ArduPilot/ardupilot/master/Tools/autotest/'

# Flight-mode names by vehicle class (HEARTBEAT.custom_mode).
$ModeNames = @{
    plane  = @{ 0='MANUAL'; 1='CIRCLE'; 2='STABILIZE'; 3='TRAINING'; 4='ACRO'; 5='FBWA'; 6='FBWB'; 7='CRUISE'; 8='AUTOTUNE'; 10='AUTO'; 11='RTL'; 12='LOITER'; 13='TAKEOFF'; 14='AVOID_ADSB'; 15='GUIDED'; 16='INITIALISING'; 17='QSTABILIZE'; 18='QHOVER'; 19='QLOITER'; 20='QLAND'; 21='QRTL'; 22='QAUTOTUNE'; 23='QACRO'; 24='THERMAL'; 25='LOITER_ALT_QLAND'; 26='AUTOLAND' }
    copter = @{ 0='STABILIZE'; 1='ACRO'; 2='ALT_HOLD'; 3='AUTO'; 4='GUIDED'; 5='LOITER'; 6='RTL'; 7='CIRCLE'; 9='LAND'; 11='DRIFT'; 13='SPORT'; 14='FLIP'; 15='AUTOTUNE'; 16='POSHOLD'; 17='BRAKE'; 18='THROW'; 19='AVOID_ADSB'; 20='GUIDED_NOGPS'; 21='SMART_RTL'; 22='FLOWHOLD'; 23='FOLLOW'; 24='ZIGZAG'; 25='SYSTEMID'; 26='AUTOROTATE'; 27='AUTO_RTL'; 28='TURTLE' }
    rover  = @{ 0='MANUAL'; 1='ACRO'; 3='STEERING'; 4='HOLD'; 5='LOITER'; 6='FOLLOW'; 7='SIMPLE'; 8='DOCK'; 9='CIRCLE'; 10='AUTO'; 11='RTL'; 12='SMART_RTL'; 15='GUIDED'; 16='INITIALISING' }
    sub    = @{ 0='STABILIZE'; 1='ACRO'; 2='ALT_HOLD'; 3='AUTO'; 4='GUIDED'; 7='CIRCLE'; 9='SURFACE'; 16='POSHOLD'; 19='MANUAL'; 20='MOTORDETECT' }
}
function Mode-Class([int]$mavType) {
    switch ($mavType) {
        { $_ -in 1, 19, 20, 21, 22, 23, 24, 25 } { return 'plane' }
        { $_ -in 10, 11 } { return 'rover' }
        12 { return 'sub' }
        default { return 'copter' }
    }
}

# ── Settings ─────────────────────────────────────────────────────────────────────────────────────
$Defaults = [ordered]@{
    vehicle = 'Plane'; frame = 'plane'; count = 1; sysidBase = 1; speedup = 1.0; wipe = $false
    preset = 'CMAC (ArduPilot default)'; formation = 'line'; spacingM = 12
    layout = 'tcp'; udpPort = 14550; monitorPort = 14650; autoRestart = $true
    extraParams = ''
    binSource = 'auto'; channel = 'latest (daily build)'
    presets = @(
        @{ name = 'CMAC (ArduPilot default)'; lat = -35.363261; lon = 149.165230; alt = 584; hdg = 353 }
    )
}
function Load-Settings {
    $s = [ordered]@{}
    foreach ($k in $Defaults.Keys) { $s[$k] = $Defaults[$k] }
    if (Test-Path $SettingsPath) {
        try {
            $j = Get-Content $SettingsPath -Raw | ConvertFrom-Json
            foreach ($p in $j.PSObject.Properties) {
                if ($p.Name -eq 'presets') {
                    $s.presets = @($p.Value | ForEach-Object { @{ name = $_.name; lat = [double]$_.lat; lon = [double]$_.lon; alt = [double]$_.alt; hdg = [double]$_.hdg } })
                } elseif ($null -ne $p.Value) { $s[$p.Name] = $p.Value }
            }
        } catch { }
    }
    return $s
}
function Save-Settings($s) {
    $s | ConvertTo-Json -Depth 5 | Set-Content -Path $SettingsPath -Encoding utf8
}
$Settings = Load-Settings

# ── Binaries + parameter files ───────────────────────────────────────────────────────────────────
function Bin-Dir {
    # Where the vehicle .exe and cygwin DLLs come from: the chosen downloaded channel, else MP's folder.
    if ($Settings.binSource -eq 'download') { return (Join-Path $BinRoot ($Settings.channel -replace '[^A-Za-z]', '_')) }
    if ($Settings.binSource -eq 'auto') {
        $dl = Join-Path $BinRoot ($Settings.channel -replace '[^A-Za-z]', '_')
        if (Test-Path (Join-Path $dl 'cygwin1.dll')) { return $dl }
        return $MpSitlDir
    }
    return $MpSitlDir
}
function Download-File([string]$url, [string]$dest) {
    New-Item -ItemType Directory -Force (Split-Path $dest) | Out-Null
    $tmp = "$dest.part"
    Invoke-WebRequest -Uri $url -OutFile $tmp -UseBasicParsing
    Move-Item -Force $tmp $dest
}
function Download-Channel([string]$channelName, [string[]]$vehicles, [scriptblock]$progress) {
    $sub = $Channels[$channelName]
    $base = $FirmwareUrl; if ($sub) { $base = "$FirmwareUrl$sub/" }
    $dir = Join-Path $BinRoot ($channelName -replace '[^A-Za-z]', '_')
    $files = @()
    foreach ($v in $vehicles) { $files += @{ url = "$base$($Binaries[$v]).elf"; dest = Join-Path $dir "$($Binaries[$v]).exe" } }
    foreach ($d in $CygwinDlls) { $files += @{ url = "$base$d"; dest = Join-Path $dir $d } }
    $i = 0
    foreach ($f in $files) {
        $i++
        & $progress "Downloading $i/$($files.Count): $(Split-Path $f.url -Leaf)"
        Download-File $f.url $f.dest
    }
    & $progress "Channel '$channelName' ready in $dir"
    return $dir
}
function Resolve-ParamFile([string]$rel) {
    # MP's folder first (it has the common ones), then our cache, then the ArduPilot tree.
    foreach ($base in @($MpSitlDir, $ParamsDir)) {
        $p = Join-Path $base ($rel -replace '/', '\')
        if (Test-Path $p) { return $p }
    }
    $dest = Join-Path $ParamsDir ($rel -replace '/', '\')
    Download-File "$ParamUrl$rel" $dest
    return $dest
}

# ── MAVLink (v2, the little we need) ──────────────────────────────────────────────────────────────
$script:MavSeq = 0
$MgrSysid = 254; $MgrCompid = 190
function Mav-Crc([byte[]]$bytes, [int]$extra) {
    $c = 0xffff
    foreach ($b in ($bytes + [byte]$extra)) {
        $t = ($b -bxor ($c -band 0xff)) -band 0xff
        $t = ($t -bxor (($t -shl 4) -band 0xff)) -band 0xff
        $c = (($c -shr 8) -bxor ($t -shl 8) -bxor ($t -shl 3) -bxor ($t -shr 4)) -band 0xffff
    }
    return $c
}
function Mav-Frame([int]$msgid, [byte[]]$payload, [int]$crcExtra) {
    $hdr = [byte[]]@(0xFD, $payload.Length, 0, 0, ($script:MavSeq -band 0xff), $MgrSysid, $MgrCompid, ($msgid -band 0xff), (($msgid -shr 8) -band 0xff), (($msgid -shr 16) -band 0xff))
    $script:MavSeq++
    $body = $hdr[1..9] + $payload
    $crc = Mav-Crc $body $crcExtra
    return $hdr + $payload + [byte[]]@(($crc -band 0xff), (($crc -shr 8) -band 0xff))
}
function Mav-Heartbeat {
    # custom_mode u32, type u8 (6 = GCS), autopilot u8 (8 = invalid), base_mode, system_status, mavlink_version
    Mav-Frame 0 ([byte[]]@(0, 0, 0, 0, 6, 8, 0, 0, 3)) 50
}
function Mav-ParamSet([int]$target, [string]$name, [double]$value) {
    $pl = New-Object byte[] 23
    [BitConverter]::GetBytes([single]$value).CopyTo($pl, 0)
    $pl[4] = $target; $pl[5] = 1
    $id = [Text.Encoding]::ASCII.GetBytes($name.PadRight(16, [char]0).Substring(0, 16)); $id.CopyTo($pl, 6)
    $pl[22] = 9  # MAV_PARAM_TYPE_REAL32 — ArduPilot converts to the parameter's own type
    Mav-Frame 23 $pl 168
}
function Mav-MessageInterval([int]$target, [int]$msgid, [double]$hz) {
    # COMMAND_LONG: param1..7 f32, command u16 (511 SET_MESSAGE_INTERVAL), target_system, target_component, confirmation
    $pl = New-Object byte[] 33
    [BitConverter]::GetBytes([single]$msgid).CopyTo($pl, 0)
    [BitConverter]::GetBytes([single](1e6 / $hz)).CopyTo($pl, 4)
    [BitConverter]::GetBytes([uint16]511).CopyTo($pl, 28)
    $pl[30] = $target; $pl[31] = 1; $pl[32] = 0
    Mav-Frame 76 $pl 152
}
function Pad-Payload([byte[]]$p, [int]$len) {
    # MAVLink 2 trims trailing zero bytes; restore the declared length before reading fields.
    if ($p.Length -ge $len) { return $p }
    $out = New-Object byte[] $len; $p.CopyTo($out, 0); return $out
}

# ── Instances ────────────────────────────────────────────────────────────────────────────────────
# One entry per vehicle: process, config and the live state decoded from the monitor channel.
$script:Instances = New-Object System.Collections.ArrayList
$script:Udp = $null   # the monitor socket (UdpClient) while a swarm is up
$script:EndpointSysid = @{}  # "ip:port" → sysid bound on the first packet (chain forwarding dedupe)

function New-InstanceRecord([int]$k) {
    [pscustomobject]@{
        Index = $k; Instance = $k; Sysid = $Settings.sysidBase + $k
        Frame = $Settings.frame; Vehicle = $Settings.vehicle
        Dir = Join-Path $InstancesDir "$($k + 1)"
        Args = @(); Process = $null; Restarts = 0; StartedAt = $null; State = 'stopped'
        # live state
        Endpoint = $null; MavType = 0; Mode = ''; Armed = $false; LastHb = $null
        Lat = 0.0; Lon = 0.0; RelAlt = 0.0; Hdg = 0.0; GroundSpeed = 0.0; Airspeed = 0.0; Throttle = 0
        FixType = 0; Sats = 0; Voltage = 0.0; Current = 0.0; BattPct = -1; StatusText = ''
        ParamsApplied = $false; IntervalsRequested = $false
        # extra params still unconfirmed (name → value); PARAM_VALUE echoes clear them, unanswered ones are resent
        PendingParams = @{}; ParamTries = 0; LastParamSend = [datetime]::MinValue
    }
}
function Instance-Home([int]$k) {
    $pr = $Settings.presets | Where-Object { $_.name -eq $Settings.preset } | Select-Object -First 1
    if (-not $pr) { $pr = $Settings.presets[0] }
    $dLat = $Settings.spacingM / 111320.0
    $dLon = $Settings.spacingM / (111320.0 * [math]::Cos($pr.lat * [math]::PI / 180))
    if ($Settings.formation -eq 'grid') {
        $cols = [math]::Ceiling([math]::Sqrt([math]::Max(1, $Settings.count)))
        $row = [math]::Floor($k / $cols); $col = $k % $cols
        $lat = $pr.lat - $row * $dLat; $lon = $pr.lon + $col * $dLon
    } else {
        $lat = $pr.lat; $lon = $pr.lon + $k * $dLon
    }
    $inv = [Globalization.CultureInfo]::InvariantCulture
    return ('{0},{1},{2},{3}' -f $lat.ToString('F7', $inv), $lon.ToString('F7', $inv), $pr.alt.ToString('F1', $inv), $pr.hdg.ToString('F0', $inv))
}
function Build-Args($inst) {
    $fr = $Frames | Where-Object { $_.f -eq $Settings.frame } | Select-Object -First 1
    $paramFiles = @($fr.p | ForEach-Object { Resolve-ParamFile $_ }) + @('identity.parm')
    $a = @(
        "-M$($Settings.frame)", "-O$(Instance-Home $inst.Index)", "-s$($Settings.speedup)",
        '--instance', "$($inst.Instance)", '--sysid', "$($inst.Sysid)",
        '--serial0', 'tcp:0',
        '--serial5', "udpclient:127.0.0.1:$($Settings.monitorPort)",
        '--defaults', ('"' + ($paramFiles -join ',') + '"')
    )
    if ($Settings.wipe) { $a += '-w' }
    if ($Settings.layout -eq 'chain' -and $inst.Index -lt $Settings.count - 1) {
        # Vehicle k's SERIAL2 dials vehicle k+1's SERIAL1 server (5762 + 10·instance) — MP's swarm chain.
        $a += @('--serial2', "tcpclient:127.0.0.1:$(5762 + 10 * ($inst.Instance + 1))")
    } elseif ($Settings.layout -eq 'udp') {
        $a += @('--serial6', "udpclient:127.0.0.1:$($Settings.udpPort)")
    }
    return $a
}
function Write-Identity($inst) {
    # Applied as parameter DEFAULTS at boot (a value already stored in the EEPROM wins — that is why the
    # extra params are also pushed over MAVLink after boot). SERIAL5/6 default to "no protocol".
    $lines = @('SERIAL0_PROTOCOL=2', 'SERIAL1_PROTOCOL=2', 'SERIAL2_PROTOCOL=2', 'SERIAL5_PROTOCOL=2', 'SERIAL6_PROTOCOL=2',
               'SIM_TERRAIN=0', 'TERRAIN_ENABLE=0', 'SIM_DRIFT_SPEED=0', 'SIM_DRIFT_TIME=0')
    $lines += Extra-Params | ForEach-Object { "$($_.name)=$($_.value)" }
    $lines | Set-Content -Path (Join-Path $inst.Dir 'identity.parm') -Encoding ascii
}
function Extra-Params {
    $out = @()
    foreach ($line in ($Settings.extraParams -split "`n")) {
        $l = $line.Trim(); if (-not $l -or $l.StartsWith('#')) { continue }
        $m = [regex]::Match($l, '^([A-Za-z0-9_]{1,16})\s*[=,]\s*(-?[0-9.]+)$')
        if ($m.Success) { $out += @{ name = $m.Groups[1].Value.ToUpper(); value = [double]$m.Groups[2].Value } }
    }
    return $out
}
function Start-Instance($inst) {
    New-Item -ItemType Directory -Force $inst.Dir | Out-Null
    if ($Settings.wipe -and $inst.Restarts -eq 0) { Remove-Item -Force (Join-Path $inst.Dir 'eeprom.bin') -ErrorAction SilentlyContinue }
    Write-Identity $inst
    $exe = Join-Path (Bin-Dir) "$($Binaries[$Settings.vehicle]).exe"
    if (-not (Test-Path $exe)) { throw "$exe not found — download the channel first, or start that vehicle once in Mission Planner" }
    $inst.Args = Build-Args $inst
    $inst.Process = Start-Process -FilePath $exe -ArgumentList $inst.Args -WorkingDirectory $inst.Dir -PassThru -WindowStyle Minimized `
        -RedirectStandardOutput (Join-Path $inst.Dir 'stdout.txt') -RedirectStandardError (Join-Path $inst.Dir 'stderr.txt')
    try { [KiteJob]::Assign($inst.Process.Handle) } catch { }
    $inst.StartedAt = Get-Date; $inst.State = 'starting'; $inst.LastHb = $null
    $inst.ParamsApplied = $false; $inst.IntervalsRequested = $false; $inst.Endpoint = $null
    $inst.PendingParams = @{}; $inst.ParamTries = 0; $inst.LastParamSend = [datetime]::MinValue
}
function Stop-Instance($inst) {
    if ($inst.Process -and -not $inst.Process.HasExited) { Stop-Process -Id $inst.Process.Id -Force -ErrorAction SilentlyContinue }
    $inst.Process = $null; $inst.State = 'stopped'; $inst.Mode = ''; $inst.Armed = $false
}
function Ports-Busy {
    # Anything already listening on a SERIAL0 port we are about to use (a Mission Planner SITL, an old swarm).
    $mine = @(); foreach ($i in $script:Instances) { if ($i.Process -and -not $i.Process.HasExited) { $mine += $i.Process.Id } }
    $lo = 5760; $hi = 5760 + 10 * $Settings.count
    $hits = Get-NetTCPConnection -State Listen -ErrorAction SilentlyContinue | Where-Object { $_.LocalPort -ge $lo -and $_.LocalPort -lt $hi -and (($_.LocalPort - 5760) % 10) -eq 0 -and $mine -notcontains $_.OwningProcess }
    return @($hits | ForEach-Object { "$($_.LocalPort) (PID $($_.OwningProcess) $((Get-Process -Id $_.OwningProcess -ErrorAction SilentlyContinue).ProcessName))" })
}
function Start-All {
    Stop-All
    $busy = Ports-Busy
    if ($busy) { throw "SITL ports already in use: $($busy -join ', '). Stop Mission Planner's simulation first." }
    $script:Udp = New-Object System.Net.Sockets.UdpClient($Settings.monitorPort)
    $script:Udp.Client.Blocking = $false
    $script:EndpointSysid = @{}
    $script:Instances.Clear()
    $order = 0..($Settings.count - 1)
    if ($Settings.layout -eq 'chain') { [array]::Reverse($order) }  # the chain dials "up": start the last vehicle first
    foreach ($k in $order) {
        $inst = New-InstanceRecord $k
        Start-Instance $inst
        [void]$script:Instances.Add($inst)
        Start-Sleep -Milliseconds 400
    }
    $sorted = @($script:Instances | Sort-Object Index); $script:Instances.Clear(); foreach ($i in $sorted) { [void]$script:Instances.Add($i) }
}
function Stop-All {
    foreach ($i in $script:Instances) { Stop-Instance $i }
    if ($script:Udp) { $script:Udp.Close(); $script:Udp = $null }
}

# ── Monitor: decode the vehicles' pushes, request rates, apply params, answer with a heartbeat ────
$script:LastMgrHb = [datetime]::MinValue
function Send-To($inst, [byte[]]$frame) {
    if ($script:Udp -and $inst.Endpoint) { [void]$script:Udp.Send($frame, $frame.Length, $inst.Endpoint) }
}
function Poll-Monitor {
    if (-not $script:Udp) { return }
    $remote = New-Object System.Net.IPEndPoint([System.Net.IPAddress]::Any, 0)
    $n = 0
    while ($script:Udp.Available -gt 0 -and $n -lt 1000) {   # ~32 vehicles × 30 msg/s × 250 ms tick, with headroom
        $n++
        try { $d = $script:Udp.Receive([ref]$remote) } catch { break }
        $key = $remote.ToString()
        $i = 0
        while ($i + 12 -le $d.Length) {
            if ($d[$i] -ne 0xFD) { $i++; continue }
            $len = $d[$i + 1]; $total = 12 + $len; if ($d[$i + 2] -band 1) { $total += 13 }
            if ($i + $total -gt $d.Length) { break }
            $sysid = $d[$i + 5]
            $msgid = $d[$i + 7] -bor ($d[$i + 8] -shl 8) -bor ($d[$i + 9] -shl 16)
            # Bind the endpoint to the first sysid heard on it: in a chain, ArduPilot forwards the OTHER
            # vehicles' broadcasts here once it knows us — those are duplicates of their own pushes.
            if (-not $script:EndpointSysid.ContainsKey($key)) { $script:EndpointSysid[$key] = $sysid }
            if ($script:EndpointSysid[$key] -eq $sysid) {
                $inst = $script:Instances | Where-Object { $_.Sysid -eq $sysid } | Select-Object -First 1
                if ($inst) {
                    if (-not $inst.Endpoint) { $inst.Endpoint = New-Object System.Net.IPEndPoint($remote.Address, $remote.Port) }
                    $pl = $d[($i + 10)..($i + 10 + $len - 1)]
                    if ($len -eq 0) { $pl = [byte[]]@() }
                    Decode-Message $inst $msgid $pl
                }
            }
            $i += $total
        }
    }
    # Housekeeping per vehicle: GCS heartbeat (1 Hz), stream rates + extra params once it is alive.
    $now = Get-Date
    $hbDue = ($now - $script:LastMgrHb).TotalSeconds -ge 1
    foreach ($inst in $script:Instances) {
        if (-not $inst.Endpoint) { continue }
        if ($hbDue) { Send-To $inst (Mav-Heartbeat) }
        if ($inst.LastHb -and -not $inst.IntervalsRequested) {
            foreach ($m in @(@{ id = 33; hz = 3 }, @{ id = 74; hz = 3 }, @{ id = 24; hz = 1 }, @{ id = 1; hz = 1 })) { Send-To $inst (Mav-MessageInterval $inst.Sysid $m.id $m.hz) }
            $inst.IntervalsRequested = $true
        }
        # Extra params: PARAM_SET each one, confirmed by the PARAM_VALUE echo; unanswered ones go again
        # every second (ArduPilot drops the odd request right after boot), five tries, then give up loudly.
        if ($inst.LastHb -and -not $inst.ParamsApplied -and ($now - $inst.StartedAt).TotalSeconds -ge 4) {
            if ($inst.ParamTries -eq 0) { foreach ($p in Extra-Params) { $inst.PendingParams[$p.name] = $p.value } }
            if ($inst.PendingParams.Count -eq 0) {
                $inst.ParamsApplied = $true
                if ($inst.ParamTries -gt 0) { $inst.StatusText = "extra params applied ($((Extra-Params).Count))" }
            } elseif (($now - $inst.LastParamSend).TotalSeconds -ge 1) {
                if ($inst.ParamTries -ge 5) {
                    $inst.StatusText = "params NOT confirmed: $($inst.PendingParams.Keys -join ', ')"; $inst.ParamsApplied = $true
                } else {
                    foreach ($name in @($inst.PendingParams.Keys)) { Send-To $inst (Mav-ParamSet $inst.Sysid $name $inst.PendingParams[$name]) }
                    $inst.ParamTries++; $inst.LastParamSend = $now
                }
            }
        }
    }
    if ($hbDue) { $script:LastMgrHb = $now }
}
function Decode-Message($inst, [int]$msgid, [byte[]]$pl) {
    switch ($msgid) {
        0 {   # HEARTBEAT
            $p = Pad-Payload $pl 9
            $inst.MavType = $p[4]
            $mode = [BitConverter]::ToUInt32($p, 0)
            $names = $ModeNames[(Mode-Class $inst.MavType)]
            $inst.Mode = if ($names.ContainsKey([int]$mode)) { $names[[int]$mode] } else { "mode $mode" }
            $inst.Armed = ($p[6] -band 128) -ne 0
            $inst.LastHb = Get-Date; $inst.State = 'running'
        }
        1 {   # SYS_STATUS: voltage_battery u16 @14, current_battery i16 @16, battery_remaining i8 @30
            $p = Pad-Payload $pl 31
            $inst.Voltage = [BitConverter]::ToUInt16($p, 14) / 1000.0
            $inst.Current = [BitConverter]::ToInt16($p, 16) / 100.0
            $inst.BattPct = [sbyte]$p[30]
        }
        24 {  # GPS_RAW_INT: fix_type u8 @28, satellites_visible u8 @29
            $p = Pad-Payload $pl 30
            $inst.FixType = $p[28]; $inst.Sats = $p[29]
        }
        33 {  # GLOBAL_POSITION_INT: lat i32 @4, lon i32 @8, relative_alt i32 @16, hdg u16 @26
            $p = Pad-Payload $pl 28
            $inst.Lat = [BitConverter]::ToInt32($p, 4) / 1e7; $inst.Lon = [BitConverter]::ToInt32($p, 8) / 1e7
            $inst.RelAlt = [BitConverter]::ToInt32($p, 16) / 1000.0
            $h = [BitConverter]::ToUInt16($p, 26); if ($h -ne 65535) { $inst.Hdg = $h / 100.0 }
        }
        74 {  # VFR_HUD: airspeed f32 @0, groundspeed f32 @4, heading i16 @16, throttle u16 @18
            $p = Pad-Payload $pl 20
            $inst.Airspeed = [BitConverter]::ToSingle($p, 0); $inst.GroundSpeed = [BitConverter]::ToSingle($p, 4)
            $inst.Throttle = [BitConverter]::ToUInt16($p, 18)
        }
        22 {  # PARAM_VALUE: value f32 @0, id char[16] @8 — the echo that confirms a PARAM_SET
            $p = Pad-Payload $pl 25
            $name = [Text.Encoding]::ASCII.GetString($p, 8, 16).TrimEnd([char]0)
            if ($inst.PendingParams.ContainsKey($name)) {
                $v = [BitConverter]::ToSingle($p, 0)
                if ([math]::Abs($v - $inst.PendingParams[$name]) -lt 1e-3) { $inst.PendingParams.Remove($name) }
            }
        }
        253 { # STATUSTEXT: severity u8 @0, text char[50] @1
            $p = Pad-Payload $pl 51
            $txt = [Text.Encoding]::ASCII.GetString($p, 1, 50).TrimEnd([char]0)
            # MAV_SEVERITY: 0–3 emergency…error, 4 warning, 5 notice, 6 info, 7 debug
            $inst.StatusText = $(if ($p[0] -le 3) { "ERROR: $txt" } elseif ($p[0] -eq 4) { "warn: $txt" } else { $txt })
        }
    }
}
function Watchdog {
    foreach ($inst in $script:Instances) {
        if ($inst.State -eq 'stopped') { continue }
        if ($inst.Process -and $inst.Process.HasExited) {
            if ($Settings.autoRestart) {
                $inst.Restarts++; $inst.State = 'restarting'
                try { Start-Instance $inst } catch { $inst.State = "failed: $($_.Exception.Message)" }
            } else { $inst.State = 'died'; $inst.Mode = ''; $inst.Armed = $false }
        } elseif ($inst.LastHb -and ((Get-Date) - $inst.LastHb).TotalSeconds -gt 5) {
            $inst.State = 'silent'
        }
    }
}

# ── Headless mode ────────────────────────────────────────────────────────────────────────────────
if ($Headless) {
    if ($Count -gt 0) { $Settings.count = $Count }
    if ($Frame) { $Settings.frame = $Frame; $Settings.vehicle = ($Frames | Where-Object { $_.f -eq $Frame } | Select-Object -First 1).v }
    if ($Layout) { $Settings.layout = $Layout }
    if ($Wipe) { $Settings.wipe = $true }
    Start-All
    "Started $($Settings.count) x $($Settings.frame) ($($Settings.layout)); monitor on UDP $($Settings.monitorPort). Ctrl+C stops everything."
    $t0 = Get-Date; $lastPrint = [datetime]::MinValue
    try {
        while ($true) {
            Poll-Monitor; Watchdog
            if (((Get-Date) - $lastPrint).TotalSeconds -ge 2) {
                $lastPrint = Get-Date
                $script:Instances | ForEach-Object {
                    '{0,3}  {1,-10} {2,-12} {3,-8} {4,-12} {5,-8} {6,6:F0} m {7,5:F1} m/s {8,4:F0}deg  gps {9}/{10,-2} {11,5:F1} V  {12}' -f $_.Sysid, $_.Frame, $_.State, $(if ($_.Process -and -not $_.Process.HasExited) { $_.Process.Id } else { '-' }), $_.Mode, $(if ($_.Armed) { 'ARMED' } else { 'disarmed' }), $_.RelAlt, $_.GroundSpeed, $_.Hdg, $_.FixType, $_.Sats, $_.Voltage, $_.StatusText
                }
                ''
            }
            if ($Seconds -gt 0 -and ((Get-Date) - $t0).TotalSeconds -ge $Seconds) { break }
            Start-Sleep -Milliseconds 100
        }
    } finally { Stop-All; 'stopped' }
    return
}

# ── UI ───────────────────────────────────────────────────────────────────────────────────────────
$form = New-Object System.Windows.Forms.Form
$form.Text = 'Kite SITL manager'
$form.StartPosition = 'CenterScreen'
$form.Size = New-Object System.Drawing.Size(1180, 780)
$form.MinimumSize = New-Object System.Drawing.Size(980, 640)
$form.Font = New-Object System.Drawing.Font('Segoe UI', 9)

function New-Label([string]$text, [int]$x, [int]$y, [int]$w = 90) {
    $l = New-Object System.Windows.Forms.Label; $l.Text = $text; $l.Location = New-Object System.Drawing.Point($x, ($y + 3)); $l.Size = New-Object System.Drawing.Size($w, 20); $l
}
function New-Group([string]$title, [int]$x, [int]$y, [int]$w, [int]$h) {
    $g = New-Object System.Windows.Forms.GroupBox; $g.Text = $title; $g.Location = New-Object System.Drawing.Point($x, $y); $g.Size = New-Object System.Drawing.Size($w, $h); $form.Controls.Add($g); $g
}
function Add-Ctl($parent, $ctl, [int]$x, [int]$y, [int]$w, [int]$h = 24) {
    $ctl.Location = New-Object System.Drawing.Point($x, $y); $ctl.Size = New-Object System.Drawing.Size($w, $h); $parent.Controls.Add($ctl); $ctl
}

# Vehicle
$gVeh = New-Group 'Vehicle' 12 10 380 130
$gVeh.Controls.Add((New-Label 'Type' 12 24))
$cbVehicle = Add-Ctl $gVeh (New-Object System.Windows.Forms.ComboBox) 100 22 110
$cbVehicle.DropDownStyle = 'DropDownList'; [void]$cbVehicle.Items.AddRange(@('Plane', 'Copter', 'Heli', 'Rover', 'Sub'))
$gVeh.Controls.Add((New-Label 'Frame' 220 24 50))
$cbFrame = Add-Ctl $gVeh (New-Object System.Windows.Forms.ComboBox) 265 22 100
$cbFrame.DropDownStyle = 'DropDownList'
$gVeh.Controls.Add((New-Label 'Vehicles' 12 56))
$numCount = Add-Ctl $gVeh (New-Object System.Windows.Forms.NumericUpDown) 100 54 60
$numCount.Minimum = 1; $numCount.Maximum = 32   # sysids are a byte and ports go to 65535 — CPU per instance is the real limit (a 16-core desktop is at its knees well before 32)
$gVeh.Controls.Add((New-Label 'first sysid' 170 56 70))
$numSysid = Add-Ctl $gVeh (New-Object System.Windows.Forms.NumericUpDown) 245 54 60
$numSysid.Minimum = 1; $numSysid.Maximum = 250
$gVeh.Controls.Add((New-Label 'Speedup' 12 88))
$numSpeed = Add-Ctl $gVeh (New-Object System.Windows.Forms.NumericUpDown) 100 86 60
$numSpeed.Minimum = 1; $numSpeed.Maximum = 20; $numSpeed.DecimalPlaces = 1; $numSpeed.Increment = 0.5
$chkWipe = Add-Ctl $gVeh (New-Object System.Windows.Forms.CheckBox) 170 86 200
$chkWipe.Text = 'Wipe EEPROM (fresh params)'

# Start position
$gPos = New-Group 'Start position' 404 10 400 130
$gPos.Controls.Add((New-Label 'Preset' 12 24 50))
$cbPreset = Add-Ctl $gPos (New-Object System.Windows.Forms.ComboBox) 60 22 200
$cbPreset.DropDownStyle = 'DropDown'
$btnSavePreset = Add-Ctl $gPos (New-Object System.Windows.Forms.Button) 266 21 60; $btnSavePreset.Text = 'Save'
$btnDelPreset = Add-Ctl $gPos (New-Object System.Windows.Forms.Button) 330 21 60; $btnDelPreset.Text = 'Delete'
$gPos.Controls.Add((New-Label 'Lat' 12 56 30)); $txtLat = Add-Ctl $gPos (New-Object System.Windows.Forms.TextBox) 42 54 100
$gPos.Controls.Add((New-Label 'Lon' 150 56 30)); $txtLon = Add-Ctl $gPos (New-Object System.Windows.Forms.TextBox) 182 54 100
$gPos.Controls.Add((New-Label 'Alt m' 290 56 40)); $txtAlt = Add-Ctl $gPos (New-Object System.Windows.Forms.TextBox) 330 54 60
$gPos.Controls.Add((New-Label 'Hdg°' 12 88 36)); $txtHdg = Add-Ctl $gPos (New-Object System.Windows.Forms.TextBox) 50 86 50
$gPos.Controls.Add((New-Label 'Formation' 110 88 65))
$cbFormation = Add-Ctl $gPos (New-Object System.Windows.Forms.ComboBox) 182 86 70
$cbFormation.DropDownStyle = 'DropDownList'; [void]$cbFormation.Items.AddRange(@('line', 'grid'))
$gPos.Controls.Add((New-Label 'spacing m' 260 88 65))
$numSpacing = Add-Ctl $gPos (New-Object System.Windows.Forms.NumericUpDown) 330 86 60
$numSpacing.Minimum = 2; $numSpacing.Maximum = 500

# Kite link layout
$gLink = New-Group 'Kite connects via' 816 10 340 130
$rbTcp = Add-Ctl $gLink (New-Object System.Windows.Forms.RadioButton) 12 22 320; $rbTcp.Text = 'TCP link per vehicle (5760, 5770, …)'
$rbUdp = Add-Ctl $gLink (New-Object System.Windows.Forms.RadioButton) 12 46 230; $rbUdp.Text = 'one UDP port, all vehicles (fan-in)'
$numUdp = Add-Ctl $gLink (New-Object System.Windows.Forms.NumericUpDown) 250 45 70
$numUdp.Minimum = 1024; $numUdp.Maximum = 65535
$rbChain = Add-Ctl $gLink (New-Object System.Windows.Forms.RadioButton) 12 70 320; $rbChain.Text = 'one TCP link 5760, vehicles chained (MP swarm)'
$chkRestart = Add-Ctl $gLink (New-Object System.Windows.Forms.CheckBox) 12 98 320; $chkRestart.Text = 'Restart vehicles that exit (TCP disconnect kills them)'

# Extra params + binaries
$gParams = New-Group 'Extra parameters (NAME=VALUE per line)' 12 148 380 120
$txtParams = Add-Ctl $gParams (New-Object System.Windows.Forms.TextBox) 12 22 356 88
$txtParams.Multiline = $true; $txtParams.AcceptsReturn = $true; $txtParams.ScrollBars = 'Vertical'; $txtParams.Font = New-Object System.Drawing.Font('Consolas', 9)

$gBin = New-Group 'Binaries' 404 148 752 120
$gBin.Controls.Add((New-Label 'Source' 12 24 50))
$cbSource = Add-Ctl $gBin (New-Object System.Windows.Forms.ComboBox) 60 22 260
$cbSource.DropDownStyle = 'DropDownList'
[void]$cbSource.Items.AddRange(@('auto (downloaded channel if present, else Mission Planner)', 'Mission Planner folder', 'downloaded channel'))
$gBin.Controls.Add((New-Label 'Channel' 330 24 55))
$cbChannel = Add-Ctl $gBin (New-Object System.Windows.Forms.ComboBox) 390 22 170
$cbChannel.DropDownStyle = 'DropDownList'; [void]$cbChannel.Items.AddRange(@($Channels.Keys))
$btnDownload = Add-Ctl $gBin (New-Object System.Windows.Forms.Button) 570 21 170; $btnDownload.Text = 'Download / update channel'
$lblBin = Add-Ctl $gBin (New-Object System.Windows.Forms.Label) 12 56 728 50
$lblBin.Text = ''

# Actions
$btnStart = Add-Ctl $form (New-Object System.Windows.Forms.Button) 12 278 120 32; $btnStart.Text = 'Start'
$btnStart.Font = New-Object System.Drawing.Font('Segoe UI', 9, [System.Drawing.FontStyle]::Bold)
$btnStop = Add-Ctl $form (New-Object System.Windows.Forms.Button) 140 278 120 32; $btnStop.Text = 'Stop all'
$btnRestartOne = Add-Ctl $form (New-Object System.Windows.Forms.Button) 268 278 140 32; $btnRestartOne.Text = 'Restart selected'
$lblConnect = Add-Ctl $form (New-Object System.Windows.Forms.Label) 420 278 736 32
$lblConnect.Font = New-Object System.Drawing.Font('Consolas', 9)

# Grid
$grid = New-Object System.Windows.Forms.DataGridView
$grid.Location = New-Object System.Drawing.Point(12, 318); $grid.Size = New-Object System.Drawing.Size(1144, 240)
$grid.Anchor = 'Top,Left,Right'
$grid.ReadOnly = $true; $grid.AllowUserToAddRows = $false; $grid.AllowUserToDeleteRows = $false; $grid.AllowUserToResizeRows = $false
$grid.RowHeadersVisible = $false; $grid.SelectionMode = 'FullRowSelect'; $grid.MultiSelect = $false
$grid.AutoSizeColumnsMode = 'Fill'
$cols = @(
    @{ n = 'Sysid'; w = 5 }, @{ n = 'Frame'; w = 9 }, @{ n = 'Link'; w = 11 }, @{ n = 'PID'; w = 6 }, @{ n = 'State'; w = 8 },
    @{ n = 'Mode'; w = 9 }, @{ n = 'Armed'; w = 6 }, @{ n = 'Alt m'; w = 6 }, @{ n = 'GS m/s'; w = 6 }, @{ n = 'Hdg'; w = 5 },
    @{ n = 'GPS'; w = 7 }, @{ n = 'Batt'; w = 9 }, @{ n = 'Last status text'; w = 23 }
)
foreach ($c in $cols) { $col = New-Object System.Windows.Forms.DataGridViewTextBoxColumn; $col.HeaderText = $c.n; $col.FillWeight = $c.w; [void]$grid.Columns.Add($col) }
$form.Controls.Add($grid)

# Log tail
$txtLog = New-Object System.Windows.Forms.TextBox
$txtLog.Location = New-Object System.Drawing.Point(12, 566); $txtLog.Size = New-Object System.Drawing.Size(1144, 150)
$txtLog.Anchor = 'Top,Bottom,Left,Right'
$txtLog.Multiline = $true; $txtLog.ReadOnly = $true; $txtLog.ScrollBars = 'Vertical'; $txtLog.Font = New-Object System.Drawing.Font('Consolas', 8.5)
$form.Controls.Add($txtLog)
$status = New-Object System.Windows.Forms.StatusStrip
$statusLabel = New-Object System.Windows.Forms.ToolStripStatusLabel; $statusLabel.Text = 'idle'
[void]$status.Items.Add($statusLabel); $form.Controls.Add($status)

# ── UI ↔ settings ────────────────────────────────────────────────────────────────────────────────
function Fill-Frames {
    $cbFrame.Items.Clear()
    foreach ($fr in ($Frames | Where-Object { $_.v -eq $cbVehicle.SelectedItem })) { [void]$cbFrame.Items.Add($fr.f) }
    if ($Settings.frame -and $cbFrame.Items.Contains($Settings.frame)) { $cbFrame.SelectedItem = $Settings.frame } else { $cbFrame.SelectedIndex = 0 }
}
function Fill-Presets {
    $cbPreset.Items.Clear()
    foreach ($p in $Settings.presets) { [void]$cbPreset.Items.Add($p.name) }
    if ($Settings.preset -and $cbPreset.Items.Contains($Settings.preset)) { $cbPreset.SelectedItem = $Settings.preset } elseif ($cbPreset.Items.Count) { $cbPreset.SelectedIndex = 0 }
}
function Show-Preset {
    $p = $Settings.presets | Where-Object { $_.name -eq $cbPreset.Text } | Select-Object -First 1
    if ($p) {
        $inv = [Globalization.CultureInfo]::InvariantCulture
        $txtLat.Text = $p.lat.ToString($inv); $txtLon.Text = $p.lon.ToString($inv); $txtAlt.Text = $p.alt.ToString($inv); $txtHdg.Text = $p.hdg.ToString($inv)
    }
}
function Ui-ToSettings {
    $Settings.vehicle = $cbVehicle.SelectedItem; $Settings.frame = $cbFrame.SelectedItem
    $Settings.count = [int]$numCount.Value; $Settings.sysidBase = [int]$numSysid.Value; $Settings.speedup = [double]$numSpeed.Value
    $Settings.wipe = $chkWipe.Checked; $Settings.preset = $cbPreset.Text
    $Settings.formation = $cbFormation.SelectedItem; $Settings.spacingM = [double]$numSpacing.Value
    $Settings.layout = if ($rbUdp.Checked) { 'udp' } elseif ($rbChain.Checked) { 'chain' } else { 'tcp' }
    $Settings.udpPort = [int]$numUdp.Value; $Settings.autoRestart = $chkRestart.Checked
    $Settings.extraParams = $txtParams.Text
    $Settings.binSource = @('auto', 'mp', 'download')[$cbSource.SelectedIndex]; $Settings.channel = $cbChannel.SelectedItem
    Save-Settings $Settings
}
$script:Loading = $false
function Settings-ToUi {
    $script:Loading = $true
    $cbVehicle.SelectedItem = $Settings.vehicle; Fill-Frames
    $numCount.Value = $Settings.count; $numSysid.Value = $Settings.sysidBase; $numSpeed.Value = [decimal]$Settings.speedup
    $chkWipe.Checked = [bool]$Settings.wipe
    Fill-Presets; Show-Preset
    $cbFormation.SelectedItem = $Settings.formation; $numSpacing.Value = [decimal]$Settings.spacingM
    switch ($Settings.layout) { 'udp' { $rbUdp.Checked = $true } 'chain' { $rbChain.Checked = $true } default { $rbTcp.Checked = $true } }
    $numUdp.Value = $Settings.udpPort; $chkRestart.Checked = [bool]$Settings.autoRestart
    $txtParams.Text = ([string]$Settings.extraParams) -replace "`r?`n", "`r`n"
    $cbSource.SelectedIndex = [array]::IndexOf(@('auto', 'mp', 'download'), $Settings.binSource); if ($cbSource.SelectedIndex -lt 0) { $cbSource.SelectedIndex = 0 }
    if ($Settings.channel -and $cbChannel.Items.Contains($Settings.channel)) { $cbChannel.SelectedItem = $Settings.channel } else { $cbChannel.SelectedIndex = 0 }
    $script:Loading = $false
}
function Parse-Preset {
    $inv = [Globalization.CultureInfo]::InvariantCulture
    try {
        return @{ name = $cbPreset.Text.Trim(); lat = [double]::Parse($txtLat.Text, $inv); lon = [double]::Parse($txtLon.Text, $inv); alt = [double]::Parse($txtAlt.Text, $inv); hdg = [double]::Parse($txtHdg.Text, $inv) }
    } catch { throw 'Lat / Lon / Alt / Hdg must be numbers (decimal point, e.g. -35.363261)' }
}
function Refresh-BinLabel {
    $dir = Bin-Dir
    $have = @(); foreach ($v in $Binaries.Keys) { if (Test-Path (Join-Path $dir "$($Binaries[$v]).exe")) { $have += $v } }
    $missing = @($Binaries.Keys | Where-Object { $have -notcontains $_ })
    $cyg = Test-Path (Join-Path $dir 'cygwin1.dll')
    $lblBin.Text = "Using: $dir`r`nVehicles present: $(if ($have) { $have -join ', ' } else { 'none' })$(if ($missing) { "   missing: $($missing -join ', ')" })$(if (-not $cyg) { '   (cygwin DLLs missing!)' })"
}
function Connect-Hint {
    if (-not $script:Instances.Count) { $lblConnect.Text = ''; return }
    switch ($Settings.layout) {
        'udp'   { $lblConnect.Text = "Kite: MAVLink / UDP  127.0.0.1:$($Settings.udpPort)   (all $($Settings.count) vehicle(s) on that one link)" }
        'chain' { $lblConnect.Text = "Kite: MAVLink / TCP  127.0.0.1:5760   (all $($Settings.count) vehicle(s) on that one link via the chain)" }
        default { $lblConnect.Text = "Kite: MAVLink / TCP  " + (($script:Instances | ForEach-Object { "127.0.0.1:$(5760 + 10 * $_.Instance)" }) -join ', ') }
    }
}
function Refresh-Grid {
    if ($grid.Rows.Count -ne $script:Instances.Count) {
        $grid.Rows.Clear()
        foreach ($inst in $script:Instances) { [void]$grid.Rows.Add() }
    }
    for ($r = 0; $r -lt $script:Instances.Count; $r++) {
        $inst = $script:Instances[$r]
        $link = switch ($Settings.layout) { 'udp' { "udp:$($Settings.udpPort)" } 'chain' { if ($inst.Index -eq 0) { 'tcp:5760 (head)' } else { "chained" } } default { "tcp:$(5760 + 10 * $inst.Instance)" } }
        $procId = if ($inst.Process -and -not $inst.Process.HasExited) { $inst.Process.Id } else { '' }
        $state = $inst.State; if ($inst.Restarts) { $state += " (×$($inst.Restarts))" }
        $gps = if ($inst.LastHb) { "$(@('none', 'none', '2D', '3D', 'DGPS', 'RTKf', 'RTKx')[[math]::Min($inst.FixType, 6)]) $($inst.Sats)" } else { '' }
        $batt = if ($inst.LastHb -and $inst.Voltage -gt 0) { "$($inst.Voltage.ToString('F1')) V $(if ($inst.BattPct -ge 0) { "$($inst.BattPct) %" })" } else { '' }
        $vals = @($inst.Sysid, $inst.Frame, $link, $procId, $state, $inst.Mode, $(if ($inst.LastHb) { if ($inst.Armed) { 'ARMED' } else { 'disarmed' } } else { '' }),
                  $(if ($inst.LastHb) { $inst.RelAlt.ToString('F0') } else { '' }), $(if ($inst.LastHb) { $inst.GroundSpeed.ToString('F1') } else { '' }),
                  $(if ($inst.LastHb) { $inst.Hdg.ToString('F0') } else { '' }), $gps, $batt, $inst.StatusText)
        for ($c = 0; $c -lt $vals.Count; $c++) { if ($grid.Rows[$r].Cells[$c].Value -ne $vals[$c]) { $grid.Rows[$r].Cells[$c].Value = $vals[$c] } }
        $grid.Rows[$r].DefaultCellStyle.ForeColor = if ($inst.State -eq 'running') { [System.Drawing.Color]::Black } elseif ($inst.State -in 'starting', 'restarting') { [System.Drawing.Color]::DarkGoldenrod } else { [System.Drawing.Color]::Firebrick }
    }
}
function Refresh-Log {
    if ($grid.SelectedRows.Count -eq 0 -or $script:Instances.Count -eq 0) { return }
    $inst = $script:Instances[$grid.SelectedRows[0].Index]
    $f = Join-Path $inst.Dir 'stdout.txt'
    if (Test-Path $f) {
        $tail = (Get-Content $f -Tail 40 -ErrorAction SilentlyContinue) -join "`r`n"
        if ($txtLog.Text -ne $tail) { $txtLog.Text = $tail; $txtLog.SelectionStart = $txtLog.Text.Length; $txtLog.ScrollToCaret() }
    }
}

# ── Events ───────────────────────────────────────────────────────────────────────────────────────
$cbVehicle.Add_SelectedIndexChanged({ Fill-Frames })
$cbPreset.Add_SelectedIndexChanged({ Show-Preset })
$btnSavePreset.Add_Click({
    try {
        $p = Parse-Preset
        if (-not $p.name) { throw 'Give the preset a name (type it into the Preset box)' }
        $Settings.presets = @($Settings.presets | Where-Object { $_.name -ne $p.name }) + @($p)
        $Settings.preset = $p.name; Fill-Presets; Ui-ToSettings; $statusLabel.Text = "Preset '$($p.name)' saved"
    } catch { [System.Windows.Forms.MessageBox]::Show($_.Exception.Message, 'Preset') | Out-Null }
})
$btnDelPreset.Add_Click({
    if ($Settings.presets.Count -le 1) { return }
    $Settings.presets = @($Settings.presets | Where-Object { $_.name -ne $cbPreset.Text })
    $Settings.preset = $Settings.presets[0].name; Fill-Presets; Show-Preset; Ui-ToSettings
})
$cbSource.Add_SelectedIndexChanged({ if (-not $script:Loading) { Ui-ToSettings; Refresh-BinLabel } })
$cbChannel.Add_SelectedIndexChanged({ if (-not $script:Loading) { Ui-ToSettings; Refresh-BinLabel } })
$btnDownload.Add_Click({
    Ui-ToSettings
    $btnDownload.Enabled = $false
    try {
        $vehicles = @('Plane', 'Copter', 'Heli', 'Rover', 'Sub')
        Download-Channel $Settings.channel $vehicles { param($m) $statusLabel.Text = $m; [System.Windows.Forms.Application]::DoEvents() } | Out-Null
        if ($Settings.binSource -eq 'mp') { $cbSource.SelectedIndex = 2 }
    } catch { [System.Windows.Forms.MessageBox]::Show($_.Exception.Message, 'Download failed') | Out-Null; $statusLabel.Text = 'download failed' }
    $btnDownload.Enabled = $true; Refresh-BinLabel
})
$btnStart.Add_Click({
    try {
        $p = Parse-Preset
        # An edited position without a saved preset still flies: keep it as the preset's live values.
        $Settings.presets = @($Settings.presets | Where-Object { $_.name -ne $p.name }) + @($p)
        $Settings.preset = $p.name; Fill-Presets
        Ui-ToSettings
        $statusLabel.Text = 'starting…'; [System.Windows.Forms.Application]::DoEvents()
        Start-All
        Connect-Hint; Refresh-Grid
        $statusLabel.Text = "$($Settings.count) × $($Settings.frame) started — monitor on UDP $($Settings.monitorPort)"
    } catch {
        [System.Windows.Forms.MessageBox]::Show($_.Exception.Message, 'Start failed') | Out-Null; $statusLabel.Text = 'start failed'
    }
})
$btnStop.Add_Click({ Stop-All; Refresh-Grid; Connect-Hint; $statusLabel.Text = 'stopped' })
$btnRestartOne.Add_Click({
    if ($grid.SelectedRows.Count -eq 0) { return }
    $inst = $script:Instances[$grid.SelectedRows[0].Index]
    Stop-Instance $inst; $inst.Restarts++
    try { Start-Instance $inst } catch { [System.Windows.Forms.MessageBox]::Show($_.Exception.Message, 'Restart failed') | Out-Null }
})
$grid.Add_SelectionChanged({ Refresh-Log })
$form.Add_FormClosing({ Ui-ToSettings; Stop-All })

$timer = New-Object System.Windows.Forms.Timer
$timer.Interval = 250
$timer.Add_Tick({
    try { Poll-Monitor; Watchdog; Refresh-Grid } catch { $statusLabel.Text = "monitor: $($_.Exception.Message)" }
})
$logTimer = New-Object System.Windows.Forms.Timer
$logTimer.Interval = 1500
$logTimer.Add_Tick({ try { Refresh-Log } catch { } })

Settings-ToUi
Refresh-BinLabel
$timer.Start(); $logTimer.Start()
if ($AutoStart) { $form.Add_Shown({ $btnStart.PerformClick() }) }
[void]$form.ShowDialog()
$timer.Stop(); $logTimer.Stop()
