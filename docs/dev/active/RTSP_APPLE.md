# RTSP Apple sink — handover brief for the macOS/iOS session (P3)

> Created 2026-09-01 on the Windows machine, after P2.1 (Windows, PR #85), P2.2 (Android,
> PR #88) and P2.3 (Linux, PR #89) all merged into `development`. This file is the COMPLETE
> briefing for a Claude session on the Mac: what exists, what to build, in which order, and
> every trap the other three ports already paid for. It is a plan, not a contract — the
> Linux session deviated from its brief where the platform demanded it and that was right;
> do the same, but document the deltas at the top of this file as you go (see the Linux
> pattern: a PROGRESS block above the original text).

## Scope and platform policy (user decisions, fixed)

- **One sink for both Apple OSes**: `AVSampleBufferDisplayLayer` + VideoToolbox. macOS and
  iOS share the sink code; only the view-hosting layer differs (AppKit vs UIKit).
- **macOS behaves like Windows/Linux**: the "native client" toggle stays visible, the
  classic MediaMTX/ffmpeg path remains the switchable reserve engine (it is validated on
  macOS — do not regress it).
- **iOS behaves like Android**: the Kite client is the ONLY route — iOS cannot spawn
  sidecar processes at all (no ffmpeg, no MediaMTX), so there is no classic path to
  toggle. Force it on in the store, hide the toggle.
- **iPad only — NO video support on iPhone.** On an iPhone the video panel shows a
  placeholder and offers nothing (no camera kind either). Gate with `isPhone` from
  `src/lib/platform.ts` (iPad = `isTablet`). This goes into the user docs too.
- **Picture-in-Picture is NOT built now, but the path must stay open.**
  `AVSampleBufferDisplayLayer` is exactly the PiP-capable primitive
  (`AVPictureInPictureController.ContentSource(sampleBufferDisplayLayer:playbackDelegate:)`,
  iOS 15+). Rules that keep the door open: never tie the decoder's lifetime to
  app-background transitions (Android lesson — its codec is keyed to surface generation,
  not to onPause), and keep the display layer owned by one dedicated view that could later
  be handed to a PiP controller.
- **Mirror and rotate-180 are EXPECTED TO WORK live on Apple** (unlike Android, where
  mirror is measured-impossible): a `CATransform3D` on the layer transforms the displayed
  content, no decoder involvement. If reality disproves this, copy the Android fallback
  pattern exactly (disabled Toggle + explanatory tooltip + i18n key in all five locales —
  see `mirrorUnavailable` in `VideoPanel.svelte`).

## Session setup on the Mac (do this first)

- Clone/pull the Kite-GC repo, branch `feat/rtsp-apple` cut from `development`.
- **CLAUDE.md is gitignored (machine-local)** — ask Marc for a copy BEFORE working; it
  carries the binding project rules. The critical ones, in case it is missing: respond in
  the user's language, code/comments/commits in English; NEVER commit or push without the
  user's explicit order; commit only user-verified stages; dual-commit (docs before code,
  two separate commits); commits end `Co-Authored-By: Claude Opus 4.8`; no generated
  footer in PR bodies; regression marker on every fix (`Affects: <release> and earlier`
  vs `Regression: introduced by <sha>, never released`); `npm run check` at 0 errors +
  `cargo test --no-run` before presenting anything; i18n in ALL FIVE locales
  (`src/lib/i18n/locales/{en,de,fr,bg,zh}.json`); Svelte 5 runes only (no `export let`,
  no `$:`, no `on:click`); no TypeScript `any`; NumberStepper for numeric inputs; `log::`
  convention (warn = default-visible, debug = verbose; `eprintln!` is dev-only/temporary).
- Every new source file needs the SPDX header
  (`// SPDX-License-Identifier: GPL-3.0-or-later`) — copy the two-line header from any
  existing file in `src-tauri/src/video/`.
- The private Dev-Docs repo (plan history, CHANGELOG) lives on Marc's side — report
  progress and decisions back to him; he keeps `active/MOBILE_RTSP.md` and the CHANGELOG
  current there.
- Dev loop: `npx tauri dev` on macOS (realtime iteration, no packaging). For iOS:
  `npx tauri ios dev` on a REAL iPad — the simulator decodes in software and hosts views
  differently, it proves nothing about the hole punch or hardware decode. In release-style
  device builds the Debug Monitor is unlocked by baking `KITE_DEBUG_UI=1` into the build
  environment (compile-time `option_env!` in `src-tauri/src/debug_mode.rs` — mobile has no
  CLI flags); desktop dev builds have the monitor anyway.

## What already exists (all merged in `development`)

- **Shared Rust core** `src-tauri/src/video/rtsp/`: RTSP state machine (Basic/Digest
  auth), SDP, UDP + TCP-interleaved with auto-fallback (UDP first, ~2 s), RTP reorder
  window, depacketizers H264/HEVC/MJPEG (Annex-B access units, in-band parameter sets),
  `LiveRtspStats` atomics. Platform-independent, fully unit-tested — DO NOT touch it for
  this port.
- **`video/rtsp_native.rs`**: the orchestrator. Routes MJPEG → the multipart HTTP
  broadcast (the frontend renders it in an `<img>`; **MJPEG never enters a decode sink on
  any platform** — the Apple sink only ever sees H264/H265) and H264/H265 → a per-OS
  `PlatformSink` (type alias, cfg-gated). `Started::{Mjpeg{port}, Sink{codec}}` decides
  the frontend mode. `SinkTs` unwraps the RTP 32-bit timestamp into monotonic 64-bit
  90 kHz ticks — that is what `push(data, ts)` receives; a 90 kHz `CMTime` timescale maps
  it 1:1.
- **Reference sinks**: `video/win_sink.rs` (Media Foundation, ~1000 lines),
  `video/android_sink.rs` (MediaCodec, ~460 lines — **the closest template for Apple**:
  decode thread, resume-on-keyframe via `has_intra_frame`, EMA-paced presentation, the
  `Shared` state struct), `video/linux_sink.rs` + `video/linux_host.rs` (GStreamer + GTK —
  the host/sink split and the startup `install()` pattern are the model for the Apple
  host). The sink method surface all three share: `start(codec) -> Result<Self, String>`,
  `push(&self, au, ts90k)`, `error() -> Option<String>`,
  `set_rect(x,y,w,h,cx,cy,cw,ch)`, `set_visible(bool)`, `set_buffer(u32)`,
  `set_orient(mirror, rotate180)`, `frames_presented() -> u64`,
  `picture_size() -> Option<(u32,u32)>`.
- **Frontend — the sink route itself needs ZERO changes**: `controllers/nativeVideo.ts`
  (surface router: picks ONE surface by priority main > floating > widget > preview,
  pushes the two-rect contract, cuts rounded clip-path holes in every layer under the
  video); `stores/video.ts` handles `mode: "sink"` generically (rect sync, orient +
  buffer sync on start, 1 Hz stats monitor with stall→reconnect); the Debug Monitor's
  video tab reads `video_rtsp_native_stats` and lights up by itself once `sink_stats()`
  returns data. What DOES change in the frontend is the platform gating — see the
  checklist below.
- **Two-rect sink contract** (`video_rtsp_native_sink_rect`, PHYSICAL px, main-window
  client coords): `x/y/w/h` = the surface's FULL box (video layout, aspect-fit),
  `cx/cy/cw/ch` = the VISIBLE part after scroll-container clipping. The sink lays the
  video out in the full box and CUTS it at the visible edge — a scrolled panel crops the
  picture like DOM content, it never shrinks it. Hole corners produced by clipping stay
  square (the router handles that in the DOM).
- **What Apple does TODAY**: the accept list for "other OS" is MJPEG-only
  (`rtsp_native.rs` — the `#[cfg(not(any(...)))]` arm), so on macOS the Kite-client
  toggle already works for MJPEG sources and H264/HEVC end in a readable codec error. On
  iOS the RTSP kind is blocked in the UI entirely (start-button disable + placeholder in
  `VideoPanel.svelte` — both fall in this port).

## The Apple sink — recommended architecture

**Decode + present: enqueue COMPRESSED samples into `AVSampleBufferDisplayLayer`** and let
VideoToolbox decode inside it — do not run a manual `VTDecompressionSession` unless the
layer route fails; the layer route is less code, hardware-decoded, and is the PiP
primitive. (API generation note: since macOS 14 / iOS 17 the enqueue/flush/timebase
surface lives on `layer.sampleBufferRenderer` (`AVSampleBufferVideoRenderer`) and the
direct layer methods are soft-deprecated — decide the minimum OS with Marc and target one
generation cleanly rather than maintaining both.)

- **Annex-B → length-prefixed conversion** (the real new work in this sink, ~a day):
  the depacketizer delivers Annex-B AUs with in-band parameter sets. Per AU: split NALUs,
  capture SPS/PPS (H264) / VPS/SPS/PPS (HEVC), build the
  `CMVideoFormatDescription` via `CMVideoFormatDescriptionCreateFromH264ParameterSets` /
  `...FromHEVCParameterSets`, re-emit the remaining NALUs with 4-byte big-endian length
  prefixes into a `CMBlockBuffer`, wrap as `CMSampleBuffer` with the format description
  and the 90 kHz PTS. Recreate the format description when parameter sets change (some
  encoders resend identical sets every keyframe — compare bytes, don't rebuild per AU).
- **Pacing / the 0–3 frame smoothing buffer** (same semantics as the other sinks — the
  `NumberStepper` in the panel is wired end-to-end already): depth 0 = present on decode
  (attach `kCMSampleAttachmentKey_DisplayImmediately`); depths 1–3 = schedule each frame
  `depth × frame-interval` behind the media timeline via the control timebase. Port the
  Android `Pacing` struct's logic 1:1 (anchor {host-time, pts, depth}, EMA interval
  `interval += (delta − interval)/8` clamped 5–100 ms seeded at 16 667 µs, re-anchor on
  jumps > 100 ms past the cushion or on depth change) — it is proven and its semantics
  are documented as the product behavior.
- **Resume on keyframe**: after any flush/rebuild (error, orientation rebuild if you end
  up needing one, iOS foregrounding — see below) drop AUs until one contains an IDR/IRAP
  slice. Copy `has_intra_frame` from `android_sink.rs` (H264 NAL type 5, HEVC types
  16..=21; a shared-helper extraction is a separate refactor proposal, copying is fine).
- **Error contract**: a fatal decode error goes into the sink's error slot →
  `rtsp_native.rs` ends the stream → the frontend reconnects with a fresh sink. Watch
  `layer.status == .failed` / the renderer's `requiresFlushToResumeDecoding` and surface
  `error.localizedDescription`.
- **Stats contract**: `frames_presented()` must ADVANCE while frames flow — the
  frontend's 1 Hz monitor declares a stall and reconnects when it stops. Counting
  successfully enqueued samples is acceptable. `picture_size()` from
  `CMVideoFormatDescriptionGetDimensions` minus the clean aperture (the 1280×736
  coded-size trap: report the DISPLAY size, or the panel's aspect goes subtly wrong).

**Hosting / hole punch: a sibling view BELOW the WKWebView — do NOT re-parent the
WebView.** The Linux port learned the hard way that wry makes assumptions about its view
tree (its resize handler walked the parent chain and aborted the process); AppKit/UIKit
don't even need a re-host, overlapping siblings are enough.

- **macOS**: get the window's `contentView` (`WebviewWindow::ns_view()` /
  `ns_window()`), create one layer-backed `NSView` (`wantsLayer`) and
  `addSubview(_:positioned:.below, relativeTo: <the WKWebView>)`. Make the WebView
  see-through: add a `windows` entry with `"transparent": true` to
  `src-tauri/tauri.macos.conf.json` (the file exists, bundle-only today —
  `tauri.linux.conf.json` shows the exact shape; wry translates the flag into
  WKWebView/window transparency). The DOM paints an opaque background everywhere except
  the hole, so nothing visible changes until a hole is cut — but verify window shadow and
  rounded corners still look right with the flag on.
- **iOS**: reach the WKWebView via `WebviewWindow::with_webview` (the `PlatformWebview`
  exposes the WKWebView and the UIViewController), set `isOpaque = false`, clear
  `backgroundColor` on the webview AND its `scrollView`, and
  `insertSubview(videoView, belowSubview: webview)` in its superview.
- **Structure = the Android/Linux clip pattern**: a container view/layer at the VISIBLE
  box with `masksToBounds` and an opaque BLACK background (the letterbox must be opaque —
  Linux lesson: a transparent gap shows whatever is behind), and the display layer at the
  FULL box offset inside it. `set_visible(false)` hides the view but keeps decoding
  (no Apple equivalent of Android's surface destruction — verify enqueue keeps working
  hidden).
- **Threading**: all view/layer-tree mutations on the main thread —
  `window.run_on_main_thread(...)` is the Tauri-side hop (the Linux host uses the glib
  equivalent; same shape). The decode/feed path runs on the RTSP thread; sample enqueue
  from a background thread is commonly done, but verify it against the API generation you
  target and serialize on one queue if in doubt.
- **Physical → logical px**: the rect contract arrives in PHYSICAL px;
  AppKit/UIKit lay out in points — divide by `backingScaleFactor` (macOS) /
  `UIScreen.main.scale` or the view's `contentScaleFactor` (iOS).
- **Host install pattern**: mirror `video::linux_host::install(app.handle())` called from
  Tauri's setup in `lib.rs` — install the view scaffolding once at startup; the sink then
  attaches/detaches its layer. Keeps `start()` fast, which matters: the orchestrator's
  first-frame window is 12 s total including RTSP negotiation — defer anything slow to
  the decode thread (the Android sink's `wait_for_surface` shows the pattern).
- **iOS background behavior** (expected, not a bug): iOS revokes hardware decode for
  backgrounded apps without an active PiP session. Treat it like Android surface loss —
  expect enqueue failures / a flush requirement, and resume on the next keyframe when
  foregrounded. Do not fight it, and do not tear the sink down on the transition (the
  PiP rule).

## Wiring checklist (mirror the Android/Linux ports)

1. **Cargo** `[target.'cfg(any(target_os = "macos", target_os = "ios"))'.dependencies]`:
   the objc2 framework crates for AVFoundation/CoreMedia/QuartzCore, plus AppKit
   (macOS-only section) / UIKit (iOS-only section) for the hosting views. Precedent in
   the tree: iOS BLE already uses `objc2 = "0.6"` + `objc2-foundation = "0.3"` +
   `objc2-core-bluetooth` — match that generation so exactly one `objc2` ends up in the
   binary, and enable only the feature flags for the types actually used (see how the
   CoreBluetooth dep lists them).
2. New `src-tauri/src/video/apple_sink.rs` (shared decode/present logic, the sink method
   surface above) + `apple_host.rs` (view scaffolding; internally cfg-split AppKit/UIKit)
   — declare both in `video/mod.rs` under `#[cfg(any(target_os = "macos", target_os = "ios"))]`.
3. `video/rtsp_native.rs`: extend every `cfg(any(windows, android, linux))` with
   `macos`/`ios` (they are all internal plumbing around the `PlatformSink` alias), add the
   alias arm, the accept-list arm (all three codecs; no parent handle — like
   Android/Linux, `let _ = parent_hwnd;`) and the start arm
   (`AppleVideoSink::start(frame.codec)`).
4. `lib.rs` setup: `video::apple_host::install(...)` next to the linux_host precedent.
5. `commands/video.rs` needs NO signature changes (the Windows-only `parent` resolution
   already falls through to `None` elsewhere).
6. **iOS project regeneration trap**: keep EVERYTHING in-crate (Rust + objc2), exactly
   like the iOS BLE transport did — `cargo tauri ios init` can regenerate the Xcode
   project and destroys manual customizations. If an Info.plist key turns out to be
   needed, document it in this file; plain RTSP/RTP sockets and the localhost MJPEG port
   are not expected to need ATS exceptions.

**Frontend changes** (this port is NOT frontend-zero, unlike Linux):

- `stores/video.ts`: extend the forced-client boot (`if (isAndroid) boot.rtspNativeClient = true;`,
  ~line 291) to iOS.
- `VideoPanel.svelte`: hide the native-client toggle on iOS too (`{#if !isAndroid}`,
  ~line 618); REMOVE the iOS start-button disable (~line 364) and the iOS RTSP
  placeholder (~line 680) — both exist only because P3 hadn't happened; add the
  iPhone-only "no video on this device" placeholder gated by `isPhone` (new i18n key in
  all five locales).
- `platform.ts` has everything needed (`isIOS`, `isPhone`, `isTablet`, `isMacOS`) — note
  `isIOS` correctly catches iPadOS's desktop-Safari UA via the touch heuristic.
- Mirror/rotate rows: leave them fully enabled — `mirrorUnavailable` stays
  `isAndroid && nativeSink`. Only if mirror turns out impossible: copy the Android
  pattern (disabled Toggle + `video.mirrorNativeAndroid`-style key ×5).
- `stores/pulseBlink.ts` already gates all of mobile into hard-blink mode (that was a
  measured 150 %-of-a-core lesson on Android) — nothing to do, just don't undo it.
- User docs (same PR, code-verified detail by detail — project rule): the platform notes
  in `docs/user/guides/video.md` need a macOS/iOS rewrite (macOS: native client optional
  next to the classic engine; iPad: built-in client only, hardware H264/HEVC; iPhone: no
  video).

## Order of work — spike first (this method carried all three ports)

- **Stage A — hole-punch spike (macOS)**: transparent-flag conf + a solid magenta
  layer-backed NSView below the WebView at a fixed rect. Verify: color visible where the
  DOM opens a hole, panels/OSD composite ABOVE it, normal UI unaffected, window
  shadow/corners intact. Falsifiable and cheap — Windows, Android and Linux each needed
  exactly this proof before anything else. Measure idle CPU with the transparent flag on
  (composition cost — get the number early).
- **Stage B — decode spike (macOS)**: feed the sink from the client at a fixed rect,
  H264 first, then HEVC. Bench sources below.
- **Stage C — wiring + parity (macOS)**: accept list + PlatformSink + rect/visible/stats
  → the frontend router lights up on its own. Verify panel/widget/floating/map-swap, the
  scroll-crop (shrink the window until the panel scrolls — the video must be CUT at the
  container edge, never shrink), buffer depths, mirror/rotate, Debug Monitor numbers,
  error paths (kill the source mid-stream → reconnect loop; dead server → readable
  error), and that the classic MediaMTX path still works both ways across the toggle.
- **Stage D — iOS port**: apple_host UIKit half, forced client + toggle hidden, iPad
  device test (hole punch, hardware decode, background/foreground resume), iPhone
  placeholder.
- **Stage E — gates, docs, cleanup**: i18n ×5, user-docs rewrite, and **REMOVE ALL SPIKE
  CODE** — test layers, temporary buttons, temporary i18n keys, `eprintln!`s. The Android
  port added spike buttons + two i18n keys ×5 and deleted every trace before the PR;
  same bar here. `npm run check` 0 errors, `cargo test --no-run`, then the PR.

## Test sources (copy-paste)

- MediaMTX (the app's own engine binary works standalone): minimal yml with
  `rtspAddress: :8601`, all other servers off. Publish H264:
  `ffmpeg -re -f lavfi -i testsrc2=size=1280x720:rate=30 -c:v libx264 -preset veryfast -tune zerolatency -g 30 -bf 0 -f rtsp -rtsp_transport tcp rtsp://127.0.0.1:8601/live`
  (HEVC: `-c:v libx265 … -x265-params bframes=0:keyint=30`). MJPEG needs
  `-c:v mjpeg -huffman default` — **RFC 2435 rejects optimized Huffman tables**; without
  that flag every frame is silently dropped and the client loops in its first-frame
  timeout (a lost hour on the Windows side).
- Do NOT try `ffmpeg -rtsp_flags listen` as a server — it serves no PLAY clients
  (another lost hour). MediaMTX + an ffmpeg publisher is the working pair.
- Bench test in-repo:
  `KITE_RTSP_URL=rtsp://127.0.0.1:8601/live cargo test serves_multipart --lib -- --ignored --nocapture`
  (MJPEG end-to-end through the client; env var in the SAME shell). The Windows-only
  `streams_h264_into_the_native_sink` bench shows what an equivalent macOS bench would
  look like if one is worth adding.
- Real sources: a UAV-Link unit on the local network, or OBS's RTSP output from another
  machine. Ask Marc for live endpoints.

## Traps already paid for (do not rediscover)

- **Wi-Fi location scans kill RTP on tablets**: on Android, the continuous GCS location
  watch triggered an OS Wi-Fi scan every ~10 s and burst-dropped packets. The fix
  (`stores/gcsLocation.ts`: pause the watch while RTSP streams over Wi-Fi) is gated
  `isAndroid` today. If the iPad shows periodic ~10 s loss on Wi-Fi streams, widen that
  gate before hunting anything else — iOS may or may not behave the same; verify, don't
  assume either way.
- **Per-frame fixed cost on mobile GPUs**: any new looping animation must go through the
  blink clock (`stores/pulseBlink.ts`) — an 8 px pulsing dot cost 150 % of a core on the
  Android tablet. Measure CPU per stage on the iPad; don't assume.
- **Panel glass loses its backdrop blur** while it hosts native video (clip-path +
  backdrop-filter engine quirk) — known on Windows/Android, accepted; if WebKit shows the
  same, do not chase it.
- **Tauri invoke rejects with plain STRINGS** — frontend `err instanceof Error` never
  matches; `stores/video.ts` already normalizes, don't copy the old pattern into new code.
- **First-frame semantics**: the orchestrator reports start success only after the sink
  is up AND fed (the `sink_first` signal). A slow sink start eats the shared 12 s window
  — keep `start()` allocation-cheap and move waiting into the decode thread.
- **The user tests visually**: autonomous screenshots don't clear the bar for visual
  behavior — user-verified stages only, then commit (never commit or push unbidden).
- **Coded-size vs display-size** (the 1280×736 trap): report clean-aperture display size
  from `picture_size()` or aspect ratios go wrong in the panel.

## Definition of done

Parity with Windows/Android/Linux: MJPEG (multipart) + H264 + HEVC through the Kite
client on macOS AND iPad; hole punch under the DOM; scroll-crop correct (two-rect
contract); mirror/rotate live (or the Android-style disabled-toggle fallback with
rationale); smoothing-buffer depths 0–3 wired through the existing stepper; Debug Monitor
video tab live; readable errors + stall-reconnect working; macOS classic path regression-
free across the toggle; iPhone shows the no-video placeholder; all spike/test code
removed; i18n complete in en/de/fr/bg/zh; `docs/user/guides/video.md` platform notes
rewritten and code-verified. CPU numbers recorded per route (macOS + iPad). PR
`feat/rtsp-apple` → `development`, docs-first dual commits, regression markers where
applicable, no generated PR footer. Report the merged state and every architectural delta
back to Marc so the private plan (`MOBILE_RTSP.md`) and CHANGELOG stay true.
