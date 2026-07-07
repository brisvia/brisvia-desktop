// Punto de entrada. En release, sin consola en Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    brisvia_miner_lib::run()
}
