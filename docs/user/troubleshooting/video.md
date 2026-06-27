# Video problems

Trouble getting a feed? Work through the checks below. For how the video feature works in the first
place, see the **[Video guide](../guides/video.md)**.

## A local camera / capture card doesn't appear

- **Connect it before opening the source list**, then reopen the **Camera** dropdown — devices are
  enumerated when the list opens.
- **Close other apps using it.** A webcam or capture card can usually be opened by **one app at a time**;
  quit anything else holding it (OBS, a browser tab, another GCS).
- **Check OS permissions.** The operating system may need to grant camera access to Kite.

## An RTSP stream won't start

RTSP playback uses a small bundled engine (**go2rtc**); Kite downloads it automatically the first time
you start an RTSP source. If a stream won't come up:

- **Let the engine download finish.** The Video panel shows the go2rtc status and offers the download if
  it's missing — RTSP can't play until it's installed. If the download fails (no internet, or an
  unsupported CPU architecture), you'll get a hint to install it manually.
- **Check the URL and reachability.** A typo, a camera that's off, a firewall, or a source on a different
  network will all stop it. Confirm the exact `rtsp://…` URL works in another player (e.g. VLC) from the
  same machine.

## The picture stutters or shows "frames out of order"

This is almost always the **source encoder**, not Kite. The live path uses WebRTC, which tolerates
**B-frames** poorly:

- **Real FPV / DJI / IP-camera** streams are usually fine (no B-frames).
- **OBS** and similar software encoders must be set to **B-frames = 0**, a **baseline / main** profile and
  an **ultra-low-latency** tuning to play smoothly.

This is a property of the stream Kite receives — it can't be fixed downstream for a pass-through feed.

## A specific RTSP server gives a black screen

Some servers (notably the **OBS RTSP server**) reject go2rtc's native reader. Kite automatically retries
such sources through an **ffmpeg fallback** reader — but that needs **ffmpeg**, which is a separate
optional download:

- The Video panel offers an **ffmpeg (fallback)** download when it's missing. Install it and start the
  stream again.
- The panel shows **which reader is live** (go2rtc native vs the ffmpeg fallback), so you can tell which
  path your source is using.

## Latency is high

With a sensibly-configured encoder, end-to-end latency is low (roughly a couple of hundred milliseconds).
If it's much worse, the cause is usually the **source**: a large keyframe interval, a big encoder buffer,
or a non-low-latency tuning. Tune the encoder/camera for low-latency streaming.

## Still stuck?

Grab a **diagnostic log** (**Settings → Diagnostics → Log Level = Debug**, reproduce, then **Open Log
Folder**) and attach it when reporting the problem — it records the go2rtc / ffmpeg startup and any error.
See the [connection troubleshooting](connection.md#getting-a-diagnostic-log) page for the log locations.
