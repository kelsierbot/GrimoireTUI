// The desktop entry point. On Android the system loads the library and calls
// run() through the mobile entry point instead.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    grimoire_mobile_lib::run()
}
