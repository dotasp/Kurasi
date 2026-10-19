//! Command line: `kurasi` (no arguments) opens the app; `app`, `mcp`, `scan`, `demo`, `export`
//! and `serve` mirror the Python CLI. Only `app` initialises Tauri.

use std::time::Instant;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use kurasi_core::scanner::ScanMode;

use crate::bridge::{self, AppPaths};
use crate::paths::ShellPaths;

#[derive(Parser, Debug)]
#[command(name = "kurasi", version, about = "Kurasi reference curation", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Open the desktop window (the default with no subcommand)
    App {
        /// Port for the local server (default 7343 or the next free one)
        #[arg(long)]
        port: Option<u16>,
    },
    /// Run the MCP server over stdio for an AI agent
    Mcp {
        /// Project id or project folder
        #[arg(long)]
        project: String,
        /// What the agent may do: write (the default), or the stricter propose or read
        #[arg(long, default_value = "write", value_parser = bridge::parse_mode)]
        mode: bridge::Mode,
    },
    /// Scan a project folder once and print the result
    Scan {
        /// Project id or folder (a new folder is registered)
        #[arg(long)]
        project: String,
        /// Hash every file and re-read every PDF
        #[arg(long)]
        full: bool,
        /// Mark the project read-only before scanning
        #[arg(long)]
        read_only: bool,
        /// Project name for a newly registered folder
        #[arg(long)]
        name: Option<String>,
    },
    /// Build or rebuild the sample project
    Demo,
    /// Write a reference export under <folder>\_exports
    Export {
        #[arg(long)]
        project: String,
        #[arg(long, value_parser = ["apa-md", "bibtex", "csl-json", "highlights-json"])]
        format: String,
        #[arg(long, num_args = 0..)]
        ref_ids: Vec<String>,
    },
    /// Run the local HTTP server without a window
    Serve {
        #[arg(long)]
        port: Option<u16>,
        /// Leave the last project closed
        #[arg(long)]
        no_startup: bool,
        /// Run without the folder watcher
        #[arg(long)]
        no_watch: bool,
    },
}

pub fn dispatch(cli: Cli, t0: Instant) -> Result<i32> {
    match cli.cmd {
        None | Some(Cmd::App { port: None }) => {
            crate::app::run_app(None, t0)?;
            Ok(0)
        }
        Some(Cmd::App { port }) => {
            crate::app::run_app(port, t0)?;
            Ok(0)
        }
        Some(Cmd::Mcp { project, mode }) => {
            // stdout belongs to the protocol: the log goes to the file and stderr only.
            let shell = ShellPaths::detect();
            crate::logging::init(&shell.log_file(), true, t0);
            let paths = AppPaths::detect();
            let rt = runtime()?;
            rt.block_on(async {
                let p = bridge::open_by_id_or_folder(&paths, &project, None)?;
                log::info!("MCP server over {} in {} mode after {:.0} ms", p.folder().display(), mode, t0.elapsed().as_secs_f64() * 1000.0);
                bridge::mcp_stdio(p, mode).await
            })?;
            Ok(0)
        }
        other => {
            crate::console::attach();
            let shell = ShellPaths::detect();
            crate::logging::init(&shell.log_file(), true, t0);
            let paths = AppPaths::detect();
            log::info!("app data root {}", shell.root.display());
            match other.expect("subcommand") {
                Cmd::Scan {
                    project,
                    full,
                    read_only,
                    name,
                } => {
                    let p = bridge::open_by_id_or_folder(&paths, &project, name.as_deref())?;
                    if read_only {
                        p.set_read_only(true)?;
                    }
                    let mode = if full { ScanMode::Full } else { ScanMode::Incremental };
                    let report = p.scan(mode, |_| {})?;
                    let info = p.info();
                    print_json(&serde_json::json!({
                        "project": info.id, "folder": info.folder, "read_only": info.read_only,
                        "result": serde_json::to_value(&report)?
                    }));
                    Ok(0)
                }
                Cmd::Demo => {
                    let info = bridge::demo_build(&paths)?;
                    print_json(&serde_json::to_value(&info)?);
                    Ok(0)
                }
                Cmd::Export { project, format, ref_ids } => {
                    let p = bridge::open_by_id_or_folder(&paths, &project, None)?;
                    let ids = if ref_ids.is_empty() { None } else { Some(ref_ids.as_slice()) };
                    let path = p.export_references(&format, ids)?;
                    print_json(&serde_json::json!({"format": format, "path": path.to_string_lossy()}));
                    Ok(0)
                }
                Cmd::Serve { port, no_startup, no_watch } => {
                    let rt = runtime()?;
                    rt.block_on(async move {
                        let status = |t: &str| log::info!("{t}");
                        let project = if no_startup { None } else { bridge::open_last(&paths, &status)? };
                        let name = project.as_ref().map(|p| p.name());
                        let (addr, handle) = bridge::start_server(&paths, project, port, !no_watch).await?;
                        if name.is_some() {
                            let current = handle.workspace().read().await;
                            if let Ok(r) = current.runner() {
                                r.start(false, kurasi_core::model::Origin::Scan, true);
                            }
                        }
                        println!("Kurasi serving http://{}/ (project: {}; Ctrl+C stops it)", addr, name.as_deref().unwrap_or("none"));
                        tokio::signal::ctrl_c().await.context("Ctrl+C handler")?;
                        handle.stop().await;
                        Ok::<i32, anyhow::Error>(0)
                    })
                }
                Cmd::App { .. } | Cmd::Mcp { .. } => unreachable!(),
            }
        }
    }
}

fn runtime() -> Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("tokio runtime")
}

fn print_json(v: &serde_json::Value) {
    println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode_of(args: &[&str]) -> bridge::Mode {
        match Cli::try_parse_from(args).expect("arguments parse").cmd {
            Some(Cmd::Mcp { mode, .. }) => mode,
            other => panic!("not the mcp subcommand: {:?}", other),
        }
    }

    #[test]
    fn mcp_mode_defaults_to_write() {
        assert_eq!(mode_of(&["kurasi", "mcp", "--project", "D:\\Papers"]), bridge::Mode::Write);
        assert_eq!(mode_of(&["kurasi", "mcp", "--project", "x", "--mode", "propose"]), bridge::Mode::Propose);
        assert_eq!(mode_of(&["kurasi", "mcp", "--project", "x", "--mode", "read"]), bridge::Mode::Read);
        assert!(Cli::try_parse_from(["kurasi", "mcp", "--project", "x", "--mode", "admin"]).is_err());
    }
}
