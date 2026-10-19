//! kurasi-core: project storage, scanner, search, sidecar, exports and the sterile sample.
//!
//! The behaviour reference is the Python build (`CONTRACT.md`); the SQLite schema is identical,
//! so a `kurasi.sqlite` made by either build opens in the other. PDF access goes through the
//! [`pdf::PdfBackend`] trait: `kurasi-pdf` supplies the pdfium implementation, tests and the
//! sample project use the built-in [`simplepdf`] reader, and [`pdf::NoPdf`] is the fallback.

pub mod ailog;
pub mod catalog;
pub mod change;
pub mod config;
pub mod db;
pub mod demo;
pub mod error;
pub mod export;
pub mod lock;
pub mod media;
pub mod model;
pub mod pdf;
pub mod removal;
pub mod scanner;
pub mod search;
pub mod sidecar;
pub mod simplepdf;
pub mod util;

use std::fs::Metadata;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use serde_json::Value;

pub use config::{AppPaths, ProjectInfo};
pub use db::Db;
pub use error::{Error, Result};
pub use pdf::{NoPdf, PdfBackend, PdfDoc};

pub type CloudCheck = Box<dyn Fn(&Path, &Metadata) -> bool + Send + Sync>;

pub(crate) struct Inner {
    paths: AppPaths,
    id: String,
    folder: PathBuf,
    dir: PathBuf,
    db: Db,
    lock: lock::WriteLock,
    changes: change::ChangeFeed,
    pdf: Arc<dyn PdfBackend>,
    scan_lock: Mutex<()>,
    defer_sidecar: AtomicBool,
    cloud_check: RwLock<Option<CloudCheck>>,
    text_index: Mutex<Option<search::FullText>>,
}

/// One opened project. Cheap to clone (Arc inside). Thread safe.
#[derive(Clone)]
pub struct Project {
    pub(crate) inner: Arc<Inner>,
}

impl Project {
    /// Opens or creates `<appdata>/projects/<id>/kurasi.sqlite` for a registered project (by id
    /// or by folder). An unregistered folder that exists is registered first. Never writes into
    /// the folder unless a write method is called and `read_only` is false.
    pub fn open(paths: &AppPaths, id: &str) -> Result<Project> {
        Project::open_with(paths, id, Arc::new(NoPdf))
    }

    /// `open` with a PDF backend.
    pub fn open_with(paths: &AppPaths, id: &str, pdf: Arc<dyn PdfBackend>) -> Result<Project> {
        let entry = match paths.resolve(id) {
            Some(e) => e,
            None => {
                let folder = Path::new(id);
                if !folder.is_dir() {
                    return Err(Error::not_found(format!("No project has the id or folder {}.", id)));
                }
                paths.register(None, folder, None, false)?
            }
        };
        Project::open_entry(paths, &entry, None, None, pdf)
    }

    /// Registers `folder` under `name` (or returns the entry that already has it) and opens it.
    pub fn create(paths: &AppPaths, name: &str, folder: &Path) -> Result<Project> {
        Project::create_with(paths, name, folder, Arc::new(NoPdf))
    }

    pub fn create_with(paths: &AppPaths, name: &str, folder: &Path, pdf: Arc<dyn PdfBackend>) -> Result<Project> {
        if !folder.is_dir() {
            return Err(Error::bad(format!("The folder {} does not exist or is unreachable.", folder.display())));
        }
        if util::norm_key(folder) == util::norm_key(&paths.demo_dir()) {
            return Err(Error::bad("That folder holds the sample project. Use Help, Open sample project."));
        }
        let name = name.trim();
        let entry = paths.register(if name.is_empty() { None } else { Some(name) }, folder, None, false)?;
        Project::open_entry(paths, &entry, if name.is_empty() { None } else { Some(name) }, None, pdf)
    }

    /// Opens a registry entry with an optional name and read-only override (the demo builder
    /// and the CLI use this).
    pub fn open_entry(
        paths: &AppPaths,
        entry: &ProjectInfo,
        name: Option<&str>,
        read_only: Option<bool>,
        pdf: Arc<dyn PdfBackend>,
    ) -> Result<Project> {
        let folder = util::absolute(Path::new(&entry.folder));
        let dir = paths.project_dir(&entry.id);
        let db = Db::open(&dir.join("kurasi.sqlite"))?;
        let inner = Inner {
            paths: paths.clone(),
            id: entry.id.clone(),
            folder: folder.clone(),
            dir: dir.clone(),
            db,
            lock: lock::WriteLock::new(dir.join("write.lock")),
            changes: change::ChangeFeed::new(),
            pdf,
            scan_lock: Mutex::new(()),
            defer_sidecar: AtomicBool::new(false),
            cloud_check: RwLock::new(None),
            text_index: Mutex::new(None),
        };
        let project = Project { inner: Arc::new(inner) };
        {
            let _g = project.inner.lock.acquire()?;
            let base = folder.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let folder_str = folder.to_string_lossy().into_owned();
            project.inner.db.tx(|c| {
                if db::get_setting(c, "folder").is_none() {
                    db::set_setting(c, "folder", &Value::String(folder_str.clone()))?;
                }
                if name.is_some() || db::get_setting(c, "name").is_none() {
                    let n = name.map(str::to_string).filter(|n| !n.is_empty()).unwrap_or_else(|| {
                        if entry.name.is_empty() { base.clone() } else { entry.name.clone() }
                    });
                    db::set_setting(c, "name", &Value::String(n))?;
                }
                if let Some(ro) = read_only {
                    db::set_setting(c, "read_only", &Value::Bool(ro))?;
                }
                Ok(())
            })?;
        }
        sidecar::import_if_empty(&project)?;
        Ok(project)
    }

    pub fn id(&self) -> &str {
        &self.inner.id
    }

    pub fn folder(&self) -> &Path {
        &self.inner.folder
    }

    /// The app-data folder of this project (database and write.lock).
    pub fn dir(&self) -> &Path {
        &self.inner.dir
    }

    pub fn paths(&self) -> &AppPaths {
        &self.inner.paths
    }

    pub fn db(&self) -> &Db {
        &self.inner.db
    }

    pub fn pdf(&self) -> &Arc<dyn PdfBackend> {
        &self.inner.pdf
    }

    pub fn write_lock(&self) -> &lock::WriteLock {
        &self.inner.lock
    }

    pub(crate) fn changes(&self) -> &change::ChangeFeed {
        &self.inner.changes
    }

    pub fn name(&self) -> String {
        match self.setting("name") {
            Value::String(s) if !s.is_empty() => s,
            _ => self.inner.folder.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
        }
    }

    pub fn is_demo(&self) -> bool {
        self.inner.id == config::DEMO_ID
    }

    pub fn read_only(&self) -> bool {
        self.setting("read_only").as_bool().unwrap_or(false)
    }

    pub fn require_writable(&self) -> Result<()> {
        if self.read_only() {
            Err(Error::ReadOnly)
        } else {
            Ok(())
        }
    }

    /// id, name, folder, read_only, created and last_opened from the registry.
    pub fn info(&self) -> ProjectInfo {
        let mut entry = self.inner.paths.resolve(&self.inner.id).unwrap_or(ProjectInfo {
            id: self.inner.id.clone(),
            name: self.name(),
            folder: self.inner.folder.to_string_lossy().into_owned(),
            created: None,
            last_opened: None,
            demo: self.is_demo(),
            read_only: None,
        });
        entry.name = self.name();
        entry.read_only = Some(self.read_only());
        entry
    }

    pub fn set_read_only(&self, on: bool) -> Result<()> {
        let mut body = serde_json::Map::new();
        body.insert("read_only".into(), Value::Bool(on));
        self.update_settings(&body, model::Origin::Ui).map(|_| ())
    }

    pub fn change_seq(&self) -> i64 {
        self.inner.db.change_seq()
    }

    /// Fires with the sequence number after every change row this process writes.
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<i64> {
        self.inner.changes.subscribe()
    }

    /// Resolves a relative path inside the folder; a path that escapes it is refused.
    pub fn abspath(&self, relpath: &str) -> Result<PathBuf> {
        let path = util::normalize(&self.inner.folder.join(relpath));
        let root = util::norm_key(&self.inner.folder);
        let key = util::norm_key(&path);
        if key != root && !key.starts_with(&(root.clone() + "\\")) {
            return Err(Error::bad("The path points outside the project folder."));
        }
        Ok(path)
    }

    pub fn relpath(&self, path: &Path) -> String {
        util::relpath(&self.inner.folder, path)
    }

    /// Holds the sidecar back during a batch of writes (the demo builder uses it).
    pub fn set_defer_sidecar(&self, on: bool) {
        self.inner.defer_sidecar.store(on, Ordering::SeqCst);
    }

    pub(crate) fn sidecar_deferred(&self) -> bool {
        self.inner.defer_sidecar.load(Ordering::SeqCst)
    }

    /// Replaces the cloud-only test (tests simulate a OneDrive placeholder with it).
    #[doc(hidden)]
    pub fn set_cloud_check(&self, check: Option<CloudCheck>) {
        *self.inner.cloud_check.write().unwrap_or_else(|e| e.into_inner()) = check;
    }

    pub(crate) fn is_cloud_only(&self, path: &Path, meta: &Metadata) -> bool {
        let guard = self.inner.cloud_check.read().unwrap_or_else(|e| e.into_inner());
        match guard.as_ref() {
            Some(f) => f(path, meta),
            None => scanner::is_cloud_only(meta),
        }
    }

    pub(crate) fn scan_guard(&self) -> std::sync::MutexGuard<'_, ()> {
        self.inner.scan_lock.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Runs one scan in the calling thread. Scans of one project run one at a time.
    pub fn scan(&self, mode: scanner::ScanMode, progress: impl FnMut(scanner::Progress)) -> Result<scanner::ScanReport> {
        scanner::scan_project(self, mode, progress, model::Origin::Scan)
    }

    pub fn scan_as(&self, mode: scanner::ScanMode, origin: model::Origin, progress: impl FnMut(scanner::Progress)) -> Result<scanner::ScanReport> {
        scanner::scan_project(self, mode, progress, origin)
    }

    /// Watches the folder; touched paths are rescanned after a 2 s debounce.
    pub fn start_watcher(&self) -> Result<scanner::WatcherHandle> {
        scanner::start_watcher(self, std::time::Duration::from_secs(2))
    }
}

#[cfg(test)]
mod tests {
    use super::Project;

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn project_is_send_and_sync() {
        // The server keeps a Project behind a RwLock and hands clones to request threads.
        assert_send_sync::<Project>();
    }
}
