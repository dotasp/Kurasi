//! kurasi-server: the HTTP API of CONTRACT.md section 6 (axum) and the MCP server of
//! section 7 (rmcp). `docs/CORE_API.md` binds the two entry points:
//!
//! - [`serve`] binds 127.0.0.1 on port 7343 or the next free one and serves the routes and
//!   the static UI; `project` is `None` when `projects.json` has no usable last project.
//! - [`mcp_stdio`] runs the stdio MCP server over one project in one of three modes.

pub mod api;
pub mod error;
pub mod mcp;
pub mod scan;
pub mod workspace;

use std::net::SocketAddr;
use std::sync::Arc;

pub use api::build_app;
pub use error::{Error, Result};
pub use mcp::{mcp_stdio, KurasiMcp, Mode, Tools, RESOURCES, TOOLS};
pub use scan::ScanRunner;
pub use workspace::{default_ui_dir, ServeOptions, Workspace};

use kurasi_core::config::DEFAULT_PORT;
use kurasi_core::{AppPaths, Project};

const PORT_TRIES: u16 = 50;

/// Stops the server when `shutdown` is called or the handle is dropped.
pub struct ServerHandle {
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<()>>,
    workspace: Arc<Workspace>,
}

impl ServerHandle {
    /// Asks the server to stop accepting connections.
    pub fn shutdown(mut self) {
        self.signal();
    }

    /// Stops the server and waits for it to finish, then closes the open project.
    pub async fn stop(mut self) {
        self.signal();
        if let Some(t) = self.task.take() {
            let _ = t.await;
        }
        self.workspace.close().await;
    }

    fn signal(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }

    pub fn workspace(&self) -> &Arc<Workspace> {
        &self.workspace
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.signal();
    }
}

/// Binds 127.0.0.1 on `bind` (0 picks any port), or on 7343 and the next 50 ports.
async fn bind_listener(bind: Option<u16>) -> Result<tokio::net::TcpListener> {
    let first = bind.unwrap_or(DEFAULT_PORT);
    let last = if bind.is_some() { first } else { first.saturating_add(PORT_TRIES - 1) };
    let mut port = first;
    loop {
        match tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port))).await {
            Ok(l) => return Ok(l),
            Err(e) if port < last => {
                tracing::info!("port {} is busy ({}); trying {}", port, e, port + 1);
                port += 1;
            }
            Err(e) => return Err(Error::other(format!("No free port from {} to {}: {}", first, last, e))),
        }
    }
}

/// Starts the HTTP server (CONTRACT.md section 6) with the default options.
pub async fn serve(paths: AppPaths, project: Option<Project>, bind: Option<u16>) -> Result<(SocketAddr, ServerHandle)> {
    serve_with(paths, project, bind, ServeOptions::default()).await
}

/// `serve` with explicit options (UI folder, watcher, PDF backend, opener).
pub async fn serve_with(
    paths: AppPaths,
    project: Option<Project>,
    bind: Option<u16>,
    mut options: ServeOptions,
) -> Result<(SocketAddr, ServerHandle)> {
    if options.pdf.is_none() {
        if let Some(p) = &project {
            options.pdf = Some(p.pdf().clone());
        }
    }
    let listener = bind_listener(bind).await?;
    let addr = listener.local_addr()?;
    // Removed projects wait in the trash for TRASH_DAYS days (kurasi_core::removal).
    let purge_paths = paths.clone();
    tokio::task::spawn_blocking(move || {
        let age = std::time::Duration::from_secs(kurasi_core::removal::TRASH_DAYS * 86_400);
        match purge_paths.purge_removed(age) {
            Ok(0) => {}
            Ok(n) => tracing::info!("purged {} removed project(s) from the trash", n),
            Err(e) => tracing::warn!("the trash of removed projects was not purged: {}", e),
        }
    });
    let workspace = Arc::new(Workspace::new(paths, options, addr.port()));
    workspace.adopt(project).await?;
    tracing::info!("Kurasi API on http://{}/ (ui from {})", addr, workspace.ui_dir().display());
    let app = build_app(workspace.clone());
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        let result = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = rx.await;
            })
            .await;
        if let Err(e) = result {
            tracing::error!("the HTTP server stopped: {}", e);
        }
    });
    Ok((addr, ServerHandle { shutdown: Some(tx), task: Some(task), workspace }))
}
