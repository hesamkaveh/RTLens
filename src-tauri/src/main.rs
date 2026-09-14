// A release build must not spawn a console window on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    rtlens_lib::run()
}
