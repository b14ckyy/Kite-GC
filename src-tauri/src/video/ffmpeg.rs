// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! On-demand ffmpeg — the RTSP video bridge's external runtime dependency. Mirrors the
//! `blackbox_decode` model (`flightlog::decoder`): not bundled in the installer, discovered next to
//! the app / on PATH / in the writable app-data `bin/` dir, and fetched on demand from
//! BtbN/FFmpeg-Builds. Windows ships a self-contained `.zip` we unpack here; other OSes install
//! manually for now (the error points at the releases page), exactly like the decoder.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(target_os = "linux")]
use std::process::Stdio;
#[cfg(target_os = "linux")]
use std::sync::OnceLock;
#[cfg(target_os = "linux")]
use std::time::{Duration, Instant};

const REPO: &str = "BtbN/FFmpeg-Builds";
const RELEASES_PAGE: &str = "https://github.com/BtbN/FFmpeg-Builds/releases";
/// The pinned ffmpeg release branch (BtbN `n<branch>-latest` assets follow it). See `download`.
const FFMPEG_BRANCH: &str = "9.0";
const HTTP_USER_AGENT: &str = "Kite-GC ffmpeg-fetch";

/// Filename of ffmpeg for the current platform.
pub fn binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    }
}

/// Discover ffmpeg: next to the exe → app-data install dir (where the download lands) → PATH. Same
/// search order + install dir as `blackbox::find_decoder`, so a once-downloaded ffmpeg is found later.
pub fn find_ffmpeg() -> Option<PathBuf> {
    let name = binary_name();

    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
    {
        let c = dir.join(name);
        if c.is_file() {
            return Some(c);
        }
    }

    let installed = crate::flightlog::decoder::install_dir().join(name);
    if installed.is_file() {
        return Some(installed);
    }

    let path_var = std::env::var_os("PATH")?;
    for d in std::env::split_paths(&path_var) {
        let c = d.join(name);
        if c.is_file() {
            return Some(c);
        }
    }
    None
}

/// First line of `ffmpeg -version` (e.g. "ffmpeg version n7.1 ..."), or None if absent/failed.
pub fn version() -> Option<String> {
    let ff = find_ffmpeg()?;
    let mut cmd = std::process::Command::new(&ff);
    cmd.arg("-version");
    crate::child_env::sanitize(&mut cmd);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW — don't flash a console
    }
    let out = cmd.output().ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .map(|l| l.trim().to_string())
}

/// Whether this host decodes H.264 through the kernel's V4L2 M2M interface — i.e. in hardware.
/// True on a Raspberry Pi 3/4 and similar SoCs; false on a Pi 5, which has no H.264 decode block at
/// all.
///
/// It matters only for the MJPEG fallback, and there it matters a lot: that path decodes **every**
/// frame of the source before re-encoding it, and a board that needs hardware for 720p60 cannot do it
/// any other way. No engine arranges this for us, so we select the decoder ourselves on the ffmpeg
/// command line.
///
/// Probed by actually decoding a one-second clip, because the codec being listed proves nothing: every
/// Linux ffmpeg build compiles `h264_v4l2m2m` in, device or no device. Cached for the process and
/// logged once, so a tester's log states the verdict.
/// ARM-only: V4L2 M2M is the SoC (Raspberry Pi class) interface. Desktop GPUs expose VAAPI instead —
/// see `vaapi_mjpeg_transcode_available` — so probing this on x86 spawns two ffmpeg processes to
/// answer a question whose answer is already known.
#[cfg(all(target_os = "linux", any(target_arch = "aarch64", target_arch = "arm")))]
pub fn v4l2_h264_decode_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let ok = probe_v4l2_h264_decode();
        log::warn!(
            "[ffmpeg] V4L2 hardware H.264 decoding: {}",
            if ok {
                "available — used for the MJPEG transcode"
            } else {
                "unavailable — the MJPEG transcode decodes in software"
            }
        );
        ok
    })
}

#[cfg(not(all(target_os = "linux", any(target_arch = "aarch64", target_arch = "arm"))))]
pub fn v4l2_h264_decode_available() -> bool {
    false
}

/// The DRM render node VAAPI runs on, if hardware H.264-decode **and** MJPEG-encode both work there.
///
/// Both halves are required together and that is the whole point: a half-hardware chain is *slower*
/// than staying in software, because every decoded frame has to be copied back out of GPU memory for
/// the software encoder. Measured on an 8th-gen Intel iGPU, 300 frames, CPU time:
///
/// | chain                        | 720p60 | 1080p60 |
/// |------------------------------|--------|---------|
/// | software decode + encode     | 5.52 s | 7.83 s  |
/// | **VAAPI decode + encode**    | 0.79 s | 1.00 s  |
/// | VAAPI decode + sw encode     | 6.47 s | 11.63 s |
///
/// So the readback penalty grows with frame size, and the answer is end-to-end or not at all.
///
/// Probed by actually transcoding a clip, because the encoder being listed proves nothing (every
/// Linux ffmpeg build compiles `mjpeg_vaapi` in, GPU or no GPU). Cached for the process and logged
/// once, so a tester's log states the verdict.
#[cfg(target_os = "linux")]
pub fn vaapi_render_node() -> Option<&'static str> {
    static NODE: OnceLock<Option<String>> = OnceLock::new();
    NODE.get_or_init(|| {
        let found = probe_vaapi_render_node();
        match &found {
            Some(node) => log::warn!(
                "[ffmpeg] VAAPI hardware H.264-decode + MJPEG-encode: available on {node} — used for the MJPEG transcode"
            ),
            None => log::warn!(
                "[ffmpeg] VAAPI hardware H.264-decode + MJPEG-encode: unavailable — the MJPEG transcode runs in software"
            ),
        }
        found
    })
    .as_deref()
}

#[cfg(not(target_os = "linux"))]
pub fn vaapi_render_node() -> Option<&'static str> {
    None
}

/// How long a single probe step may take before it is declared a failure and killed.
///
/// Generous on purpose — a cold static ffmpeg off an SD card needs a second or two just to start —
/// but a hard ceiling, because the alternative is unbounded (see `run_probe`).
#[cfg(target_os = "linux")]
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Run one probe step to completion, or kill it at `PROBE_TIMEOUT` and call it a failure.
///
/// Load-bearing: `Command::status()` waits forever, and these probes drive kernel media devices.
/// `h264_v4l2m2m` parks in `VIDIOC_DQBUF` when the decoder is busy or wedged — a documented Pi
/// failure mode — and VAAPI on a render node without a matching driver can stall in the DRM ioctl.
/// A single such stall used to park the `OnceLock` initialiser below *and*, because
/// `get_or_init` blocks every concurrent caller, every later start attempt with it: the stream stayed
/// in "starting" forever and no Stop could reach the ffmpeg behind it, because nothing owns these
/// children once `status()` is waiting on them. Bounded, a bad device costs one slow first start and
/// then reads as "software", which is exactly what it is.
#[cfg(target_os = "linux")]
fn run_probe(ff: &Path, args: &[&str]) -> bool {
    let mut cmd = Command::new(ff);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::child_env::sanitize(&mut cmd);
    let Ok(mut child) = cmd.spawn() else {
        return false;
    };
    let deadline = Instant::now() + PROBE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Err(_) => return false,
            Ok(None) => {}
        }
        if Instant::now() >= deadline {
            log::warn!(
                "[ffmpeg] hardware probe did not finish within {}s — killing it and treating this host \
                 as software-only",
                PROBE_TIMEOUT.as_secs()
            );
            let _ = child.kill();
            let _ = child.wait();
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Try each `/dev/dri/renderD*` in turn (multi-GPU boxes number them 128, 129, …) and return the
/// first that survives a real decode+encode round trip.
#[cfg(target_os = "linux")]
fn probe_vaapi_render_node() -> Option<String> {
    let Some(ff) = find_ffmpeg() else {
        return None;
    };
    let mut nodes: Vec<String> = std::fs::read_dir("/dev/dri")
        .ok()?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.starts_with("renderD").then(|| format!("/dev/dri/{name}"))
        })
        .collect();
    nodes.sort();
    nodes.into_iter().find(|node| probe_vaapi_transcode(&ff, node))
}

/// Encode a throwaway H.264 clip in software, then run it through the full VAAPI chain: hardware
/// decode, frames kept in GPU memory (`-hwaccel_output_format vaapi`), hardware MJPEG encode. Any
/// failure anywhere means "software".
#[cfg(target_os = "linux")]
fn probe_vaapi_transcode(ff: &Path, node: &str) -> bool {
    let clip = std::env::temp_dir().join("kite-vaapi-probe.h264");
    let clip_arg = clip.to_string_lossy().to_string();
    let run = |args: &[&str]| -> bool { run_probe(ff, args) };
    let made = run(&[
        "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i",
        "testsrc=size=320x240:rate=10:duration=1", "-pix_fmt", "yuv420p", "-c:v", "libx264", "-f",
        "h264", &clip_arg,
    ]);
    let ok = made
        && run(&[
            "-hide_banner", "-loglevel", "error", "-hwaccel", "vaapi", "-hwaccel_device", node,
            "-hwaccel_output_format", "vaapi", "-i", &clip_arg, "-c:v", "mjpeg_vaapi", "-f", "null",
            "-",
        ]);
    let _ = std::fs::remove_file(&clip);
    ok
}

/// Encode a throwaway clip in software, then decode it back through `h264_v4l2m2m`. Both steps must
/// succeed; anything else (no ffmpeg, no libx264, no decode device) means "software".
#[cfg(all(target_os = "linux", any(target_arch = "aarch64", target_arch = "arm")))]
fn probe_v4l2_h264_decode() -> bool {
    let Some(ff) = find_ffmpeg() else {
        return false;
    };
    let clip = std::env::temp_dir().join("kite-hwdecode-probe.h264");
    let clip_arg = clip.to_string_lossy().to_string();
    let run = |args: &[&str]| -> bool { run_probe(&ff, args) };
    // `-pix_fmt yuv420p` is load-bearing, not tidiness: `testsrc` emits rgb24 and libx264 then picks
    // **yuv444p / High 4:4:4 Predictive**, which no hardware H.264 block decodes — the Pi's does 4:2:0
    // only. Without it the probe failed on a Pi 4 whose decoder works perfectly for real streams, and
    // the transcode silently stayed in software. The VAAPI probe hit the identical trap.
    let made = run(&[
        "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i",
        "testsrc=size=320x240:rate=10:duration=1", "-pix_fmt", "yuv420p", "-c:v", "libx264", "-f",
        "h264", &clip_arg,
    ]);
    let ok = made
        && run(&[
            "-hide_banner", "-loglevel", "error", "-c:v", "h264_v4l2m2m", "-i", &clip_arg, "-f",
            "null", "-",
        ]);
    let _ = std::fs::remove_file(&clip);
    ok
}

/// BtbN asset selector for this OS+arch: (filename substring, archive extension), or None when we
/// don't auto-download here (manual install). Windows = self-contained `.zip`; Linux = static
/// `.tar.xz` (x86_64 / aarch64). armv7/32-bit and other platforms are intentionally unsupported.
fn asset_match() -> Option<(&'static str, &'static str)> {
    if cfg!(target_os = "windows") {
        Some(("win64", ".zip"))
    } else if cfg!(target_os = "linux") {
        match std::env::consts::ARCH {
            "x86_64" => Some(("linux64", ".tar.xz")),
            "aarch64" => Some(("linuxarm64", ".tar.xz")),
            _ => None,
        }
    } else {
        None
    }
}

/// User-facing "do it yourself" message when auto-download isn't available/possible.
fn manual_install_msg() -> String {
    format!(
        "Automatic ffmpeg download isn't available for this system. Install ffmpeg manually (e.g. from \
         {}, or your distro's package manager) and place it next to the app, on your PATH, or in {}.",
        RELEASES_PAGE,
        crate::flightlog::decoder::install_dir().display()
    )
}

/// Download ffmpeg into the app-data `bin/` dir, reporting coarse progress (0..100). Windows
/// (self-contained GPL `.zip`) + Linux x86_64/aarch64 (static GPL `.tar.xz`, unpacked via the system
/// `tar`). Other platforms/arches return a manual-install hint. Returns the binary path.
pub async fn download<F: FnMut(u8, &str)>(mut report: F) -> Result<PathBuf, String> {
    // Same platform rule as MediaMTX: a downloaded binary can never execute on either mobile OS.
    if cfg!(any(target_os = "android", target_os = "ios")) {
        return Err("ffmpeg cannot run on this device: mobile systems forbid executing a downloaded \
                    binary. Camera and capture-device sources work without it."
            .into());
    }
    let (want_substr, want_ext) = asset_match().ok_or_else(manual_install_msg)?;

    // Self-contained static GPL build for this platform, NOT the -shared variant (needs separate
    // libs). Pinned to the STABLE RELEASE BRANCH, not `master`: the old `ffmpeg-master-latest-…`
    // names handed every user whatever nightly happened to be current that day, so no two installs
    // ran the same build and no field report was reproducible. The `n9.0-latest` assets track the
    // release/9.0 branch instead — today that is 9.0.1, and later downloads pick up 9.0.x patch
    // fixes only, never a new major. BtbN's names are fixed across releases, so this still resolves
    // through the CDN path instead of the rate-limited REST API (which 403s for everyone behind a
    // shared IP — see `crate::github_release`). Bump `FFMPEG_BRANCH` deliberately, with a re-test.
    let asset_name = format!("ffmpeg-n{FFMPEG_BRANCH}-latest-{want_substr}-gpl-{FFMPEG_BRANCH}{want_ext}");
    let url = crate::github_release::latest_asset_url(REPO, &asset_name);

    let client = reqwest::Client::builder()
        .user_agent(HTTP_USER_AGENT)
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    report(25, "Downloading ffmpeg (~80 MB)");
    let bytes = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Download failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Download failed: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("Download read failed: {e}"))?;

    report(70, "Extracting");
    let dir = crate::flightlog::decoder::install_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    let target = dir.join(binary_name());
    if want_ext == ".zip" {
        extract_from_zip(&bytes, &target)?;
    } else {
        extract_from_tar_xz(&bytes, &target, &dir)?;
    }
    make_executable(&target)?;

    report(100, "Done");
    log::info!("ffmpeg installed from {} -> {}", asset_name, target.display());
    Ok(target)
}

/// Extract the ffmpeg binary from a BtbN `.tar.xz` (it nests it under `<root>/bin/ffmpeg`) using the
/// system `tar` (with xz). If `tar`/`xz` isn't available or extraction fails, returns a manual-install
/// hint — we don't bundle an xz decoder.
fn extract_from_tar_xz(bytes: &[u8], target: &Path, dir: &Path) -> Result<(), String> {
    let archive = dir.join("ffmpeg-download.tar.xz");
    let out = dir.join("ffmpeg-download-extract");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::write(&archive, bytes)
        .map_err(|e| format!("Cannot write {}: {e}", archive.display()))?;
    std::fs::create_dir_all(&out).map_err(|e| format!("Cannot create {}: {e}", out.display()))?;

    let cleanup = || {
        let _ = std::fs::remove_file(&archive);
        let _ = std::fs::remove_dir_all(&out);
    };

    // `-xJf` = extract + xz-decompress. Needs `tar` (and xz support) on the system — universal on
    // desktop distros, occasionally absent on minimal images.
    let mut tar = Command::new("tar");
    tar.arg("-xJf").arg(&archive).arg("-C").arg(&out);
    crate::child_env::sanitize(&mut tar);
    let status = tar.status();
    let ok = match status {
        Ok(s) if s.success() => true,
        Ok(_) => false,
        Err(_) => {
            cleanup();
            return Err(format!(
                "Could not run `tar` to unpack ffmpeg (is `tar` with xz support installed?). {}",
                manual_install_msg()
            ));
        }
    };
    if !ok {
        cleanup();
        return Err(format!("`tar` failed to unpack the ffmpeg archive. {}", manual_install_msg()));
    }

    let found = find_file(&out, binary_name());
    let result = match found {
        Some(src) => std::fs::copy(&src, target)
            .map(|_| ())
            .map_err(|e| format!("Cannot place ffmpeg at {}: {e}", target.display())),
        None => Err(format!("'{}' was not found inside the downloaded archive", binary_name())),
    };
    cleanup();
    result
}

/// Recursively find the first file named `name` under `dir`.
fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if let Some(found) = find_file(&p, name) {
                return Some(found);
            }
        } else if p.file_name().and_then(|n| n.to_str()) == Some(name) {
            return Some(p);
        }
    }
    None
}

/// Mark a freshly written binary executable (no-op on Windows).
fn make_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)
            .map_err(|e| format!("Cannot stat {}: {e}", path.display()))?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms)
            .map_err(|e| format!("Cannot chmod {}: {e}", path.display()))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Extract the ffmpeg binary from the release zip by basename (BtbN nests it under `<root>/bin/`).
fn extract_from_zip(zip_bytes: &[u8], target: &Path) -> Result<(), String> {
    let reader = std::io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(reader).map_err(|e| format!("Bad zip archive: {e}"))?;
    let want = binary_name();

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Zip read error: {e}"))?;
        if !entry.is_file() {
            continue;
        }
        let entry_name = entry.name().replace('\\', "/");
        if entry_name.rsplit('/').next() == Some(want) {
            let mut buf = Vec::with_capacity(entry.size() as usize);
            entry
                .read_to_end(&mut buf)
                .map_err(|e| format!("Zip extract error: {e}"))?;
            std::fs::write(target, &buf)
                .map_err(|e| format!("Cannot write {}: {e}", target.display()))?;
            return Ok(());
        }
    }
    Err(format!("'{}' was not found inside the downloaded archive", want))
}
