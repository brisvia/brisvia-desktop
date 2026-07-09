// Entry point. In release builds, no console window on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    brisvia_miner_lib::run()
}
