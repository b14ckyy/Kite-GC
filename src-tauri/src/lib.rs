// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)

mod aero;
mod commands;
mod debug_mode;
mod flightlog;
mod flightmode;
mod hid;
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
mod video;

use commands::connection::{connect, disconnect, list_serial_ports, scan_ble_devices, ble_scan_start, ble_scan_stop, inav_set_craft_name, inav_read_stats};
use commands::flightlog::{
    flightlog_list, flightlog_get, flightlog_get_track, flightlog_get_battery_records, flightlog_delete,
    flightlog_update_notes, flightlog_update_craft_name, flightlog_update_platform_type, flightlog_update_pilot, flightlog_update_weather, flightlog_geocode, flightlog_fetch_weather,
    flightlog_default_db_path, flightlog_default_raw_log_path, flightlog_import_blackbox,
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
use commands::system::system_on_battery;
use commands::video::{
    video_ffmpeg_status, video_ffmpeg_download,
    video_go2rtc_status, video_go2rtc_download, video_webrtc_start, video_webrtc_offer,
    video_webrtc_stop,
};
use video::Go2Rtc;
use commands::logging::{set_log_level, get_log_path, log_session_settings};
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
    mav_set_home_here, mav_abort_landing, mav_set_param,
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

/// Detect portable mode: if a `.portable` marker file exists next to the
/// executable, redirect all application data into a `data/` folder beside
/// the exe.  Must be called **before** `run()` so the WebView picks up the
/// environment variables.
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

    let data_str = data_dir.to_string_lossy().to_string();

    // Windows: redirect WebView2 user-data folder
    #[cfg(target_os = "windows")]
    {
        std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", &data_str);
    }

    // Linux: redirect XDG directories so WebKitGTK stores data next to the binary
    #[cfg(target_os = "linux")]
    {
        std::env::set_var("XDG_DATA_HOME", &data_str);
        std::env::set_var("XDG_CONFIG_HOME", &data_str);
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

    // Persist + restore the main window's size/position/maximized state across launches.
    // The plugin saves to the OS app-config dir, which portable mode cannot redirect on
    // Windows (Known-Folder API, not env-driven) — so only enable it in installed mode.
    // Portable builds trade window-geometry persistence for a clean, system-path-free runtime.
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
                        // user controls. (`settings()` returns the always-present WebKitSettings; if a
                        // future binding makes it `Option`, wrap in `if let Some`.)
                        let settings = WebViewExt::settings(&wv);
                        settings.set_enable_media_stream(true);
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
            }
            Ok(())
        })
        .manage(AppState::new())
        .manage(MissionStore::new())
        .manage(TerrainProvider::new())
        .manage(RelayHub::new())
        .manage(HidManager::new())
        .manage(Go2Rtc::new())
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
            video_ffmpeg_status,
            video_ffmpeg_download,
            video_go2rtc_status,
            video_go2rtc_download,
            video_webrtc_start,
            video_webrtc_offer,
            video_webrtc_stop,
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
        .run(tauri::generate_context!())
        .expect("error while running Kite Ground Control");
}
