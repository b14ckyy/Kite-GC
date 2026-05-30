// Embedded video — source router (v1: local webcam / USB capture).
//
// The router opens a source once and exposes its MediaStream; multiple sinks
// (the NavRail panel preview, the dock widget, the floating window, the
// map-swap view) bind the *same* stream to their own <video> element — a
// MediaStream attaches to many elements at once, so one decode feeds them all.
//
// `getUserMedia` works in both WebView2 (Windows) and WebKitGTK (Linux), so the
// webcam path needs no backend. Network streams (RTSP/UDP via a backend
// gstreamer pipeline) and OS-window detach are v2, layered on this same router.

import { writable, get } from 'svelte/store';

export interface VideoDevice {
  deviceId: string;
  label: string;
}

export type VideoStatus = 'off' | 'starting' | 'live' | 'error';
export type VideoResolution = 'auto' | '720p' | '1080p';

export interface VideoState {
  /** User wants video on (source open). */
  enabled: boolean;
  status: VideoStatus;
  devices: VideoDevice[];
  /** Selected video input device (null = system default). */
  deviceId: string | null;
  resolution: VideoResolution;
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
}

const INITIAL: VideoState = {
  enabled: false,
  status: 'off',
  devices: [],
  deviceId: null,
  resolution: 'auto',
  mirror: false,
  aspect: 16 / 9,
  width: null,
  height: null,
  frameRate: null,
  capFrameRate: null,
  error: null,
};

export const videoState = writable<VideoState>({ ...INITIAL });

/** The live MediaStream (or null). Sinks subscribe and set `<video>.srcObject`. */
export const videoStream = writable<MediaStream | null>(null);

function patch(p: Partial<VideoState>): void {
  videoState.update((s) => ({ ...s, ...p }));
}

// Without a frameRate hint the browser may negotiate an uncompressed camera
// mode (YUY2/NV12) that is USB-bandwidth-limited to a few fps at high resolution.
// There is no way to request MJPEG directly, so we ask for a high rate (60, the
// FPV standard): only the camera's MJPEG mode can satisfy it, nudging the browser
// to pick it. (If the cam still won't deliver, the native backend source — v2 —
// is the real fix.)
const RES_CONSTRAINTS: Record<VideoResolution, MediaTrackConstraints> = {
  auto: { frameRate: { ideal: 60 } },
  '720p': { width: { ideal: 1280 }, height: { ideal: 720 }, frameRate: { ideal: 60 } },
  '1080p': { width: { ideal: 1920 }, height: { ideal: 1080 }, frameRate: { ideal: 60 } },
};

function mediaDevicesAvailable(): boolean {
  return typeof navigator !== 'undefined' && !!navigator.mediaDevices?.getUserMedia;
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
    const devices = all
      .filter((d) => d.kind === 'videoinput')
      .map((d, i) => ({ deviceId: d.deviceId, label: d.label || `Camera ${i + 1}` }));
    patch({ devices });
    // Drop a stale selection that no longer exists.
    const sel = get(videoState).deviceId;
    if (sel && !devices.some((d) => d.deviceId === sel)) patch({ deviceId: null });
  } catch (e) {
    patch({ error: `Device enumeration failed: ${e}` });
  }
}

function stopTracks(): void {
  const s = get(videoStream);
  if (s) for (const tr of s.getTracks()) tr.stop();
  videoStream.set(null);
}

/** Open (or re-open) the webcam with the current device/resolution selection. */
export async function startVideo(): Promise<void> {
  if (!mediaDevicesAvailable()) {
    patch({ enabled: true, status: 'error', error: 'Camera API unavailable' });
    return;
  }
  stopTracks();
  patch({ enabled: true, status: 'starting', error: null });
  const st = get(videoState);
  const video: MediaTrackConstraints = { ...RES_CONSTRAINTS[st.resolution] };
  if (st.deviceId) video.deviceId = { exact: st.deviceId };
  try {
    const stream = await navigator.mediaDevices.getUserMedia({ video, audio: false });
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

/** Stop the source and release the camera. */
export function stopVideo(): void {
  stopTracks();
  patch({ enabled: false, status: 'off', error: null });
}

export function toggleVideo(): void {
  if (get(videoState).enabled) stopVideo();
  else void startVideo();
}

/** Switch device / resolution; restarts the stream if currently live. */
export async function setVideoDevice(deviceId: string | null): Promise<void> {
  patch({ deviceId });
  if (get(videoState).enabled) await startVideo();
}

export async function setVideoResolution(resolution: VideoResolution): Promise<void> {
  patch({ resolution });
  if (get(videoState).enabled) await startVideo();
}

export function setVideoMirror(mirror: boolean): void {
  patch({ mirror });
}
