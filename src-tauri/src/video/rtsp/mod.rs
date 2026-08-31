// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! Native RTSP client — the shared core of the "own client + OS-native decode" video path
//! (Dev-Docs active/MOBILE_RTSP.md). Replaces the external engines (MediaMTX/ffmpeg) on
//! platforms that cannot run helper processes (Android/iOS) and, staged, on desktop.
//!
//! Layering:
//! * `client` — RTSP control (OPTIONS/DESCRIBE/SETUP/PLAY, Basic + Digest auth, keepalive)
//!   and the two data transports: **RTP/UDP** (NAT pinhole punch + RTCP receiver reports)
//!   and **TCP-interleaved**, with automatic UDP→TCP fallback when no RTP arrives after
//!   PLAY (CGNAT/firewall paths). UDP first — Kite flies over lossy, high-latency links
//!   where TCP retransmits stack delay exactly when the link degrades.
//! * `sdp` — minimal SDP: media sections, rtpmap, control URLs.
//! * `rtp` — RTP header parse + a small bounded reorder window (latency over completeness:
//!   a handful of packets, deliberately NOT a jitter buffer) + loss/reorder statistics.
//! * `mjpeg` — RFC 2435 depacketizer: reassembles fragments and re-synthesizes the JPEG
//!   headers the RFC strips (quant tables scaled from Q or taken inline, standard Annex-K
//!   Huffman tables).
//!
//! P0 handles MJPEG only (P1 feeds it to `mjpeg_server` → `<img>`, zero native code on any
//! platform). The H264 (RFC 6184) / HEVC (RFC 7798) depacketizers arrive with the Windows
//! Media Foundation sink (P2.1). No audio — this is a GCS.

mod client;
mod mjpeg;
mod rtp;
mod sdp;

pub use client::{run_rtsp, RtspConfig, RtspTransport};
