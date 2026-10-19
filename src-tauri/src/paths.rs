//! App-data root detection, mirroring `kurasi_core::AppPaths::detect` so the shell can log
//! the root and open its log file before the core library loads.
//!
//! Order (CONTRACT_RUST.md section 6 and CORE_API.md):
//! 1. environment variable `KURASI_HOME`;
//! 2. `portable.txt` next to the executable: `<exe dir>\data\`;
//! 3. `%LOCALAPPDATA%\Kurasi`.

use std::path::{Path, PathBuf};

pub const PORTABLE_MARKER: &str = "portable.txt";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootSource {
    Env,
    Portable,
    LocalAppData,
}

#[derive(Debug, Clone)]
pub struct ShellPaths {
    pub root: PathBuf,
    pub source: RootSource,
    pub exe_dir: PathBuf,
}

impl ShellPaths {
    pub fn detect() -> ShellPaths {
        let exe_dir = exe_dir();
        let (root, source) = detect_root(&exe_dir, std::env::var_os("KURASI_HOME"), local_app_data());
        ShellPaths { root, source, exe_dir }
    }

    pub fn is_portable(&self) -> bool {
        self.source == RootSource::Portable
    }

    pub fn log_file(&self) -> PathBuf {
        self.root.join("logs").join("kurasi.log")
    }

    /// Folder with the bundled `ui/` copy (installed next to the executable) or, during
    /// development, the repository's `ui/` folder.
    pub fn ui_dir(&self) -> Option<PathBuf> {
        let installed = self.exe_dir.join("ui");
        if installed.join("index.html").is_file() {
            return Some(installed);
        }
        let dev = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("ui");
        if dev.join("index.html").is_file() {
            return Some(dev);
        }
        None
    }
}

pub fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn local_app_data() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
}

/// Pure decision, separated for the unit tests.
pub fn detect_root(
    exe_dir: &Path,
    env_home: Option<std::ffi::OsString>,
    local_app_data: Option<PathBuf>,
) -> (PathBuf, RootSource) {
    if let Some(home) = env_home.filter(|h| !h.is_empty()) {
        return (PathBuf::from(home), RootSource::Env);
    }
    if exe_dir.join(PORTABLE_MARKER).is_file() {
        return (exe_dir.join("data"), RootSource::Portable);
    }
    let base = local_app_data.unwrap_or_else(|| exe_dir.to_path_buf());
    (base.join("Kurasi"), RootSource::LocalAppData)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_wins() {
        let dir = std::env::temp_dir().join("kurasi-paths-env");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join(PORTABLE_MARKER), "x").unwrap();
        let (root, src) = detect_root(&dir, Some("C:\\somewhere".into()), Some(PathBuf::from("C:\\lad")));
        assert_eq!(src, RootSource::Env);
        assert_eq!(root, PathBuf::from("C:\\somewhere"));
    }

    #[test]
    fn portable_marker_puts_data_beside_exe() {
        let dir = std::env::temp_dir().join("kurasi-paths-portable");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join(PORTABLE_MARKER), "x").unwrap();
        let (root, src) = detect_root(&dir, None, Some(PathBuf::from("C:\\lad")));
        assert_eq!(src, RootSource::Portable);
        assert_eq!(root, dir.join("data"));
    }

    #[test]
    fn default_is_local_app_data() {
        let dir = std::env::temp_dir().join("kurasi-paths-plain");
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::remove_file(dir.join(PORTABLE_MARKER));
        let (root, src) = detect_root(&dir, None, Some(PathBuf::from("C:\\lad")));
        assert_eq!(src, RootSource::LocalAppData);
        assert_eq!(root, PathBuf::from("C:\\lad\\Kurasi"));
    }
}
