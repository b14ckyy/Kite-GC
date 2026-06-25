// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! go2rtc — the RTSP→WebRTC streaming engine (replaces the old ffmpeg→fMP4 bridge for live video).
//! go2rtc ingests an RTSP source and republishes it as low-latency WebRTC, which the webview plays
//! natively in a `<video>` (a real MediaStream → shares across all sinks like the camera path).
//!
//! Discovery/download mirror the `blackbox_decode`/ffmpeg model (`flightlog::decoder`): not bundled,
//! found next to the app / on PATH / in the writable app-data `bin/` dir, fetched on demand from
//! AlexxIT/go2rtc releases. We run one local instance bound to 127.0.0.1 on an ephemeral port and
//! drive it over its HTTP API (add stream + WebRTC SDP exchange), proxied from Rust to avoid CORS.

use std::io::{BufRead, Read};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::Value;

const RELEASES_API: &str = "https://api.github.com/repos/AlexxIT/go2rtc/releases/latest";
const RELEASES_PAGE: &str = "https://github.com/AlexxIT/go2rtc/releases";
const HTTP_USER_AGENT: &str = "Kite-GC go2rtc-fetch";

/// Filename of go2rtc for the current platform.
pub fn binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "go2rtc.exe"
    } else {
        "go2rtc"
    }
}

/// Discover go2rtc: next to the exe → app-data install dir (where the download lands) → PATH.
pub fn find_go2rtc() -> Option<PathBuf> {
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

/// Presence string for the UI ("go2rtc <version>" when readable, else "go2rtc installed"); None if
/// not found anywhere.
pub fn status() -> Option<String> {
    let bin = find_go2rtc()?;
    let mut cmd = Command::new(&bin);
    cmd.arg("--version");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    if let Ok(out) = cmd.output() {
        let text = String::from_utf8_lossy(&out.stdout);
        if let Some(line) = text.lines().find(|l| !l.trim().is_empty()) {
            return Some(line.trim().to_string());
        }
    }
    Some("go2rtc installed".to_string())
}

/// Download go2rtc into the app-data `bin/` dir (Windows-only auto-install for now). Returns the path.
pub async fn download<F: FnMut(u8, &str)>(mut report: F) -> Result<PathBuf, String> {
    if !cfg!(target_os = "windows") {
        return Err(format!(
            "Automatic go2rtc download is currently Windows-only. Install go2rtc manually (e.g. from \
             {}) and place it next to the app or on PATH.",
            RELEASES_PAGE
        ));
    }

    report(5, "Querying latest go2rtc release");
    let client = reqwest::Client::builder()
        .user_agent(HTTP_USER_AGENT)
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let release: Value = client
        .get(RELEASES_API)
        .send()
        .await
        .map_err(|e| format!("Release query failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Release query failed: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Release JSON parse failed: {e}"))?;

    let assets = release
        .get("assets")
        .and_then(Value::as_array)
        .ok_or("Latest release has no downloadable assets")?;

    // Windows x64 zip (contains go2rtc.exe at the archive root).
    let (asset_name, url) = assets
        .iter()
        .find_map(|a| {
            let name = a.get("name").and_then(Value::as_str)?;
            let url = a.get("browser_download_url").and_then(Value::as_str)?;
            (name == "go2rtc_win64.zip").then(|| (name.to_string(), url.to_string()))
        })
        .ok_or("No go2rtc_win64.zip asset found in the latest release")?;

    report(25, "Downloading go2rtc");
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
    extract_from_zip(&bytes, &target)?;

    report(100, "Done");
    eprintln!("go2rtc installed from {} -> {}", asset_name, target.display());
    Ok(target)
}

/// Extract the go2rtc binary from the release zip by basename.
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

// ── Running instance ─────────────────────────────────────────────────

struct Running {
    child: Child,
    api_port: u16,
}

/// Managed Tauri state: at most one local go2rtc process bound to 127.0.0.1.
#[derive(Default)]
pub struct Go2Rtc {
    inner: Mutex<Option<Running>>,
}

impl Go2Rtc {
    pub fn new() -> Self {
        Self::default()
    }

    /// The API port if go2rtc is currently running.
    pub fn port(&self) -> Option<u16> {
        let guard = self.inner.lock().unwrap();
        guard.as_ref().map(|r| r.api_port)
    }

    /// Ensure go2rtc is running; spawn it (bound to an ephemeral 127.0.0.1 API port) if not.
    /// Returns the API port. Synchronous (spawn + readiness poll, no await).
    pub fn ensure_running(&self) -> Result<u16, String> {
        let mut guard = self.inner.lock().unwrap();

        // Reap a dead instance.
        if let Some(r) = guard.as_mut() {
            if matches!(r.child.try_wait(), Ok(Some(_))) {
                *guard = None;
            }
        }
        if let Some(r) = guard.as_ref() {
            return Ok(r.api_port);
        }

        let bin = find_go2rtc().ok_or(
            "go2rtc not found — download it in the Video panel or place it next to the app / on PATH.",
        )?;

        // Pick free loopback ports: one for the HTTP API, one for go2rtc's own RTSP server (used as
        // the internal target for the ffmpeg-source fallback — must NOT collide with the user's RTSP
        // source, e.g. obs-rtspserver also defaults to 8554).
        let api_port = free_loopback_port()?;
        let rtsp_port = free_loopback_port()?;
        // A guaranteed-free WebRTC port: if go2rtc's default (8555) is busy, pion's UDP mux stays nil
        // and any ICE op panics the whole process (go2rtc #1851/#1855). Pin it + advertise the
        // loopback host candidate so same-machine ICE connects directly.
        let webrtc_port = free_loopback_port()?;
        let dir = crate::flightlog::decoder::install_dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
        let cfg_path = dir.join("kite-go2rtc.yaml");

        // JSON is valid YAML — go2rtc parses it. A real file (not inline) so its config-patch on
        // PUT /api/streams succeeds. Point go2rtc at our bundled ffmpeg so the `ffmpeg:` source
        // fallback works for quirky RTSP servers go2rtc's native client can't read.
        // Point go2rtc at ffmpeg by its resolved path, or — if not installed yet — at the path the
        // guided download WILL write to. go2rtc spawns ffmpeg per-source on demand, so a later
        // download is picked up on the next stream start without restarting go2rtc.
        let ffmpeg_bin = super::ffmpeg::find_ffmpeg()
            .unwrap_or_else(|| dir.join(super::ffmpeg::binary_name()));
        let cfg = serde_json::json!({
            "api": { "listen": format!("127.0.0.1:{api_port}") },
            "rtsp": { "listen": format!("127.0.0.1:{rtsp_port}") },
            "webrtc": {
                "listen": format!("127.0.0.1:{webrtc_port}"),
                "candidates": [format!("127.0.0.1:{webrtc_port}")],
            },
            "ffmpeg": { "bin": ffmpeg_bin.to_string_lossy() },
            "log": { "level": "warn" },
        });
        std::fs::write(&cfg_path, cfg.to_string())
            .map_err(|e| format!("Cannot write go2rtc config: {e}"))?;

        let mut cmd = Command::new(&bin);
        cmd.arg("-config").arg(&cfg_path);
        cmd.stdout(Stdio::null()).stderr(Stdio::piped()).stdin(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }

        let mut child = cmd.spawn().map_err(|e| format!("Cannot start go2rtc: {e}"))?;
        // Drain stderr to the terminal for diagnostics.
        if let Some(err) = child.stderr.take() {
            std::thread::spawn(move || {
                let reader = std::io::BufReader::new(err);
                for line in reader.lines().map_while(Result::ok) {
                    eprintln!("go2rtc: {line}");
                }
            });
        }

        // Wait for the API port to accept connections (≈3 s budget).
        let addr: SocketAddr = ([127, 0, 0, 1], api_port).into();
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut ready = false;
        while Instant::now() < deadline {
            if TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() {
                ready = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if !ready {
            let _ = child.kill();
            return Err("go2rtc did not become ready on its API port".to_string());
        }

        eprintln!("go2rtc running on 127.0.0.1:{api_port}");
        *guard = Some(Running { child, api_port });
        Ok(api_port)
    }

    /// Stop the running go2rtc process (if any). Idempotent.
    pub fn stop(&self) {
        if let Some(mut r) = self.inner.lock().unwrap().take() {
            let _ = r.child.kill();
            let _ = r.child.wait();
            eprintln!("go2rtc stopped (was on :{}).", r.api_port);
        }
    }
}

/// Grab a free loopback TCP port by binding to :0 and reading it back.
fn free_loopback_port() -> Result<u16, String> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
        .map_err(|e| format!("Cannot allocate a port: {e}"))?;
    listener
        .local_addr()
        .map(|a| a.port())
        .map_err(|e| format!("Cannot read allocated port: {e}"))
}
