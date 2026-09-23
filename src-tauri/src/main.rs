// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Err(error) = hive_lib::run() {
        eprintln!("hive-app: {error}");
        std::process::exit(1);
    }
}
