// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

mod aero;
#[cfg(target_os = "android")]
mod android;
mod child_env;
mod commands;
mod debug_mode;
mod flightlog;
mod flightmode;
mod github_release;
mod hid;
mod link_presence;
mod link_stats;
mod logging;
mod mavlink_proto;
mod mission;
mod msp;
mod passive_telemetry;
mod radar;
mod scheduler;
mod state;
mod telemetry_forward;
mod terrain;
mod transport;
mod user_file;
mod video;

use commands::connection::{connect, disconnect, inav_set_craft_name, inav_read_stats, scan_ble_devices, ble_scan_start, ble_scan_stop};
use commands::connection::list_serial_ports;
use commands::flightlog::{
    flightlog_list, flightlog_get, flightlog_get_track, flightlog_get_battery_records, flightlog_delete,
    flightlog_update_notes, flightlog_update_craft_name, flightlog_update_platform_type, flightlog_update_pilot, flightlog_update_weather, flightlog_geocode, flightlog_fetch_weather,
    flightlog_default_db_path, flightlog_default_raw_log_path, flightlog_import_blackbox,
    flightlog_blackbox_log_count,
    flightlog_export, flightlog_export_blackbox, flightlog_blackbox_file_info, flightlog_delete_blackbox_file, flightlog_compact_db, flightlog_export_track, flightlog_import_kflight,
    flightlog_kflight_list, flightlog_kflight_get, flightlog_kflight_track,
    flightlog_probe_ardupilot, flightlog_decode_ardupilot_csv,
    flightlog_import_ardupilot, flightlog_import_raw,
    blackbox_decoder_available, blackbox_decoder_version, download_blackbox_decode,
    flightlog_link_flights, flightlog_unlink_flight, flightlog_find_linkable,
    flightlog_commit_pending_session, flightlog_discard_pending_session,
    flightlog_continue_pending_session,
    flightlog_scan_orphan_sessions, flightlog_recover_discard, flightlog_recover_save_incomplete,
    flightlog_recover_continue,
    mission_db_save, mission_db_get, mission_db_for_flight, flight_link_mission,
    flight_logged_wp_count, mission_db_geocode, mission_db_find_by_hash, mission_db_update,
    flight_unlink_mission, mission_db_delete, mission_db_flights, mission_db_list,
    mission_db_set_meta,
    battery_db_create, battery_db_update, battery_db_list, battery_db_get,
    battery_db_find_by_serial, battery_db_delete, battery_db_add_usage, battery_db_aggregate,
    battery_db_flights, flight_set_battery_serial, battery_db_set_baseline,
    battery_file_write, battery_file_read,
    vehicle_db_create, vehicle_db_update, vehicle_db_list, vehicle_db_get,
    vehicle_db_find_by_craft_name, vehicle_db_delete, vehicle_db_aggregate, vehicle_db_flights,
    vehicle_db_set_baseline, vehicle_file_write, vehicle_file_read,
};
use commands::aero::{aero_fetch, aero_cache_stats, aero_cache_clear};
use commands::hid::{
    hid_start, hid_stop, hid_select_device,
    hid_profiles_dir, hid_profile_list, hid_profile_save, hid_profile_delete,
};
use commands::rc::{
    rc_read_fc_config, rc_set_override_bitmask, rc_read_channels,
    rc_stream_update, rc_stream_set_aux, rc_stream_enable, rc_stream_set_rate,
    rc_stream_set_override, rc_stream_set_manual,
};
use commands::safehome::{safehome_read_all, safehome_write_all};
use commands::geozone::{geozone_read_all, geozone_write_all};
use commands::fence::{fence_read_all, fence_write_all};
use commands::rally::{rally_read_all, rally_write_all};
use commands::info::{get_app_version, is_debug_mode};
use commands::system::{system_active_net_is_wifi, system_on_battery};
use commands::storage::{share_file, storage_pick_folder};
use commands::video::{
    video_ffmpeg_status, video_ffmpeg_download,
    video_engine_status, video_engine_download, video_webrtc_start, video_webrtc_offer,
    video_webrtc_stop,
    video_list_native_devices, video_probe_device,
    video_native_mjpeg_start, video_native_mjpeg_stop, video_rtsp_mjpeg_start,
    video_rtsp_native_start, video_rtsp_native_stop,
    video_rtsp_native_sink_rect, video_rtsp_native_sink_visible, video_rtsp_native_stats,
    video_rtsp_native_sink_buffer, video_rtsp_native_sink_orient,
};
use video::{MediaMtx, MjpegServer};
use commands::logging::{set_log_level, get_log_path, log_session_settings, log_frontend};
use commands::tiles::fetch_tile;
use commands::radar::{radar_configure, radar_set_center, radar_set_node_pos, radar_snapshot};
use commands::terrain::{
    terrain_cache_clear, terrain_cache_stats, terrain_elevation, terrain_elevations, terrain_fan,
    terrain_profile,
};
use terrain::TerrainProvider;
use commands::mission::{
    mission_get, mission_clear, mission_set, mission_add_wp, mission_insert_wp,
    mission_remove_wp, mission_update_wp, mission_reorder_wp,
    mission_download, mission_upload, mission_upload_multi, mission_get_active_index,
    mission_fc_info, mission_export_xml, mission_import_xml,
    mission_save_file, mission_save_file_from_json, mission_load_file,
    read_text_file, write_text_file,
    ardu_mission_download, ardu_mission_upload,
};
use commands::control::{
    mav_set_mode, mav_arm, mav_takeoff, mav_land, mav_rtl, mav_rc_release, mav_reposition,
    mav_change_speed, mav_mission_start, mav_mission_pause, mav_mission_set_current,
    mav_set_home_here, mav_abort_landing, mav_set_param, mav_read_param,
    mav_guided_change_heading, mav_guided_clear_heading, mav_condition_yaw,
    mav_vtol_transition,
};
use commands::update_check::check_for_update;
use hid::HidManager;
use mission::store::MissionStore;
use state::AppState;
use telemetry_forward::{relay_configure, relay_clear, RelayHub};

/// True when a `.portable` marker file sits next to the executable. Used both to
/// redirect data (`setup_portable_mode`) and to gate plugins whose storage path we
/// cannot redirect in portable mode (e.g. window-state on Windows).
pub fn is_portable() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.join(".portable").exists()))
        .unwrap_or(false)
}

/// Log which of the GStreamer elements WebKitGTK depends on are actually installed.
///
/// WebKitGTK implements neither WebRTC nor video decoding itself — it delegates both to GStreamer.
/// When `webrtcbin` is missing, `RTCPeerConnection` simply never appears: no error, no warning, and
/// RTSP silently degrades to the far more expensive MJPEG transcode. Two different WebKit builds on the
/// same Raspberry Pi produced the identical failure, which is the signature of a host-side plugin gap
/// rather than a WebKit one — but confirming that meant asking the tester to run `gst-inspect-1.0` by
/// hand and copy the output back, which on a remote-desktop session is genuinely awkward. The app can
/// just say it.
///
/// These plugins are **necessary, not sufficient**, and the message says so: WebRTC is a compile-time
/// option of WebKitGTK (`-DENABLE_WEB_RTC=ON`) and plenty of distributions build without it, where no
/// package on earth will bring `RTCPeerConnection` back. Verified on Debian 13's 2.52.5 — every plugin
/// below present, the library linking no `gstwebrtc` and carrying none of the implementation, and the
/// API absent in a WebView whose setting was applied before its first load. Reading the old wording as
/// a shopping list cost exactly that measurement to disprove, so it no longer reads like one.
///
/// Runs on a background thread (each `gst-inspect` call costs ~100 ms) and reports at warn level, since
/// a missing element is a real, actionable degradation.
#[cfg(target_os = "linux")]
fn probe_gstreamer_support() {
    std::thread::spawn(|| {
        let run = |args: &[&str]| -> bool {
            std::process::Command::new("gst-inspect-1.0")
                .args(args)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .stdin(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        };
        // `gst-inspect-1.0` ships separately (gstreamer1.0-tools) and is exactly the package a minimal
        // install lacks — i.e. missing on the very systems where this diagnostic matters most. Fall back
        // to looking for the plugin libraries themselves, which is what GStreamer would load anyway.
        if !run(&["--version"]) {
            let (webrtc, transport, decoders) = probe_gstreamer_plugin_files();
            log::warn!(
                "[gstreamer] gst-inspect-1.0 not found (gstreamer1.0-tools); by plugin file: \
                 webrtc={webrtc} · transport plugins=[{}] · h264 plugins=[{}] — RTCPeerConnection needs \
                 webrtc+dtls+srtp (gstreamer1.0-plugins-bad), nice (gstreamer1.0-nice), rtpmanager \
                 (gstreamer1.0-plugins-good) AND a WebKitGTK built with -DENABLE_WEB_RTC=ON, which \
                 several distributions leave off and no package can add. Video decode always needs an \
                 H.264 decoder (gstreamer1.0-libav)",
                transport.join(", "),
                decoders.join(", ")
            );
            return;
        }
        let webrtc = run(&["webrtcbin"]);
        // WebKit exposes RTCPeerConnection only when the WHOLE transport stack is loadable, not just
        // webrtcbin: ICE comes from libnice's plugin (gstreamer1.0-nice — a separate package and the
        // classic gap), DTLS/SRTP from plugins-bad, RTP session management from plugins-good.
        let ice = run(&["nicesrc"]);
        let dtls = run(&["dtlssrtpenc"]);
        let srtp = run(&["srtpenc"]);
        let rtp = run(&["rtpbin"]);
        // Software (libav/openh264) plus the hardware decoders that matter in practice: V4L2 (Raspberry
        // Pi 4, Rockchip), and Intel VA under both its plugin generations — `vah264dec` from the current
        // `va` plugin and `vaapih264dec` from the older gstreamer-vaapi. Which of the two a distribution
        // ships decides whether an Intel laptop decodes in hardware or silently on the CPU.
        let decoders: Vec<&str> = [
            "avdec_h264",
            "openh264dec",
            "v4l2h264dec",
            "vah264dec",
            "vaapih264dec",
        ]
        .into_iter()
        .filter(|e| run(&[e]))
        .collect();
        log::warn!(
            "[gstreamer] webrtcbin={webrtc} · ice(nice)={ice} · dtls={dtls} · srtp={srtp} · rtpbin={rtp} \
             · h264 decoders=[{}] — RTCPeerConnection needs all five of those (gstreamer1.0-plugins-bad, \
             gstreamer1.0-nice, gstreamer1.0-plugins-good) AND a WebKitGTK built with -DENABLE_WEB_RTC=ON. \
             Several distributions build without it (measured: Debian 13 / WebKitGTK 2.52.5), and there no \
             package can add it — the MJPEG path is the normal one. Video decode always needs an H.264 \
             decoder (gstreamer1.0-libav)",
            decoders.join(", ")
        );
    });
}

/// Raspberry Pi workaround: force the WebKit framebuffer to be reallocated once the UI is up.
///
/// With GPU acceleration enabled, the Pi's **first** framebuffer allocation is reliably broken — the
/// window shows garbage until something makes WebKit allocate a new one. Any change of the drawing
/// surface does that, so we perform the smallest one available for the window's current state and
/// immediately undo it. The same workaround is in use on the maintainer's Pi dashboard project.
///
/// One nudge per run, triggered by `PageLoadEvent::Finished` (the DOM being ready is the earliest
/// point at which there is a real frame to fix), plus a short settle delay because "document loaded"
/// is not yet "first frame composited".
#[cfg(target_os = "linux")]
fn nudge_framebuffer_on_pi(window: tauri::Window) {
    use std::sync::atomic::{AtomicBool, Ordering};
    /// `on_page_load` also fires for reloads and in-app navigation; the surface only needs fixing once.
    static DONE: AtomicBool = AtomicBool::new(false);

    // `/proc/device-tree/model` is the canonical Pi identifier ("Raspberry Pi 5 Model B Rev 1.0"). It's
    // absent on every non-device-tree machine, so this is a no-op on ordinary Linux desktops — the bug
    // is specific to this GPU.
    let model = std::fs::read_to_string("/proc/device-tree/model").unwrap_or_default();
    if !model.contains("Raspberry Pi") {
        return;
    }
    if DONE.swap(true, Ordering::SeqCst) {
        return;
    }

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        // Long enough for the compositor to actually act on the change before we undo it.
        let settle = std::time::Duration::from_millis(120);

        // Each window state needs its own smallest possible disturbance: resizing a fullscreen or
        // maximized window would fight the window manager (and was simply skipped before, which left
        // exactly the full-screen case — the normal one on a Pi — unfixed).
        if window.is_fullscreen().unwrap_or(false) {
            let _ = window.set_fullscreen(false);
            tokio::time::sleep(settle).await;
            let _ = window.set_fullscreen(true);
            log::info!("[gpu] Raspberry Pi framebuffer nudge: fullscreen off/on");
        } else if window.is_maximized().unwrap_or(false) {
            let _ = window.unmaximize();
            tokio::time::sleep(settle).await;
            let _ = window.maximize();
            log::info!("[gpu] Raspberry Pi framebuffer nudge: unmaximize/maximize");
        } else if let Ok(size) = window.inner_size() {
            let _ = window.set_size(tauri::PhysicalSize::new(size.width, size.height + 1));
            tokio::time::sleep(settle).await;
            let _ = window.set_size(size);
            log::info!("[gpu] Raspberry Pi framebuffer nudge: {}x{} +1px", size.width, size.height);
        }
    });
}

/// Detect portable mode: if a `.portable` marker file exists next to the
/// executable, redirect all application data into a `data/` folder beside
/// the exe.  Must be called **before** `run()` so the WebView picks up the
/// environment variables.
/// Look for GStreamer plugin **files** when `gst-inspect-1.0` isn't installed. Returns
/// `(webrtc present, names of the WebRTC transport plugins found, names of the H.264-capable
/// plugins found)`.
///
/// Plugins live in `<libdir>/gstreamer-1.0/libgst<name>.so`; the multiarch libdir differs per
/// architecture, and `GST_PLUGIN_PATH` can add more. This can't tell whether a plugin actually
/// *registers* its elements (a broken driver may still fail), so it is reported as "by plugin file" —
/// weaker evidence than `gst-inspect`, but the difference between an answer and none at all.
#[cfg(target_os = "linux")]
fn probe_gstreamer_plugin_files() -> (bool, Vec<&'static str>, Vec<&'static str>) {
    // The _1_0/SYSTEM variants matter inside the AppImage: linuxdeploy's gstreamer hook points
    // GST_PLUGIN_SYSTEM_PATH_1_0 at the bundled plugin set, which is exactly what WebKit sees there.
    let mut dirs: Vec<std::path::PathBuf> = [
        "GST_PLUGIN_PATH",
        "GST_PLUGIN_PATH_1_0",
        "GST_PLUGIN_SYSTEM_PATH",
        "GST_PLUGIN_SYSTEM_PATH_1_0",
    ]
    .iter()
    .filter_map(std::env::var_os)
    .flat_map(|v| std::env::split_paths(&v).collect::<Vec<_>>())
    .collect();
    for base in [
        "/usr/lib/aarch64-linux-gnu",
        "/usr/lib/x86_64-linux-gnu",
        "/usr/lib/arm-linux-gnueabihf",
        "/usr/lib64",
        "/usr/lib",
        "/usr/local/lib",
    ] {
        dirs.push(std::path::Path::new(base).join("gstreamer-1.0"));
    }
    let has = |file: &str| dirs.iter().any(|d| d.join(file).is_file());

    // libav covers avdec_h264, the `va`/`vaapi` plugins the Intel/AMD hardware paths, and
    // video4linux2 the Raspberry Pi 4 / Rockchip stateful decoders.
    let decoders = [
        ("libav", "libgstlibav.so"),
        ("openh264", "libgstopenh264.so"),
        ("va", "libgstva.so"),
        ("vaapi", "libgstvaapi.so"),
        ("v4l2", "libgstvideo4linux2.so"),
    ]
    .into_iter()
    .filter(|(_, file)| has(file))
    .map(|(name, _)| name)
    .collect();

    // The transport stack RTCPeerConnection depends on beyond the webrtc plugin itself: ICE from
    // libnice's plugin (gstreamer1.0-nice — a separate package and the classic gap), DTLS/SRTP from
    // plugins-bad, RTP session management from plugins-good.
    let transport = [
        ("nice", "libgstnice.so"),
        ("dtls", "libgstdtls.so"),
        ("srtp", "libgstsrtp.so"),
        ("rtpmanager", "libgstrtpmanager.so"),
    ]
    .into_iter()
    .filter(|(_, file)| has(file))
    .map(|(name, _)| name)
    .collect();

    (has("libgstwebrtc.so"), transport, decoders)
}

pub fn setup_portable_mode() {
    let exe_dir = match std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
    {
        Some(d) => d,
        None => return,
    };

    if !exe_dir.join(".portable").exists() {
        return;
    }

    let data_dir = exe_dir.join("data");
    std::fs::create_dir_all(&data_dir).ok();

    // Underscore-prefixed: consumed only by the Windows/Linux branches below. macOS has no env-based
    // WebView redirect (WKWebView uses WKWebsiteDataStore, set programmatically), so portable-mode
    // WebView state is not redirected there — the DB/logs/terrain still go to `<exe>/data` via the
    // path resolvers.
    let _data_str = data_dir.to_string_lossy().to_string();

    // Windows: redirect WebView2 user-data folder
    #[cfg(target_os = "windows")]
    {
        std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", &_data_str);
    }

    // Linux: redirect XDG directories so WebKitGTK stores data next to the binary
    #[cfg(target_os = "linux")]
    {
        std::env::set_var("XDG_DATA_HOME", &_data_str);
        std::env::set_var("XDG_CONFIG_HOME", &_data_str);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // `--debug` (release builds) turns on the in-app Debug Monitor + verbose logging at runtime. Debug
    // builds default to on (see debug_mode). Parse it before anything else.
    let debug_flag = debug_mode::debug_flag_present();
    if debug_flag {
        debug_mode::set(true);
    }

    // Install the file logger before anything else so startup + connection diagnostics are captured.
    // Default Warning so early failures are recorded without flooding the file in normal operation; a
    // `--debug` start raises it to Debug. The frontend re-applies the persisted level on startup (and
    // keeps Debug when in debug mode).
    let log_level = if debug_flag { log::LevelFilter::Debug } else { log::LevelFilter::Warn };
    logging::init(log_level, is_portable());

    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init());

    // Mobile only: use native CoreLocation/Android location for the GCS position so the OS permission
    // prompt is labelled with the app name instead of the WebView origin ("localhost"). Desktop keeps
    // the browser `navigator.geolocation` path (WebKitGTK grants are handled in setup() below).
    #[cfg(mobile)]
    {
        builder = builder.plugin(tauri_plugin_geolocation::init());
    }

    // Persist + restore the main window's size/position/maximized state across launches.
    // The plugin saves to the OS app-config dir, which portable mode cannot redirect on
    // Windows (Known-Folder API, not env-driven) — so only enable it in installed mode.
    // Portable builds trade window-geometry persistence for a clean, system-path-free runtime.
    // Desktop only: there is no free-floating window to persist on mobile.
    #[cfg(desktop)]
    if !is_portable() {
        use tauri_plugin_window_state::StateFlags;
        // Persist everything EXCEPT the decorations flag: we run with a custom titlebar
        // (`decorations: false` in tauri.conf.json), and the state plugin would otherwise
        // restore a previously-saved `decorations: true` and re-add the native title bar.
        builder = builder.plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(StateFlags::all() & !StateFlags::DECORATIONS)
                .build(),
        );
    }

    builder
        .setup(|_app| {
            // Android: record where app data actually landed, and flag any disagreement with Tauri's
            // own idea of it (see `android::log_resolved_dirs`).
            #[cfg(target_os = "android")]
            android::log_resolved_dirs(_app.handle());

            // Linux/WebKitGTK: stop trackpad/keyboard gestures from zooming the whole WebView frame.
            // WebKitGTK handles these natively in GTK and ignores any JS `preventDefault`, so they can
            // only be suppressed here (Windows/WebView2 + macOS use the JS guard in `+layout.svelte`).
            // There are TWO distinct zoom paths and each needs a different fix:
            #[cfg(target_os = "linux")]
            {
                use tauri::Manager;
                if let Some(window) = _app.get_webview_window("main") {
                    let _ = window.with_webview(|webview| {
                        use webkit2gtk::WebViewExt;
                        use webkit2gtk::{SettingsExt, PermissionRequestExt};
                        use webkit2gtk::glib::gobject_ffi;
                        use webkit2gtk::glib::prelude::{ObjectExt, Cast};

                        let wv = webview.inner();

                        // (1) Page zoom — Ctrl+wheel / Ctrl+(+/-) — goes through the `zoom-level`
                        // property. Pin it at 1.0 (reset on every change keeps it pinned). The map
                        // keeps its own Leaflet/Cesium zoom; only the chrome zoom is suppressed.
                        wv.set_zoom_level(1.0);
                        wv.connect_zoom_level_notify(|wv| {
                            if (wv.zoom_level() - 1.0).abs() > f64::EPSILON {
                                wv.set_zoom_level(1.0);
                            }
                        });

                        // (2) The touchpad PINCH is a *separate* visual zoom driven by a private
                        // GtkGestureZoom that bypasses `zoom-level` (and JS) entirely — so (1) cannot
                        // catch it. The only known way to disable it is to destroy the signal handlers
                        // WebKit attached to that gesture. Private API (`wk-view-zoom-gesture` qdata),
                        // GTK3-only (tao 0.34 uses GTK3); a no-op if the key is absent. We only DESTROY
                        // handlers — we do NOT free the gesture data (that path is known to segfault).
                        // Ref: tauri-apps/wry#544 (upstream has no setting; confirmed by WebKit devs).
                        unsafe {
                            if let Some(gesture) =
                                wv.data::<gobject_ffi::GObject>("wk-view-zoom-gesture")
                            {
                                gobject_ffi::g_signal_handlers_destroy(gesture.as_ptr());
                            }
                        }

                        // (3) Permissions + media. WebKitGTK ships with getUserMedia
                        // (`enable-media-stream`) OFF by default, and leaves permission requests to a
                        // default that varies by distro/WebKit version — so the integrated camera and the
                        // GCS geolocation both silently fail on some builds (e.g. Zorin OS) while working
                        // on others (Debian). Enable the media engine and grant geolocation + camera/mic
                        // requests ourselves; the real gate stays the OS-level Location/Camera toggle the
                        // user controls. (`settings()` returns `Option<WebKitSettings>` in this binding.)
                        if let Some(settings) = WebViewExt::settings(&wv) {
                            settings.set_enable_media_stream(true);
                            // WebRTC is a SEPARATE switch and it is **off by default** in WebKitGTK
                            // ≥ 2.38 — without it `RTCPeerConnection` is undefined, the RTSP source
                            // silently degrades to the MJPEG fallback, and that fallback then depends
                            // on an ffmpeg transcode (and on the WebView rendering multipart images at
                            // all). Set by NAME rather than through `set_enable_webrtc()`: that setter
                            // sits behind the crate's `v2_38` feature, which would raise our build-time
                            // WebKitGTK requirement (CI deliberately builds against ubuntu-22.04). By
                            // name it is simply a no-op on older runtimes that lack the property.
                            //
                            // Logged in full: a Pi field test showed `webrtc=false` in the frontend
                            // even with this in place, and "property missing", "set but ignored" and
                            // "set but GStreamer can't back it" are three different problems that look
                            // identical from the outside. The runtime WebKitGTK version comes along
                            // because it decides which of them is even possible — and because a Linux
                            // bug report is worth little without it.
                            //
                            // Note what the read-back does NOT prove: the setter only writes a
                            // WebPreferences bool, so it reads back `true` even where WebKitGTK was
                            // built without `ENABLE_WEB_RTC` and no implementation exists behind it.
                            // Measured on Debian 13's 2.52.5: `RTCPeerConnection` stays undefined in a
                            // freshly created WebView with the setting applied *before* the first load,
                            // and the library links no gstwebrtc and carries none of the implementation
                            // (`createOffer`, `addIceCandidate`, `setLocalDescription`). Only the
                            // frontend's own `typeof RTCPeerConnection` is the verdict — which is why
                            // there is no "reload once to expose it" repair here any more.
                            let (major, minor, micro) = unsafe {
                                (
                                    webkit2gtk::ffi::webkit_get_major_version(),
                                    webkit2gtk::ffi::webkit_get_minor_version(),
                                    webkit2gtk::ffi::webkit_get_micro_version(),
                                )
                            };
                            if settings.find_property("enable-webrtc").is_some() {
                                settings.set_property("enable-webrtc", true);
                                let now: bool = settings.property("enable-webrtc");
                                log::warn!(
                                    "[webkit] WebKitGTK {major}.{minor}.{micro} — enable-webrtc set, reads back as {now}"
                                );
                            } else {
                                log::warn!(
                                    "[webkit] WebKitGTK {major}.{minor}.{micro} — no 'enable-webrtc' property (needs ≥ 2.38); RTSP will fall back to MJPEG"
                                );
                            }
                        }
                        wv.connect_permission_request(|_wv, req| {
                            if req.downcast_ref::<webkit2gtk::GeolocationPermissionRequest>().is_some()
                                || req.downcast_ref::<webkit2gtk::UserMediaPermissionRequest>().is_some()
                            {
                                req.allow();
                                true // handled
                            } else {
                                false // leave anything else to the default
                            }
                        });
                    });
                }
                probe_gstreamer_support();
            }
            Ok(())
        })
        .on_page_load(|_webview, _payload| {
            // The Pi's first framebuffer is garbage until the surface changes — do it once the page is
            // actually up (see `nudge_framebuffer_on_pi`).
            #[cfg(target_os = "linux")]
            if _payload.event() == tauri::webview::PageLoadEvent::Finished {
                nudge_framebuffer_on_pi(_webview.window());
            }
        })
        .manage(AppState::new())
        .manage(MissionStore::new())
        .manage(TerrainProvider::new())
        .manage(RelayHub::new())
        .manage(HidManager::new())
        // As an `Arc` so the start command can hand it to a blocking task (process spawn +
        // readiness poll) without pinning an async runtime thread.
        .manage(std::sync::Arc::new(MediaMtx::new()))
        .manage(MjpegServer::new())
        .manage(video::rtsp_native::NativeRtsp::new())
        .invoke_handler(tauri::generate_handler![
            list_serial_ports,
            scan_ble_devices,
            ble_scan_start,
            ble_scan_stop,
            inav_set_craft_name,
            inav_read_stats,
            connect,
            disconnect,
            get_app_version,
            is_debug_mode,
            set_log_level,
            get_log_path,
            log_session_settings,
            log_frontend,
            fetch_tile,
            mission_get,
            mission_clear,
            mission_set,
            mission_add_wp,
            mission_insert_wp,
            mission_remove_wp,
            mission_update_wp,
            mission_reorder_wp,
            mission_download,
            mission_upload,
            mission_upload_multi,
            mission_get_active_index,
            mission_fc_info,
            mission_export_xml,
            mission_import_xml,
            mission_save_file,
            mission_save_file_from_json,
            mission_load_file,
            read_text_file,
            write_text_file,
            ardu_mission_download,
            ardu_mission_upload,
            mav_set_mode,
            mav_arm,
            mav_takeoff,
            mav_land,
            mav_rtl,
            mav_rc_release,
            check_for_update,
            mav_reposition,
            mav_change_speed,
            mav_mission_start,
            mav_mission_pause,
            mav_mission_set_current,
            mav_set_home_here,
            mav_abort_landing,
            mav_set_param,
            mav_read_param,
            mav_guided_change_heading,
            mav_guided_clear_heading,
            mav_condition_yaw,
            mav_vtol_transition,
            flightlog_list,
            flightlog_get,
            flightlog_get_track,
            flightlog_get_battery_records,
            flightlog_delete,
            mission_db_save,
            mission_db_get,
            mission_db_for_flight,
            flight_link_mission,
            flight_logged_wp_count,
            mission_db_geocode,
            mission_db_find_by_hash,
            mission_db_update,
            flight_unlink_mission,
            mission_db_delete,
            mission_db_flights,
            mission_db_list,
            mission_db_set_meta,
            battery_db_create,
            battery_db_update,
            battery_db_list,
            battery_db_get,
            battery_db_find_by_serial,
            battery_db_delete,
            battery_db_add_usage,
            battery_db_aggregate,
            battery_db_flights,
            flight_set_battery_serial,
            battery_db_set_baseline,
            battery_file_write,
            battery_file_read,
            vehicle_db_create,
            vehicle_db_update,
            vehicle_db_list,
            vehicle_db_get,
            vehicle_db_find_by_craft_name,
            vehicle_db_delete,
            vehicle_db_aggregate,
            vehicle_db_flights,
            vehicle_db_set_baseline,
            vehicle_file_write,
            vehicle_file_read,
            flightlog_update_notes,
            flightlog_update_craft_name,
            flightlog_update_platform_type,
            flightlog_update_pilot,
            flightlog_update_weather,
            flightlog_geocode,
            flightlog_fetch_weather,
            flightlog_default_db_path,
            flightlog_default_raw_log_path,
            flightlog_import_blackbox,
            flightlog_blackbox_log_count,
            flightlog_export,
            flightlog_export_blackbox,
            flightlog_blackbox_file_info,
            flightlog_delete_blackbox_file,
            flightlog_compact_db,
            flightlog_export_track,
            flightlog_import_kflight,
            flightlog_kflight_list,
            flightlog_kflight_get,
            flightlog_kflight_track,
            flightlog_probe_ardupilot,
            flightlog_decode_ardupilot_csv,
            flightlog_import_ardupilot,
            flightlog_import_raw,
            blackbox_decoder_available,
            blackbox_decoder_version,
            download_blackbox_decode,
            flightlog_link_flights,
            flightlog_unlink_flight,
            flightlog_find_linkable,
            flightlog_commit_pending_session,
            flightlog_discard_pending_session,
            flightlog_continue_pending_session,
            flightlog_scan_orphan_sessions,
            flightlog_recover_discard,
            flightlog_recover_save_incomplete,
            flightlog_recover_continue,
            terrain_elevation,
            terrain_elevations,
            terrain_profile,
            terrain_fan,
            terrain_cache_stats,
            terrain_cache_clear,
            system_on_battery,
            system_active_net_is_wifi,
            storage_pick_folder,
            share_file,
            video_ffmpeg_status,
            video_ffmpeg_download,
            video_engine_status,
            video_engine_download,
            video_webrtc_start,
            video_webrtc_offer,
            video_webrtc_stop,
            video_list_native_devices,
            video_probe_device,
            video_native_mjpeg_start,
            video_rtsp_mjpeg_start,
            video_rtsp_native_start,
            video_rtsp_native_stop,
            video_rtsp_native_sink_rect,
            video_rtsp_native_sink_visible,
            video_rtsp_native_stats,
            video_rtsp_native_sink_buffer,
            video_rtsp_native_sink_orient,
            video_native_mjpeg_stop,
            radar_configure,
            radar_set_center,
            radar_set_node_pos,
            radar_snapshot,
            aero_fetch,
            aero_cache_stats,
            aero_cache_clear,
            relay_configure,
            relay_clear,
            hid_start,
            hid_stop,
            hid_select_device,
            hid_profiles_dir,
            hid_profile_list,
            hid_profile_save,
            hid_profile_delete,
            rc_read_fc_config,
            rc_set_override_bitmask,
            rc_read_channels,
            rc_stream_update,
            rc_stream_set_aux,
            rc_stream_enable,
            rc_stream_set_rate,
            rc_stream_set_override,
            rc_stream_set_manual,
            safehome_read_all,
            safehome_write_all,
            geozone_read_all,
            geozone_write_all,
            fence_read_all,
            fence_write_all,
            rally_read_all,
            rally_write_all,
        ])
        .build(tauri::generate_context!())
        .expect("error while running Kite Ground Control")
        .run(|app, event| {
            // Tauri tears the process down without dropping managed state, so the video helpers must
            // be stopped here. A surviving engine (or its ffmpeg publisher) keeps the RTSP session on
            // the remote server alive indefinitely (this is what wedged the UAV-Link Pi's shared
            // media), and a surviving capture ffmpeg keeps holding the camera.
            if matches!(event, tauri::RunEvent::Exit) {
                use tauri::Manager;
                app.state::<std::sync::Arc<MediaMtx>>().stop();
                app.state::<MjpegServer>().stop();
                app.state::<video::rtsp_native::NativeRtsp>().stop();
            }
        });
}
