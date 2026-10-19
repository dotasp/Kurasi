//! Removing a project from `projects.json` (CONTRACT.md sections 2 and 6, CORE_API.md).
//!
//! Removal takes the entry out of the registry. With `with_data` it also moves Kurasi's data for
//! the project into `<root>\trash\projects\<removal id>\`:
//!
//! - `app\`: the folder `<root>\projects\<id>\` (the database and `write.lock`), moved whole;
//! - `sidecar\`: the files Kurasi wrote into `<folder>\.kurasi\` (`metadata.json`,
//!   `AI_USE_LOG.md`, its own temporary files). `.kurasi` itself goes only when nothing else is
//!   left in it. A read-only project keeps its `.kurasi`, because Kurasi writes nothing into
//!   its folder.
//!
//! Every removal writes `removed.json` into its trash folder before anything moves, so
//! `restore_project` can bring the entry back, and the data when it was moved. The server purges
//! trash entries older than [`TRASH_DAYS`] when it starts. The data moves into the trash and is
//! deleted only by the purge: an Undo within the toast of the window (and a crash in between)
//! never loses it.
//!
//! Every move passes [`check_target`] first. Nothing here deletes a folder that still holds a
//! file: folders are removed with `remove_dir`, which only takes an empty one, and the purge
//! only deletes folders inside the trash that carry a removal manifest.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{load_projects, AppPaths, ProjectInfo, DEMO_ID};
use crate::error::{Error, Result};
use crate::lock::atomic_write_text;
use crate::util::{absolute, new_id, norm_key, now_iso};

/// Days a removed project's data stays in the trash before the purge deletes it.
pub const TRASH_DAYS: u64 = 7;

const MANIFEST: &str = "removed.json";
const SQLITE: &str = "kurasi.sqlite";
/// Entries Kurasi creates in `<root>\projects\<id>\`.
const APP_FILES: &[&str] = &["kurasi.sqlite", "kurasi.sqlite-wal", "kurasi.sqlite-shm", "kurasi.sqlite-journal", "write.lock"];
/// Entries Kurasi creates in `<folder>\.kurasi\`.
const SIDECAR_FILES: &[&str] = &["metadata.json", "AI_USE_LOG.md"];
/// Tables every Kurasi database has (CONTRACT.md section 3).
const TABLES: &[&str] = &[
    "setting", "category", "reference", "reference_author", "reference_category", "file", "highlight", "trash", "ai_log", "change",
];
const REPARSE_POINT: u32 = 0x400;
const MOVE_TRIES: u32 = 6;
const MOVE_DELAY_MS: u64 = 100;

/// One folder "Also delete Kurasi's data" would remove: its path and the entries in it that
/// go. `whole` is true when the folder itself goes as well.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DataDir {
    pub path: String,
    pub files: Vec<String>,
    pub whole: bool,
}

/// The app-data folder and the sidecar folder of one project. A folder that is absent, fails a
/// safety check, or (sidecar) belongs to a read-only project is `None`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DataTargets {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<DataDir>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sidecar: Option<DataDir>,
}

impl DataTargets {
    pub fn is_empty(&self) -> bool {
        self.app.is_none() && self.sidecar.is_none()
    }
}

/// What `remove_project` did. `moved` lists the original paths now in the trash.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Removal {
    pub removal_id: String,
    pub entry: ProjectInfo,
    pub with_data: bool,
    pub moved: Vec<String>,
}

/// The two kinds of folder the removal may move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    AppData,
    Sidecar,
}

/// `removed.json`: what one removal took away and where it went.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Manifest {
    /// Marker: a trash folder without it is never restored or purged.
    kurasi_removal: u32,
    removal_id: String,
    at: String,
    at_secs: u64,
    entry: ProjectInfo,
    /// Position in `projects.json`, so a restore puts the entry back where it was.
    index: usize,
    with_data: bool,
    /// The original app-data folder, now `app\` in the trash folder.
    #[serde(default)]
    app: Option<String>,
    /// The original `.kurasi` folder and the files now in `sidecar\`.
    #[serde(default)]
    sidecar: Option<SidecarMove>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SidecarMove {
    dir: String,
    files: Vec<String>,
    /// `.kurasi` was left empty and removed.
    dir_removed: bool,
}

// ------------------------------------------------------------------ safety checks

fn refused(target: &Path, why: &str) -> Error {
    Error::conflict(format!("Kurasi refuses to delete {}: {}.", target.display(), why))
}

fn is_link(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & REPARSE_POINT != 0 {
            return true;
        }
    }
    let _ = REPARSE_POINT;
    false
}

/// `p-` followed by 12 lowercase hex characters, the id `register` gives (the sample is `demo`).
fn valid_project_id(id: &str) -> bool {
    id.len() == 14 && id.starts_with("p-") && id[2..].bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn valid_removal_id(id: &str) -> bool {
    id.len() == 15 && id.starts_with("rm-") && id[3..].bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn is_kurasi_temp(name: &str) -> bool {
    name.starts_with(".~kurasi-") && name.ends_with(".tmp")
}

/// `inner` lies at `outer` or below it.
fn within(inner: &Path, outer: &Path) -> bool {
    let i = norm_key(inner);
    let o = norm_key(outer);
    i == o || i.starts_with(&(o + "\\"))
}

/// What the read-only look into a Kurasi database found.
struct DbFacts {
    read_only: bool,
}

/// Opens `kurasi.sqlite` read-only and checks the SQLite header and the Kurasi tables.
fn inspect_db(path: &Path) -> Result<DbFacts> {
    let mut head = [0u8; 16];
    {
        use std::io::Read;
        let mut f = fs::File::open(path)?;
        f.read_exact(&mut head).map_err(|_| refused(path, "it is not a SQLite database"))?;
    }
    if &head != b"SQLite format 3\0" {
        return Err(refused(path, "it is not a SQLite database"));
    }
    let not_kurasi = |_| refused(path, "it is not a Kurasi database");
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(not_kurasi)?;
    conn.busy_timeout(Duration::from_secs(5)).map_err(not_kurasi)?;
    let names: Vec<String> = {
        let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type='table'").map_err(not_kurasi)?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(not_kurasi)?;
        rows.filter_map(|r| r.ok()).collect()
    };
    if let Some(missing) = TABLES.iter().find(|t| !names.iter().any(|n| n == *t)) {
        return Err(refused(path, &format!("the database has no {} table, so Kurasi did not create it", missing)));
    }
    let read_only = conn
        .query_row("SELECT value FROM setting WHERE key='read_only'", [], |r| r.get::<_, Option<String>>(0))
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_str::<Value>(&v).ok())
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    Ok(DbFacts { read_only })
}

fn verify_app_dir(dir: &Path) -> Result<DbFacts> {
    let meta = fs::symlink_metadata(dir).map_err(|_| refused(dir, "the folder does not exist"))?;
    if is_link(&meta) || !meta.is_dir() {
        return Err(refused(dir, "it is a link or a file"));
    }
    for e in fs::read_dir(dir)? {
        let e = e?;
        let name = e.file_name().to_string_lossy().into_owned();
        let m = fs::symlink_metadata(e.path())?;
        if !APP_FILES.contains(&name.as_str()) && !is_kurasi_temp(&name) {
            return Err(refused(dir, &format!("it holds {}, which Kurasi did not create", name)));
        }
        if is_link(&m) || !m.is_file() {
            return Err(refused(dir, &format!("{} is a link or a folder", name)));
        }
    }
    let db = dir.join(SQLITE);
    if !db.is_file() {
        return Err(refused(dir, "it holds no Kurasi database"));
    }
    inspect_db(&db)
}

fn verify_sidecar(dir: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(dir).map_err(|_| refused(dir, "the folder does not exist"))?;
    if is_link(&meta) || !meta.is_dir() {
        return Err(refused(dir, "it is a link or a file"));
    }
    let md = dir.join("metadata.json");
    let ok = fs::symlink_metadata(&md).map(|m| m.is_file() && !is_link(&m)).unwrap_or(false);
    if !ok || crate::sidecar::read(&md).is_none() {
        return Err(refused(dir, "it holds no Kurasi metadata.json"));
    }
    Ok(())
}

/// The safety gate of every move: `target` must be `<root>\projects\<entry.id>` holding a
/// Kurasi database (SQLite header, Kurasi tables) and nothing else Kurasi did not create, or
/// `<entry.folder>\.kurasi` holding a valid `metadata.json`. Drive roots, the project folder, any
/// folder that contains the project folder or the app root, links and junctions are refused.
pub fn check_target(paths: &AppPaths, entry: &ProjectInfo, target: &Path) -> Result<TargetKind> {
    let t = absolute(target);
    let normal = t.components().filter(|c| matches!(c, Component::Normal(_))).count();
    if normal < 2 {
        return Err(refused(&t, "it is a drive root or a folder directly below one"));
    }
    let folder = absolute(Path::new(&entry.folder));
    if within(&folder, &t) {
        return Err(refused(&t, "it is the project folder or a folder that holds it"));
    }
    let root = absolute(&paths.root);
    if within(&root, &t) {
        return Err(refused(&t, "it is Kurasi's data folder or a folder that holds it"));
    }
    if entry.demo || entry.id == DEMO_ID {
        return Err(refused(&t, "it belongs to the sample project"));
    }
    let key = norm_key(&t);
    if valid_project_id(&entry.id) && key == norm_key(&root.join("projects").join(&entry.id)) {
        verify_app_dir(&t)?;
        return Ok(TargetKind::AppData);
    }
    if key == norm_key(&folder.join(".kurasi")) {
        verify_sidecar(&t)?;
        return Ok(TargetKind::Sidecar);
    }
    Err(refused(&t, "it lies outside Kurasi's data for this project"))
}

// ------------------------------------------------------------------ moves

fn retry<T>(mut f: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    let mut delay = Duration::from_millis(MOVE_DELAY_MS);
    let mut last = None;
    for attempt in 0..MOVE_TRIES {
        match f() {
            Ok(v) => return Ok(v),
            Err(e) => {
                last = Some(e);
                if attempt + 1 < MOVE_TRIES {
                    std::thread::sleep(delay);
                    delay *= 2;
                }
            }
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("move failed")))
}

/// Moves one file: a rename, or a copy and delete when the two paths lie on different drives.
fn move_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    match retry(|| fs::rename(src, dst)) {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(17) => {
            // ERROR_NOT_SAME_DEVICE: copy, check the size, then delete the source.
            let n = fs::copy(src, dst)?;
            if n != fs::metadata(src)?.len() {
                let _ = fs::remove_file(dst);
                return Err(std::io::Error::other("the copy is incomplete"));
            }
            retry(|| fs::remove_file(src))
        }
        Err(e) => Err(e),
    }
}

fn busy(entry: &ProjectInfo, e: &std::io::Error) -> Error {
    Error::conflict(format!(
        "Kurasi's data for {} is in use by another program, for example a Claude Code session with the Kurasi MCP server ({}). Close it, then try again. Nothing was changed.",
        entry.name, e
    ))
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn read_manifest(dir: &Path) -> Option<Manifest> {
    let text = fs::read_to_string(dir.join(MANIFEST)).ok()?;
    let m: Manifest = serde_json::from_str(&text).ok()?;
    (m.kurasi_removal == 1).then_some(m)
}

fn write_manifest(dir: &Path, m: &Manifest) -> Result<()> {
    atomic_write_text(&dir.join(MANIFEST), &(serde_json::to_string_pretty(m)? + "\n"))
}

impl AppPaths {
    /// `<root>\trash\projects`, where removed projects wait for the purge.
    pub fn removed_dir(&self) -> PathBuf {
        self.root.join("trash").join("projects")
    }

    /// The folders "Also delete Kurasi's data" would remove for `entry`, after the safety checks.
    pub fn data_targets(&self, entry: &ProjectInfo) -> DataTargets {
        let mut out = DataTargets::default();
        if entry.demo || entry.id == DEMO_ID {
            return out;
        }
        let app_dir = self.root.join("projects").join(&entry.id);
        // A database that cannot be read counts as read-only: the folder then stays untouched.
        let mut read_only = true;
        if app_dir.exists() {
            if let Ok(TargetKind::AppData) = check_target(self, entry, &app_dir) {
                read_only = inspect_db(&app_dir.join(SQLITE)).map(|f| f.read_only).unwrap_or(true);
                let mut files: Vec<String> = fs::read_dir(&app_dir)
                    .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
                    .unwrap_or_default();
                files.sort();
                out.app = Some(DataDir { path: app_dir.to_string_lossy().into_owned(), files, whole: true });
            }
        }
        let side = absolute(Path::new(&entry.folder)).join(".kurasi");
        if !read_only && side.is_dir() {
            if let Ok(TargetKind::Sidecar) = check_target(self, entry, &side) {
                let mut files = Vec::new();
                let mut others = 0;
                for e in fs::read_dir(&side).into_iter().flatten().flatten() {
                    let name = e.file_name().to_string_lossy().into_owned();
                    let plain = fs::symlink_metadata(e.path()).map(|m| m.is_file() && !is_link(&m)).unwrap_or(false);
                    if plain && (SIDECAR_FILES.contains(&name.as_str()) || is_kurasi_temp(&name)) {
                        files.push(name);
                    } else {
                        others += 1;
                    }
                }
                files.sort();
                out.sidecar = Some(DataDir { path: side.to_string_lossy().into_owned(), files, whole: others == 0 });
            }
        }
        out
    }

    /// Takes a project off the list; with `with_data` its data goes to the trash as well.
    /// 400 for the sample project, 404 for an unknown id, 409 when a data file is held by another
    /// program (nothing changes then).
    pub fn remove_project(&self, id: &str, with_data: bool) -> Result<Removal> {
        let lock = self.registry_lock();
        let _g = lock.acquire()?;
        let mut entries = load_projects(&self.projects_file());
        let index = entries
            .iter()
            .position(|e| e.id == id)
            .ok_or_else(|| Error::not_found(format!("The project {} is not in the list.", id)))?;
        let entry = entries[index].clone();
        if entry.demo || entry.id == DEMO_ID {
            return Err(Error::bad("The sample project stays in the list. Reset rebuilds it from scratch."));
        }
        let targets = if with_data { self.data_targets(&entry) } else { DataTargets::default() };
        let removal_id = new_id("rm");
        let trash = self.removed_dir().join(&removal_id);
        fs::create_dir_all(&trash)?;
        let mut manifest = Manifest {
            kurasi_removal: 1,
            removal_id: removal_id.clone(),
            at: now_iso(),
            at_secs: unix_now(),
            entry: entry.clone(),
            index,
            with_data,
            app: targets.app.as_ref().map(|d| d.path.clone()),
            sidecar: targets.sidecar.as_ref().map(|d| SidecarMove { dir: d.path.clone(), files: d.files.clone(), dir_removed: false }),
        };
        // The plan is on disk before anything moves.
        if let Err(e) = write_manifest(&trash, &manifest) {
            let _ = fs::remove_dir(&trash);
            return Err(e);
        }
        let undo_all = |moved_app: bool, moved_files: &[String]| {
            if let Some(side) = &targets.sidecar {
                for f in moved_files {
                    let _ = move_file(&trash.join("sidecar").join(f), &Path::new(&side.path).join(f));
                }
                let _ = fs::remove_dir(trash.join("sidecar"));
            }
            if moved_app {
                if let Some(app) = &targets.app {
                    let _ = retry(|| fs::rename(trash.join("app"), &app.path));
                }
            }
            let _ = fs::remove_file(trash.join(MANIFEST));
            let _ = fs::remove_dir(&trash);
        };
        let mut moved = Vec::new();
        let mut moved_app = false;
        if let Some(app) = &targets.app {
            let src = PathBuf::from(&app.path);
            if let Err(e) = check_target(self, &entry, &src).and_then(|k| {
                if k == TargetKind::AppData {
                    Ok(())
                } else {
                    Err(refused(&src, "it is the wrong kind of folder"))
                }
            }) {
                undo_all(false, &[]);
                return Err(e);
            }
            if let Err(e) = retry(|| fs::rename(&src, trash.join("app"))) {
                undo_all(false, &[]);
                return Err(busy(&entry, &e));
            }
            moved_app = true;
            moved.push(app.path.clone());
        }
        if let Some(side) = &targets.sidecar {
            let dir = PathBuf::from(&side.path);
            if let Err(e) = check_target(self, &entry, &dir).and_then(|k| {
                if k == TargetKind::Sidecar {
                    Ok(())
                } else {
                    Err(refused(&dir, "it is the wrong kind of folder"))
                }
            }) {
                undo_all(moved_app, &[]);
                return Err(e);
            }
            let mut done: Vec<String> = Vec::new();
            for f in &side.files {
                if let Err(e) = move_file(&dir.join(f), &trash.join("sidecar").join(f)) {
                    undo_all(moved_app, &done);
                    return Err(busy(&entry, &e));
                }
                done.push(f.clone());
                moved.push(dir.join(f).to_string_lossy().into_owned());
            }
            // Only an empty folder goes; anything left in it stays where it is.
            let removed = side.whole && fs::remove_dir(&dir).is_ok();
            if let Some(s) = manifest.sidecar.as_mut() {
                s.dir_removed = removed;
            }
        }
        if let Err(e) = write_manifest(&trash, &manifest).and_then(|_| {
            entries.remove(index);
            self.save(&entries)
        }) {
            if let Some(side) = &targets.sidecar {
                let _ = fs::create_dir_all(&side.path);
            }
            undo_all(moved_app, targets.sidecar.as_ref().map(|s| s.files.as_slice()).unwrap_or(&[]));
            return Err(e);
        }
        Ok(Removal { removal_id, entry, with_data, moved })
    }

    /// Puts a removed project back in the list, with its data when the removal moved it.
    /// 404 for an unknown removal id, 409 when the id, the folder or a data path is taken again
    /// (nothing changes then).
    pub fn restore_project(&self, removal_id: &str) -> Result<ProjectInfo> {
        let gone = || Error::not_found("That removal is no longer in Kurasi's trash, so it cannot be undone.");
        if !valid_removal_id(removal_id) {
            return Err(gone());
        }
        let trash = self.removed_dir().join(removal_id);
        let m = read_manifest(&trash).ok_or_else(gone)?;
        if m.removal_id != removal_id {
            return Err(gone());
        }
        let lock = self.registry_lock();
        let _g = lock.acquire()?;
        let mut entries = load_projects(&self.projects_file());
        let folder_key = norm_key(Path::new(&m.entry.folder));
        if entries.iter().any(|e| e.id == m.entry.id || norm_key(Path::new(&e.folder)) == folder_key) {
            return Err(Error::conflict(format!(
                "The folder {} is in the project list again, so the removed entry stays removed.",
                m.entry.folder
            )));
        }
        // Destinations come from the entry, never from paths stored in the trash.
        let app_dst = self.root.join("projects").join(&m.entry.id);
        let side_dst = absolute(Path::new(&m.entry.folder)).join(".kurasi");
        let app_src = trash.join("app");
        let side_src = trash.join("sidecar");
        if m.app.is_some() {
            if !valid_project_id(&m.entry.id) || norm_key(Path::new(m.app.as_deref().unwrap_or(""))) != norm_key(&app_dst) {
                return Err(gone());
            }
            if app_dst.exists() && fs::remove_dir(&app_dst).is_err() {
                return Err(Error::conflict(format!("{} exists again, so the data stays in the trash.", app_dst.display())));
            }
        }
        if let Some(s) = &m.sidecar {
            if norm_key(Path::new(&s.dir)) != norm_key(&side_dst) {
                return Err(gone());
            }
            if !Path::new(&m.entry.folder).is_dir() {
                return Err(Error::conflict(format!(
                    "The folder {} is unreachable, so the project stays removed. Reconnect the drive, then undo again.",
                    m.entry.folder
                )));
            }
            if let Some(f) = s.files.iter().find(|f| side_dst.join(f).exists()) {
                return Err(Error::conflict(format!("{} exists again, so the data stays in the trash.", side_dst.join(f).display())));
            }
        }
        if m.app.is_some() && app_src.is_dir() {
            fs::create_dir_all(self.root.join("projects"))?;
            retry(|| fs::rename(&app_src, &app_dst)).map_err(|e| busy(&m.entry, &e))?;
        }
        if let Some(s) = &m.sidecar {
            fs::create_dir_all(&side_dst)?;
            for f in &s.files {
                let src = side_src.join(f);
                if src.is_file() {
                    move_file(&src, &side_dst.join(f)).map_err(|e| busy(&m.entry, &e))?;
                }
            }
            let _ = fs::remove_dir(&side_src);
        }
        let at = m.index.min(entries.len());
        entries.insert(at, m.entry.clone());
        self.save(&entries)?;
        let _ = fs::remove_file(trash.join(MANIFEST));
        let _ = fs::remove_dir(&trash);
        Ok(m.entry)
    }

    /// Deletes the trash entries of removals older than `older_than`. Returns how many went.
    pub fn purge_removed(&self, older_than: Duration) -> Result<usize> {
        let dir = self.removed_dir();
        let Ok(rd) = fs::read_dir(&dir) else {
            return Ok(0);
        };
        let now = unix_now();
        let mut n = 0;
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = e.path();
            let plain_dir = fs::symlink_metadata(&path).map(|m| m.is_dir() && !is_link(&m)).unwrap_or(false);
            if !valid_removal_id(&name) || !plain_dir {
                continue;
            }
            let Some(m) = read_manifest(&path) else {
                continue;
            };
            if m.removal_id != name || now.saturating_sub(m.at_secs) < older_than.as_secs() {
                continue;
            }
            match fs::remove_dir_all(&path) {
                Ok(()) => n += 1,
                Err(err) => log::warn!("the trash entry {} could not be purged: {}", path.display(), err),
            }
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids() {
        assert!(valid_project_id("p-0123456789ab"));
        assert!(!valid_project_id("p-0123456789AB"));
        assert!(!valid_project_id("demo"));
        assert!(!valid_project_id("p-..\\..\\x0000"));
        assert!(valid_removal_id("rm-0123456789ab"));
        assert!(!valid_removal_id("rm-0123456789a/"));
        assert!(is_kurasi_temp(".~kurasi-metadata.json.tmp"));
        assert!(!is_kurasi_temp("notes.tmp"));
    }

    #[test]
    fn within_compares_whole_components() {
        assert!(within(Path::new("C:\\a\\b"), Path::new("C:\\a")));
        assert!(within(Path::new("C:\\a"), Path::new("C:\\A")));
        assert!(!within(Path::new("C:\\ab"), Path::new("C:\\a")));
    }
}
