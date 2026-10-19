// No console window on double-click in release builds. The console subcommands attach to the
// parent console themselves (see console.rs).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    std::process::exit(kurasi_shell::run());
}
