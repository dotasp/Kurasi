//! The only place the shell touches `kurasi-core`, `kurasi-server` and `kurasi-pdf`.
//!
//! Signatures follow `docs/CORE_API.md`. The shell opens every project with the pdfium
//! backend of `kurasi-pdf` and hands the same backend to the server for projects opened
//! through the `/api/project` routes.
//!
//! Folder watching belongs to the server's `Workspace`: it starts the watcher of the project
//! it opens and swaps it when the open project changes through the API, which the shell
//! cannot see. The shell therefore runs no watcher of its own and asks for `watch: true`.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use kurasi_core::model::Origin;
use kurasi_core::pdf::PdfBackend;
pub use kurasi_core::{AppPaths, Project, ProjectInfo};
use kurasi_pdf::PdfiumBackend;
pub use kurasi_server::{Mode, ServerHandle};
use kurasi_server::{serve_with, ServeOptions};

use crate::paths::ShellPaths;

/// Parses the `--mode` value of the `mcp` subcommand.
pub fn parse_mode(s: &str) -> std::result::Result<Mode, String> {
    s.parse()
}

/// The PDF backend of this build: pdfium through `pdfium.dll` next to the executable.
pub fn backend() -> Arc<dyn PdfBackend> {
    Arc::new(PdfiumBackend)
}

/// The backend for one project: the sample project keeps the built-in reader for its own
/// files and uses pdfium for any other PDF in its folder, as the server's workspace does;
/// every other project gets pdfium.
pub fn backend_for(id: &str) -> Arc<dyn PdfBackend> {
    if id == kurasi_core::config::DEMO_ID {
        Arc::new(kurasi_core::simplepdf::SimpleFirst::new(backend()))
    } else {
        backend()
    }
}

/// Server options of the shell: the bundled `ui\` folder, the pdfium backend, the auto-scan
/// timer, and the folder watcher when `watch` is set.
pub fn serve_options(watch: bool) -> ServeOptions {
    ServeOptions { ui_dir: ShellPaths::detect().ui_dir(), watch, auto_scan: true, pdf: Some(backend()), opener: None }
}

/// What the desktop shell needs after startup.
pub struct Started {
    pub url: String,
    pub project_name: Option<String>,
    pub handle: ServerHandle,
}

/// Starts the HTTP server over `project` (or over no project) on `port` or the next free one.
pub async fn start_server(paths: &AppPaths, project: Option<Project>, port: Option<u16>, watch: bool) -> Result<(SocketAddr, ServerHandle)> {
    serve_with(paths.clone(), project, port, serve_options(watch)).await.context("the local server did not start")
}

/// Opens the last project (if its folder exists), starts the HTTP server and runs the
/// incremental scan in the background through the server's scan runner, so the page sees
/// its progress on `/api/scan/status`. `status` receives the splash texts.
pub async fn start_desktop(paths: &AppPaths, port: Option<u16>, status: &(dyn Fn(&str) + Sync)) -> Result<Started> {
    let project = open_last(paths, status)?;
    status("Starting the local server");
    let (addr, handle) = start_server(paths, project.clone(), port, true).await?;
    let url = format!("http://{}/", addr);
    let project_name = project.as_ref().map(|p| p.name());
    if project.is_some() {
        status("Loading references");
        let current = handle.workspace().read().await;
        match current.runner() {
            Ok(r) => {
                r.start(false, Origin::Scan, true);
            }
            Err(e) => log::warn!("the startup scan did not start: {e}"),
        }
    }
    Ok(Started { url, project_name, handle })
}

/// Resolves `projects.json` `last_opened` to an open project when its folder exists.
pub fn open_last(paths: &AppPaths, status: &(dyn Fn(&str) + Sync)) -> Result<Option<Project>> {
    let Some(id) = paths.last_opened() else {
        log::info!("no last project in projects.json");
        return Ok(None);
    };
    let Some(info) = paths.resolve(&id) else {
        log::info!("last project {id} is unknown");
        return Ok(None);
    };
    if !Path::new(&info.folder).is_dir() {
        log::warn!("folder of project {} is unreachable: {}", info.name, info.folder);
        return Ok(None);
    }
    status(&format!("Opening {}", info.name));
    let project = Project::open_with(paths, &info.id, backend_for(&info.id)).with_context(|| format!("project {} did not open", info.name))?;
    paths.set_last_opened(&info.id)?;
    Ok(Some(project))
}

/// Builds or rebuilds the sterile sample project and returns its registry entry.
pub fn demo_build(paths: &AppPaths) -> Result<ProjectInfo> {
    kurasi_core::demo::build(paths).context("the demo project could not be built")
}

/// Resolves an id or a folder to an open project for the console subcommands. An unknown
/// folder that exists is registered under `name` (default: the folder name).
pub fn open_by_id_or_folder(paths: &AppPaths, id_or_folder: &str, name: Option<&str>) -> Result<Project> {
    if let Some(info) = paths.resolve(id_or_folder) {
        return Project::open_with(paths, &info.id, backend_for(&info.id)).with_context(|| format!("project {} did not open", info.name));
    }
    let folder = Path::new(id_or_folder);
    if folder.is_dir() {
        let name = name
            .map(str::to_string)
            .or_else(|| folder.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| "Project".to_string());
        return Project::create_with(paths, &name, folder, backend()).context("the project could not be registered");
    }
    anyhow::bail!("`{id_or_folder}` is neither a known project id nor an existing folder")
}

/// Serves `project` over stdio to an MCP client until the client closes the session.
pub async fn mcp_stdio(project: Project, mode: Mode) -> Result<()> {
    kurasi_server::mcp_stdio(project, mode).await.context("the MCP server stopped with an error")
}
