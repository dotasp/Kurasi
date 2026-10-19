//! The project open in the server (a port of workspace.py).
//!
//! The workspace owns the scan runner, the folder watcher and the auto-scan timer of the open
//! project, the startup record, and the switch to and from the sample project. Requests hold
//! the read side of `current` while they run; the switch routes take the write side, so the
//! old project (and its database) is dropped only after every request in flight has finished.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kurasi_core::config::DEMO_ID;
use kurasi_core::error::Error as CoreError;
use kurasi_core::model::Origin;
use kurasi_core::pdf::{NoPdf, PdfBackend};
use kurasi_core::scanner::WatcherHandle;
use kurasi_core::simplepdf::SimpleFirst;
use kurasi_core::{demo, AppPaths, ProjectInfo, Project};
use serde_json::{json, Map, Value};
use tokio::sync::{Notify, RwLock, RwLockReadGuard};

use crate::scan::ScanRunner;

pub type CoreResult<T> = std::result::Result<T, CoreError>;

/// Opens a file or URL with the Windows default app (`default`) or reveals it in File
/// Explorer (`reveal`). Tests replace it to record the calls.
pub type Opener = Arc<dyn Fn(&str, &str) -> CoreResult<()> + Send + Sync>;

/// Options of `serve_with`. `serve` uses the defaults.
#[derive(Clone)]
pub struct ServeOptions {
    /// Folder of the static UI. Default: `KURASI_UI_DIR`, then `<exe dir>\ui`, then the
    /// repository `ui\` in debug builds.
    pub ui_dir: Option<PathBuf>,
    /// Start the folder watcher of each opened project.
    pub watch: bool,
    /// Run the auto-scan timer of each opened project.
    pub auto_scan: bool,
    /// PDF backend for projects opened through the routes. Default: the backend of the
    /// project given to `serve`, else `NoPdf`.
    pub pdf: Option<Arc<dyn PdfBackend>>,
    pub opener: Option<Opener>,
}

impl Default for ServeOptions {
    fn default() -> Self {
        ServeOptions { ui_dir: None, watch: true, auto_scan: true, pdf: None, opener: None }
    }
}

/// `KURASI_UI_DIR`, then `<exe dir>\ui`, then the repository `ui\` (debug builds), then
/// `<exe dir>\ui` even when absent.
pub fn default_ui_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("KURASI_UI_DIR").filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    let exe_ui = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("ui")));
    if let Some(d) = &exe_ui {
        if d.join("index.html").is_file() {
            return d.clone();
        }
    }
    if cfg!(debug_assertions) {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui");
        if repo.join("index.html").is_file() {
            return kurasi_core::util::absolute(&repo);
        }
    }
    exe_ui.unwrap_or_else(|| PathBuf::from("ui"))
}

fn default_opener() -> Opener {
    Arc::new(|mode: &str, target: &str| -> CoreResult<()> {
        let mut cmd = if cfg!(windows) {
            let mut c = std::process::Command::new("explorer.exe");
            if mode == "reveal" {
                c.arg(format!("/select,{}", target));
            } else {
                c.arg(target);
            }
            c
        } else {
            let mut c = std::process::Command::new("xdg-open");
            c.arg(target);
            c
        };
        cmd.spawn().map(|_| ()).map_err(|e| CoreError::Other(format!("The file could not be opened: {}.", e)))
    })
}

/// What is open right now.
#[derive(Default)]
pub struct Current {
    pub project: Option<Project>,
    pub runner: Option<Arc<ScanRunner>>,
    watcher: Option<Mutex<WatcherHandle>>,
    timer: Option<tokio::task::JoinHandle<()>>,
}

impl Current {
    /// The open project, or the 400 error the Python build raises.
    pub fn require(&self) -> CoreResult<&Project> {
        self.project
            .as_ref()
            .ok_or_else(|| CoreError::bad("No project is open. Create a project or open the sample project first."))
    }

    pub fn runner(&self) -> CoreResult<&Arc<ScanRunner>> {
        self.require()?;
        self.runner.as_ref().ok_or_else(|| CoreError::bad("No project is open."))
    }
}

pub struct Workspace {
    paths: AppPaths,
    ui_dir: PathBuf,
    port: u16,
    options: ServeOptions,
    pdf: Arc<dyn PdfBackend>,
    opener: Opener,
    current: RwLock<Current>,
    previous_project_id: Mutex<Option<String>>,
    startup: Mutex<Value>,
    /// Counts the closes of the open project; a long-poll of `/api/changes` that sees it move
    /// answers at once and lets go of the project it holds.
    generation: AtomicU64,
    closing: Notify,
}

fn startup_record(opened_last: bool, missing: Option<&ProjectInfo>) -> Value {
    json!({
        "opened_last": opened_last,
        "folder_missing": missing.is_some(),
        "missing_name": missing.map(|m| m.name.clone()),
        "missing_folder": missing.map(|m| m.folder.clone()),
    })
}

impl Workspace {
    pub fn new(paths: AppPaths, options: ServeOptions, port: u16) -> Workspace {
        let ui_dir = options.ui_dir.clone().unwrap_or_else(default_ui_dir);
        let pdf: Arc<dyn PdfBackend> = options.pdf.clone().unwrap_or_else(|| Arc::new(NoPdf));
        let opener = options.opener.clone().unwrap_or_else(default_opener);
        Workspace {
            paths,
            ui_dir,
            port,
            options,
            pdf,
            opener,
            current: RwLock::new(Current::default()),
            previous_project_id: Mutex::new(None),
            startup: Mutex::new(startup_record(false, None)),
            generation: AtomicU64::new(0),
            closing: Notify::new(),
        }
    }

    /// Moves on every close of the open project (see [`Workspace::closing`]).
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Notified on every close of the open project.
    pub fn closing(&self) -> &Notify {
        &self.closing
    }

    pub fn paths(&self) -> &AppPaths {
        &self.paths
    }

    pub fn ui_dir(&self) -> &Path {
        &self.ui_dir
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn opener(&self) -> &Opener {
        &self.opener
    }

    /// Read access to the open project for the duration of one request.
    pub async fn read(&self) -> RwLockReadGuard<'_, Current> {
        self.current.read().await
    }

    /// The sample project reads its own files with the built-in reader and any other PDF in
    /// its folder (a sample the Python build made) with the configured backend.
    fn backend_for(&self, id: &str) -> Arc<dyn PdfBackend> {
        if id == DEMO_ID {
            Arc::new(SimpleFirst::new(self.pdf.clone()))
        } else {
            self.pdf.clone()
        }
    }

    // ------------------------------------------------------------ startup

    /// Installs the project the shell opened at startup (or records that none was usable).
    pub async fn adopt(&self, project: Option<Project>) -> CoreResult<()> {
        match project {
            Some(p) => {
                let mut cur = self.current.write().await;
                self.install(&mut cur, p);
                *self.startup.lock().unwrap_or_else(|e| e.into_inner()) = startup_record(true, None);
            }
            None => {
                let entries = self.paths.projects()?;
                let (_, missing) = kurasi_core::config::choose_last(&entries, |f| Path::new(f).is_dir());
                *self.startup.lock().unwrap_or_else(|e| e.into_inner()) = startup_record(false, missing.as_ref());
            }
        }
        Ok(())
    }

    /// Section 12 of CONTRACT.md: open the most recent reachable project and scan it in the
    /// background.
    pub async fn startup_open(&self) -> CoreResult<Value> {
        let entries = self.paths.projects()?;
        let (entry, mut missing) = kurasi_core::config::choose_last(&entries, |f| Path::new(f).is_dir());
        if let Some(e) = entry {
            match self.open(&e.id, true).await {
                Ok(()) => {
                    let rec = startup_record(true, None);
                    *self.startup.lock().unwrap_or_else(|x| x.into_inner()) = rec.clone();
                    return Ok(rec);
                }
                Err(err) => {
                    tracing::warn!("startup could not open {}: {}", e.folder, err);
                    missing = Some(e);
                }
            }
        }
        let rec = startup_record(false, missing.as_ref());
        *self.startup.lock().unwrap_or_else(|x| x.into_inner()) = rec.clone();
        Ok(rec)
    }

    // ------------------------------------------------------------ open and close

    fn install(&self, cur: &mut Current, project: Project) {
        let runner = ScanRunner::new(project.clone());
        let watcher = if self.options.watch {
            match project.start_watcher() {
                Ok(w) => Some(Mutex::new(w)),
                Err(e) => {
                    tracing::warn!("folder watcher did not start for {}: {}", project.folder().display(), e);
                    None
                }
            }
        } else {
            None
        };
        let timer = if self.options.auto_scan { Some(spawn_timer(project.clone(), runner.clone())) } else { None };
        cur.project = Some(project);
        cur.runner = Some(runner);
        cur.watcher = watcher;
        cur.timer = timer;
    }

    async fn close_locked(&self, cur: &mut Current) {
        if cur.project.is_some() {
            self.generation.fetch_add(1, Ordering::SeqCst);
            self.closing.notify_waiters();
        }
        if let Some(t) = cur.timer.take() {
            t.abort();
            let _ = t.await;
        }
        if let Some(w) = cur.watcher.take() {
            // Stopping joins the watcher thread; keep that off the async runtime.
            let _ = tokio::task::spawn_blocking(move || drop(w)).await;
        }
        if let Some(r) = cur.runner.take() {
            let _ = tokio::task::spawn_blocking(move || r.wait(Duration::from_secs(30))).await;
        }
        cur.project = None;
    }

    pub async fn close(&self) {
        let mut cur = self.current.write().await;
        self.close_locked(&mut cur).await;
    }

    /// Opens a registered project (by id), replacing the open one. `scan` starts an
    /// incremental scan in the background.
    pub async fn open(&self, pid: &str, scan: bool) -> CoreResult<()> {
        let mut cur = self.current.write().await;
        self.open_locked(&mut cur, pid, scan).await
    }

    async fn open_locked(&self, cur: &mut Current, pid: &str, scan: bool) -> CoreResult<()> {
        let entry = self
            .paths
            .resolve(pid)
            .ok_or_else(|| CoreError::not_found(format!("The project {} is not registered.", pid)))?;
        if !Path::new(&entry.folder).is_dir() {
            return Err(CoreError::not_found(format!(
                "The folder {} of the project {} is unreachable. Reconnect the drive, then open it again.",
                entry.folder, entry.name
            )));
        }
        let current_id = cur.project.as_ref().map(|p| p.id().to_string());
        if current_id.as_deref() == Some(entry.id.as_str()) {
            self.paths.set_last_opened(&entry.id)?;
            return Ok(());
        }
        self.close_locked(cur).await;
        {
            let mut prev = self.previous_project_id.lock().unwrap_or_else(|e| e.into_inner());
            if entry.demo || entry.id == DEMO_ID {
                if let Some(c) = &current_id {
                    if c != DEMO_ID {
                        *prev = Some(c.clone());
                    }
                }
            } else {
                *prev = None;
            }
        }
        let paths = self.paths.clone();
        let id = entry.id.clone();
        let backend = self.backend_for(&id);
        let project = tokio::task::spawn_blocking(move || Project::open_with(&paths, &id, backend))
            .await
            .map_err(|e| CoreError::Other(e.to_string()))??;
        self.install(cur, project);
        self.paths.set_last_opened(&entry.id)?;
        if scan {
            if let Some(r) = &cur.runner {
                r.start(false, Origin::Scan, true);
            }
        }
        Ok(())
    }

    /// `POST /api/project`: registers and opens a folder, then starts a scan.
    pub async fn create(&self, body: &Map<String, Value>) -> CoreResult<()> {
        let folder = body.get("folder").and_then(|v| v.as_str()).unwrap_or("").trim().trim_matches('"').to_string();
        if folder.is_empty() {
            return Err(CoreError::bad("Choose the folder to scan."));
        }
        let checked = check_folder(&self.paths, &folder);
        if checked["ok"] != Value::Bool(true) {
            return Err(CoreError::bad(checked["error"].as_str().unwrap_or("Choose the folder to scan.").to_string()));
        }
        let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        let mut settings = Map::new();
        if let Some(ft) = body.get("file_types").filter(|v| v.as_array().map(|a| !a.is_empty()).unwrap_or(false)) {
            settings.insert("file_types".into(), ft.clone());
        }
        if let Some(n) = body.get("auto_scan_min").filter(|v| !v.is_null()) {
            settings.insert("auto_scan_min".into(), n.clone());
        }
        if let Some(ro) = body.get("read_only") {
            settings.insert("read_only".into(), Value::Bool(truthy(ro)));
        }
        let paths = self.paths.clone();
        let backend = self.pdf.clone();
        let id = tokio::task::spawn_blocking(move || -> CoreResult<String> {
            let project = Project::create_with(&paths, &name, Path::new(&folder), backend)?;
            if !settings.is_empty() {
                project.update_settings(&settings, Origin::Ui)?;
            }
            Ok(project.id().to_string())
        })
        .await
        .map_err(|e| CoreError::Other(e.to_string()))??;
        let mut cur = self.current.write().await;
        self.open_locked(&mut cur, &id, false).await?;
        if let Some(r) = &cur.runner {
            r.start(false, Origin::Ui, false);
        }
        Ok(())
    }

    /// `POST /api/demo`: builds or resets the sample project and opens it.
    pub async fn open_demo(&self, reset: bool) -> CoreResult<()> {
        let mut cur = self.current.write().await;
        let demo_open = cur.project.as_ref().map(|p| p.is_demo()).unwrap_or(false);
        let previous = if demo_open {
            self.previous_project_id.lock().unwrap_or_else(|e| e.into_inner()).clone()
        } else {
            cur.project.as_ref().map(|p| p.id().to_string())
        };
        let built = self.paths.demo_dir().is_dir() && self.paths.resolve(DEMO_ID).is_some();
        if reset || !built {
            if demo_open {
                self.close_locked(&mut cur).await;
            }
            let paths = self.paths.clone();
            tokio::task::spawn_blocking(move || demo::build(&paths)).await.map_err(|e| CoreError::Other(e.to_string()))??;
        }
        self.open_locked(&mut cur, DEMO_ID, false).await?;
        if let Some(p) = previous.filter(|p| p != DEMO_ID) {
            *self.previous_project_id.lock().unwrap_or_else(|e| e.into_inner()) = Some(p);
        }
        Ok(())
    }

    // ------------------------------------------------------------ the project list

    /// `GET /api/projects`: every entry, last opened first (never opened last, then newest
    /// created), with `exists` (the folder is reachable) and `open`.
    pub fn projects_list(&self, open_id: Option<&str>) -> CoreResult<Vec<Value>> {
        let mut entries = self.paths.projects()?;
        entries.sort_by(|a, b| b.last_opened.cmp(&a.last_opened).then_with(|| b.created.cmp(&a.created)));
        Ok(entries
            .iter()
            .map(|e| {
                json!({
                    "id": e.id, "name": e.name, "folder": e.folder, "created": e.created,
                    "last_opened": e.last_opened, "demo": e.demo || e.id == DEMO_ID,
                    "exists": Path::new(&e.folder).is_dir(), "open": open_id == Some(e.id.as_str()),
                })
            })
            .collect())
    }

    /// `GET /api/projects/<id>`: the entry and `data`, the folders "Also delete Kurasi's data"
    /// would remove (`AppPaths::data_targets`).
    pub async fn project_entry(&self, id: &str) -> CoreResult<Value> {
        let entry = self.entry(id)?;
        let paths = self.paths.clone();
        let e = entry.clone();
        let data = tokio::task::spawn_blocking(move || paths.data_targets(&e)).await.map_err(|e| CoreError::Other(e.to_string()))?;
        let open = self.current.read().await.project.as_ref().map(|p| p.id() == entry.id).unwrap_or(false);
        Ok(json!({
            "id": entry.id, "name": entry.name, "folder": entry.folder, "created": entry.created,
            "last_opened": entry.last_opened, "demo": entry.demo || entry.id == DEMO_ID,
            "exists": Path::new(&entry.folder).is_dir(), "open": open, "data": data,
        }))
    }

    fn entry(&self, id: &str) -> CoreResult<ProjectInfo> {
        self.paths
            .projects()?
            .into_iter()
            .find(|e| e.id == id)
            .ok_or_else(|| CoreError::not_found(format!("The project {} is not in the list.", id)))
    }

    /// `DELETE /api/projects/<id>`: takes the project off the list, closing it first when it is
    /// open. With `with_data` its data goes to the trash (`AppPaths::remove_project`). When the
    /// removal fails, a project that was open is opened again.
    pub async fn remove_project(&self, id: &str, with_data: bool) -> CoreResult<Value> {
        let entry = self.entry(id)?;
        if entry.demo || entry.id == DEMO_ID {
            return Err(CoreError::bad("The sample project stays in the list. Reset rebuilds it from scratch."));
        }
        let mut cur = self.current.write().await;
        let was_open = cur.project.as_ref().map(|p| p.id() == entry.id).unwrap_or(false);
        if was_open {
            self.close_locked(&mut cur).await;
        }
        let paths = self.paths.clone();
        let pid = entry.id.clone();
        let result = tokio::task::spawn_blocking(move || paths.remove_project(&pid, with_data))
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?;
        let removal = match result {
            Ok(r) => r,
            Err(e) => {
                if was_open {
                    if let Err(again) = self.open_locked(&mut cur, &entry.id, false).await {
                        tracing::warn!("{} did not open again after the failed removal: {}", entry.name, again);
                    }
                }
                return Err(e);
            }
        };
        {
            let mut prev = self.previous_project_id.lock().unwrap_or_else(|e| e.into_inner());
            if prev.as_deref() == Some(entry.id.as_str()) {
                *prev = None;
            }
        }
        {
            let mut st = self.startup.lock().unwrap_or_else(|e| e.into_inner());
            if st["missing_folder"].as_str() == Some(entry.folder.as_str()) {
                *st = startup_record(false, None);
            }
        }
        drop(cur);
        Ok(json!({
            "removal_id": removal.removal_id, "project": removal.entry, "with_data": removal.with_data,
            "moved": removal.moved, "closed": was_open,
        }))
    }

    /// `POST /api/projects/restore`: puts a removed project back in the list, with its data when
    /// the removal moved it. The project stays closed.
    pub async fn restore_project(&self, removal_id: &str) -> CoreResult<Value> {
        let paths = self.paths.clone();
        let rid = removal_id.to_string();
        let entry = tokio::task::spawn_blocking(move || paths.restore_project(&rid))
            .await
            .map_err(|e| CoreError::Other(e.to_string()))??;
        Ok(json!({"project": entry}))
    }

    /// `POST /api/projects/<id>/reset`: rebuilds the sample project from scratch and opens it
    /// again when it was open. Any other project answers 400.
    pub async fn reset_project(&self, id: &str) -> CoreResult<Value> {
        let is_demo = id == DEMO_ID || self.paths.resolve(id).map(|e| e.demo || e.id == DEMO_ID).unwrap_or(false);
        if !is_demo {
            return Err(CoreError::bad("Only the sample project can be reset. Remove takes another project off the list."));
        }
        let mut cur = self.current.write().await;
        let open = cur.project.as_ref().map(|p| p.is_demo()).unwrap_or(false);
        let prev = self.previous_project_id.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if open {
            self.close_locked(&mut cur).await;
        }
        let paths = self.paths.clone();
        let built = tokio::task::spawn_blocking(move || demo::build(&paths)).await.map_err(|e| CoreError::Other(e.to_string()))??;
        if open {
            self.open_locked(&mut cur, DEMO_ID, false).await?;
            *self.previous_project_id.lock().unwrap_or_else(|e| e.into_inner()) = prev;
        }
        Ok(json!({"project": built, "open": open}))
    }

    // ------------------------------------------------------------ state

    pub async fn state(&self) -> CoreResult<Value> {
        let cur = self.current.read().await;
        self.state_locked(&cur)
    }

    pub fn state_locked(&self, cur: &Current) -> CoreResult<Value> {
        let projects = self.projects_list(cur.project.as_ref().map(|p| p.id()))?;
        let startup = self.startup.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let mut out = json!({
            "project": Value::Null, "projects": projects, "settings": {},
            "counts": {"references": 0, "authors": 0, "tags": 0, "highlights": 0},
            "change_seq": 0, "demo": false, "startup": startup,
            "version": env!("CARGO_PKG_VERSION"),
        });
        if let Some(p) = &cur.project {
            let obj = out.as_object_mut().expect("object");
            obj.insert("project".into(), p.project_info());
            obj.insert("settings".into(), Value::Object(p.settings()));
            obj.insert("counts".into(), serde_json::to_value(p.counts()?)?);
            obj.insert("change_seq".into(), json!(p.change_seq()));
            obj.insert("demo".into(), json!(p.is_demo()));
            if p.is_demo() {
                let prev = self.previous_project_id.lock().unwrap_or_else(|e| e.into_inner()).clone();
                obj.insert("previous_project_id".into(), json!(prev));
            }
        }
        Ok(out)
    }
}

/// `POST /api/project/check`: whether `raw` names a folder Kurasi can scan. `ok` with the full
/// path, a project name from the folder name and, when the folder is in the list already, its
/// `project_id` and `project_name`; else `ok: false` with one sentence in `error`.
pub fn check_folder(paths: &AppPaths, raw: &str) -> Value {
    let f = raw.trim().trim_matches('"').trim();
    let fail = |msg: String| json!({"ok": false, "folder": f, "error": msg});
    if f.is_empty() {
        return fail("Enter the folder to scan, for example D:\\Papers\\Thesis.".into());
    }
    let p = Path::new(f);
    if !p.is_absolute() {
        return fail(format!("Enter the full path of the folder, with its drive letter, for example D:\\Papers\\{}.", f));
    }
    match std::fs::metadata(p) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return fail(format!("The folder {} does not exist. Check the path, or reconnect the drive that holds it.", f))
        }
        Err(e) => return fail(format!("Kurasi cannot reach {}: {}.", f, e)),
        Ok(m) if !m.is_dir() => return fail(format!("{} is a file. Choose the folder that holds it.", f)),
        Ok(_) => {}
    }
    if let Err(e) = std::fs::read_dir(p) {
        return fail(format!("Kurasi cannot read the folder {}: {}.", f, e));
    }
    let abs = kurasi_core::util::absolute(p);
    if kurasi_core::util::norm_key(&abs) == kurasi_core::util::norm_key(&paths.demo_dir()) {
        return fail("That folder holds the sample project. Use Help, Open sample project.".into());
    }
    let name = abs.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| abs.to_string_lossy().into_owned());
    let mut out = json!({"ok": true, "folder": abs.to_string_lossy(), "name": name});
    if let Some(e) = paths.resolve(&abs.to_string_lossy()) {
        out["project_id"] = json!(e.id);
        out["project_name"] = json!(e.name);
    }
    out
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        Value::String(s) => !matches!(s.trim().to_lowercase().as_str(), "" | "0" | "false" | "no" | "off"),
        Value::Null => false,
        _ => true,
    }
}

/// A scan of the whole folder every `auto_scan_min` minutes (0 turns it off), checked every
/// 20 seconds.
fn spawn_timer(project: Project, runner: Arc<ScanRunner>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(20));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        tick.tick().await;
        loop {
            tick.tick().await;
            let minutes = match project.setting("auto_scan_min") {
                Value::Number(n) => n.as_i64().unwrap_or(0),
                Value::String(s) => s.trim().parse::<i64>().unwrap_or(0),
                _ => 0,
            };
            if minutes > 0 && runner.since_last_scan() >= Duration::from_secs(minutes as u64 * 60) {
                runner.touch_last_scan();
                runner.start(false, Origin::Scan, true);
            }
        }
    })
}
