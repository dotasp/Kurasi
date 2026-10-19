//! Small shared helpers: ids, timestamps, hashing, path normalisation.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::error::Result;

/// `r-`, `f-`, `h-` or `p-` followed by 12 lowercase hex characters, as the Python build made them.
pub fn new_id(prefix: &str) -> String {
    let mut bytes = [0u8; 6];
    rand::thread_rng().fill_bytes(&mut bytes);
    let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
    format!("{}-{}", prefix, hex)
}

/// Local time as `YYYY-MM-DDTHH:MM:SS`, the format of `datetime.now().isoformat(timespec="seconds")`.
pub fn now_iso() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

/// Streaming SHA-256 in 1 MB chunks, lowercase hex.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1_048_576];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

pub fn sha256_bytes(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Modified time as seconds since the epoch with fraction, like `os.stat().st_mtime`.
pub fn mtime_secs(meta: &std::fs::Metadata) -> f64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Size, mtime and SHA-256 of a file after a write.
#[derive(Debug, Clone)]
pub struct FileStat {
    pub sha256: String,
    pub size: i64,
    pub mtime: f64,
}

pub fn file_stat(path: &Path) -> Result<FileStat> {
    let sha256 = sha256_file(path)?;
    let meta = std::fs::metadata(path)?;
    Ok(FileStat { sha256, size: meta.len() as i64, mtime: mtime_secs(&meta) })
}

/// Lower-cased, normalised absolute path for comparisons (`os.path.normcase(os.path.abspath())`).
pub fn norm_key(path: &Path) -> String {
    let abs = absolute(path);
    abs.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

/// An absolute path without touching the file system more than needed.
pub fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        normalize(path)
    } else {
        match std::env::current_dir() {
            Ok(cwd) => normalize(&cwd.join(path)),
            Err(_) => path.to_path_buf(),
        }
    }
}

/// Removes `.` and resolves `..` lexically, like `os.path.normpath`.
pub fn normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                let popped = out.pop();
                if !popped {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// The relative path of `path` below `root`, with the platform separator.
pub fn relpath(root: &Path, path: &Path) -> String {
    match path.strip_prefix(root) {
        Ok(rel) => rel.to_string_lossy().into_owned(),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

/// Lower-cased file extension without the dot.
pub fn ext_of(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i + 1 < name.len() => name[i + 1..].to_lowercase(),
        _ => String::new(),
    }
}

/// `" ".join(text.split())`
pub fn norm_space(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Python `str.strip()` equivalent for an optional value.
pub fn s(value: Option<&str>) -> String {
    value.unwrap_or("").trim().to_string()
}

/// Python `str.capitalize()`: first letter upper, the rest lower.
pub fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
        None => String::new(),
    }
}
