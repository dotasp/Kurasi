//! Test binary: the stdio MCP server over one project, used to measure the cold start and
//! to drive the server from a stdio client. The real entry point is `kurasi.exe mcp`.
//!
//! Usage: `kurasi-mcp-test <project id or folder | demo> [read|propose|write]`
//! `KURASI_HOME` must point at a temporary app-data folder; the binary refuses to run
//! against the real `%LOCALAPPDATA%\Kurasi`.

use std::process::ExitCode;
use std::sync::Arc;

use kurasi_core::simplepdf::SimplePdfBackend;
use kurasi_core::{demo, AppPaths, Project};
use kurasi_server::{mcp_stdio, Mode};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(key) = args.first() else {
        eprintln!("usage: kurasi-mcp-test <project id or folder | demo> [read|propose|write]");
        return ExitCode::from(2);
    };
    let mode = match args.get(1).map(|m| m.parse::<Mode>()) {
        None => Mode::Write,
        Some(Ok(m)) => m,
        Some(Err(e)) => {
            eprintln!("{}", e);
            return ExitCode::from(2);
        }
    };
    let home = std::env::var_os("KURASI_HOME").filter(|v| !v.is_empty());
    let Some(home) = home else {
        eprintln!("Set KURASI_HOME to a temporary folder first.");
        return ExitCode::from(2);
    };
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let real = std::path::PathBuf::from(local).join("Kurasi");
        if kurasi_core::util::norm_key(std::path::Path::new(&home)) == kurasi_core::util::norm_key(&real) {
            eprintln!("KURASI_HOME points at the real app data; use a temporary folder.");
            return ExitCode::from(2);
        }
    }
    let paths = AppPaths::detect();
    let project = if key == "demo" {
        demo::open(&paths)
    } else {
        Project::open_with(&paths, key, Arc::new(SimplePdfBackend))
    };
    let project = match project {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}", e);
            return ExitCode::from(1);
        }
    };
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("tokio runtime: {}", e);
            return ExitCode::from(1);
        }
    };
    match rt.block_on(mcp_stdio(project, mode)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{}", e);
            ExitCode::from(1)
        }
    }
}
