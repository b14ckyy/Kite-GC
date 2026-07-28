// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

// Embedded video — source router with three kinds:
//   • camera — local webcam / USB capture via getUserMedia (the zero-dependency default).
//   • rtsp   — network stream via the go2rtc engine (WebRTC, MJPEG fallback).
//   • native — the OS hardware capture layer via ffmpeg (Linux V4L2 / Windows DirectShow / macOS
//              AVFoundation) → embedded MJPEG server, rendered in an <img>. The "Advanced" tier with
//              device-verified codec/resolution/framerate control (see helpers/videoCapabilities.ts).
//
// The router opens a source once and exposes its MediaStream; multiple sinks
// (the NavRail panel preview, the dock widget, the floating window, the
// map-swap view) bind the *same* stream to their own <video> element — a
// MediaStream attaches to many elements at once, so one decode feeds them all.
// (native is the exception: an MJPEG multipart feed rendered per-<img>.)
//
// `getUserMedia` works in WebView2 (Windows) and WebKitGTK (Linux) and, with the camera entitlement,
// WKWebView (macOS), so the camera path needs no backend. rtsp + native use the Rust backend.

import { writable, get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { t } from 'svelte-i18n';
import { isLinux } from '$lib/platform';
import {
  type NativeDevice,
  type CaptureMode,
  type NativeSelection,
  validateSelection,
} from '$lib/helpers/videoCapabilities';
import { setMjpegSinkHandlers, startMjpegSink, stopMjpegSink } from '$lib/controllers/mjpegSink';

export interface VideoDevice {
  deviceId: string;
  label: string;
}

export type VideoStatus = 'off' | 'starting' | 'live' | 'error';
export type VideoResolution = 'auto' | '480p' | '720p' | '1080p';
/** getUserMedia framerate wish (the camera path can't enumerate modes, only hint a rate). */
export type CameraFps = 'auto' | '30' | '60';
/** Source kind: local camera (getUserMedia MediaStream), RTSP bridge (go2rtc), or native hardware
 *  capture (V4L2 / DirectShow / AVFoundation → embedded MJPEG server). */
export type VideoKind = 'camera' | 'rtsp' | 'native';
/** Which go2rtc reader served the live RTSP feed: native client or the ffmpeg fallback. */
export type RtspEngine = 'native' | 'ffmpeg' | null;
/** RTSP transport for a connection. 'udp' → ffmpeg reader (reads UDP-only servers like the UAV-Link
 *  Pi); 'tcp' → go2rtc's native RTP-over-TCP client; 'auto' → native first, then the ffmpeg fallback. */
export type RtspTransport = 'udp' | 'tcp' | 'auto';
/** A saved, named RTSP connection the user can recall from the connection list (see VideoPanel). */
export interface RtspConnection {
  id: string;
  name: string;
  url: string;
  transport: RtspTransport;
}
/** Where the single map instance currently lives (the inverse of which surfaces show video). */
export type MapLocation = 'main' | 'floating' | 'widget';

export interface VideoState {
  /** Active source kind. `camera` → getUserMedia MediaStream; `rtsp` → go2rtc (WebRTC or MJPEG);
   *  `native` → embedded MJPEG server rendered in an `<img>`. */
  kind: VideoKind;
  /** User wants video on (source open). */
  enabled: boolean;
  status: VideoStatus;
  devices: VideoDevice[];
  /** Selected video input device (null = system default). */
  deviceId: string | null;
  resolution: VideoResolution;
  /** getUserMedia framerate wish (camera path). */
  cameraFps: CameraFps;
  // ── Native capture (Advanced) ────────────────────────────────────
  /** Native capture devices (V4L2/DirectShow/AVFoundation) — enumerated by the Rust backend. */
  nativeDevices: NativeDevice[];
  /** Selected native device id (V4L2 path / DirectShow name / AVFoundation index), null if none. */
  nativeDevice: string | null;
  /** Name of the selected native device — the tie-breaker when the id turns out to be unstable
   *  (AVFoundation index / `/dev/videoN` both renumber on re-plug). See `resolveNativeDevice`. */
  nativeDeviceName: string | null;
  /** Probed modes for the selected native device (drives the format→resolution→framerate cascade). */
  nativeModes: CaptureMode[];
  /** Chosen native capture config (format/resolution/framerate). */
  nativeSel: NativeSelection;
  // ── RTSP source ──────────────────────────────────────────────────
  /** RTSP URL (e.g. rtsp://192.168.1.10:554/live) — the active/direct-connect URL. */
  rtspUrl: string;
  /** Transport for the active RTSP connection (udp/tcp/auto). */
  rtspTransport: RtspTransport;
  /** Saved, named RTSP connections the user can recall (explicit save — never auto-added). */
  rtspConnections: RtspConnection[];
  /** Active RTSP reader once live (native go2rtc client vs ffmpeg fallback); runtime-only. */
  rtspEngine: RtspEngine;
  /** Runtime-only: true while the infinite RTSP auto-reconnect loop is running (link dropped/stalled). */
  reconnecting: boolean;
  /** Runtime-only: current reconnect attempt number, shown in the on-video overlay. */
  reconnectAttempt: number;
  /** go2rtc MJPEG HTTP URL for systems where RTCPeerConnection is unavailable. */
  mjpegUrl: string | null;
  /** What the RUNNING feed actually does, as reported by the backend: 'copy' (stream-copied, nothing
   *  to accelerate), 'software', 'vaapi', 'v4l2m2m', 'none' (no transcode at all — WebRTC), or null
   *  when nothing is live. Runtime-only. Reported rather than inferred: whether this host *can* do
   *  hardware and whether this feed *is* using it are different questions. */
  activeTranscode: string | null;
  /** User veto on hardware transcoding: force the software path even where the backend's probe says
   *  hardware works. An escape hatch for driver/hardware combinations we can't anticipate — hardware
   *  stays the default, this is the opt-out. */
  disableHwAccel: boolean;
  /** Mirror horizontally (front-facing cams) — applied by the display sinks. */
  mirror: boolean;
  /** Source aspect ratio (w/h); drives the widget / floating-window sizing. */
  aspect: number;
  /** Negotiated track settings (for the info line); null until live. */
  width: number | null;
  height: number | null;
  frameRate: number | null;
  /** Max frame rate the camera *reports* it can do at the chosen mode (diagnostic). */
  capFrameRate: number | null;
  error: string | null;

  // ── Floating window ──────────────────────────────────────────────
  /** Floating video window visible. */
  floating: boolean;
  /** Snapped to the bottom-left corner (displaces the dock) vs free-floating. */
  floatSnapped: boolean;
  /** Free position (px from top-left of the app window), used when not snapped. */
  floatX: number;
  floatY: number;
  /** Window height as a fraction of the viewport height (0.1…0.3); width = height·aspect. */
  floatHeightFrac: number;
  /** Where the single map instance currently lives (transient, not persisted). `main` = the normal
   *  full-screen map; `floating`/`widget` = the map jumped into that video surface (which double-
   *  clicked), and every other surface shows video. Double-clicking a video moves the map there. */
  mapLocation: MapLocation;
  /** Screen rect (px) of the video widget tile, published by the widget — used to overlay the map
   *  onto it when `mapLocation === 'widget'`. Null until measured. */
  widgetRect: { x: number; y: number; w: number; h: number } | null;
}

// ── Persistence ─────────────────────────────────────────────────────
// Self-contained (own localStorage key, same mechanism as the app settings
// store): we remember the device/resolution/mirror selection and whether video
// was running, so it can auto-start with the last settings on the next launch.
const STORAGE_KEY = 'kite-gc-video';

interface VideoPrefs {
  kind: VideoKind;
  enabled: boolean;
  deviceId: string | null;
  resolution: VideoResolution;
  cameraFps: CameraFps;
  rtspUrl: string;
  rtspTransport: RtspTransport;
  rtspConnections: RtspConnection[];
  nativeDevice: string | null;
  nativeCodec: string;
  nativeDeviceName: string | null;
  nativeWidth: number;
  nativeHeight: number;
  nativeFps: number;
  disableHwAccel: boolean;
  mirror: boolean;
  floating: boolean;
  floatSnapped: boolean;
  floatX: number;
  floatY: number;
  floatHeightFrac: number;
}

const PREF_DEFAULTS: VideoPrefs = {
  kind: 'camera',
  enabled: false,
  deviceId: null,
  resolution: 'auto',
  cameraFps: 'auto',
  rtspUrl: '',
  rtspTransport: 'auto',
  rtspConnections: [],
  nativeDevice: null,
  nativeCodec: 'mjpeg',
  nativeDeviceName: null,
  nativeWidth: 1280,
  nativeHeight: 720,
  nativeFps: 30,
  disableHwAccel: false,
  mirror: false,
  floating: false,
  floatSnapped: true,
  floatX: 16,
  floatY: 80,
  floatHeightFrac: 0.2,
};

function loadPrefs(): VideoPrefs {
  try {
    const raw = typeof localStorage !== 'undefined' ? localStorage.getItem(STORAGE_KEY) : null;
    if (raw) {
      const p = JSON.parse(raw) as Partial<VideoPrefs> & { v4l2Device?: string | null };
      // Pre-1.0 hard-switch: the old Linux-only 'v4l2' kind is now the generic 'native' kind.
      const kind = ((p.kind as string) === 'v4l2' ? 'native' : p.kind ?? 'camera') as VideoKind;
      return {
        ...PREF_DEFAULTS,
        ...p,
        kind,
        deviceId: p.deviceId ?? null,
        resolution: p.resolution ?? 'auto',
        cameraFps: p.cameraFps ?? 'auto',
        rtspUrl: p.rtspUrl ?? '',
        rtspTransport: p.rtspTransport ?? 'auto',
        rtspConnections: Array.isArray(p.rtspConnections) ? p.rtspConnections : [],
        nativeDevice: p.nativeDevice ?? p.v4l2Device ?? null,
        nativeCodec: p.nativeCodec ?? 'mjpeg',
        nativeDeviceName: p.nativeDeviceName ?? null,
        nativeWidth: p.nativeWidth ?? 1280,
        nativeHeight: p.nativeHeight ?? 720,
        nativeFps: p.nativeFps ?? 30,
        disableHwAccel: p.disableHwAccel ?? false,
      };
    }
  } catch {
    /* ignore */
  }
  return { ...PREF_DEFAULTS };
}

function savePrefs(): void {
  if (typeof localStorage === 'undefined') return;
  const s = get(videoState);
  try {
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({
        kind: s.kind,
        enabled: s.enabled,
        deviceId: s.deviceId,
        resolution: s.resolution,
        cameraFps: s.cameraFps,
        rtspUrl: s.rtspUrl,
        rtspTransport: s.rtspTransport,
        rtspConnections: s.rtspConnections,
        nativeDevice: s.nativeDevice,
        nativeCodec: s.nativeSel.codec,
        nativeDeviceName: s.nativeDeviceName,
        nativeWidth: s.nativeSel.width,
        nativeHeight: s.nativeSel.height,
        nativeFps: s.nativeSel.fps,
        disableHwAccel: s.disableHwAccel,
        mirror: s.mirror,
        floating: s.floating,
        floatSnapped: s.floatSnapped,
        floatX: s.floatX,
        floatY: s.floatY,
        floatHeightFrac: s.floatHeightFrac,
      }),
    );
  } catch {
    /* ignore */
  }
}

const boot = loadPrefs();

const INITIAL: VideoState = {
  kind: boot.kind,
  enabled: false, // runtime flag — auto-start (below) decides whether to turn on
  status: 'off',
  devices: [],
  deviceId: boot.deviceId,
  resolution: boot.resolution,
  cameraFps: boot.cameraFps,
  nativeDevices: [],
  nativeDevice: boot.nativeDevice,
  nativeDeviceName: boot.nativeDeviceName,
  nativeModes: [],
  nativeSel: {
    codec: boot.nativeCodec,
    width: boot.nativeWidth,
    height: boot.nativeHeight,
    fps: boot.nativeFps,
  },
  rtspUrl: boot.rtspUrl,
  rtspTransport: boot.rtspTransport,
  rtspConnections: boot.rtspConnections,
  rtspEngine: null,
  reconnecting: false,
  reconnectAttempt: 0,
  mjpegUrl: null,
  activeTranscode: null,
  disableHwAccel: boot.disableHwAccel,
  mirror: boot.mirror,
  aspect: 16 / 9,
  width: null,
  height: null,
  frameRate: null,
  capFrameRate: null,
  error: null,
  floating: boot.floating,
  floatSnapped: boot.floatSnapped,
  floatX: boot.floatX,
  floatY: boot.floatY,
  floatHeightFrac: boot.floatHeightFrac,
  mapLocation: 'main',
  widgetRect: null,
};

export const videoState = writable<VideoState>({ ...INITIAL });

/**
 * The single live MediaStream that every sink renders. For `camera` it is the
 * `getUserMedia` stream; for `rtsp` it is the `captureStream()` of a hidden driver
 * `<video>` that plays the loopback feed (see startRtsp). Either way a MediaStream
 * attaches to many `<video>` elements at once, so one decode/connection feeds all
 * sinks — and the RTSP feed has exactly one ffmpeg/loopback connection.
 */
export const videoStream = writable<MediaStream | null>(null);

/** Per-second snapshot of the WebRTC inbound video pipeline, published by the RTSP stall monitor
 *  (which polls `getStats()` once a second anyway). Splits an unstable picture into its stages:
 *  `recvFps` (frames arriving from go2rtc — a shortfall here is upstream of the WebView),
 *  `decodeFps`/`framesDropped` (decoder keeping up or not), and the engine's own playout counters
 *  (`freezeCount`, `playoutDelayMs`). Consumed by the Debug Monitor's Video tab; null when no
 *  WebRTC feed is running. */
export interface VideoRtcStats {
  /** framesReceived per second over the last poll interval. */
  recvFps: number;
  /** framesDecoded per second over the last poll interval. */
  decodeFps: number;
  /** The decoder's own frames-per-second estimate, when the engine reports one. */
  engineFps: number | null;
  /** Cumulative frames dropped before presentation (received but never shown). */
  framesDropped: number;
  /** Cumulative RTP packets lost on the (loopback) transport — anything but 0 is remarkable. */
  packetsLost: number;
  /** Cumulative playout freezes counted by the engine, if reported. */
  freezeCount: number | null;
  /** Total frozen time in ms, if reported. */
  freezeMs: number | null;
  /** RFC 3550 interarrival jitter in ms, if reported. */
  jitterMs: number | null;
  /** Average jitter-buffer (playout) delay per emitted frame in ms, if reported. */
  playoutDelayMs: number | null;
  /** Received bitrate over the last poll interval, in kbit/s. */
  kbps: number;
  /** The negotiated video codec, prettified from the codec report's mime type (e.g. `H.264`). */
  codec: string | null;
}
export const videoRtcStats = writable<VideoRtcStats | null>(null);

function patch(p: Partial<VideoState>): void {
  videoState.update((s) => ({ ...s, ...p }));
}

/** Mirror a video-pipeline event into the **backend log file** (and the console).
 *
 *  The whole source router — including the RTSP reconnect loop and its stall detection — runs here in
 *  the frontend, so until now the answer to "why did the stream drop?" existed only as a `console.warn`
 *  in DevTools. A tester on a Raspberry Pi has neither: a release build has no console, and the log file
 *  the Diagnostics page hands out never saw a word of it. Stream aborts go in at **warn**, so they show
 *  up at the default log level; routine lifecycle detail is `info` (captured at Debug). */
function logVideo(level: 'warn' | 'info' | 'debug', message: string): void {
  if (level === 'warn') console.warn(`[video] ${message}`);
  else console.log(`[video] ${message}`);
  void invoke('log_frontend', { level, area: 'video', message }).catch(() => {});
}

/** Bind a sink's `<video>` element to the shared MediaStream (camera or rtsp). */
export function bindVideoEl(el: HTMLVideoElement | null, stream: MediaStream | null): void {
  if (!el) return;
  el.srcObject = stream;
}

/** Report the natural size of the live source (from a sink's `loadedmetadata`) so the
 *  floating window / widget can size to the real aspect ratio (RTSP has no upfront caps). */
export function reportVideoSize(width: number, height: number): void {
  if (!width || !height) return;
  rtspMjpegFailures = 0; // a frame was decoded → whatever decoder is in use is working
  // Change-guarded: the MJPEG `<img>` path reports from onload, which SOME engines fire per multipart
  // frame — an unguarded patch would churn the store at frame rate.
  const s = get(videoState);
  if (s.width === width && s.height === height) return;
  patch({ width, height, aspect: width / height });
}

const RES_DIMS: Record<VideoResolution, MediaTrackConstraints> = {
  auto: {},
  '480p': { width: { ideal: 640 }, height: { ideal: 480 } },
  '720p': { width: { ideal: 1280 }, height: { ideal: 720 } },
  '1080p': { width: { ideal: 1920 }, height: { ideal: 1080 } },
};

// Without a frameRate hint the browser may negotiate an uncompressed camera mode (YUY2/NV12) that is
// USB-bandwidth-limited to a few fps at high resolution. There is no way to request MJPEG directly, so
// a high ideal rate (60, the FPV standard) nudges the browser toward the camera's MJPEG mode. The
// `native` source is the real fix when the browser still won't deliver. 'auto' keeps the 60 nudge; an
// explicit 30 asks for the lower rate.
function cameraConstraints(res: VideoResolution, fps: CameraFps): MediaTrackConstraints {
  const ideal = fps === '30' ? 30 : 60;
  return { ...RES_DIMS[res], frameRate: { ideal } };
}

function mediaDevicesAvailable(): boolean {
  return typeof navigator !== 'undefined' && !!navigator.mediaDevices?.getUserMedia;
}

/** Check if RTCPeerConnection is available (WebRTC support in the WebView). */
export function isWebrtcAvailable(): boolean {
  return typeof RTCPeerConnection !== 'undefined';
}

/** Start a native device via the built-in ffmpeg→MJPEG server (no getUserMedia, no go2rtc). The chosen
 *  codec is the capture input format: MJPEG is stream-copied (`-c copy`, the camera's HW JPEG encoder
 *  does the work), raw/H.264 is transcoded to MJPEG for the `<img>` sink. */
async function startNativeMjpeg(
  sel: NativeSelection,
  id: string,
): Promise<{ url: string; transcode: string }> {
  return await invoke<{ url: string; transcode: string }>('video_native_mjpeg_start', {
    id,
    codec: sel.codec,
    width: sel.width,
    height: sel.height,
    fps: sel.fps,
  });
}

/** Stop the built-in MJPEG server. */
async function stopNativeMjpeg(): Promise<void> {
  await invoke('video_native_mjpeg_stop').catch(() => {});
}

/** Enumerate video input devices. Labels are only populated once permission has
 *  been granted (i.e. after the first successful getUserMedia). */
export async function enumerateVideoDevices(): Promise<void> {
  if (!mediaDevicesAvailable()) {
    patch({ error: 'Camera API unavailable' });
    return;
  }
  try {
    const all = await navigator.mediaDevices.enumerateDevices();
    // WebKitGTK lists every V4L2 node as a separate videoinput, and a UVC / Windows-Hello camera
    // exposes several (colour + IR + metadata). The non-colour nodes can't be opened as a normal
    // stream → selecting one throws and the picker snaps back to "Default". Keep one entry per
    // physical camera: dedup by groupId (else label), keeping the first node (the colour one,
    // /dev/video0 before video1…).
    const seen = new Set<string>();
    const devices = all
      .filter((d) => d.kind === 'videoinput')
      .filter((d) => {
        // Label first: WebKitGTK gives each V4L2 node of one camera the *same* label but often a
        // *different* groupId, so groupId-dedup would keep the duplicates. deviceId is the last resort.
        const key = d.label || d.groupId || d.deviceId;
        if (seen.has(key)) return false;
        seen.add(key);
        return true;
      })
      .map((d, i) => ({ deviceId: d.deviceId, label: d.label || `Camera ${i + 1}` }));
    patch({ devices });
    // Drop a stale selection that no longer exists.
    const sel = get(videoState).deviceId;
    if (sel && !devices.some((d) => d.deviceId === sel)) patch({ deviceId: null });
  } catch (e) {
    patch({ error: `Device enumeration failed: ${e}` });
  }
}

/** Re-resolve the persisted native device against a fresh enumeration.
 *
 *  Device ids are only stable up to a point: AVFoundation hands out a running **index** (`"0"`, `"1"`)
 *  and V4L2 a `/dev/videoN` path — both renumber when hardware is re-plugged or the machine reboots, so
 *  a saved id can silently denote a *different* camera. Checking "does the id still exist" (the old
 *  behaviour) can't see that. The saved name is the tie-breaker: keep the id while it still names the
 *  same device, otherwise follow the name to its new id, and only fall back to the first device when
 *  neither matches. */
function resolveNativeDevice(
  devices: NativeDevice[],
  id: string | null,
  name: string | null,
): NativeDevice | null {
  if (devices.length === 0) return null;
  const byId = id ? devices.find((d) => d.id === id) : undefined;
  if (byId && (!name || byId.name === name)) return byId;
  const byName = name ? devices.find((d) => d.name === name) : undefined;
  return byName ?? byId ?? devices[0];
}

/** Enumerate native capture devices via the Rust backend (V4L2/DirectShow/AVFoundation), then repair
 *  the persisted selection against the fresh list (see `resolveNativeDevice`). */
export async function enumerateNativeDevices(): Promise<void> {
  try {
    const devices = await invoke<NativeDevice[]>('video_list_native_devices');
    patch({ nativeDevices: devices });
    const st = get(videoState);
    const want = resolveNativeDevice(devices, st.nativeDevice, st.nativeDeviceName);
    if (!want) {
      if (st.nativeDevice) patch({ nativeDevice: null, nativeDeviceName: null, nativeModes: [] });
      return;
    }
    if (want.id !== st.nativeDevice) {
      // Genuinely a different device (first run, hardware swapped, or the id moved) → full switch.
      await setNativeDevice(want.id);
    } else {
      // Same device: only backfill the name (nothing to restart) and refresh its modes.
      if (want.name !== st.nativeDeviceName) patch({ nativeDeviceName: want.name });
      await probeNativeDevice(want.id);
    }
  } catch {
    // Native capture not available (no ffmpeg / unsupported platform) — that's fine.
    patch({ nativeDevices: [] });
  }
}

/** Probe the selected device's supported modes and repair the current selection against them. An
 *  empty probe (macOS / ffmpeg missing) keeps the persisted selection and the full curated catalog. */
export async function probeNativeDevice(id: string): Promise<void> {
  let modes: CaptureMode[] = [];
  try {
    modes = await invoke<CaptureMode[]>('video_probe_device', { id });
  } catch {
    modes = [];
  }
  const cur = get(videoState).nativeSel;
  const sel = modes.length === 0 ? cur : validateSelection(modes, cur);
  patch({ nativeModes: modes, nativeSel: sel });
  savePrefs();
}

function stopTracks(): void {
  const s = get(videoStream);
  if (s) for (const tr of s.getTracks()) tr.stop();
  videoStream.set(null);
  closeRtc();
}

// ── RTSP via WebRTC (go2rtc) ─────────────────────────────────────────
// go2rtc ingests the RTSP source and republishes it as WebRTC: the browser negotiates a
// peer connection (SDP exchange proxied through Rust to avoid CORS) and gets a real, native,
// low-latency MediaStream — which slots straight into the shared `videoStream` so every sink
// renders it via srcObject exactly like the camera (no fMP4/MSE/captureStream gymnastics).
let rtcConn: RTCPeerConnection | null = null;

function closeRtc(): void {
  if (!rtcConn) return;
  const pc = rtcConn;
  rtcConn = null;
  try {
    pc.getReceivers().forEach((r) => r.track?.stop());
    pc.close();
  } catch {
    /* ignore */
  }
  videoRtcStats.set(null);
}

/** Resolve once ICE gathering completes (or a short timeout) — HTTP signaling can't trickle,
 *  so the offer must already carry candidates; on loopback they gather almost instantly. */
function waitIceGathering(pc: RTCPeerConnection): Promise<void> {
  if (pc.iceGatheringState === 'complete') return Promise.resolve();
  return new Promise((resolve) => {
    const finish = () => {
      pc.removeEventListener('icegatheringstatechange', check);
      resolve();
    };
    const check = () => {
      if (pc.iceGatheringState === 'complete') finish();
    };
    pc.addEventListener('icegatheringstatechange', check);
    setTimeout(finish, 800);
  });
}

/** Open (or re-open) the webcam with the current device/resolution selection. */
export async function startVideo(): Promise<void> {
  if (!mediaDevicesAvailable()) {
    patch({ enabled: true, status: 'error', error: 'Camera API unavailable' });
    return;
  }
  stopTracks();
  patch({ kind: 'camera', enabled: true, status: 'starting', error: null });
  savePrefs(); // remember the intent immediately
  const st = get(videoState);
  const base: MediaTrackConstraints = cameraConstraints(st.resolution, st.cameraFps);
  try {
    let stream: MediaStream;
    try {
      const video: MediaTrackConstraints = { ...base };
      if (st.deviceId) video.deviceId = { exact: st.deviceId };
      stream = await navigator.mediaDevices.getUserMedia({ video, audio: false });
    } catch (e) {
      // Saved device gone / busy / over-constrained → fall back to the default
      // device (e.g. the camera was unplugged or is on another machine).
      const name = e instanceof Error ? e.name : '';
      if (st.deviceId && ['OverconstrainedError', 'NotFoundError', 'NotReadableError'].includes(name)) {
        patch({ deviceId: null });
        savePrefs();
        stream = await navigator.mediaDevices.getUserMedia({ video: { ...base }, audio: false });
      } else {
        throw e;
      }
    }
    videoStream.set(stream);
    const track = stream.getVideoTracks()[0];
    const s = track?.getSettings();
    const caps = track?.getCapabilities?.() as MediaTrackCapabilities | undefined;
    const aspect = s?.width && s?.height ? s.width / s.height : get(videoState).aspect;
    // Diagnostic: log the camera's full capability set so we can see whether a
    // high-fps (MJPEG) mode is even being offered to the browser.
    console.log('[video] track settings', s, 'capabilities', caps);
    patch({
      status: 'live',
      aspect,
      width: s?.width ?? null,
      height: s?.height ?? null,
      frameRate: s?.frameRate ?? null,
      capFrameRate: caps?.frameRate?.max ?? null,
      error: null,
    });
    // Labels are available now → refresh the device list.
    await enumerateVideoDevices();
  } catch (e) {
    const err = e instanceof Error ? e.message : String(e);
    patch({ status: 'error', error: err });
  }
}

/** Bring the feed up on the MJPEG image path: one ffmpeg reads the source and its `-f mpjpeg` output
 *  is broadcast by our own HTTP server, which the sinks read. Returns false if it could not be
 *  started — the caller decides whether that means a reconnect or a fall-through.
 *
 *  Used from two places, and the second is not a fallback for a broken WebView: an MJPEG source
 *  cannot be carried over WebRTC by any browser, so it needs this path even where WebRTC works.
 *  `requireCopy` is what makes that safe to try blindly — see the call site.
 *
 *  **Deliberately not through go2rtc.** go2rtc drives an `ffmpeg:` source by having ffmpeg publish
 *  back into it over RTSP/TCP, which for an already-MJPEG stream means packetising into RTP/JPEG,
 *  reassembling and repacking. Measured over the same 120 s: zero arrival gaps above 200 ms at the
 *  source, 69 of them (each ~338 ms) at go2rtc's output. That was the freeze. The transport choice is
 *  irrelevant here for the same reason it was before — ffmpeg negotiates it and reads UDP-only
 *  servers, which is why nothing forces `-rtsp_transport`. */
async function startMjpegPath(url: string, requireCopy = false): Promise<boolean> {
  // The image path is ffmpeg's alone now, for the copy as much as for the transcode. Missing ffmpeg
  // is a dead end no reconnect fixes.
  const ffmpeg = await invoke<string | null>('video_ffmpeg_status').catch(() => null);
  if (!ffmpeg) {
    logVideo('warn', 'MJPEG path needs ffmpeg and it is not installed');
    if (!requireCopy) {
      patch({ status: 'error', error: get(t)('video.ffmpegNativeMissing'), reconnecting: false, reconnectAttempt: 0 });
    }
    return false;
  }
  try {
    // Hardware H.264 decoding (Pi 3/4 class boards) is the backend's call — but only while this feed
    // has actually been delivering. Two failures in a row and we ask for the software decoder, so a
    // host that passes the backend's probe yet cannot hold a live stream on the hardware path still
    // ends up with a picture instead of an endless reconnect loop.
    const res = await invoke<{ url: string; transcode: string }>('video_rtsp_mjpeg_start', {
      url,
      requireCopy,
      // Two vetoes, either of which forces the software transcode: the user's explicit setting, and
      // the automatic one after repeated failures.
      allowHwDecode: !get(videoState).disableHwAccel && rtspMjpegFailures < 2,
    });
    // Stopped while we were away? The awaits above straddle a Stop easily — the backend's hardware
    // probe alone can take seconds on a first start. Publishing 'live' + a URL regardless put the
    // sinks back on screen, which reconnects a consumer and keeps a small board's CPU pinned after
    // the user stopped it.
    if (get(videoState).kind !== 'rtsp' || !get(videoState).enabled) {
      void stopNativeMjpeg();
      return true; // not a failure — the user stopped it
    }
    // 'ffmpeg', always: this path IS an ffmpeg reader, whatever the transport setting says.
    patch({ status: 'live', mjpegUrl: res.url, activeTranscode: res.transcode, error: null, rtspEngine: 'ffmpeg', reconnecting: false, reconnectAttempt: 0 });
    return true;
  } catch (e) {
    logVideo('warn', `RTSP (MJPEG path) failed: ${e instanceof Error ? e.message : String(e)}`);
    return false;
  }
}

/** Register the source with go2rtc and complete one WebRTC negotiation. Throws on failure. */
async function negotiateWebrtc(url: string, useFfmpeg: boolean): Promise<void> {
  await invoke('video_webrtc_start', { url, useFfmpeg, mjpeg: false });

  const pc = new RTCPeerConnection({ iceServers: [] });
  rtcConn = pc;
  pc.addTransceiver('video', { direction: 'recvonly' });
  pc.oniceconnectionstatechange = () => {
    if (rtcConn === pc) logVideo('debug', `ICE state: ${pc.iceConnectionState}`);
  };
  pc.ontrack = (e) => {
    if (rtcConn !== pc) return;
    const stream = e.streams[0] ?? new MediaStream([e.track]);
    videoStream.set(stream);
    patch({ status: 'live', error: null });
  };
  pc.onconnectionstatechange = () => {
    // A genuine drop (failed) enters the infinite reconnect loop; 'closed' is our own teardown.
    if (rtcConn === pc && pc.connectionState === 'failed') {
      logVideo('warn', 'WebRTC peer connection failed');
      void logIceOutcome(pc);
      scheduleRtspReconnect();
    }
  };
  const offer = await pc.createOffer();
  await pc.setLocalDescription(offer);
  await waitIceGathering(pc);
  if (rtcConn !== pc) return; // stopped while gathering
  const local = candidateLines(pc.localDescription?.sdp ?? offer.sdp ?? '');
  lastIce = { local, remote: [] };
  logVideo('debug', `ICE local candidates (${local.length}, gathering=${pc.iceGatheringState}): ${local.join(' · ') || 'NONE'}`);
  const answerSdp = await invoke<string>('video_webrtc_offer', {
    sdp: pc.localDescription?.sdp ?? offer.sdp,
  });
  if (rtcConn !== pc) return;
  const remote = candidateLines(answerSdp);
  lastIce.remote = remote;
  logVideo('debug', `ICE remote candidates (${remote.length}): ${remote.join(' · ') || 'NONE'}`);
  await pc.setRemoteDescription({ type: 'answer', sdp: answerSdp });
  patch({ rtspEngine: useFfmpeg ? 'ffmpeg' : 'native' });
}

// ── ICE diagnostics ──────────────────────────────────────────────────────────────────────────────
// Everything about go2rtc is logged; about our own peer connection, nothing was. When a stream came
// up black with `peer connection failed` and no inbound-rtp, there was no way to tell whether the
// browser had even produced a candidate able to reach go2rtc's loopback address — the failure looked
// identical from outside whatever the cause.
//
// Level split: the per-negotiation lines are `debug` (one pair of them on every stream start would
// otherwise fill a tester's log for a feed that is working), while the failure dump repeats them at
// `warn`. So normal operation stays quiet and a broken connection still reports itself in full,
// without anyone having to reproduce it under `--debug` first.

/** Candidate summaries of the negotiation in progress, replayed by `logIceOutcome` when it fails. */
let lastIce: { local: string[]; remote: string[] } = { local: [], remote: [] };

/** `a=candidate:1 1 udp 2130706431 127.0.0.1 51319 typ host …` → `host/udp 127.0.0.1:51319`. */
function candidateLines(sdp: string): string[] {
  return sdp
    .split(/\r?\n/)
    .filter((l) => l.startsWith('a=candidate:'))
    .map((l) => {
      const p = l.slice('a=candidate:'.length).split(' ');
      const typ = p[p.indexOf('typ') + 1] ?? '?';
      return `${typ}/${p[2] ?? '?'} ${p[4] ?? '?'}:${p[5] ?? '?'}`;
    });
}

/** Which pairs ICE actually tried, and how far each got. `requestsSent` with no `responsesReceived`
 *  means our checks went unanswered; no pairs at all means the two sides never had a route to try. */
async function logIceOutcome(pc: RTCPeerConnection): Promise<void> {
  interface PairStats extends RTCStats {
    localCandidateId: string;
    remoteCandidateId: string;
    state?: string;
    nominated?: boolean;
    requestsSent?: number;
    responsesReceived?: number;
  }
  interface CandStats extends RTCStats {
    address?: string;
    port?: number;
    protocol?: string;
    candidateType?: string;
  }
  try {
    const stats = await pc.getStats();
    const named = new Map<string, string>();
    stats.forEach((r) => {
      if (r.type !== 'local-candidate' && r.type !== 'remote-candidate') return;
      const c = r as CandStats;
      named.set(c.id, `${c.candidateType ?? '?'}/${c.protocol ?? '?'} ${c.address ?? '?'}:${c.port ?? '?'}`);
    });
    const pairs: string[] = [];
    stats.forEach((r) => {
      if (r.type !== 'candidate-pair') return;
      const p = r as PairStats;
      const from = named.get(p.localCandidateId) ?? p.localCandidateId;
      const to = named.get(p.remoteCandidateId) ?? p.remoteCandidateId;
      pairs.push(
        `${from} → ${to} [${p.state ?? '?'}${p.nominated ? ' nominated' : ''}]` +
          ` sent=${p.requestsSent ?? 0} answered=${p.responsesReceived ?? 0}`,
      );
    });
    logVideo('warn', `ICE pairs tried (${pairs.length}): ${pairs.join(' · ') || 'NONE'}`);
    logVideo(
      'warn',
      `ICE candidates were local (${lastIce.local.length}): ${lastIce.local.join(' · ') || 'NONE'}` +
        ` — remote (${lastIce.remote.length}): ${lastIce.remote.join(' · ') || 'NONE'}`,
    );
  } catch (e) {
    logVideo('warn', `ICE stats unavailable: ${e instanceof Error ? e.message : String(e)}`);
  }
}

// ── RTSP auto-reconnect (infinite until frames return or the user stops) ──────────────────────────
// The UAV-Link Pi is UDP-only with a "session lottery" (its FPS-watchdog EOSes bad sessions), and a
// flying aircraft can drop into a radio hole. So the client reconnects INDEFINITELY and visibly until
// frames flow again — only an explicit stop ends it. Trigger: a frame timeout (no new frames/bytes on
// the WebRTC inbound track for RTSP_STALL_MS). UDP stays (latency over resends, per the UAV-Link design).
// Two-phase stall detection (LTE-tested against the UAV-Link Pi):
// - CONNECT phase (no frame seen yet): a fresh session that delivers nothing is a losing ticket in
//   the server's session lottery → re-roll fast (also beats the Pi watchdog's ~10 s kill window).
// - LIVE phase (frames were flowing): brief UDP gaps (LTE fluctuation) recover in-stream on their
//   own — only a sustained silence means the session really died (watchdog EOS / radio hole).
const RTSP_STALL_CONNECT_MS = 4000;
const RTSP_STALL_LIVE_MS = 10_000;
const RTSP_RECONNECT_BACKOFF_MS = 1500;
let rtspMonitor: ReturnType<typeof setInterval> | undefined;
let rtspReconnectTimer: ReturnType<typeof setTimeout> | undefined;
/** Consecutive MJPEG `<img>` failures on the RTSP feed; resets as soon as frames arrive. Used to
 *  give up on the hardware decoder rather than loop forever on a host it doesn't work on. */
let rtspMjpegFailures = 0;

/** `video/H264` → `H.264`. The RTP mime type is the only place the negotiated codec is named, and it
 *  spells the common ones without the dot everyone reads them with. */
function prettyCodec(mimeType: string): string {
  const name = (mimeType.split('/')[1] ?? mimeType).toUpperCase();
  return { H264: 'H.264', H265: 'H.265', HEVC: 'H.265' }[name] ?? name;
}

function clearRtspTimers(): void {
  if (rtspMonitor) { clearInterval(rtspMonitor); rtspMonitor = undefined; }
  if (rtspReconnectTimer) { clearTimeout(rtspReconnectTimer); rtspReconnectTimer = undefined; }
}

/** Watch the inbound WebRTC stats; if frames/bytes stop advancing the link has stalled → reconnect.
 *  Two-phase: 4 s until the FIRST frames arrive (dead session → re-roll fast), 10 s once the stream
 *  was delivering (tolerate transient UDP gaps before a full reconnect). */
function startRtspStallMonitor(pc: RTCPeerConnection): void {
  if (rtspMonitor) clearInterval(rtspMonitor);
  let last = -1;
  let sawFrames = false;
  let lastChange = performance.now();
  let warnedNoReport = false;
  let prevFrames = -1;
  let prevDecoded = 0;
  let prevBytes = 0;
  let prevAt = performance.now();
  let statTick = 0;
  videoRtcStats.set(null);
  rtspMonitor = setInterval(() => {
    if (rtcConn !== pc) { if (rtspMonitor) { clearInterval(rtspMonitor); rtspMonitor = undefined; } return; }
    void pc.getStats().then((stats) => {
      if (rtcConn !== pc) return;
      let frames = 0;
      let bytes = 0;
      let decoded = 0;
      let dropped = 0;
      let lost = 0;
      let engineFps: number | null = null;
      let freezes: number | null = null;
      let freezeMs: number | null = null;
      let jitterMs: number | null = null;
      let playoutMs: number | null = null;
      let codecId: string | undefined;
      let sawReport = false;
      stats.forEach((report) => {
        if (report.type !== 'inbound-rtp') return;
        // WebKit has historically reported the legacy `mediaType` instead of the spec's `kind`, and a
        // report carrying neither is counted too: mistaking a healthy feed for a dead one would put the
        // stream into a permanent reconnect loop, which is far worse than a missed stall.
        // The pipeline-quality fields beyond framesReceived/bytesReceived are optional per spec and
        // engine-dependent — each is read defensively and shown as absent when not reported.
        const rr = report as RTCInboundRtpStreamStats & {
          mediaType?: string;
          framesDecoded?: number;
          framesDropped?: number;
          framesPerSecond?: number;
          freezeCount?: number;
          totalFreezesDuration?: number;
          jitter?: number;
          jitterBufferDelay?: number;
          jitterBufferEmittedCount?: number;
          packetsLost?: number;
        };
        const kind = rr.kind ?? rr.mediaType;
        if (kind && kind !== 'video') return;
        sawReport = true;
        frames += rr.framesReceived ?? 0;
        bytes += rr.bytesReceived ?? 0;
        decoded += rr.framesDecoded ?? 0;
        dropped += rr.framesDropped ?? 0;
        lost += rr.packetsLost ?? 0;
        if (rr.framesPerSecond !== undefined) engineFps = rr.framesPerSecond;
        if (rr.freezeCount !== undefined) freezes = rr.freezeCount;
        if (rr.totalFreezesDuration !== undefined) freezeMs = rr.totalFreezesDuration * 1000;
        if (rr.jitter !== undefined) jitterMs = rr.jitter * 1000;
        if (rr.jitterBufferDelay !== undefined && rr.jitterBufferEmittedCount) {
          playoutMs = (rr.jitterBufferDelay / rr.jitterBufferEmittedCount) * 1000;
        }
        if (rr.codecId) codecId = rr.codecId;
      });
      // The codec lives in its own report, referenced by the inbound one.
      const codecMime = codecId
        ? (stats.get(codecId) as (RTCStats & { mimeType?: string }) | undefined)?.mimeType
        : undefined;
      if (!sawReport) {
        // No inbound stats at all — we cannot measure, so we must not judge. Say so once.
        if (!warnedNoReport) {
          warnedNoReport = true;
          logVideo('warn', 'RTSP stall monitor: no inbound-rtp stats from this WebView — stall detection is inactive');
        }
        return;
      }
      const progress = frames || bytes;
      const now = performance.now();
      // Publish the per-second pipeline snapshot (needs two polls for the rates).
      if (prevFrames >= 0) {
        const dt = (now - prevAt) / 1000;
        if (dt > 0) {
          videoRtcStats.set({
            recvFps: Math.max(0, (frames - prevFrames) / dt),
            decodeFps: Math.max(0, (decoded - prevDecoded) / dt),
            engineFps,
            framesDropped: dropped,
            packetsLost: lost,
            freezeCount: freezes,
            freezeMs,
            jitterMs,
            playoutDelayMs: playoutMs,
            kbps: Math.max(0, ((bytes - prevBytes) * 8) / 1000 / dt),
            codec: codecMime ? prettyCodec(codecMime) : null,
          });
        }
      }
      prevFrames = frames;
      prevDecoded = decoded;
      prevBytes = bytes;
      prevAt = now;
      // A periodic trace of the same numbers, so a tester's Debug-level log shows where a jittery
      // picture loses its frames (receive vs decode vs playout) without the Debug Monitor.
      statTick++;
      if (statTick % 10 === 0) {
        // Helper instead of inline ternaries: TS does not apply assignments made inside the forEach
        // callback to the outer flow, so `x !== null` on these would narrow to `never`.
        const ms = (v: number | null, digits = 0): string => (v === null ? '–' : `${v.toFixed(digits)}ms`);
        logVideo(
          'debug',
          `RTC inbound: recv=${frames} decoded=${decoded} dropped=${dropped} lost=${lost}` +
            ` freezes=${freezes ?? '–'}/${ms(freezeMs)} jitter=${ms(jitterMs, 1)} playoutDelay=${ms(playoutMs)}`,
        );
      }
      if (progress !== last) {
        if (last >= 0 && progress > last) sawFrames = true; // real advance, not the initial read
        last = progress;
        lastChange = now;
      } else if (now - lastChange > (sawFrames ? RTSP_STALL_LIVE_MS : RTSP_STALL_CONNECT_MS)) {
        // The frames/bytes pair is the diagnosis: bytes > 0 with frames == 0 means the media arrives
        // but nothing decodes it (a missing H.264 decoder in the WebView's GStreamer stack); bytes == 0
        // means nothing arrives at all (transport / source).
        logVideo(
          'warn',
          `RTSP stalled after ${((now - lastChange) / 1000).toFixed(1)}s ` +
            `(${sawFrames ? 'live feed went silent' : 'no first frame'}; ` +
            `framesReceived=${frames} bytesReceived=${bytes}) — reconnecting`,
        );
        scheduleRtspReconnect();
      }
    }).catch(() => {});
  }, 1000);
}

/** Enter/continue the infinite reconnect loop: mark the visible reconnecting state and re-attempt
 *  after a short backoff. Guarded so an explicit stop (kind changed / disabled) ends it. */
function scheduleRtspReconnect(): void {
  const st = get(videoState);
  if (st.kind !== 'rtsp' || !st.enabled) return; // user stopped → do not reconnect
  // The loop is unbounded by design, so logging every attempt would fill the file on a source that
  // never comes back. Log the first few, then every tenth — enough to see it is still going.
  const attempt = st.reconnectAttempt + 1;
  if (attempt <= 3 || attempt % 10 === 0) {
    logVideo('warn', `RTSP reconnect attempt ${attempt} (${st.rtspUrl}, transport=${st.rtspTransport})`);
  }
  clearRtspTimers();
  closeRtc();
  // Release the image path too. Its ffmpeg holds the source open, and a WebRTC-capable host retries
  // WebRTC on every cycle — without this, each attempt stacked a second reader on the same stream.
  void stopNativeMjpeg();
  videoStream.set(null);
  patch({
    reconnecting: true,
    reconnectAttempt: st.reconnectAttempt + 1,
    status: 'starting',
    rtspEngine: null,
    mjpegUrl: null,
  });
  rtspReconnectTimer = setTimeout(() => { void startRtsp({ reconnect: true }); }, RTSP_RECONNECT_BACKOFF_MS);
}

/** Negotiate the RTSP source honouring the connection's transport: udp → ffmpeg reader (reads
 *  UDP-only servers like the UAV-Link Pi); tcp → native go2rtc client; auto → native, then ffmpeg. */
async function negotiateRtsp(url: string, transport: RtspTransport): Promise<void> {
  if (transport === 'udp') {
    await negotiateWebrtc(url, true);
  } else if (transport === 'tcp') {
    await negotiateWebrtc(url, false);
  } else {
    try {
      await negotiateWebrtc(url, false); // native go2rtc RTSP client
    } catch (nativeErr) {
      logVideo('warn', `native go2rtc RTSP reader failed, retrying via ffmpeg: ${nativeErr instanceof Error ? nativeErr.message : String(nativeErr)}`);
      closeRtc();
      if (get(videoState).kind !== 'rtsp' || !get(videoState).enabled) return; // stopped meanwhile
      await negotiateWebrtc(url, true); // ffmpeg reader fallback
    }
  }
}

/** Open (or re-open) the RTSP feed via go2rtc, honouring the active transport. Once live, a stall
 *  monitor watches for frame timeouts; any failure/drop enters the infinite reconnect loop (until
 *  frames return or the user stops). `reconnect` distinguishes a loop retry from a fresh start. */
export async function startRtsp(opts?: { reconnect?: boolean }): Promise<void> {
  const reconnect = opts?.reconnect ?? false;
  clearRtspTimers();
  stopTracks(); // release the camera / previous peer connection
  const st = get(videoState);
  const url = st.rtspUrl.trim();
  const transport = st.rtspTransport;
  if (!url) {
    patch({ kind: 'rtsp', enabled: true, status: 'error', error: 'No RTSP URL', reconnecting: false, reconnectAttempt: 0 });
    return;
  }
  patch({
    kind: 'rtsp',
    enabled: true,
    status: 'starting',
    error: null,
    rtspEngine: null,
    mjpegUrl: null,
    ...(reconnect ? {} : { reconnecting: false, reconnectAttempt: 0 }),
  });
  if (!reconnect) {
    savePrefs();
    rtspMjpegFailures = 0; // a deliberate (re)start gets the hardware decoder another chance
  }

  // MJPEG fallback for webviews without RTCPeerConnection (rare in Tauri). Decided BEFORE the engine
  // gate below: the image path is ffmpeg's alone now and never touches go2rtc, so demanding the
  // engine here would block a machine that already has everything this path needs.
  if (!isWebrtcAvailable()) {
    // Degraded mode, and a silent one: it needs a WebView that renders the image path, and for an
    // H.264 source a transcode as well. Worth a warning every time, not just a console line.
    if (!reconnect) {
      logVideo('warn', 'WebRTC is unavailable in this WebView — falling back to the MJPEG image path');
      logVideo('info', `RTSP start ${url} (transport=${transport}, webrtc=false, go2rtc not used)`);
    }
    if (!(await startMjpegPath(url))) scheduleRtspReconnect();
    return;
  }

  if (!reconnect) {
    // A missing engine cannot be fixed by retrying, so it must not enter the loop: before this, an
    // auto-start without go2rtc installed produced an endless "Reconnecting… (n)" with no explanation
    // (seen on the Pi). Checked once per fresh start — a reconnect attempt inherits the verdict.
    const engine = await invoke<string | null>('video_go2rtc_status').catch(() => null);
    if (!engine) {
      logVideo('warn', 'RTSP start aborted: the go2rtc engine is not installed');
      patch({ status: 'error', error: get(t)('video.engineMissing'), reconnecting: false, reconnectAttempt: 0 });
      return;
    }
    logVideo('info', `RTSP start ${url} (transport=${transport}, webrtc=true, engine=${engine})`);
  }

  try {
    // Hard cap on one attempt: the backend invokes are bounded (10/15 s reqwest timeouts), but if
    // any path ever hangs anyway, the loop must keep cycling instead of freezing mid-"Reconnecting…"
    // (a wedged RTSP server once parked go2rtc's answer indefinitely — observed with the UAV-Link Pi).
    await Promise.race([
      negotiateRtsp(url, transport),
      new Promise<never>((_, reject) =>
        setTimeout(() => reject(new Error('RTSP negotiation timeout')), 20_000),
      ),
    ]);
    if (get(videoState).kind !== 'rtsp' || !get(videoState).enabled) return; // stopped during negotiation
    patch({ reconnecting: false, reconnectAttempt: 0 });
    if (rtcConn) startRtspStallMonitor(rtcConn);
  } catch (err) {
    logVideo('warn', `RTSP connect failed: ${err instanceof Error ? err.message : String(err)}`);
    // An MJPEG source cannot travel over WebRTC at all — its video codecs are H.264/VP8/VP9/AV1 — so
    // even a WebRTC-capable WebView has to use the image path for one, and negotiation failing is the
    // first moment we could know. `requireCopy` keeps this honest: the image path is accepted only if
    // the backend reports an actual stream copy, i.e. the source really was MJPEG. Anything else means
    // the source was H.264 and WebRTC failed for some other reason (server down, transport), where
    // silently settling for a transcode would be a permanent downgrade — so that keeps reconnecting.
    if (await startMjpegPath(url, true)) {
      logVideo('warn', 'source is MJPEG, which WebRTC cannot carry — switched to the image path');
      return;
    }
    scheduleRtspReconnect();
  }
}

/** A sink's MJPEG `<img>` failed to load. Unlike the WebRTC path there are no stats to poll on a
 *  multipart feed — the element's own `error` event is the ONLY signal that it died, or that this
 *  WebView can't render `multipart/x-mixed-replace` at all. Without this the feed just showed the
 *  WebView's broken-image placeholder while the app still claimed to be `live` (reported on Debian /
 *  WebKitGTK). RTSP re-enters the reconnect loop; native capture has no remote to retry, so it reports
 *  an error. Idempotent — every sink fires it, and the first one to arrive does the work. */
export function reportMjpegError(): void {
  const st = get(videoState);
  if (!st.enabled || !st.mjpegUrl) return;
  if (st.kind === 'rtsp') {
    logVideo('warn', `MJPEG image failed to load (${st.mjpegUrl}) — reconnecting`);
    rtspMjpegFailures++;
    scheduleRtspReconnect();
    return;
  }
  logVideo('warn', `MJPEG image failed to load (${st.mjpegUrl})`);
  patch({ status: 'error', mjpegUrl: null, error: get(t)('video.mjpegLoadFailed') });
}

// ── Off-thread MJPEG reader ──────────────────────────────────────────────────────────────────────
// Where the WebView can do it, the multipart feed is read, decoded and drawn by a worker instead of
// by an <img> on the main thread (see controllers/mjpegWorker.ts). The surfaces branch on the
// `canvasSink` store; the stream itself follows `mjpegUrl`, which every path — RTSP MJPEG, native
// capture, stop, reconnect — already sets or clears, so one subscription covers all of them. Both
// calls are no-ops while the reader isn't in use.
setMjpegSinkHandlers({
  onSize: reportVideoSize,
  onError: reportMjpegError,
  onLog: (level, message) => logVideo(level, message),
});
let mjpegSinkUrl: string | null = null;
videoState.subscribe((s) => {
  if (s.mjpegUrl === mjpegSinkUrl) return;
  mjpegSinkUrl = s.mjpegUrl;
  if (mjpegSinkUrl) startMjpegSink(mjpegSinkUrl);
  else stopMjpegSink();
});

/** Open a native capture device via ffmpeg → the embedded MJPEG server → `<img>`. This path is
 *  deliberately independent of getUserMedia/WebKit: the backend enumerates and opens the exact device
 *  (V4L2/DirectShow/AVFoundation), so device selection + codec/resolution/framerate are reliable even
 *  where the browser's capture stack is flaky. Software `<img>` decode is the trade-off. */
export async function startNative(): Promise<void> {
  stopTracks();
  // Release a previous MJPEG capture FIRST. Its ffmpeg holds the device exclusively (DirectShow
  // always, V4L2 usually), so leaving it running makes the re-open below fail with a busy device —
  // which is exactly what a format/resolution/framerate change does: stop, then start with the new
  // capture spec.
  await stopNativeMjpeg();
  const st = get(videoState);
  const id = st.nativeDevice;
  if (!id) {
    patch({ kind: 'native', enabled: true, status: 'error', error: 'No capture device selected' });
    return;
  }
  patch({ kind: 'native', enabled: true, status: 'starting', error: null, rtspEngine: null, mjpegUrl: null });
  savePrefs();
  const sel = st.nativeSel;
  try {
    const { url, transcode } = await startNativeMjpeg(sel, id);
    // Same straddled-Stop hazard as the RTSP path: the backend holds this call until the capture
    // produces its first bytes, so a Stop in between runs its `stopNativeMjpeg` before this server
    // even exists. Undo it rather than announcing a feed nobody asked for any more.
    if (get(videoState).kind !== 'native' || !get(videoState).enabled) {
      void stopNativeMjpeg();
      return;
    }
    patch({
      status: 'live',
      mjpegUrl: url,
      activeTranscode: transcode,
      error: null,
      rtspEngine: 'ffmpeg',
      width: sel.width,
      height: sel.height,
      aspect: sel.width / sel.height,
    });
  } catch (e) {
    patch({ status: 'error', error: e instanceof Error ? e.message : String(e), mjpegUrl: null });
  }
}

/** Start whichever source kind is currently selected. */
export function startActive(): Promise<void> {
  const kind = get(videoState).kind;
  if (kind === 'rtsp') return startRtsp();
  if (kind === 'native') return startNative();
  return startVideo();
}

/** Stop the source and release the camera / go2rtc engine. */
export function stopVideo(): void {
  const wasBackend = get(videoState).kind === 'rtsp' || get(videoState).kind === 'native';
  clearRtspTimers(); // end the RTSP reconnect loop on an explicit stop
  stopTracks();
  if (wasBackend) {
    void invoke('video_webrtc_stop').catch(() => {});
    void stopNativeMjpeg();
  }
  patch({ enabled: false, status: 'off', error: null, rtspEngine: null, mjpegUrl: null, activeTranscode: null, reconnecting: false, reconnectAttempt: 0 });
  savePrefs();
}

export function toggleVideo(): void {
  if (get(videoState).enabled) stopVideo();
  else void startActive();
}

/** Switch source kind (camera ⇄ rtsp); restarts the new source if video was running. */
export async function setVideoKind(kind: VideoKind): Promise<void> {
  if (get(videoState).kind === kind) return;
  const wasEnabled = get(videoState).enabled;
  if (wasEnabled) stopVideo();
  patch({ kind, status: 'off', error: null });
  savePrefs();
  if (wasEnabled) await startActive();
}

export function setRtspUrl(rtspUrl: string): void {
  patch({ rtspUrl });
  savePrefs();
}

/** Set the active RTSP transport (udp/tcp/auto); restart if currently on a live RTSP feed. */
export async function setRtspTransport(transport: RtspTransport): Promise<void> {
  patch({ rtspTransport: transport });
  savePrefs();
  const st = get(videoState);
  if (st.enabled && st.kind === 'rtsp') await startRtsp();
}

function genRtspId(): string {
  try {
    return crypto.randomUUID();
  } catch {
    return `rtsp-${Date.now()}-${Math.round(Math.random() * 1e6)}`;
  }
}

/** Save the current URL + transport as a named entry in the connection list. Explicit action only —
 *  connections are NEVER auto-saved. Name defaults to the host (rename inline); dedupes by URL. */
export function saveRtspConnection(): void {
  const st = get(videoState);
  const url = st.rtspUrl.trim();
  if (!url) return;
  let host = url;
  try {
    host = new URL(url).host || url;
  } catch {
    /* keep the raw url as the name */
  }
  const list = st.rtspConnections.slice();
  const i = list.findIndex((c) => c.url === url);
  if (i >= 0) {
    list[i] = { ...list[i], transport: st.rtspTransport };
  } else {
    list.push({ id: genRtspId(), name: host, url, transport: st.rtspTransport });
  }
  patch({ rtspConnections: list });
  savePrefs();
}

/** Edit a saved connection (name / url / transport). */
export function updateRtspConnection(id: string, p: Partial<Omit<RtspConnection, 'id'>>): void {
  const list = get(videoState).rtspConnections.map((c) => (c.id === id ? { ...c, ...p } : c));
  patch({ rtspConnections: list });
  savePrefs();
}

/** Remove a saved connection. */
export function removeRtspConnection(id: string): void {
  const list = get(videoState).rtspConnections.filter((c) => c.id !== id);
  patch({ rtspConnections: list });
  savePrefs();
}

/** Load a saved connection into the active URL + transport and connect it. */
export async function selectRtspConnection(id: string): Promise<void> {
  const c = get(videoState).rtspConnections.find((x) => x.id === id);
  if (!c) return;
  if (get(videoState).kind !== 'rtsp' && get(videoState).enabled) stopVideo();
  patch({ kind: 'rtsp', rtspUrl: c.url, rtspTransport: c.transport });
  savePrefs();
  await startRtsp();
}

/** Switch native device: probe it, repair the selection, restart if live. The device *name* is stored
 *  alongside the id so an unstable id (AVFoundation index / `/dev/videoN`) can be re-resolved later. */
export async function setNativeDevice(id: string | null): Promise<void> {
  const name = id ? (get(videoState).nativeDevices.find((d) => d.id === id)?.name ?? null) : null;
  patch({ nativeDevice: id, nativeDeviceName: name });
  if (id) await probeNativeDevice(id);
  else patch({ nativeModes: [] });
  savePrefs();
  if (id && get(videoState).enabled && get(videoState).kind === 'native') await startNative();
}

/** Change native resolution; re-validate framerate; restart if live. */
export async function setNativeResolution(width: number, height: number): Promise<void> {
  const st = get(videoState);
  const sel = validateSelection(st.nativeModes, { ...st.nativeSel, width, height });
  patch({ nativeSel: sel });
  savePrefs();
  if (st.enabled && st.kind === 'native') await startNative();
}

/** Change native capture format (codec); re-validate resolution+framerate down the cascade; restart
 *  if live. */
export async function setNativeCodec(codec: string): Promise<void> {
  const st = get(videoState);
  const sel = validateSelection(st.nativeModes, { ...st.nativeSel, codec });
  patch({ nativeSel: sel });
  savePrefs();
  if (st.enabled && st.kind === 'native') await startNative();
}

/** Change native framerate; restart if live. */
export async function setNativeFramerate(fps: number): Promise<void> {
  const st = get(videoState);
  patch({ nativeSel: { ...st.nativeSel, fps } });
  savePrefs();
  if (st.enabled && st.kind === 'native') await startNative();
}

/** Change the getUserMedia framerate wish (camera path); restarts the stream if currently live. */
export async function setCameraFps(cameraFps: CameraFps): Promise<void> {
  patch({ cameraFps });
  savePrefs();
  if (get(videoState).enabled && get(videoState).kind === 'camera') await startVideo();
}

/** Switch device / resolution; restarts the stream if currently live. */
export async function setVideoDevice(deviceId: string | null): Promise<void> {
  patch({ deviceId });
  savePrefs();
  if (get(videoState).enabled) await startVideo();
}

export async function setVideoResolution(resolution: VideoResolution): Promise<void> {
  patch({ resolution });
  savePrefs();
  if (get(videoState).enabled) await startVideo();
}

export function setVideoMirror(mirror: boolean): void {
  patch({ mirror });
  savePrefs();
}

/** Force the software transcode regardless of what the backend's hardware probe found. Restarts a
 *  live RTSP feed so the change takes effect immediately — the decision is made when the source is
 *  registered with go2rtc, not per frame. */
export async function setDisableHwAccel(disableHwAccel: boolean): Promise<void> {
  if (get(videoState).disableHwAccel === disableHwAccel) return;
  patch({ disableHwAccel });
  savePrefs();
  const s = get(videoState);
  if (s.enabled && s.kind === 'rtsp') await startRtsp();
}

// ── Floating window ──────────────────────────────────────────────────
export function toggleFloating(): void {
  patch({ floating: !get(videoState).floating });
  savePrefs();
}

export function setFloatSnapped(floatSnapped: boolean): void {
  patch({ floatSnapped });
  savePrefs();
}

/** Free position (px). Snapping is decided by the caller (drag near corner). */
export function setFloatPos(floatX: number, floatY: number): void {
  patch({ floatX, floatY });
  savePrefs();
}

const FLOAT_MIN = 0.1;
const FLOAT_MAX = 0.3;
export function setFloatHeightFrac(frac: number): void {
  patch({ floatHeightFrac: Math.min(FLOAT_MAX, Math.max(FLOAT_MIN, frac)) });
  savePrefs();
}

// ── Map ⇄ video placement ────────────────────────────────────────────
/** Move the single map instance to a surface. Double-clicking a video calls this with that surface;
 *  the map jumps there and every other surface shows video. Fires a resize so Leaflet/Cesium re-fit
 *  to the new container size (the Map also has a ResizeObserver as a backstop). */
export function setMapLocation(loc: MapLocation): void {
  patch({ mapLocation: loc });
  if (typeof window !== 'undefined') {
    setTimeout(() => window.dispatchEvent(new Event('resize')), 60);
  }
}

/** Publish the video widget's on-screen rect so the map can overlay it in `widget` mode. No-op when
 *  unchanged — callers fire it from ResizeObserver/resize handlers, and a redundant patch would churn
 *  the store (and could feed an effect loop). */
export function setWidgetRect(rect: { x: number; y: number; w: number; h: number } | null): void {
  const cur = get(videoState).widgetRect;
  if (cur === rect) return;
  if (
    cur &&
    rect &&
    cur.x === rect.x &&
    cur.y === rect.y &&
    cur.w === rect.w &&
    cur.h === rect.h
  ) {
    return;
  }
  patch({ widgetRect: rect });
}

// ── Native Picture-in-Picture ────────────────────────────────────────
// PiP is bound to its source <video> element, so the source must be a
// persistently-mounted element (not the panel preview, which unmounts when the
// panel closes — that would kill the PiP). The app root registers a hidden video
// element here; `enterPiP()` pops it out into a free-floating OS window that
// survives closing the panel.
export const pipSupported = typeof document !== 'undefined' && !!document.pictureInPictureEnabled;

let pipEl: HTMLVideoElement | null = null;
export function registerPiPElement(el: HTMLVideoElement | null): void {
  pipEl = el;
}

export async function enterPiP(): Promise<void> {
  const el = pipEl as (HTMLVideoElement & { requestPictureInPicture?: () => Promise<unknown> }) | null;
  try {
    if (
      el?.requestPictureInPicture &&
      typeof document !== 'undefined' &&
      document.pictureInPictureEnabled &&
      document.pictureInPictureElement !== el
    ) {
      await el.requestPictureInPicture();
    }
  } catch (e) {
    console.warn('[video] Picture-in-Picture failed', e);
  }
}

/** Delay before auto-starting the Linux `camera` source, so the UI paints first (see `initVideo`). */
const LINUX_CAMERA_AUTOSTART_DELAY_MS = 1200;

/** One-time record of what this WebView can actually play. WebRTC decides the entire video strategy —
 *  with it, H.264 goes straight to the decoder; without it, the expensive MJPEG transcode is the only
 *  way — and on Linux that varies by distro and build in ways nothing else reveals.
 *  `webkitRTCPeerConnection` is checked too, so a merely *prefixed* implementation can't masquerade as
 *  "no WebRTC at all". MediaSource/captureStream are logged because they were the third candidate path
 *  (fMP4 into the WebView's own decoder): WebKitGTK has MediaSource but **no** `captureStream`, so an
 *  MSE-driven element cannot be published to Kite's shared multi-sink stream — that is why the path was
 *  built, measured to never once engage on a Pi, and removed again. */
function logWebViewMediaSupport(): void {
  const has = (name: string) => name in globalThis;
  logVideo(
    'info',
    `WebView media support: RTCPeerConnection=${has('RTCPeerConnection')} ` +
      `webkitRTCPeerConnection=${has('webkitRTCPeerConnection')} ` +
      `MediaSource=${has('MediaSource')} ManagedMediaSource=${has('ManagedMediaSource')} ` +
      `captureStream=${typeof document !== 'undefined' && 'captureStream' in document.createElement('video')}`,
  );
}

/**
 * App-startup hook: enumerate devices and, if video was running at last close,
 * auto-start it with the persisted settings (device falls back to default if the
 * saved one is gone). Call once, client-side.
 */
export async function initVideo(): Promise<void> {
  logWebViewMediaSupport();
  // The backend's word that a live MJPEG source died (`MJPEG_ENDED_EVENT` in `mjpeg_server.rs`).
  // The off-thread reader notices by itself because it reads the stream — the `<img>` fallback
  // cannot: on WebKit a multipart `<img>` fires **no** error event when the server closes mid-stream
  // (measured on 2.52.5), so the picture sat on a dead `src` with `complete` still true and nothing
  // ever started a reconnect. One signal, same on every platform and both render paths.
  void listen('video-mjpeg-ended', () => reportMjpegError()).catch(() => {});
  // Skip getUserMedia enumeration at startup on Linux: it drives WebKit's GStreamer capture stack
  // (pipewire), which hangs ~35 s and freezes launch on boxes with an unreachable pipewire (the
  // symptom the native/MJPEG path was meant to avoid). Only the `camera` source needs this list, and
  // it's enumerated lazily when the panel shows the camera dropdown. Windows/macOS enumerate fast.
  if (mediaDevicesAvailable() && !isLinux) await enumerateVideoDevices();
  if (!boot.enabled) return;

  // Same stack, second entry point: on Linux the `camera` source's getUserMedia can stall the WebView
  // process just like the enumeration did, and auto-starting it inline would do that *before the app
  // has painted* — a blank window for half a minute. Skipping the enumeration alone didn't close that.
  // Deferring past first paint can't prevent a stall inside WebKit, but it does mean the user gets a
  // running, usable app either way. `native` and `rtsp` never touch that stack, so they start inline.
  if (isLinux && get(videoState).kind === 'camera') {
    setTimeout(() => void startActive(), LINUX_CAMERA_AUTOSTART_DELAY_MS);
    return;
  }
  await startActive();
}
