# Embedded Video — planning & design

Status: **in progress.** Core (router + webcam source + NavRail panel + live preview) is built; widget / floating window / map-swap and network streams follow.

Goal: embed an FPV / camera feed in the GCS — local webcam & USB capture now, network streams (RTSP/UDP) later — cross-platform (Windows / Linux), modular, no hard dependency on a specific source.

## 1. Architecture — two layers + a router

The design separates **where the picture comes from** (platform-specific) from **where it is shown** (platform-agnostic), with a router in between so one decoded feed serves many views.

```
VideoSource(s) ──▶ VideoRouter ──┬──▶ Sink: NavRail panel preview
 (webcam/url/be)   (ref-counted)  ├──▶ Sink: dock widget (2×1 wide)
                                  ├──▶ Sink: floating window
                                  └──▶ Sink: map-swap (main view)
```

- **Display layer (agnostic):** every sink is just an HTML `<video>` (or `<canvas>`). It binds one of three **attachments**: a `MediaStream` (→ `srcObject`), a URL (→ `src` / `<img>`), or a frame callback (→ canvas). It never knows the origin.
- **Source layer (platform):** produces such an attachment. Webcam is portable (Web API); network/native sources carry the platform code but expose the same interface (typically a shareable local endpoint URL).
- **Router** (`stores/video.ts`): opens a source once, holds the live attachment, hands it to N sinks. For a webcam this is free — **one `MediaStream` attaches to many `<video>` elements at once**, so one decode feeds the panel preview, the widget, the floating window and the swap view simultaneously.

## 2. Sources

| Source | Path | Status |
|---|---|---|
| **Webcam / USB capture** | `getUserMedia` (works in WebView2 **and** WebKitGTK, no backend) | ✅ v1 |
| MJPEG / HLS (IP cameras) | URL into `<video>` / `<img>` | v1.5 |
| RTSP / UDP / RTP (FPV) | backend gstreamer/ffmpeg → local WebRTC/HTTP endpoint | v2 |
| Native webcam (format/fps control) | Rust `nokhwa` (MediaFoundation / V4L2) → local MJPEG endpoint | v2 fallback |

### Frame-rate / format finding (webcam)
The browser camera API has **no format constraint** — you cannot ask for MJPEG directly. Many UVC cameras expose both an **uncompressed** mode (YUY2/NV12 — USB-bandwidth-limited to a few fps at 720p/1080p) and an **MJPEG** mode (full fps at high res). Without a frame-rate hint the browser may pick the slow uncompressed mode (observed: 13 fps @720p, 6 fps @1080p).

**Fix:** request a high frame rate — `frameRate: { ideal: 60 }` (the FPV standard; analog PAL/NTSC and digital are 50/60/100/120). Only the MJPEG mode can satisfy 60 fps, so the browser selects it. Verified: a Dell 4K webcam then delivered ~60 fps even on AUTO (picked 720p60). If a camera still refuses, the **native `nokhwa` backend source** (explicit MJPEG + resolution + fps) is the real fix — and reuses the v2 backend-endpoint path.

What the Web API offers and nothing more: `getUserMedia` + constraints, `applyConstraints()` (same system), `getCapabilities()` (inspect only), `MediaStreamTrackProcessor`/WebCodecs (downstream raw frames). No capture-format selection — hence the native route for full control.

## 3. UI integration

- **NavRail "Video" panel** (✅ control center): start/stop, device picker, resolution (auto / 720p / 1080p, all with the 60 fps hint), mirror, live preview, an info line (resolution · measured/set fps). Measured fps via `requestVideoFrameCallback`.
- **Dock widget** (2×1 `wide`, planned next): a router sink in the standard widget card; **crop-to-fill** (`object-fit: cover`) so the 2:1 is full (too small to read OSD anyway); thin rounded border; **no settings** (panel owns control).
- **Floating window** (planned): activated from the panel; **snaps bottom-left**, displacing the bottom widget dock from that corner (dock reflows to the remaining width); **drag** to float free (dock reclaims full width); re-snappable. **Resize** by corner drag, **relative to the home window**, aspect **fixed to the source**, height **10–30 % of the viewport height**. NavRail floating panels render **above** the video (z-order) so their height limits aren't affected.
- **Double-click** swaps the main map view with video (video fills the map zone, map → PiP).

Detach is **in-app only** for v1 (same WebView context → MediaStream sharing is trivial). A true **separate OS window** (Tauri multi-window) is v2 and would force every source through a **shareable local endpoint** (a `MediaStream` can't cross window contexts, and a webcam is single-owner).

## 4. Status

- ✅ Router (`stores/video.ts`) + `WebcamSource` (device enumeration, start/stop, device/resolution switch, 60 fps hint, aspect from track)
- ✅ NavRail "Video" panel + live preview + info line
- ☐ Dock widget (2×1)
- ☐ Floating window (snap / drag / dock-reflow / corner-resize)
- ☐ Double-click map↔video swap
- ☐ v2: network streams (RTSP/UDP), native `nokhwa` source, OS-window detach, snapshot/record

---

*Living doc — updated as the video feature is built out step by step.*
