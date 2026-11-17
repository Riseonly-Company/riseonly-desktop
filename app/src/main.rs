#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    riseonly::riseonly_app::RiseonlyApp::main();
}
