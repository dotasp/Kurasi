//! Locates and binds pdfium.dll once per process.
//!
//! Search order: the exe folder, `resources\pdfium.dll` next to the exe, the env var
//! `KURASI_PDFIUM` (a file or a folder), then the repository path used by the tests.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use pdfium_render::prelude::Pdfium as Lib;

use crate::error::{Error, Result};

static LIB: OnceLock<std::result::Result<Lib, Error>> = OnceLock::new();

/// Handle to the process-wide pdfium library.
pub struct Pdfium;

impl Pdfium {
    /// Candidate locations of pdfium.dll in search order. Only existing files are returned.
    pub fn candidates() -> Vec<PathBuf> {
        let name = Lib::pdfium_platform_library_name();
        let mut out = Vec::new();
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                out.push(dir.join(&name));
                out.push(dir.join("resources").join(&name));
            }
        }
        if let Some(env) = std::env::var_os("KURASI_PDFIUM") {
            let p = PathBuf::from(env);
            if p.is_dir() {
                out.push(p.join(&name));
            } else {
                out.push(p);
            }
        }
        // Repository locations, for `cargo test` where the test exe sits in target\.
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        out.push(manifest.join("../../src-tauri/resources").join(&name));
        out.push(PathBuf::from(r"D:\PythonScripts\Kurasi\src-tauri\resources").join(&name));
        out.into_iter().filter(|p| p.is_file()).collect()
    }

    /// The bound library. The first call loads the DLL; later calls return the same binding.
    pub fn get() -> Result<&'static Lib> {
        let slot = LIB.get_or_init(|| {
            let candidates = Pdfium::candidates();
            let Some(path) = candidates.first() else {
                let looked = [
                    "the exe folder",
                    "resources\\ next to the exe",
                    "KURASI_PDFIUM",
                    "src-tauri\\resources in the repository",
                ]
                .join(", ");
                return Err(Error::LibraryNotFound(looked));
            };
            let bindings = Lib::bind_to_library(path)
                .map_err(|e| Error::Library(format!("{} ({:?})", path.display(), e)))?;
            Ok(Lib::new(bindings))
        });
        match slot {
            Ok(lib) => Ok(lib),
            Err(e) => Err(clone_error(e)),
        }
    }
}

fn clone_error(e: &Error) -> Error {
    match e {
        Error::LibraryNotFound(s) => Error::LibraryNotFound(s.clone()),
        Error::Library(s) => Error::Library(s.clone()),
        other => Error::Library(other.to_string()),
    }
}
