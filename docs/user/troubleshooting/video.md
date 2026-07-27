# Video problems

Trouble getting a feed? Work through the checks below. For how the video feature works in the first
place, see the **[Video guide](../guides/video.md)**.

## A local camera / capture card doesn't appear

- **Connect it before opening the source list**, then reopen the **Camera** dropdown — devices are
  enumerated when the list opens.
- **Close other apps using it.** A webcam or capture card can usually be opened by **one app at a time**;
  quit anything else holding it (OBS, a browser tab, another GCS).
- **Check OS permissions.** The operating system may need to grant camera access to Kite.
- **Try Native Capture (Advanced).** Some capture devices — notably certain USB HDMI dongles, especially
  on Linux — aren't exposed to the ordinary **Camera** list but *are* reachable through **Native Capture
  (Advanced)**. Switch the source kind and reopen the device dropdown. (Native Capture uses the **ffmpeg**
  helper, which Kite downloads automatically the first time.)

## An RTSP stream won't start

RTSP playback uses a small bundled engine (**go2rtc**); Kite downloads it automatically the first time
you start an RTSP source. If a stream won't come up:

- **Let the engine download finish.** The Video panel shows the go2rtc status and offers the download if
  it's missing — RTSP can't play until it's installed. If the download fails (no internet, or an
  unsupported CPU architecture), you'll get a hint to install it manually.
- **Check the URL and reachability.** A typo, a camera that's off, a firewall, or a source on a different
  network will all stop it. Confirm the exact `rtsp://…` URL works in another player (e.g. VLC) from the
  same machine.
- **Match the transport.** Some servers are **UDP-only** (they reject TCP clients) — set the connection's
  **Transport** to **UDP** in the Video panel (needs the ffmpeg helper). Conversely, if UDP is blocked on
  your network, try **TCP**. **Auto** tries the native reader first and falls back to ffmpeg.

## The stream keeps showing "Reconnecting…" — check the codec first

Before chasing the network: **Kite supports H.264 and MJPEG over RTSP**, and nothing else. A stream
in **HEVC/H.265**, VP8, VP9 or AV1 typically fails in exactly this way — it retries forever instead
of reporting a codec problem, because from Kite's side an unplayable stream and an unreachable one
look much alike.

If another player shows your stream fine while Kite only reconnects, the codec is the first thing to
check at the source. Switching it to **H.264** is the quickest test. (Codec support is limited by
what we can test, not by what is possible — a request is welcome.)

## The stream keeps showing "Reconnecting…"

The overlay means Kite lost the feed and is retrying — it will **keep trying until the stream returns or
you press Stop**. If it never comes back: the source is down or unreachable (check it in another player),
the transport doesn't match the server (see above), or a firewall started blocking the media packets. A
few reconnect cycles in a row are normal when a source restarts or the network path changes (e.g.
switching between Wi-Fi and a cellular link).

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

## Linux: high CPU load or a stuttering picture

On Linux, Kite depends on your distribution's browser-engine build for video — see the
**[platform notes](../guides/video.md#platform-notes-what-to-expect-per-operating-system)** in the
guide. The log tells you what your system offers. Restart Kite, start your stream, then open the log
(**Settings → Diagnostics**, the default **Warning** level is enough) and look for:

```
[webkit]    WebKitGTK 2.xx.y — enable-webrtc set, reads back as true
[gstreamer] webrtcbin=… · h264 decoders=[…]
            WebRTC is unavailable in this WebView — falling back to the MJPEG image path
```

**If the last line is there, the direct playback path isn't available on this machine.** Kite then
converts the stream frame by frame, which is what drives the CPU up and limits the resolution you can
sustain. Two things are worth knowing:

- **This is usually not something you can install your way out of.** Several Linux distributions —
  Raspberry Pi OS among them — ship a browser engine built *without* the direct video path, and it then
  stays unavailable no matter which media packages are present. On a Raspberry Pi 5 we measured the
  engine reporting the feature as enabled, all the expected media plugins installed, and it still wasn't
  there. If `webrtcbin=false` appears in your log it is still worth installing
  `gstreamer1.0-plugins-bad` (and `gstreamer1.0-libav` if the decoder list is empty) — that is a genuine
  prerequisite — but do not expect it to be sufficient.
  Note that these package installs only affect the **system installation (.deb)**: the **AppImage**
  brings its own browser engine and does not use the system's media packages at all. If video matters
  to you on Linux, prefer the **.deb / system installation** — it uses your distribution's engine and
  plugins, which you can at least influence.
- **Hardware conversion is used automatically where it exists.** Kite tests the machine once at
  start-up and writes the verdict to the log:
    - Desktop graphics (Intel and AMD, via VAAPI) can do **both halves** — decoding and re-encoding —
      with the frames never leaving the GPU. Look for `[ffmpeg] VAAPI hardware H.264-decode +
      MJPEG-encode: …`. On an Intel laptop this cut ffmpeg's CPU use to roughly a seventh.
      **NVIDIA cards are not specifically covered**; if the test fails, Kite converts on the CPU.
    - Boards whose chip decodes H.264 itself (Raspberry Pi 3 and 4 among them) accelerate the
      **decoding half** — see `[ffmpeg] V4L2 hardware H.264 decoding: …`. A Raspberry Pi 5 has no such
      decoder, so there everything is done on the CPU.
  Kite deliberately uses hardware only when it can do the *whole* conversion on a desktop GPU: doing
  just one half there means copying every frame back out of graphics memory, which is slower than
  staying on the CPU altogether.
- **You can force the CPU path.** If the picture is broken or unstable with hardware conversion, switch
  on **Disable hardware acceleration** in the Video panel. The Video panel also states which path is
  live — `Transcode: Hardware`, `Software`, or `Copy` when nothing needs converting at all.
- **Send a smaller or slower stream from the source.** This is the one lever that really works, because
  it removes the work at *every* stage — network, decoding, conversion and display. On a single-board
  computer, 480p at 30 fps is comfortable where 720p60 is not. Set it where the stream is produced (your
  video transmitter, camera or streaming server), not in Kite.
- **Show the video in fewer places at once.** Every visible surface (panel preview, widget, floating
  window) is drawn separately. Closing the ones you don't need frees noticeable load.

Converting a 720p60 stream in software is simply beyond a small machine, and no setting inside Kite
changes that arithmetic — the picture has to get smaller or slower at the source.

## Linux: the CPU stays busy after I stop the video

If stopping the feed leaves a processor core fully loaded — and it never drops until you quit Kite —
check which browser engine your system provides. Kite logs it on every start
(**Settings → Diagnostics**):

```
[webkit] WebKitGTK 2.50.6 — enable-webrtc set, reads back as true
```

**A 2.50.x engine has this fault.** It is in the engine, not in Kite: the video helpers shut down
correctly and drawing stops, but the engine keeps working on something afterwards. Nothing inside
Kite can switch it off.

WebKitGTK 2.50 is what **Debian 12, Raspberry Pi OS Bookworm and Ubuntu 22.04** ship, and it is the
reason Kite's Linux downloads are built for newer systems (see
**[Installation](../getting-started/installation.md)**). Debian 13, Raspberry Pi OS Trixie and Ubuntu
24.04 carry 2.52, where the engine settles back to idle. **Upgrading the distribution is the fix** —
reinstalling Kite, or building it yourself, cannot change which engine your system provides.

## Raspberry Pi: garbled or black window right after start

A known quirk of the Pi's graphics driver: the very first drawing surface is often invalid. Kite detects
a Raspberry Pi and briefly resizes its own window once the interface has loaded, which forces a clean
surface. If you still see it, the log line `[gpu] Raspberry Pi framebuffer nudge: …` tells you the
workaround ran and which variant it used.

## Still stuck?

Grab a **diagnostic log** (**Settings → Diagnostics → Log Level = Debug**, reproduce, then **Open Log
Folder**) and attach it when reporting the problem — it records the go2rtc / ffmpeg startup and any error.
See the [connection troubleshooting](connection.md#getting-a-diagnostic-log) page for the log locations.
