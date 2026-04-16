// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    kite_gc_lib::setup_portable_mode();
    kite_gc_lib::run()
}
