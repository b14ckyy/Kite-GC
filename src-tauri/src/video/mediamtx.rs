// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

//! MediaMTX — the RTSP→WebRTC engine (replaces go2rtc + its ffmpeg republish).
//!
//! go2rtc could not read UDP-only RTSP servers with its native client, so every such source (the
//! UAV-Link Pi among them) went through a spawned ffmpeg that published **back into go2rtc over
//! RTSP/TCP on loopback**. That publish leg was measured to stall in a near-constant ~316–334 ms
//! class — reproduced independently on a second machine with a file source, located to the
//! TCP publish direction (a `-re`-paced sender writing keyframe bursts), and gone the moment the
//! same chain published over UDP. MediaMTX reads UDP-only servers natively, so the ffmpeg middleman
//! disappears for the normal case entirely: one process pulls the source and serves WHEP.
//! Measured against the same Pi, side by side with the old chain on the same afternoon: the old
//! chain produced 3+ such stalls per minute once it tipped (17–59 min uptime), MediaMTX' output had
//! a worst inter-frame gap of 47 ms over 50 minutes — and ~22 ms less end-to-end latency.
//!
//! ffmpeg remains as the *fallback reader* for quirky servers MediaMTX cannot pull (and as the
//! future transcode hook): it reads the source and publishes into MediaMTX — **over UDP**, never
//! TCP, for the reason above.
//!
//! Discovery/download mirror the `blackbox_decode`/ffmpeg model (`flightlog::decoder`): not
//! bundled, found next to the app / on PATH / in the writable app-data `bin/` dir, fetched on
//! demand from bluenviron/mediamtx releases. Unlike go2rtc the version is **pinned**: MediaMTX
//! asset names carry the version (so `releases/latest/download/…` cannot work), and a pin is what
//! we wanted anyway — an unknown config key fails the start hard, so config and binary must move
//! together, and every user runs the build we tested.

use std::io::{BufRead, Read};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The pinned MediaMTX release. Config keys below are validated against exactly this version —
/// an unknown key makes MediaMTX refuse to start, so bump the pin and re-test as one step.
const VERSION: &str = "v1.20.0";
const REPO_RELEASES: &str = "https://github.com/bluenviron/mediamtx/releases";
const HTTP_USER_AGENT: &str = "Kite-GC mediamtx-fetch";

/// The single stream path name (fixed — one live feed).
const STREAM_NAME: &str = "kite";

/// Filename of MediaMTX for the current platform.
pub fn binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "mediamtx.exe"
    } else {
        "mediamtx"
    }
}

/// Discover MediaMTX: next to the exe → app-data install dir (where the download lands) → PATH.
pub fn find_mediamtx() -> Option<PathBuf> {
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

/// Presence string for the UI ("MediaMTX v1.20.0" when readable, else "MediaMTX installed");
/// None if not found anywhere.
pub fn status() -> Option<String> {
    let bin = find_mediamtx()?;
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
            return Some(format!("MediaMTX {}", line.trim()));
        }
    }
    Some("MediaMTX installed".to_string())
}

/// The MediaMTX release asset for this OS+arch, or None if we don't auto-download here (manual
/// install). Windows = zip; Linux/macOS = tar.gz. armv6/armv7 assets exist upstream but stay
/// unsupported here — too little RAM/CPU to run Kite + video usefully (same policy as before).
fn release_asset_name() -> Option<String> {
    let platform = if cfg!(target_os = "windows") {
        // Upstream ships no windows_arm64 asset; amd64 runs under emulation there.
        Some(("windows_amd64", "zip"))
    } else if cfg!(target_os = "linux") {
        match std::env::consts::ARCH {
            "x86_64" => Some(("linux_amd64", "tar.gz")),
            "aarch64" => Some(("linux_arm64", "tar.gz")),
            _ => None,
        }
    } else if cfg!(target_os = "macos") {
        match std::env::consts::ARCH {
            "x86_64" => Some(("darwin_amd64", "tar.gz")),
            "aarch64" => Some(("darwin_arm64", "tar.gz")),
            _ => None,
        }
    } else {
        None
    };
    platform.map(|(plat, ext)| format!("mediamtx_{VERSION}_{plat}.{ext}"))
}

/// User-facing "do it yourself" message when auto-download isn't available/possible.
fn manual_install_msg() -> String {
    format!(
        "Automatic MediaMTX download isn't available for this system. Install MediaMTX {} manually \
         (from {}) and place it next to the app, on your PATH, or in {}.",
        VERSION,
        REPO_RELEASES,
        crate::flightlog::decoder::install_dir().display()
    )
}

/// Download the pinned MediaMTX into the app-data `bin/` dir. Returns the path.
pub async fn download<F: FnMut(u8, &str)>(mut report: F) -> Result<PathBuf, String> {
    // Both mobile systems forbid executing a downloaded binary (Android by W^X on writable app
    // storage, iOS by codesigning), so the engine can never run there — refuse up front with the
    // reason instead of downloading a file that then fails to spawn. Camera / capture sources need
    // no engine and work; RTSP on mobile is the Phase E item (ANDROID_SUPPORT.md §5b).
    if cfg!(any(target_os = "android", target_os = "ios")) {
        return Err("The RTSP video engine cannot run on this device: mobile systems forbid executing \
                    a downloaded binary. Camera and capture-device sources work without it."
            .into());
    }
    let asset_name = release_asset_name().ok_or_else(manual_install_msg)?;
    let url = format!("{REPO_RELEASES}/download/{VERSION}/{asset_name}");

    let client = reqwest::Client::builder()
        .user_agent(HTTP_USER_AGENT)
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    report(25, "Downloading MediaMTX");
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
        extract_from_tar_gz(&bytes, &target)?;
    }
    make_executable(&target)?;

    report(100, "Done");
    log::info!("MediaMTX installed from {} -> {}", asset_name, target.display());
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

/// Extract the MediaMTX binary from the release zip by basename (the archive also carries
/// LICENSE + a sample mediamtx.yml, which we don't want).
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

/// Extract the MediaMTX binary from the Linux/macOS tar.gz by basename.
fn extract_from_tar_gz(archive_bytes: &[u8], target: &Path) -> Result<(), String> {
    let cursor = std::io::Cursor::new(archive_bytes);
    let decoder = flate2::read::GzDecoder::new(cursor);
    let mut archive = tar::Archive::new(decoder);
    let want = binary_name();

    for entry in archive.entries().map_err(|e| format!("Bad tar archive: {e}"))? {
        let mut entry = entry.map_err(|e| format!("Tar read error: {e}"))?;
        let path = entry
            .path()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        if path.rsplit('/').next() != Some(want) {
            continue;
        }
        let mut buf = Vec::new();
        entry
            .read_to_end(&mut buf)
            .map_err(|e| format!("Tar extract error: {e}"))?;
        std::fs::write(target, &buf)
            .map_err(|e| format!("Cannot write {}: {e}", target.display()))?;
        return Ok(());
    }
    Err(format!("'{}' was not found inside the downloaded archive", want))
}

// ── Running instance ─────────────────────────────────────────────────

/// What a running instance was started for. A `start()` with the same spec keeps the process; a
/// different spec (URL/transport/reader changed) tears it down and starts fresh.
#[derive(PartialEq, Eq, Clone)]
pub struct StreamSpec {
    pub url: String,
    /// `udp` | `tcp` | `automatic` — MediaMTX' `rtspTransport` for its native RTSP client.
    pub transport: String,
    /// Read the source with our ffmpeg and publish it into MediaMTX instead of letting MediaMTX
    /// pull it. The fallback for quirky servers (e.g. obs-rtspserver, which 461s any forced
    /// transport and only yields to ffmpeg's UDP→TCP auto-retry dance).
    pub use_ffmpeg: bool,
}

struct Running {
    child: Child,
    /// The ffmpeg fallback reader publishing into MediaMTX (spec.use_ffmpeg only).
    publisher: Option<Child>,
    api_port: u16,
    whep_port: u16,
    spec: StreamSpec,
}

/// Managed Tauri state: at most one local MediaMTX process.
#[derive(Default)]
pub struct MediaMtx {
    inner: Mutex<Option<Running>>,
}

impl MediaMtx {
    pub fn new() -> Self {
        Self::default()
    }

    /// The WHEP HTTP port if MediaMTX is currently running.
    pub fn whep_port(&self) -> Option<u16> {
        let guard = self.inner.lock().unwrap();
        guard.as_ref().map(|r| r.whep_port)
    }

    /// Ensure MediaMTX is running with `spec` and the stream path is ready (source connected,
    /// tracks known). Reuses a live instance with the same spec; anything else is restarted.
    /// Synchronous (spawn + readiness poll, no await) — call from a blocking-capable context.
    pub fn start(&self, spec: StreamSpec) -> Result<(), String> {
        let mut guard = self.inner.lock().unwrap();

        // Reap a dead instance.
        if let Some(r) = guard.as_mut() {
            if matches!(r.child.try_wait(), Ok(Some(_))) {
                take_down(guard.take());
            }
        }
        // Same spec and still ready → nothing to do (the reconnect loop lands here when only the
        // browser-side peer connection died). A publisher that has meanwhile exited fails the
        // readiness check below via `ready == false`, so it re-starts cleanly.
        if let Some(r) = guard.as_ref() {
            if r.spec == spec && path_ready_once(r.api_port) {
                return Ok(());
            }
            take_down(guard.take());
        }

        let bin = find_mediamtx().ok_or(
            "MediaMTX not found — download it in the Video panel or place it next to the app / on PATH.",
        )?;

        // Fresh spawn → any surviving publisher from a dead instance is stale; sweep it first.
        kill_stale_publishers();

        let api_port = free_loopback_port()?;
        let rtsp_port = free_loopback_port()?;
        let whep_port = free_loopback_port()?;
        // The RTSP server's UDP receive pair, used by the ffmpeg publisher below (RTP on the first,
        // RTCP on the second). Loopback — nothing outside the machine ever publishes into us.
        let (rtp_port, rtcp_port) = free_loopback_udp_pair()?;
        // The single ICE mux port for WebRTC media, bound on every interface. NOT loopback, and
        // that is deliberate: a browser never generates a `127.0.0.1` ICE candidate, and Windows
        // drops loopback-addressed packets arriving from another interface — a loopback-only
        // engine can never be paired with (measured on go2rtc: 23 STUN checks, 0 answers, and the
        // session's entire video leaving the machine via NAT hairpin). MediaMTX advertises the
        // real interface addresses (`webrtcIPsFromInterfaces`), the kernel routes them internally,
        // and an empty ICE-server list keeps any STUN detour from coming back.
        let ice_port = free_webrtc_port()?;

        let dir = crate::flightlog::decoder::install_dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
        let cfg_path = dir.join("kite-mediamtx.yml");

        // JSON is valid YAML — MediaMTX parses it (verified against the pinned build). Every key
        // below exists in the pinned version; an unknown key fails the start hard, which is why
        // the version is pinned in the first place.
        let source = if spec.use_ffmpeg {
            // The ffmpeg publisher RECORDs into this path.
            serde_json::json!({ "source": "publisher" })
        } else {
            serde_json::json!({ "source": spec.url, "rtspTransport": spec.transport })
        };
        let cfg = serde_json::json!({
            "logLevel": "warn",
            // The API is loopback-only and exists for exactly one consumer: the readiness poll
            // below (`/v3/paths/get/kite` → `"ready"`).
            "api": true,
            "apiAddress": format!("127.0.0.1:{api_port}"),
            "metrics": false,
            "pprof": false,
            "playback": false,
            "rtsp": true,
            "rtspAddress": format!("127.0.0.1:{rtsp_port}"),
            "rtspTransports": ["udp", "tcp"],
            "rtpAddress": format!("127.0.0.1:{rtp_port}"),
            "rtcpAddress": format!("127.0.0.1:{rtcp_port}"),
            "rtmp": false,
            "hls": false,
            "webrtc": true,
            // WHEP signaling — loopback HTTP, proxied through Rust (CORS + no port guessing in the
            // frontend). Media never flows here; that is the ICE mux above.
            "webrtcAddress": format!("127.0.0.1:{whep_port}"),
            "webrtcEncryption": false,
            "webrtcLocalUDPAddress": format!(":{ice_port}"),
            "webrtcLocalTCPAddress": "",
            "webrtcIPsFromInterfaces": true,
            "webrtcICEServers2": [],
            "srt": false,
            "moq": false,
            "paths": { STREAM_NAME: source },
        });
        std::fs::write(&cfg_path, cfg.to_string())
            .map_err(|e| format!("Cannot write MediaMTX config: {e}"))?;

        let mut cmd = Command::new(&bin);
        cmd.arg(&cfg_path);
        crate::child_env::sanitize(&mut cmd);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }

        let mut child = cmd.spawn().map_err(|e| format!("Cannot start MediaMTX: {e}"))?;
        // MediaMTX logs to STDOUT (not stderr like go2rtc did). At `warn` level these lines are the
        // engine's account of why a source failed — drain them into our log file so a tester's
        // Diagnostics export contains them. stderr carries config errors → same treatment.
        drain_into_log(child.stdout.take());
        drain_into_log(child.stderr.take());

        // Wait for the API to accept connections (spawn budget ≈3 s).
        if !wait_port(api_port, Duration::from_secs(3)) {
            let _ = child.kill();
            return Err("MediaMTX did not become ready on its API port".to_string());
        }

        // The fallback reader: ffmpeg reads the source (NO forced input transport — the only mode
        // quirky servers accept) and publishes into MediaMTX. The publish leg is **UDP with RTP
        // packets under the MTU** — publishing over TCP is precisely the measured ~320 ms stall
        // (a `-re`-class sender writing keyframe bursts into a loopback TCP socket), reproduced
        // and bisected on an independent setup; the same chain over UDP was clean.
        let publisher = if spec.use_ffmpeg {
            Some(spawn_publisher(&spec.url, rtsp_port)?)
        } else {
            None
        };

        // Wait for the stream path to actually carry tracks. Covers the RTSP connect + probe to
        // the source (LAN cameras: <2 s measured; the budget is for flaky links, not the happy
        // path). Without this, the WHEP offer below would 404/blackhole on a source that never
        // came up — the caller gets the error *here*, where "the source is unreachable" is
        // distinguishable from "WebRTC failed".
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if path_ready_once(api_port) {
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                if let Some(mut p) = publisher {
                    let _ = p.kill();
                }
                return Err(format!(
                    "the RTSP source did not come up within 8 s ({} reader) — check the URL/transport; \
                     MediaMTX' own error lines are in the log",
                    if spec.use_ffmpeg { "ffmpeg" } else { "native" }
                ));
            }
            std::thread::sleep(Duration::from_millis(150));
        }

        log::info!(
            "MediaMTX running (api :{api_port}, whep :{whep_port}, reader {})",
            if spec.use_ffmpeg { "ffmpeg" } else { "native" }
        );
        *guard = Some(Running { child, publisher, api_port, whep_port, spec });
        Ok(())
    }

    /// Stop the running MediaMTX (and its ffmpeg publisher, if any). Idempotent.
    pub fn stop(&self) {
        let taken = self.inner.lock().unwrap().take();
        if let Some(r) = &taken {
            log::info!("MediaMTX stopped (was api :{}).", r.api_port);
        }
        take_down(taken);
    }
}

/// Never let a MediaMTX (or its publisher) outlive the app. Tauri isn't guaranteed to drop managed
/// state on exit, so this is the backstop; the primary path is the `RunEvent::Exit` hook in `lib.rs`.
impl Drop for MediaMtx {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Kill a Running's processes: the publisher first (so MediaMTX sees an orderly publisher
/// disconnect rather than the reverse), then the engine. Unlike go2rtc there is no child ffmpeg
/// *inside* the engine to reap — MediaMTX pulls sources itself; the only ffmpeg is ours.
fn take_down(running: Option<Running>) {
    let Some(mut r) = running else { return };
    if let Some(mut p) = r.publisher.take() {
        let _ = p.kill();
        let _ = p.wait();
    }
    let _ = r.child.kill();
    let _ = r.child.wait();
}

/// Spawn the fallback reader: ffmpeg pulls `url` (transport negotiated, nothing forced) and
/// publishes the video track into MediaMTX over RTSP/**UDP**.
fn spawn_publisher(url: &str, rtsp_port: u16) -> Result<Child, String> {
    let ffmpeg = super::ffmpeg::find_ffmpeg()
        .ok_or("the ffmpeg fallback reader needs ffmpeg and it is not installed")?;
    let mut cmd = Command::new(ffmpeg);
    cmd.args([
        "-hide_banner",
        "-v", "error",
        "-allowed_media_types", "video",
        "-fflags", "nobuffer",
        "-flags", "low_delay",
        "-timeout", "5000000",
        "-i", url,
        "-c:v", "copy",
        "-an",
        // UDP publish, packets under the MTU: a lost fragment otherwise costs the whole packet.
        // `-muxdelay 0` drops the RTSP muxer's 0.7 s initial buffer — pointless on loopback.
        "-f", "rtsp",
        "-rtsp_transport", "udp",
        "-pkt_size", "1316",
        "-flush_packets", "1",
        "-muxdelay", "0",
    ]);
    cmd.arg(format!("rtsp://127.0.0.1:{rtsp_port}/{STREAM_NAME}"));
    crate::child_env::sanitize(&mut cmd);
    cmd.stdout(Stdio::null()).stderr(Stdio::piped()).stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().map_err(|e| format!("Cannot start the ffmpeg reader: {e}"))?;
    drain_into_log(child.stderr.take());
    Ok(child)
}

/// Drain a child's output stream into our log file at `warn` — these lines are the helper's own
/// account of why something failed, and the Diagnostics export must contain them.
fn drain_into_log<R: std::io::Read + Send + 'static>(pipe: Option<R>) {
    let Some(pipe) = pipe else { return };
    std::thread::spawn(move || {
        let reader = std::io::BufReader::new(pipe);
        for line in reader.lines().map_while(Result::ok) {
            let line = line.trim();
            if !line.is_empty() {
                log::warn!("[video][mediamtx] {line}");
            }
        }
    });
}

/// One readiness probe against the local API: `GET /v3/paths/get/kite` and a substring check for
/// `"ready":true`. Raw std TcpStream — this runs in sync contexts (start under the state mutex,
/// app exit) where no async runtime is guaranteed.
fn path_ready_once(api_port: u16) -> bool {
    let addr: SocketAddr = ([127, 0, 0, 1], api_port).into();
    let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(300)) else {
        return false;
    };
    let _ = s.set_write_timeout(Some(Duration::from_millis(300)));
    let _ = s.set_read_timeout(Some(Duration::from_millis(700)));
    let req = format!(
        "GET /v3/paths/get/{STREAM_NAME} HTTP/1.1\r\nHost: 127.0.0.1:{api_port}\r\nConnection: close\r\n\r\n"
    );
    use std::io::Write as _;
    if s.write_all(req.as_bytes()).is_err() {
        return false;
    }
    let mut body = String::new();
    let _ = s.read_to_string(&mut body);
    body.contains("\"ready\":true") || body.contains("\"ready\": true")
}

/// Wait until a loopback TCP port accepts connections.
fn wait_port(port: u16, budget: Duration) -> bool {
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        if TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

/// Kill ffmpeg publishers left over from a previous, no-longer-running engine instance (a hard app
/// exit orphans them). They keep consuming the remote RTSP server — which has wedged the UAV-Link
/// Pi's shared media before.
///
/// Publishers are identified by their publish target (`rtsp://127.0.0.1:<port>/kite`), which no
/// other ffmpeg we spawn (MJPEG server, device probes) and no user ffmpeg ever has. The port tells
/// us **whose** publisher it is: if something is still listening there, the owning engine is alive
/// — a second Kite instance, or a dev build beside the installed one — and killing its reader would
/// black out that instance's video. So: enumerate, and only kill publishers whose engine is gone.
fn kill_stale_publishers() {
    let Some(listing) = list_ffmpeg_processes() else { return };
    for (pid, port) in parse_publisher_candidates(&listing) {
        let addr: SocketAddr = ([127, 0, 0, 1], port).into();
        if TcpStream::connect_timeout(&addr, Duration::from_millis(150)).is_ok() {
            log::debug!("[video] ffmpeg publisher pid {pid} left alone — its engine on :{port} is alive");
            continue;
        }
        log::warn!("[video] killing orphaned ffmpeg publisher pid {pid} (engine on :{port} is gone)");
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

/// Parse a `pid <command line>` listing into `(pid, engine rtsp port)` for every ffmpeg that
/// publishes into a local engine. Pure (unit-tested) — the per-OS part is only the listing itself.
fn parse_publisher_candidates(listing: &str) -> Vec<(u32, u16)> {
    let mut out = Vec::new();
    for line in listing.lines() {
        let line = line.trim_start();
        let Some((pid, rest)) = line.split_once(char::is_whitespace) else { continue };
        let Ok(pid) = pid.parse::<u32>() else { continue };
        if !rest.contains("ffmpeg") {
            continue;
        }
        if let Some(port) = publisher_target_port(rest) {
            out.push((pid, port));
        }
    }
    out
}

/// The engine RTSP port a publisher publishes to, from `rtsp://127.0.0.1:<port>/kite…` in its
/// command line. None if this ffmpeg isn't one of our publishers.
fn publisher_target_port(cmdline: &str) -> Option<u16> {
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

/// Grab an adjacent free loopback **UDP** port pair (RTP on the first, RTCP on the second — the
/// RTSP convention MediaMTX' server expects). Rejected candidates are held so Windows' roughly
/// sequential allocator moves on instead of handing the same block back (see `free_webrtc_port`).
fn free_loopback_udp_pair() -> Result<(u16, u16), String> {
    const ATTEMPTS: usize = 64;
    let mut held = Vec::new();
    for _ in 0..ATTEMPTS {
        let first = std::net::UdpSocket::bind(("127.0.0.1", 0))
            .map_err(|e| format!("Cannot allocate a UDP port: {e}"))?;
        let port = first
            .local_addr()
            .map_err(|e| format!("Cannot read allocated UDP port: {e}"))?
            .port();
        if port == u16::MAX {
            held.push(first);
            continue;
        }
        match std::net::UdpSocket::bind(("127.0.0.1", port + 1)) {
            Ok(_) => return Ok((port, port + 1)),
            Err(_) => held.push(first),
        }
    }
    Err(format!("Cannot find an adjacent free UDP port pair after {ATTEMPTS} attempts"))
}

/// Grab a port that is free for **both UDP and TCP** on all interfaces — the WebRTC ICE mux binds
/// UDP there, and probing both keeps the port usable if a TCP mux is ever enabled. Every rejected
/// socket is HELD until we are done: on Windows, Hyper-V/WSL/Docker reserve 100-port blocks of the
/// dynamic range (bind fails with WSAEACCES 10013, a *permission* error), the allocator hands
/// ports out roughly sequentially, and releasing a rejected socket immediately makes every retry
/// come back from the same reserved block — which turned a busy default port into an instant,
/// self-repeating reconnect loop before. Holding the sockets forces the cursor forward.
fn free_webrtc_port() -> Result<u16, String> {
    const ATTEMPTS: usize = 256;
    let mut held = Vec::with_capacity(ATTEMPTS);
    let mut last = String::new();
    for _ in 0..ATTEMPTS {
        let udp = std::net::UdpSocket::bind(("0.0.0.0", 0))
            .map_err(|e| format!("Cannot allocate a WebRTC port: {e}"))?;
        let port = udp
            .local_addr()
            .map_err(|e| format!("Cannot read allocated WebRTC port: {e}"))?
            .port();
        match std::net::TcpListener::bind(("0.0.0.0", port)) {
            Ok(_) => return Ok(port),
            Err(e) => {
                last = e.to_string();
                held.push(udp);
            }
        }
    }
    Err(format!(
        "Cannot find a port free for both UDP and TCP after {ATTEMPTS} attempts. \
         On Windows this is usually a reserved port range — check \
         `netsh interface ipv4 show excludedportrange protocol=tcp`. Last error: {last}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publisher_candidates_from_ps_listing() {
        // Realistic `ps -ww -eo pid=,args=` output: one publisher, one native-capture ffmpeg
        // (MJPEG server — must NOT match), one unrelated user ffmpeg, one non-ffmpeg process.
        let listing = "\
  1234 /home/u/.local/share/kite-gc/bin/ffmpeg -hide_banner -i rtsp://cam.local:8554/cam -c:v copy -f rtsp -rtsp_transport udp rtsp://127.0.0.1:41233/kite
  1235 ffmpeg -f v4l2 -framerate 30 -video_size 1280x720 -i /dev/video0 -c:v mjpeg -f mpjpeg -
  1236 ffmpeg -i movie.mp4 -c copy out.mkv
  9999 /usr/bin/mediamtx /home/u/.local/share/kite-gc/bin/kite-mediamtx.yml";
        assert_eq!(parse_publisher_candidates(listing), vec![(1234, 41233)]);
    }

    #[test]
    fn publisher_candidates_from_windows_listing() {
        // Raw string: real Win32 command lines carry quotes and backslashes. CRLF line ends too,
        // since the listing comes back from PowerShell.
        let listing = concat!(
            r#"4312 "C:\Users\u\AppData\Roaming\kite-gc\bin\ffmpeg.exe" -i rtsp://10.0.0.5:8554/cam -c:v copy -f rtsp rtsp://127.0.0.1:52001/kite"#,
            "\r\n",
            r#"4400 ffmpeg.exe -f dshow -i video=Cam -f mpjpeg -"#,
            "\r\n",
        );
        assert_eq!(parse_publisher_candidates(listing), vec![(4312, 52001)]);
    }

    #[test]
    fn target_port_needs_the_kite_path() {
        assert_eq!(publisher_target_port("-f rtsp rtsp://127.0.0.1:8554/kite"), Some(8554));
        // A source that merely happens to be on loopback is not a publish target.
        assert_eq!(publisher_target_port("-i rtsp://127.0.0.1:8554/cam -f mpjpeg -"), None);
        // Loopback appears twice; the publish target is the later occurrence.
        assert_eq!(
            publisher_target_port("-i rtsp://127.0.0.1:8554/cam -f rtsp rtsp://127.0.0.1:41233/kite"),
            Some(41233)
        );
        assert_eq!(publisher_target_port("no loopback here"), None);
    }

    #[test]
    fn pinned_asset_names_are_wellformed() {
        // The pin appears in both the directory segment and the file name — a malformed constant
        // would 404 at download time, so lock the shape down here.
        if let Some(name) = release_asset_name() {
            assert!(name.starts_with(&format!("mediamtx_{VERSION}_")));
            assert!(name.ends_with(".zip") || name.ends_with(".tar.gz"));
        }
    }
}
