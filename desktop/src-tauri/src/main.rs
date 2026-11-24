// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(all(windows, feature = "vulkan"))]
mod no_vulkan;

fn main() {
    three_yiddish_desktop_lib::run()
}
