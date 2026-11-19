// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

mod aero;
#[cfg(target_os = "android")]
mod android;
mod child_env;
mod commands;
mod custom_maps;
mod debug_mode;
mod fleet_mode;
mod flightlog;
mod flightmode;
mod github_release;
mod hid;
#[cfg(target_os = "ios")]
mod ios_device;
mod link_presence;
mod link_stats;
mod link_status;
/// macOS operator location via CoreLocation. macOS is the one desktop target with no Web Geolocation
/// API, so `navigator.geolocation` cannot resolve there and the GCS marker needs a native source.
/// Every other target keeps the WebView path and compiles the stub instead.
#[cfg(target_os = "macos")]
#[path = "location_macos.rs"]
mod location;
#[cfg(not(target_os = "macos"))]
#[path = "location_stub.rs"]
mod location;
mod logging;
mod mavlink_proto;
mod mission;
mod msp;
mod passive_telemetry;
mod radar;
mod scheduler;
mod state;
mod telemetry_api;
mod telemetry_forward;
mod terrain;
mod transport;
mod user_agent;
mod user_file;
mod vehicle_registry;
mod video;
mod weather;

use commands::connection::{connect, disconnect, set_platform_type, inav_set_craft_name, inav_read_stats, scan_ble_devices, ble_scan_start, ble_scan_stop, debug_tunnel_msp_request, debug_tunnel_stats_snapshot};
use commands::vehicles::{list_links, set_active_vehicle, get_active_vehicle, announce_vehicles, set_fleet_enabled, debug_group_state};
use commands::connection::list_serial_ports;
use commands::flightlog::{
    flightlog_list, flightlog_get, flightlog_get_track, flightlog_get_battery_records, flightlog_delete,
    flightlog_update_notes, flightlog_update_craft_name, flightlog_update_platform_type, flightlog_update_pilot, flightlog_update_weather, flightlog_geocode, flightlog_fetch_weather,
    flightlog_default_db_path, flightlog_default_raw_log_path, flightlog_import_blackbox,
    flightlog_blackbox_log_count,
    flightlog_export, flightlog_export_blackbox, flightlog_blackbox_file_info, flightlog_delete_blackbox_file, flightlog_compact_db, flightlog_export_track, flightlog_import_kflight,
    flightlog_backup_info, flightlog_backup_delete, flightlog_schema_state, flightlog_upgrade_run,
    flightlog_folder_state, flightlog_db_move, flightlog_db_retire,
    flightlog_export_database, flightlog_import_database_stage, flightlog_import_database_finish, flightlog_export_all,
    flightlog_original_logs_info, flightlog_original_logs_remove,
    flightlog_kflight_list, flightlog_kflight_get, flightlog_kflight_track,
    flightlog_probe_ardupilot, flightlog_decode_ardupilot_csv,
    flightlog_import_ardupilot, flightlog_import_raw, flightlog_import_ulog,
    blackbox_legacy_mode, blackbox_decoder_available, blackbox_decoder_version, download_blackbox_decode,
    flightlog_link_flights, flightlog_unlink_flight, flightlog_find_linkable,
    flightlog_commit_pending_session, flightlog_discard_pending_session,
    flightlog_continue_pending_session,
    flightlog_scan_orphan_sessions, flightlog_recover_discard, flightlog_recover_save_incomplete,
    flightlog_recover_continue,
    flightlog_hires_info, flightlog_hires_parse, flightlog_hires_sample, flightlog_hires_drop,
    flightlog_hires_cleanup, flightlog_scratch_dir, flightlog_scratch_clear,
    mission_db_save, mission_db_get, mission_db_for_flight, flight_link_mission,
    flight_logged_wp_count, flight_alt_relative, mission_db_geocode, mission_db_find_by_hash, mission_db_update,
    flight_unlink_mission, mission_db_delete, mission_db_flights, mission_db_list,
    mission_db_set_meta, mission_db_set_hash,
    battery_db_create, battery_db_update, battery_db_list, battery_db_get,
    battery_db_find_by_serial, battery_db_delete, battery_db_add_usage, battery_db_aggregate,
    battery_db_flights, flight_set_battery_serial, battery_db_set_baseline,
    battery_file_write, battery_file_read,
    vehicle_db_create, vehicle_db_update, vehicle_db_list, vehicle_db_get,
    vehicle_db_find_by_craft_name, vehicle_db_find_by_fc_uid, vehicle_db_delete, vehicle_db_aggregate, vehicle_db_flights,
    vehicle_db_set_baseline, vehicle_file_write, vehicle_file_read,
    mission_file_write, mission_file_read, mission_db_set_extra,
    vehicle_db_resolve, vehicle_db_link_flights, vehicle_db_link_flights_count, vehicle_db_transfer_flights,
    flight_set_vehicle, vehicle_db_set_fc_uid,
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
use commands::info::{get_app_version, is_debug_mode, is_fleet_mode, is_fleet_group_recording};
use commands::system::{system_active_net_is_wifi, system_on_battery};
use commands::storage::{
    export_copy, export_scratch_clear, export_scratch_path, export_take_save_request, file_display_name,
    share_file, storage_pick_folder, storage_flush_raw_logs,
};
use commands::video::{
    video_ffmpeg_status, video_hw_transcode_available, video_ffmpeg_download,
    video_engine_status, video_engine_download, video_webrtc_start, video_webrtc_offer,
    video_webrtc_stop,
    video_list_native_devices, video_probe_device,
    video_native_mjpeg_start, video_native_mjpeg_stop, video_rtsp_mjpeg_start,
    video_rtsp_native_start, video_rtsp_native_stop,
    video_rtsp_native_sink_surfaces, video_rtsp_native_stats,
    video_linux_hole_spike,
    video_rtsp_native_sink_buffer, video_rtsp_native_sink_orient,
    video_detached_open, video_detached_close, video_detached_pin_top, video_detached_chrome, video_detached_aspect, video_detached_nudge,
};
use video::{MediaMtx, MjpegServer};
use commands::logging::{set_log_level, get_log_path, list_log_files, log_session_settings, log_frontend};
use commands::tiles::fetch_tile;
use commands::custom_maps::{custom_maps_list, custom_map_tile, custom_map_resolve, custom_map_coverage};
use commands::map_search::map_search_places;
use commands::radar::{radar_configure, radar_set_center, radar_set_node_pos, radar_snapshot};
use commands::terrain::{
    terrain_cache_clear, terrain_cache_stats, terrain_elevation, terrain_elevations, terrain_fan,
    terrain_profile,
};
use terrain::TerrainProvider;
use commands::weather::{flightlog_set_lookups, weather_set_reference, weather_watch};
use commands::mission::{
    mission_get, mission_clear, mission_set, mission_add_wp, mission_insert_wp,
    mission_remove_wp, mission_update_wp, mission_reorder_wp,
    mission_download, mission_upload, mission_upload_multi, mission_get_active_index,
    mission_fc_info, mission_export_xml, mission_import_xml,
    mission_save_file, mission_save_file_from_json, mission_load_file, mission_resolve_agl,
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
use commands::updater::{install_update, update_kind};
use hid::HidManager;
use mission::store::MissionStore;
use state::AppState;
use telemetry_api::{telemetry_api_configure, telemetry_api_set_gcs, telemetry_api_status, TelemetryApi};
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

/// Log which GStreamer elements WebKitGTK needs are installed: without `webrtcbin`, `RTCPeerConnection`
/// silently never appears. Necessary, not sufficient — WebRTC is also a WebKitGTK build option many
/// distributions leave off. Background thread (~100 ms per gst-inspect), warn level.
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
        // Software decoders plus the hardware ones that matter: V4L2 (Pi 4, Rockchip) and Intel VA in both plugin
        // generations (`vah264dec`, older `vaapih264dec`) — which one a distro ships decides HW vs CPU decode.
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

/// Raspberry Pi: force one WebKit framebuffer reallocation once the UI is up — with GPU acceleration the
/// Pi's FIRST framebuffer shows garbage until a surface change rebuilds it. One nudge per run, on
/// `PageLoadEvent::Finished` plus a settle delay (loaded is not yet composited).
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

/// Look for GStreamer plugin FILES when `gst-inspect-1.0` is missing: `(webrtc present, WebRTC transport
/// plugins, H.264 plugins)`. Weaker evidence than gst-inspect (a file may fail to register), reported so.
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

/// Portable mode (a `.portable` marker beside the executable): all app data goes to `data/` next to the
/// exe. Must be called BEFORE `run()`, so the WebView picks up the environment variables.
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

    // Consumed only by the Windows/Linux branches: macOS has no env-based WebView redirect, so portable-mode
    // WebView state stays unredirected there (DB/logs/terrain still go to `<exe>/data`).
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

    // The file logger first, so startup diagnostics are captured: Warning by default, Debug with `--debug`;
    // the frontend re-applies the persisted level on startup.
    let log_level = if debug_flag { log::LevelFilter::Debug } else { log::LevelFilter::Warn };
    logging::init(log_level, is_portable());

    // Multi-vehicle features and group recording (see fleet_mode), logged at the default level so
    // every log shows which mode ran.
    let fleet_flag = fleet_mode::flag_present();
    if fleet_flag {
        fleet_mode::set(true);
    }
    fleet_mode::apply_group_recording();
    if fleet_mode::enabled() {
        log::warn!(
            "Fleet (multi-vehicle) features enabled ({}) — group recording {}",
            if fleet_flag { "--fleet / KITE_FLEET" } else if cfg!(debug_assertions) { "debug build default" } else { "built with KITE_FLEET" },
            if fleet_mode::group_recording() { "ON (KITE_FLEET_RECORDING)" } else { "off: only the first vehicle's flights are recorded" },
        );
    }

    // Linux: ask GTK for a GLES context (a user-set value wins). GTK3's default desktop GL context has no
    // direct DMABuf import, so the Pi 5 HEVC decoder's tiled `NV12_128C8` output ABORTS the process in gst-gl;
    // GLES imports it zero-copy. Nothing else uses GDK's GL (WebKitGTK renders through its own EGL).
    #[cfg(target_os = "linux")]
    if std::env::var_os("GDK_GL").is_none() {
        std::env::set_var("GDK_GL", "gles");
    }

    // A portable update parks the previous executable as `<exe>.old` (Windows cannot overwrite a running
    // binary) — clear it now that we are the new one.
    #[cfg(desktop)]
    commands::updater::remove_stale_executable();

    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init());

    // "Update and Restart" for installed copies (commands/updater.rs drives it; the frontend never calls
    // the plugin's JS API, so no capability entry is needed). Desktop only — the plugin has no mobile
    // backend, and the stores handle updates there.
    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    }

    // Mobile only: use native CoreLocation/Android location for the GCS position so the OS permission
    // prompt is labelled with the app name instead of the WebView origin ("localhost"). Desktop keeps
    // the browser `navigator.geolocation` path (WebKitGTK grants are handled in setup() below).
    #[cfg(mobile)]
    {
        builder = builder.plugin(tauri_plugin_geolocation::init());
    }

    // Window-state persistence, installed mode only: the plugin saves to the OS app-config dir, which
    // portable mode cannot redirect on Windows. Desktop only.
    #[cfg(desktop)]
    if !is_portable() {
        use tauri_plugin_window_state::StateFlags;
        // Persist everything EXCEPT decorations (custom titlebar; a saved `decorations: true` would bring the
        // native one back). The detached video window is denylisted: the plugin restores sizes onto vanished
        // monitors (VIDEO_MULTISINK_WINDOW.md §5.5).
        builder = builder.plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(StateFlags::all() & !StateFlags::DECORATIONS)
                .with_denylist(&[crate::commands::video::DETACHED_LABEL])
                .build(),
        );
    }

    builder
        .setup(|_app| {
            // Android: record where app data actually landed, and flag any disagreement with Tauri's
            // own idea of it (see `android::log_resolved_dirs`).
            #[cfg(target_os = "android")]
            android::log_resolved_dirs(_app.handle());
            // At startup, so every mobile log carries the hardware identification.
            #[cfg(target_os = "android")]
            android::device::profile();
            #[cfg(target_os = "ios")]
            ios_device::log_model();

            {
                use tauri::Manager;
                _app.state::<weather::service::WeatherService>().start(_app.handle());
            }

            // Group-flight coordinator: its events go out app-wide, its worker drives the group-end
            // timers (GROUP_FLIGHTS.md §3.2).
            {
                use tauri::Manager;
                _app.state::<AppState>()
                    .groups
                    .start(Box::new(flightlog::group::AppSink(_app.handle().clone())));
            }

            // Linux: re-host the WebView over the native video layer (hole-punch decode sink,
            // MOBILE_RTSP.md P2.3). Needs the transparent window from tauri.linux.conf.json.
            #[cfg(target_os = "linux")]
            video::linux_host::install(_app.handle());
            // macOS / iPad: the AppKit or UIKit video layer below the WebView (hole-punch decode sink,
            // MOBILE_RTSP.md P3). Needs the transparent window from tauri.macos/ios.conf.json.
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            video::apple_host::install(_app.handle());

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

                        // (2) The touchpad PINCH is a separate visual zoom (private GtkGestureZoom) bypassing `zoom-level` and
                        // JS: destroy the signal handlers WebKit attached to it (private `wk-view-zoom-gesture` qdata, GTK3;
                        // no-op if absent). Only the handlers — freeing the gesture data segfaults. Ref: tauri-apps/wry#544.
                        unsafe {
                            if let Some(gesture) =
                                wv.data::<gobject_ffi::GObject>("wk-view-zoom-gesture")
                            {
                                gobject_ffi::g_signal_handlers_destroy(gesture.as_ptr());
                            }
                        }

                        // (3) WebKitGTK ships getUserMedia off and leaves permission requests to a distro-dependent default, so
                        // camera and GCS geolocation fail on some builds. Enable media and grant geolocation + camera/mic here;
                        // the real gate stays the OS-level toggle.
                        if let Some(settings) = WebViewExt::settings(&wv) {
                            settings.set_enable_media_stream(true);
                            // WebRTC is a SEPARATE switch, off by default in WebKitGTK ≥ 2.38. Set by NAME (the typed setter needs the
                            // crate's `v2_38` feature and a newer build-time WebKitGTK); a no-op where the property is missing. The
                            // read-back proves nothing (WebKit may be built without WebRTC): only the frontend's check decides.
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
            // macOS: start CoreLocation at launch, so the authorisation prompt comes once and never mid-flight; a
            // denial only logs (the GCS marker falls back to vehicle GPS).
            #[cfg(target_os = "macos")]
            location::start(_app.handle());
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
        .manage(TelemetryApi::new())
        .manage(weather::service::WeatherService::new())
        .manage(HidManager::new())
        // As an `Arc` so the start command can hand it to a blocking task (process spawn +
        // readiness poll) without pinning an async runtime thread.
        .manage(std::sync::Arc::new(MediaMtx::new()))
        .manage(MjpegServer::new())
        .manage(video::rtsp_native::NativeRtsp::new())
        .invoke_handler(tauri::generate_handler![
            location::location_os_last,
            location::location_os_start,
            list_serial_ports,
            scan_ble_devices,
            ble_scan_start,
            ble_scan_stop,
            inav_set_craft_name,
            inav_read_stats,
            connect,
            disconnect,
            set_platform_type,
            list_links,
            set_active_vehicle,
            get_active_vehicle,
            announce_vehicles,
            set_fleet_enabled,
            debug_group_state,
            debug_tunnel_msp_request,
            debug_tunnel_stats_snapshot,
            get_app_version,
            is_debug_mode,
            is_fleet_mode,
            is_fleet_group_recording,
            link_status::telemetry_track_since,
            link_presence::set_stay_awake,
            set_log_level,
            get_log_path,
            list_log_files,
            log_session_settings,
            log_frontend,
            fetch_tile,
            custom_maps_list,
            custom_map_tile,
            custom_map_resolve,
            custom_map_coverage,
            map_search_places,
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
            mission_resolve_agl,
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
            update_kind,
            install_update,
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
            flight_alt_relative,
            mission_db_geocode,
            mission_db_find_by_hash,
            mission_db_update,
            flight_unlink_mission,
            mission_db_delete,
            mission_db_flights,
            mission_db_list,
            mission_db_set_meta,
            mission_db_set_hash,
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
            vehicle_db_find_by_fc_uid,
            vehicle_db_delete,
            vehicle_db_aggregate,
            vehicle_db_flights,
            vehicle_db_set_baseline,
            vehicle_file_write,
            vehicle_file_read,
            mission_file_write,
            mission_file_read,
            mission_db_set_extra,
            vehicle_db_resolve,
            vehicle_db_link_flights,
            vehicle_db_link_flights_count,
            vehicle_db_transfer_flights,
            flight_set_vehicle,
            vehicle_db_set_fc_uid,
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
            flightlog_backup_info,
            flightlog_backup_delete,
            flightlog_schema_state,
            flightlog_upgrade_run,
            flightlog_folder_state,
            flightlog_db_move,
            flightlog_db_retire,
            flightlog_export_database,
            flightlog_import_database_stage,
            flightlog_import_database_finish,
            flightlog_export_all,
            flightlog_original_logs_info,
            flightlog_original_logs_remove,
            flightlog_export_track,
            flightlog_import_kflight,
            flightlog_kflight_list,
            flightlog_kflight_get,
            flightlog_kflight_track,
            flightlog_probe_ardupilot,
            flightlog_decode_ardupilot_csv,
            flightlog_import_ardupilot,
            flightlog_import_raw,
            flightlog_import_ulog,
            blackbox_legacy_mode,
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
            flightlog_hires_info,
            flightlog_hires_parse,
            flightlog_hires_sample,
            flightlog_hires_drop,
            flightlog_hires_cleanup,
            flightlog_scratch_dir,
            flightlog_scratch_clear,
            flightlog_recover_discard,
            flightlog_recover_save_incomplete,
            flightlog_recover_continue,
            terrain_elevation,
            terrain_elevations,
            terrain_profile,
            terrain_fan,
            weather_watch,
            weather_set_reference,
            flightlog_set_lookups,
            commands::flightlog::flightlog_get_weather_records,
            commands::flightlog::flightlog_get_gps_records,
            terrain_cache_stats,
            terrain_cache_clear,
            system_on_battery,
            system_active_net_is_wifi,
            storage_pick_folder,
            storage_flush_raw_logs,
            share_file,
            file_display_name,
            export_scratch_path,
            export_scratch_clear,
            export_take_save_request,
            export_copy,
            video_ffmpeg_status,
            video_hw_transcode_available,
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
            video_rtsp_native_sink_surfaces,
            video_linux_hole_spike,
            video_rtsp_native_stats,
            video_rtsp_native_sink_buffer,
            video_rtsp_native_sink_orient,
            video_native_mjpeg_stop,
            video_detached_open,
            video_detached_close,
            video_detached_pin_top,
            video_detached_chrome,
            video_detached_aspect,
            video_detached_nudge,
            radar_configure,
            radar_set_center,
            radar_set_node_pos,
            radar_snapshot,
            aero_fetch,
            aero_cache_stats,
            aero_cache_clear,
            relay_configure,
            relay_clear,
            telemetry_api_configure,
            telemetry_api_status,
            telemetry_api_set_gcs,
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
            // Tauri exits without dropping managed state: stop the video helpers here, or a surviving engine keeps
            // the remote RTSP session alive indefinitely and a capture ffmpeg keeps the camera.
            if matches!(event, tauri::RunEvent::Exit) {
                use tauri::Manager;
                app.state::<std::sync::Arc<MediaMtx>>().stop();
                app.state::<MjpegServer>().stop();
                app.state::<video::rtsp_native::NativeRtsp>().stop();
            }
            // The detached video window would keep the app alive alone (Tauri exits with the LAST window): close it
            // with — and after — the main window, so the main window's "docked back" handler cannot clear the
            // detached pref. Desktop only.
            #[cfg(desktop)]
            if let tauri::RunEvent::WindowEvent {
                label,
                event: tauri::WindowEvent::Destroyed,
                ..
            } = &event
            {
                use tauri::Manager;
                if label == "main" {
                    if let Some(win) = app.get_webview_window(commands::video::DETACHED_LABEL) {
                        let _ = win.destroy();
                    }
                }
            }
        });
}
