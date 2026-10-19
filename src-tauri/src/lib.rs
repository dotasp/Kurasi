//! Kurasi shell library: CLI, native splash, Tauri app and the bridge to the core crates.

pub mod app;
pub mod bridge;
pub mod cli;
pub mod console;
pub mod logging;
pub mod paths;
pub mod splash;

use std::time::Instant;

/// Entry point used by `main.rs`. Returns the process exit code.
pub fn run() -> i32 {
    let t0 = Instant::now();
    let cli = match <cli::Cli as clap::Parser>::try_parse() {
        Ok(c) => c,
        Err(e) => {
            // clap prints help, version and usage errors itself; attach a console first so the
            // text reaches the terminal from this GUI-subsystem executable.
            console::attach();
            let _ = e.print();
            return if e.use_stderr() { 2 } else { 0 };
        }
    };
    match cli::dispatch(cli, t0) {
        Ok(code) => code,
        Err(e) => {
            console::attach();
            eprintln!("Kurasi: {e:#}");
            log::error!("{e:#}");
            2
        }
    }
}
