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

RTSP is played in one of two ways, and which one your machine uses decides what has to be installed:

- **The direct path** uses a small bundled engine (**MediaMTX**), which Kite downloads automatically the
  first time you start an RTSP source. This is what Windows and macOS use.
- **The image path** uses only the **ffmpeg** helper — no MediaMTX at all. Kite falls back to it where the
  browser engine offers no direct video path (common on Linux, see below), and always for a source that
  already sends **MJPEG**, which the direct path cannot carry.

If a stream won't come up:

- **Let the helper download finish.** The Video panel shows the status of whichever helper your machine
  needs and offers the download if it's missing. If the download fails (no internet, or an unsupported
  CPU architecture), you'll get a hint to install it manually. On a machine that only ever uses the
  image path, MediaMTX is neither required nor offered.
- **Check the URL and reachability.** A typo, a camera that's off, a firewall, or a source on a different
  network will all stop it. Confirm the exact `rtsp://…` URL works in another player (e.g. VLC) from the
  same machine.
- **Match the transport.** Some servers are **UDP-only** (they reject TCP clients) — set the connection's
  **Transport** to **UDP** in the Video panel. Conversely, if UDP is blocked on your network, try **TCP**.
  **Auto** lets the engine negotiate, and falls back to an ffmpeg reader for servers that refuse every
  forced transport. The setting applies to the direct path; on the image path ffmpeg negotiates the
  transport itself and reads both.

## The stream keeps showing "Reconnecting…" — check the codec first

Before chasing the network: over RTSP Kite supports **H.264 and MJPEG** everywhere, and
**HEVC/H.265** only where the **Kite RTSP client** decodes (Android always; Windows and Linux with
the *Native RTSP client* toggle on — Windows additionally needs the "HEVC Video Extensions"). A
stream in an unsupported codec — HEVC elsewhere, VP8, VP9 or AV1 — typically fails in exactly this
way: it retries forever instead of reporting a codec problem, because from Kite's side an unplayable
stream and an unreachable one look much alike.

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

## The picture is smooth, then hitches — over and over, at a steady rhythm

A stutter that returns **on a regular beat** — every ten seconds or so, a second or two of jerky
picture, then smooth again — points at the **network link**, not at Kite and not at the encoder. A
decoder that can't keep up drops frames evenly; it doesn't hand you a smooth picture for ten seconds
and then a burst.

The usual cause is **Wi-Fi**: a laptop periodically looks around for other access points, and while it
does, traffic pauses for a tenth of a second or more. Either end of the link can do it — the machine
running Kite, or the one sending the stream.

You can check this without Kite at all. Leave the video running and, from a terminal, ping the source:

```
ping <address-of-your-video-source>
```

If the round-trip times jump from a few milliseconds to a hundred or more **in clusters that repeat on
the same rhythm as the stutter**, the link is the cause — and no video setting will help. What does
help:

- **Use a cable** on either machine, which is the quickest way to confirm it.
- Move closer to the access point, or onto a less crowded channel.
- If the machine has to stay on Wi-Fi, turning off its power-saving mode for the wireless adapter is
  worth a try.
- **Raise the smoothing buffer** (Video panel, below the connection list) if you have to live with the
  link. It cannot bring back frames that never arrived, but it holds a small cushion of frames and
  plays them out evenly, so a short pause in delivery no longer becomes a visible hitch. It works on
  both video paths. The cost is exactly the latency you buy: one frame time per step, ~17 ms at 60 fps.
  Match it to the gaps you measured — a pause of a tenth of a second needs about six frame times at
  60 fps to be covered completely, so a setting that is too small will smooth part of the hitch and no
  more. With the **Debug Monitor** open (Video stats), *Smoothing buffer held* shows what the cushion
  actually contains: if it keeps falling to zero, the link's pauses are longer than the depth you chose.

## A specific RTSP server gives a black screen

Some servers (notably the **OBS RTSP server**) reject the engine's native reader. Kite automatically
retries such sources through an **ffmpeg fallback** reader — but that needs **ffmpeg**, which is a
separate optional download:

- The Video panel offers an **ffmpeg (fallback)** download when it's missing. Install it and start the
  stream again.
- The panel shows **which reader is live** (native vs the ffmpeg fallback), so you can tell which
  path your source is using.

## Playback looks uneven (micro-stutter, no real freezes)

If the stream runs without dropouts but single frames look duplicated or skipped — most visible in
smooth camera pans — the frames are arriving slightly unevenly (network jitter). Raise the
**Smoothing buffer** in the Video panel's RTSP section one step at a time (see the
[video guide](../guides/video.md#rtsp-connections-transport-and-auto-reconnect)); each step trades one
frame time of latency for smoothing. On a clean local network the buffer usually makes no visible
difference — leave it at **0** there.

## Latency is high

With a sensibly-configured encoder, end-to-end latency is low (roughly a couple of hundred milliseconds).
If it's much worse, the cause is usually the **source**: a large keyframe interval, a big encoder buffer,
or a non-low-latency tuning. Tune the encoder/camera for low-latency streaming. Also check the
**Smoothing buffer** in the Video panel's RTSP section: every step above 0 deliberately adds latency —
set it back to 0 for the minimum.

## Linux: high CPU load or a stuttering picture

First: if the stream is H.264 or HEVC, switch on the **Native RTSP client** (Video panel → RTSP
section). It takes the browser engine out of the picture entirely and decodes in hardware where the
machine can — on the reference laptop that cut the total video load to roughly a third of one core at
a stable 60 fps. Everything below applies to the **classic path**, with that toggle off.

On Linux, Kite depends on your distribution's browser-engine build for video — see the
**[platform notes](../guides/video.md#platform-notes-what-to-expect-per-operating-system)** in the
guide. The log tells you what your system offers. Restart Kite, start your stream, then open the log
(**Settings → Diagnostics**, the default **Warning** level is enough) and look for:

```
[webkit]    WebKitGTK 2.xx.y — enable-webrtc set, reads back as true
[gstreamer] webrtcbin=… · ice(nice)=… · dtls=… · srtp=… · rtpbin=… · h264 decoders=[…]
            WebRTC is unavailable in this WebView — falling back to the MJPEG image path
```

**If the last line is there, the direct playback path isn't available on this machine.** Kite then
converts the stream frame by frame, which is what drives the CPU up and limits the resolution you can
sustain. Two things are worth knowing:

- **You cannot install your way out of this one.** The direct video path is a **compile-time option of
  the browser engine**. If your distribution built WebKitGTK without it, it stays missing no matter
  which media packages are present — there is no package that adds it, and the engine still reports the
  feature as "enabled" because that switch only sets a preference with nothing behind it. Several
  distributions build it that way. On **Debian 13** (engine 2.52.5) we verified it end to end: every
  plugin present, the feature reading back as enabled, the library containing none of the actual
  implementation, and the interface absent even in a freshly created window. A **Raspberry Pi 5**
  behaved identically. You can check your own machine in a terminal (`strings` comes with `binutils`):

    ```bash
    strings /usr/lib/*/libwebkit2gtk-4.1.so.0 | grep -cw createOffer
    ```

    **0** — this engine has no WebRTC compiled in. Kite's image path is then the normal, expected path
    on that machine and the log line above is not a fault to chase; skip to the points below, which are
    the ones that actually help. **1 or more** — the engine can do it, and then the plugins do matter:
    if any of `webrtcbin`, `ice(nice)`, `dtls`, `srtp` or `rtpbin` reads `false` in your log, install
    `gstreamer1.0-plugins-bad`, `gstreamer1.0-nice`, `gstreamer1.0-plugins-good`, plus
    `gstreamer1.0-libav` if the decoder list is empty (recent Kite **.deb** packages request these
    automatically on install).

    Either way, package installs only affect the **system installation (.deb)**: the **AppImage** brings
    its own browser engine and does not use the system's media packages at all. If video matters to you
    on Linux, prefer the **.deb / system installation** — it uses your distribution's engine and plugins,
    which you can at least influence.
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
- **Expect each frame to cost more here than on Windows.** Even with the conversion sorted out, the
  picture still has to be decoded for display, and the Linux engine is markedly slower at it: on one
  laptop the same 720×576 frame took about 13 ms against roughly 2 ms in the Windows engine, and longer
  again while the machine was busy. That is the reason a frame rate that is effortless on Windows can be
  out of reach on comparable Linux hardware — and the reason the point above matters so much more here.
- **Show the video in fewer places at once.** The stream is read and decoded **once** for all of them,
  so this costs less than it used to — but every visible surface (panel preview, widget, floating
  window) is still drawn and composited separately, so closing the ones you don't need still helps.

Converting a 720p60 stream in software is simply beyond a small machine, and no setting inside Kite
changes that arithmetic — the picture has to get smaller or slower at the source.

!!! tip "Before blaming the machine: does the stutter have a rhythm?"
    If the picture is smooth for several seconds, hitches briefly, and then repeats that on a steady
    beat, it is the **network link** and not the decoding — see *"The picture is smooth, then hitches"*
    above. A machine that cannot keep up loses frames evenly; it does not deliver ten good seconds and
    then a burst. This one cost us a long hunt through the video path before a plain `ping` settled it.

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
Folder**) and attach it when reporting the problem — it records the engine / ffmpeg startup and any error.
See the [connection troubleshooting](connection.md#getting-a-diagnostic-log) page for the log locations.
