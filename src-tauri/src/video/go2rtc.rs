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

const REPO: &str = "AlexxIT/go2rtc";
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
    crate::child_env::sanitize(&mut cmd);
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

/// The go2rtc release asset for this OS+arch, or None if we don't auto-download here (manual install).
/// Windows = a zip (go2rtc.exe inside); Linux = a raw binary (no archive). armv7/32-bit and other
/// platforms are intentionally unsupported — too little RAM/CPU to run Kite + video usefully.
fn release_asset_name() -> Option<&'static str> {
    if cfg!(target_os = "windows") {
        Some("go2rtc_win64.zip")
    } else if cfg!(target_os = "linux") {
        match std::env::consts::ARCH {
            "x86_64" => Some("go2rtc_linux_amd64"),
            "aarch64" => Some("go2rtc_linux_arm64"),
            _ => None,
        }
    } else if cfg!(target_os = "macos") {
        // macOS assets are zips (like Windows) with a bare `go2rtc` binary inside — the
        // download()/extract_from_zip path handles the unzip + chmod automatically.
        match std::env::consts::ARCH {
            "x86_64" => Some("go2rtc_mac_amd64.zip"),
            "aarch64" => Some("go2rtc_mac_arm64.zip"),
            _ => None,
        }
    } else {
        None
    }
}

/// User-facing "do it yourself" message when auto-download isn't available/possible.
fn manual_install_msg() -> String {
    format!(
        "Automatic go2rtc download isn't available for this system. Install go2rtc manually (from {}) \
         and place it next to the app, on your PATH, or in {}.",
        RELEASES_PAGE,
        crate::flightlog::decoder::install_dir().display()
    )
}

/// Download go2rtc into the app-data `bin/` dir (Windows + Linux x86_64/arm64 + macOS). Returns the path.
///
/// Resolved **without** the GitHub REST API: go2rtc's asset names are fixed per OS+arch, so
/// `releases/latest/download/<name>` goes straight to the newest build. The API is rate-limited per IP
/// and 403s for everyone behind a shared address — see `crate::github_release`.
pub async fn download<F: FnMut(u8, &str)>(mut report: F) -> Result<PathBuf, String> {
    let asset_name = release_asset_name().ok_or_else(manual_install_msg)?;
    let url = crate::github_release::latest_asset_url(REPO, asset_name);

    let client = reqwest::Client::builder()
        .user_agent(HTTP_USER_AGENT)
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

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
    if asset_name.ends_with(".zip") {
        extract_from_zip(&bytes, &target)?;
    } else {
        // Linux: the asset is the raw binary.
        std::fs::write(&target, &bytes)
            .map_err(|e| format!("Cannot write {}: {e}", target.display()))?;
    }
    make_executable(&target)?;

    report(100, "Done");
    eprintln!("go2rtc installed from {} -> {}", asset_name, target.display());
    Ok(target)
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

        // Fresh spawn → any surviving reader from a dead instance is stale; sweep it first.
        kill_stale_readers();

        // Pick free loopback ports: one for the HTTP API, one for go2rtc's own RTSP server (used as
        // the internal target for the ffmpeg-source fallback — must NOT collide with the user's RTSP
        // source, e.g. obs-rtspserver also defaults to 8554).
        let api_port = free_loopback_port()?;
        let rtsp_port = free_loopback_port()?;
        // A guaranteed-free WebRTC port: if go2rtc's default (8555) is busy, pion's UDP mux stays nil
        // and any ICE op panics the whole process (go2rtc #1851/#1855). Pin it + advertise the
        // loopback host candidate so same-machine ICE connects directly.
        let webrtc_port = free_loopback_webrtc_port()?;
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
        // Hardware transcode templates. They live HERE, as named `ffmpeg:` entries, rather than being
        // spelled out in the source string, because go2rtc rejects any source containing a space
        // ("source with spaces may be insecure" → HTTP 400 on PUT /api/streams). A named template is
        // referenced as `#input=NAME` / `#video=NAME`, so the source stays space-free while the
        // expansion carries as many arguments as it likes. See `commands::video::video_webrtc_start`.
        let mut ffmpeg_cfg = serde_json::Map::new();
        ffmpeg_cfg.insert("bin".into(), ffmpeg_bin.to_string_lossy().into());
        // Pi-class SoCs: hardware H.264 decode only (there is no V4L2 M2M MJPEG encoder), mirroring
        // go2rtc's own `rtsp/udp` input template so a stream that reads on one reads on the other.
        if super::ffmpeg::v4l2_h264_decode_available() {
            ffmpeg_cfg.insert(
                "kite_hw_input".into(),
                "-c:v h264_v4l2m2m -fflags nobuffer -flags low_delay -timeout {timeout} \
                 -user_agent go2rtc/ffmpeg -i {input}"
                    .into(),
            );
        } else if let Some(node) = super::ffmpeg::vaapi_render_node() {
            // Desktop GPUs: the whole chain on the GPU. `-hwaccel_output_format vaapi` is the load-
            // bearing part — it keeps decoded frames in GPU memory for the encoder below. Without it
            // every frame is copied back to system memory and the chain ends up SLOWER than software.
            ffmpeg_cfg.insert(
                "kite_hw_input".into(),
                format!(
                    "-hwaccel vaapi -hwaccel_device {node} -hwaccel_output_format vaapi \
                     -fflags nobuffer -flags low_delay -timeout {{timeout}} \
                     -user_agent go2rtc/ffmpeg -i {{input}}"
                )
                .into(),
            );
            // `-async_depth 1`: the VAAPI encoders pipeline 2 frames by default for throughput, which
            // on a live FPV feed is simply latency (~1 frame at 60 fps, more at lower rates) — and the
            // software encoder it replaces has none. We want the frame out, not the frame rate.
            ffmpeg_cfg.insert("kite_hw_mjpeg".into(), "-c:v mjpeg_vaapi -async_depth 1".into());
        }
        let cfg = serde_json::json!({
            "api": { "listen": format!("127.0.0.1:{api_port}") },
            "rtsp": { "listen": format!("127.0.0.1:{rtsp_port}") },
            "webrtc": {
                "listen": format!("127.0.0.1:{webrtc_port}"),
                "candidates": [format!("127.0.0.1:{webrtc_port}")],
            },
            "ffmpeg": serde_json::Value::Object(ffmpeg_cfg),
            "log": { "level": "warn" },
        });
        std::fs::write(&cfg_path, cfg.to_string())
            .map_err(|e| format!("Cannot write go2rtc config: {e}"))?;

        let mut cmd = Command::new(&bin);
        cmd.arg("-config").arg(&cfg_path);
        // go2rtc spawns ffmpeg itself, so a poisoned environment here breaks the readers too.
        crate::child_env::sanitize(&mut cmd);
        cmd.stdout(Stdio::null()).stderr(Stdio::piped()).stdin(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }

        let mut child = cmd.spawn().map_err(|e| format!("Cannot start go2rtc: {e}"))?;
        // Drain stderr to the terminal for diagnostics.
        // Drain go2rtc's stderr into OUR log file. It runs at its own `warn` level, so these lines are
        // the engine's account of why a source failed (RTSP connect refused, codec mismatch, a reader
        // dying) — the single most useful thing when a stream won't come up. Until now they only went
        // to `eprintln!`, i.e. nowhere a tester could reach: a release build has no console, and the log
        // file the Diagnostics page hands out never saw them.
        if let Some(err) = child.stderr.take() {
            std::thread::spawn(move || {
                let reader = std::io::BufReader::new(err);
                for line in reader.lines().map_while(Result::ok) {
                    let line = line.trim();
                    if !line.is_empty() {
                        log::warn!("[video][go2rtc] {line}");
                    }
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

        log::info!("go2rtc running on 127.0.0.1:{api_port}");
        *guard = Some(Running { child, api_port });
        Ok(api_port)
    }

    /// Stop the running go2rtc process (if any). Idempotent.
    ///
    /// Best-effort graceful teardown first: DELETE the stream via the API so go2rtc reaps its
    /// spawned ffmpeg readers. A bare `child.kill()` orphans them (observed on Windows): the
    /// leaked readers keep holding RTSP sessions on the remote server — which wedged the
    /// UAV-Link Pi's shared media (new sessions starved until a server restart).
    pub fn stop(&self) {
        if let Some(mut r) = self.inner.lock().unwrap().take() {
            delete_stream_blocking(r.api_port);
            // Give go2rtc a moment to terminate the ffmpeg producer before the hard kill.
            std::thread::sleep(Duration::from_millis(300));
            // Anything still parented to go2rtc after the DELETE is a producer it failed to reap, and
            // `child.kill()` below does not take it with it — it is merely reparented to init, where
            // nothing links it back to us any more. `kill_stale_readers` cannot help either: it
            // identifies readers by their `rtsp://127.0.0.1:<port>/kite` publish target, and the MJPEG
            // path's producer writes to a pipe (`-f mjpeg -`) and has no such target. Observed on a
            // Pi 4: a full-rate 720p transcode kept running after Stop, with go2rtc already gone.
            // Collect BEFORE the kill — afterwards the parent link is lost.
            let strays = child_pids(r.child.id());
            let _ = r.child.kill();
            let _ = r.child.wait();
            for pid in strays {
                log::warn!("[video] go2rtc left a producer behind (pid {pid}) — killing it");
                kill_pid(pid);
            }
            log::info!("go2rtc stopped (was on :{}).", r.api_port);
        }
    }
}

/// Never let a go2rtc outlive the app: it keeps its ffmpeg readers — and therefore the RTSP session on
/// the remote server — alive until something kills it. Tauri isn't guaranteed to drop managed state on
/// exit, so this is the backstop; the primary path is the `RunEvent::Exit` hook in `lib.rs`.
impl Drop for Go2Rtc {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Raw-HTTP `DELETE /api/streams?src=kite` against the local go2rtc API (std TcpStream — this runs
/// in sync contexts like `stop()`/app-exit where no async runtime is guaranteed). Best-effort.
fn delete_stream_blocking(api_port: u16) {
    let addr: SocketAddr = ([127, 0, 0, 1], api_port).into();
    if let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(300)) {
        let _ = s.set_write_timeout(Some(Duration::from_millis(300)));
        let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
        let req = format!(
            "DELETE /api/streams?src=kite HTTP/1.1\r\nHost: 127.0.0.1:{api_port}\r\nConnection: close\r\n\r\n"
        );
        use std::io::{Read as _, Write as _};
        let _ = s.write_all(req.as_bytes());
        let mut buf = [0u8; 256];
        let _ = s.read(&mut buf); // wait for the response so go2rtc actually processed it
    }
}

/// Kill ffmpeg readers left over from a previous, no-longer-running go2rtc instance (a hard app exit
/// orphans them — go2rtc's children outlive it). They keep consuming the remote RTSP server.
///
/// Readers are identified by their publish target (`rtsp://127.0.0.1:<port>/kite`), which no other
/// ffmpeg we spawn (MJPEG server, device probes) and no user ffmpeg ever has. Crucially the port tells
/// us **whose** reader it is: if something is still listening there, the owning go2rtc is alive — a
/// second Kite instance, or a dev build running beside the installed one — and killing its reader
/// would black out that instance's video. The old blanket `pkill`/`Stop-Process` did exactly that.
/// So: enumerate, and only kill readers whose go2rtc is gone.
fn kill_stale_readers() {
    let Some(listing) = list_ffmpeg_processes() else { return };
    for (pid, port) in parse_reader_candidates(&listing) {
        let addr: SocketAddr = ([127, 0, 0, 1], port).into();
        if TcpStream::connect_timeout(&addr, Duration::from_millis(150)).is_ok() {
            log::debug!("[video] ffmpeg reader pid {pid} left alone — its go2rtc on :{port} is alive");
            continue;
        }
        log::warn!("[video] killing orphaned ffmpeg reader pid {pid} (go2rtc on :{port} is gone)");
        kill_pid(pid);
    }
}

/// `pid <command line>` for every running ffmpeg, or None if the process listing isn't available.
fn list_ffmpeg_processes() -> Option<String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = Command::new("powershell");
        cmd.args([
            "-NoProfile",
            "-Command",
            "Get-CimInstance Win32_Process -Filter \"Name='ffmpeg.exe'\" | ForEach-Object { \"$($_.ProcessId) $($_.CommandLine)\" }",
        ]);
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        cmd.stdin(Stdio::null()).stderr(Stdio::null());
        let out = cmd.output().ok()?;
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    }
    #[cfg(not(windows))]
    {
        // `-ww` = don't truncate the command line (BSD/macOS ps clips to the terminal width).
        let out = Command::new("ps")
            .args(["-ww", "-eo", "pid=,args="])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

/// PIDs whose parent is `ppid`. Best-effort: an unavailable process listing means an empty list.
fn child_pids(ppid: u32) -> Vec<u32> {
    let Some(listing) = list_process_parents() else { return Vec::new() };
    parse_children(&listing, ppid)
}

/// `pid ppid` for every running process, or None if the listing isn't available.
fn list_process_parents() -> Option<String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = Command::new("powershell");
        cmd.args([
            "-NoProfile",
            "-Command",
            "Get-CimInstance Win32_Process | ForEach-Object { \"$($_.ProcessId) $($_.ParentProcessId)\" }",
        ]);
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        cmd.stdin(Stdio::null()).stderr(Stdio::null());
        let out = cmd.output().ok()?;
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    }
    #[cfg(not(windows))]
    {
        let out = Command::new("ps")
            .args(["-eo", "pid=,ppid="])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

/// Pure part of `child_pids` (unit-tested): the first column of every line whose second column is
/// `ppid`. Both listings are `pid ppid`, whitespace-padded on Unix and CRLF-terminated on Windows.
fn parse_children(listing: &str, ppid: u32) -> Vec<u32> {
    listing
        .lines()
        .filter_map(|line| {
            let mut cols = line.split_whitespace();
            let pid = cols.next()?.parse::<u32>().ok()?;
            let parent = cols.next()?.parse::<u32>().ok()?;
            (parent == ppid).then_some(pid)
        })
        .collect()
}

/// Terminate a process by pid (best-effort).
fn kill_pid(pid: u32) {
    let pid_s = pid.to_string();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = Command::new("taskkill");
        cmd.args(["/PID", &pid_s, "/F"]);
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        cmd.stdout(Stdio::null()).stderr(Stdio::null()).stdin(Stdio::null());
        let _ = cmd.status();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill")
            .args(["-9", &pid_s])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// Parse a `pid <command line>` listing into `(pid, go2rtc rtsp port)` for every ffmpeg that publishes
/// a go2rtc reader stream. Pure (unit-tested) — the per-OS part is only the listing itself.
fn parse_reader_candidates(listing: &str) -> Vec<(u32, u16)> {
    let mut out = Vec::new();
    for line in listing.lines() {
        let line = line.trim_start();
        let Some((pid, rest)) = line.split_once(char::is_whitespace) else { continue };
        let Ok(pid) = pid.parse::<u32>() else { continue };
        if !rest.contains("ffmpeg") {
            continue;
        }
        if let Some(port) = reader_target_port(rest) {
            out.push((pid, port));
        }
    }
    out
}

/// The go2rtc RTSP port a reader publishes to, from `rtsp://127.0.0.1:<port>/kite…` in its command
/// line. None if this ffmpeg isn't one of our readers.
fn reader_target_port(cmdline: &str) -> Option<u16> {
    const HOST: &str = "127.0.0.1:";
    let mut from = 0;
    while let Some(rel) = cmdline[from..].find(HOST) {
        let start = from + rel + HOST.len();
        let digits: String = cmdline[start..].chars().take_while(char::is_ascii_digit).collect();
        let after = start + digits.len();
        if !digits.is_empty() && cmdline[after..].starts_with("/kite") {
            return digits.parse().ok();
        }
        from = start;
    }
    None
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

/// Grab a loopback port that is free for **both UDP and TCP** — go2rtc's WebRTC listener binds UDP for
/// the ICE mux and also accepts TCP candidates on the same port. Probing only TCP (as this used to)
/// can hand out a UDP-occupied port, which is exactly the case that leaves pion's UDP mux nil and
/// panics go2rtc on the first ICE operation. UDP first (that's the binding that must succeed), then
/// TCP verified while the UDP socket is still held.
fn free_loopback_webrtc_port() -> Result<u16, String> {
    let mut last = String::new();
    for _ in 0..10 {
        let udp = std::net::UdpSocket::bind(("127.0.0.1", 0))
            .map_err(|e| format!("Cannot allocate a WebRTC port: {e}"))?;
        let port = udp
            .local_addr()
            .map_err(|e| format!("Cannot read allocated WebRTC port: {e}"))?
            .port();
        match std::net::TcpListener::bind(("127.0.0.1", port)) {
            Ok(_) => return Ok(port),
            Err(e) => last = e.to_string(),
        }
    }
    Err(format!("Cannot find a port free for both UDP and TCP: {last}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reader_candidates_from_ps_listing() {
        // Realistic `ps -ww -eo pid=,args=` output: one go2rtc reader, one native-capture ffmpeg
        // (MJPEG server — must NOT match), one unrelated user ffmpeg, one non-ffmpeg process.
        let listing = "\
  1234 /home/u/.local/share/kite-gc/bin/ffmpeg -hide_banner -i rtsp://cam.local:8554/cam -c copy -f rtsp rtsp://127.0.0.1:41233/kite?video
  1235 ffmpeg -f v4l2 -framerate 30 -video_size 1280x720 -i /dev/video0 -c:v mjpeg -f mpjpeg -
  1236 ffmpeg -i movie.mp4 -c copy out.mkv
  9999 /usr/bin/go2rtc -config /home/u/.local/share/kite-gc/bin/kite-go2rtc.yaml";
        assert_eq!(parse_reader_candidates(listing), vec![(1234, 41233)]);
    }

    #[test]
    fn reader_candidates_from_windows_listing() {
        // Raw string: real Win32 command lines carry quotes and backslashes. CRLF line ends too, since
        // the listing comes back from PowerShell.
        let listing = concat!(
            r#"4312 "C:\Users\u\AppData\Roaming\kite-gc\bin\ffmpeg.exe" -i rtsp://10.0.0.5:8554/cam -c copy -f rtsp rtsp://127.0.0.1:52001/kite"#,
            "\r\n",
            r#"4400 ffmpeg.exe -f dshow -i video=Cam -f mpjpeg -"#,
            "\r\n",
        );
        assert_eq!(parse_reader_candidates(listing), vec![(4312, 52001)]);
    }

    #[test]
    fn children_of_a_go2rtc_pid() {
        // `ps -eo pid=,ppid=` pads its columns; the MJPEG producer is the one that must be found,
        // and it is exactly the shape `parse_reader_candidates` cannot see (a pipe, no publish port).
        let listing = "\
    1       0
 9999    1234
 7731    9999
 7732    9999
 8100       1";
        assert_eq!(parse_children(listing, 9999), vec![7731, 7732]);
        assert_eq!(parse_children(listing, 8100), Vec::<u32>::new());
        // Windows: CRLF, no padding.
        assert_eq!(parse_children("4312 9999\r\n4400 1\r\n", 9999), vec![4312]);
    }

    #[test]
    fn target_port_needs_the_kite_path() {
        assert_eq!(reader_target_port("-f rtsp rtsp://127.0.0.1:8554/kite"), Some(8554));
        // A source that merely happens to be on loopback is not a reader target.
        assert_eq!(reader_target_port("-i rtsp://127.0.0.1:8554/cam -f mpjpeg -"), None);
        // The API base URL appears first; the reader target later on the same line.
        assert_eq!(
            reader_target_port("--api http://127.0.0.1:1984/ -f rtsp rtsp://127.0.0.1:41233/kite?video"),
            Some(41233)
        );
        assert_eq!(reader_target_port("no loopback here"), None);
    }
}
