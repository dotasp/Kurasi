//! App data paths, defaults and the projects registry (`projects.json`).
//!
//! The root is chosen in this order: the environment variable `KURASI_HOME`, then
//! `<exe folder>\data` when `portable.txt` sits next to the executable, then
//! `%LOCALAPPDATA%\Kurasi`.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::error::{Error, Result};
use crate::lock::{atomic_write_text, WriteLock};
use crate::util::{absolute, new_id, norm_key, now_iso};

pub const DEFAULT_PORT: u16 = 7343;
pub const PAGE_CACHE_LIMIT: u64 = 500 * 1024 * 1024;
pub const THUMB_WIDTH: u32 = 120;
pub const DEMO_ID: &str = "demo";

/// The default settings of a project (CONTRACT.md section 3).
pub fn default_settings() -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("name".into(), json!(""));
    m.insert("folder".into(), json!(""));
    m.insert("file_types".into(), json!([".pdf", ".md", ".png", ".jpg", ".jpeg"]));
    m.insert("exclude".into(), json!([".git", ".kurasi", ".venv", "node_modules", "__pycache__", "~$*"]));
    m.insert("auto_scan_min".into(), json!(15));
    m.insert("read_only".into(), json!(false));
    m.insert("ai_authors".into(), json!(["claude"]));
    m.insert("ai_author".into(), json!("Claude"));
    m.insert("hl_colour".into(), json!("#ffd84a"));
    m.insert("loose".into(), json!(true));
    m.insert("last_scan".into(), Value::Null);
    m.insert("mcp_hints".into(), json!(true));
    m.insert("theme".into(), json!("system"));
    m
}

/// One entry of `projects.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProjectInfo {
    pub id: String,
    pub name: String,
    pub folder: String,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub last_opened: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub demo: bool,
    /// Filled by `Project::info`; absent in the registry file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only: Option<bool>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub root: PathBuf,
}

impl AppPaths {
    /// Honours `KURASI_HOME` first, then `portable.txt` next to the exe, then `LOCALAPPDATA`.
    pub fn detect() -> AppPaths {
        if let Some(home) = std::env::var_os("KURASI_HOME").filter(|v| !v.is_empty()) {
            return AppPaths::at(PathBuf::from(home));
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                if dir.join("portable.txt").is_file() {
                    return AppPaths::at(dir.join("data"));
                }
            }
        }
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("AppData").join("Local")))
            .unwrap_or_else(|| PathBuf::from("."));
        AppPaths::at(base.join("Kurasi"))
    }

    /// An explicit root (tests point it at a temporary folder).
    pub fn at(root: impl Into<PathBuf>) -> AppPaths {
        let root = root.into();
        let _ = fs::create_dir_all(&root);
        AppPaths { root }
    }

    pub fn projects_file(&self) -> PathBuf {
        self.root.join("projects.json")
    }
    pub fn project_dir(&self, id: &str) -> PathBuf {
        let p = self.root.join("projects").join(id);
        let _ = fs::create_dir_all(&p);
        p
    }
    pub fn thumbs_dir(&self) -> PathBuf {
        let p = self.root.join("thumbs");
        let _ = fs::create_dir_all(&p);
        p
    }
    pub fn pages_dir(&self) -> PathBuf {
        let p = self.root.join("pages");
        let _ = fs::create_dir_all(&p);
        p
    }
    pub fn demo_dir(&self) -> PathBuf {
        self.root.join("demo")
    }
    pub fn logs_dir(&self) -> PathBuf {
        let p = self.root.join("logs");
        let _ = fs::create_dir_all(&p);
        p
    }

    pub(crate) fn registry_lock(&self) -> WriteLock {
        WriteLock::new(self.root.join("projects.lock"))
    }

    /// Every registered project, in file order. A broken file reads as empty.
    pub fn projects(&self) -> Result<Vec<ProjectInfo>> {
        Ok(load_projects(&self.projects_file()))
    }

    pub(crate) fn save(&self, entries: &[ProjectInfo]) -> Result<()> {
        let text = serde_json::to_string_pretty(entries)? + "\n";
        atomic_write_text(&self.projects_file(), &text)
    }

    /// The id of the project opened last, the demo excluded, whose folder exists.
    pub fn last_opened(&self) -> Option<String> {
        choose_last(&load_projects(&self.projects_file()), |p| Path::new(p).is_dir()).0.map(|e| e.id)
    }

    pub fn set_last_opened(&self, id: &str) -> Result<()> {
        let lock = self.registry_lock();
        let _g = lock.acquire()?;
        let mut entries = load_projects(&self.projects_file());
        let mut hit = false;
        for e in entries.iter_mut() {
            if e.id == id {
                e.last_opened = Some(now_iso());
                hit = true;
            }
        }
        if !hit {
            return Err(Error::not_found(format!("The project {} is not registered.", id)));
        }
        self.save(&entries)
    }

    /// A registry entry by id or by folder.
    pub fn resolve(&self, id_or_folder: &str) -> Option<ProjectInfo> {
        find_project(id_or_folder, &load_projects(&self.projects_file()))
    }

    /// Adds a project, or returns the entry that already has this folder (renaming it when a
    /// name is given). `pid` fixes the id (the demo uses `demo`).
    pub fn register(&self, name: Option<&str>, folder: &Path, pid: Option<&str>, demo: bool) -> Result<ProjectInfo> {
        let folder = absolute(folder);
        let lock = self.registry_lock();
        let _g = lock.acquire()?;
        let mut entries = load_projects(&self.projects_file());
        let folder_str = folder.to_string_lossy().into_owned();
        if let Some(pos) = entries.iter().position(|e| norm_key(Path::new(&e.folder)) == norm_key(&folder)) {
            if let Some(n) = name.filter(|n| !n.is_empty()) {
                if entries[pos].name != n {
                    entries[pos].name = n.to_string();
                    self.save(&entries)?;
                }
            }
            return Ok(entries[pos].clone());
        }
        let base = folder.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let entry = ProjectInfo {
            id: pid.map(str::to_string).unwrap_or_else(|| new_id("p")),
            name: name.filter(|n| !n.is_empty()).map(str::to_string).unwrap_or(base),
            folder: folder_str,
            created: Some(now_iso()),
            last_opened: None,
            demo,
            read_only: None,
        };
        entries.push(entry.clone());
        self.save(&entries)?;
        Ok(entry)
    }

    /// Removes an entry from the registry. With `with_data` the data goes through
    /// `remove_project` (and its safety checks) and its trash entry is purged at once.
    pub fn unregister(&self, id: &str, with_data: bool) -> Result<()> {
        if with_data {
            let removal = self.remove_project(id, true)?;
            let trash = self.removed_dir().join(&removal.removal_id);
            if trash.join("removed.json").is_file() {
                fs::remove_dir_all(&trash)?;
            }
            return Ok(());
        }
        let lock = self.registry_lock();
        let _g = lock.acquire()?;
        let mut entries = load_projects(&self.projects_file());
        entries.retain(|e| e.id != id);
        self.save(&entries)
    }

    /// The registration line shown in Tools, MCP server: user scope, write mode.
    pub fn mcp_command(&self, folder: &Path) -> String {
        let exe = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| "kurasi.exe".into());
        mcp_command_for(&exe, &folder.display().to_string())
    }
}

/// `claude mcp add kurasi -s user -- "<exe>" mcp --project "<folder>" --mode write`
pub fn mcp_command_for(exe: &str, folder: &str) -> String {
    format!("claude mcp add kurasi -s user -- \"{}\" mcp --project \"{}\" --mode write", exe, folder)
}

pub fn load_projects(path: &Path) -> Vec<ProjectInfo> {
    let text = match fs::read(path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).trim_start_matches('\u{feff}').to_string(),
        Err(_) => return Vec::new(),
    };
    let value: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    if let Value::Array(items) = value {
        for item in items {
            if let Ok(entry) = serde_json::from_value::<ProjectInfo>(item) {
                if !entry.id.is_empty() && !entry.folder.is_empty() {
                    out.push(entry);
                }
            }
        }
    }
    out
}

pub fn find_project(key: &str, entries: &[ProjectInfo]) -> Option<ProjectInfo> {
    if let Some(e) = entries.iter().find(|e| e.id == key) {
        return Some(e.clone());
    }
    let norm = norm_key(Path::new(key));
    entries.iter().find(|e| norm_key(Path::new(&e.folder)) == norm).cloned()
}

/// Startup choice: the entry with the latest `last_opened` whose folder exists. The second
/// value is the most recent project when no folder can be reached. The demo is never chosen.
pub fn choose_last(entries: &[ProjectInfo], exists: impl Fn(&str) -> bool) -> (Option<ProjectInfo>, Option<ProjectInfo>) {
    let mut ordered: Vec<&ProjectInfo> = entries.iter().filter(|e| !e.demo && e.id != DEMO_ID).collect();
    ordered.sort_by(|a, b| b.last_opened.clone().unwrap_or_default().cmp(&a.last_opened.clone().unwrap_or_default()));
    for e in &ordered {
        if exists(&e.folder) {
            return (Some((*e).clone()), None);
        }
    }
    (None, ordered.first().map(|e| (*e).clone()))
}
