//! Cross-process write lock and atomic file replacement with retries.
//!
//! The app and the MCP server are separate processes on the same project. Every project-file
//! write and every multi-row database write holds the project's `write.lock`, an exclusive
//! byte-range lock on the file (`LockFileEx`, the same Win32 lock `msvcrt.locking` takes, so
//! the Python build and this one exclude each other). Inside one process a mutex guards the
//! same lock object, so two threads of one process queue as well.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::error::{Error, Result};

pub const REPLACE_TRIES: u32 = 6;
pub const REPLACE_DELAY_MS: u64 = 100;

/// A lock file. Clone-free: share it through `Arc` or keep it inside `Project`.
pub struct WriteLock {
    path: PathBuf,
    timeout: Duration,
    inner: Mutex<()>,
}

/// Held while the lock is taken. Drop releases the file lock and the mutex.
pub struct LockGuard<'a> {
    _guard: MutexGuard<'a, ()>,
    file: Option<File>,
}

impl Drop for LockGuard<'_> {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            let _ = file.unlock();
        }
    }
}

impl WriteLock {
    pub fn new(path: impl Into<PathBuf>) -> WriteLock {
        WriteLock { path: path.into(), timeout: Duration::from_secs(30), inner: Mutex::new(()) }
    }

    pub fn with_timeout(path: impl Into<PathBuf>, timeout: Duration) -> WriteLock {
        WriteLock { path: path.into(), timeout, inner: Mutex::new(()) }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Takes the lock, waiting up to the timeout for another process to release it.
    pub fn acquire(&self) -> Result<LockGuard<'_>> {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&self.path)?;
        let deadline = Instant::now() + self.timeout;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(LockGuard { _guard: guard, file: Some(file) }),
                Err(std::fs::TryLockError::WouldBlock) => {}
                Err(std::fs::TryLockError::Error(e)) => {
                    // Some file systems refuse byte-range locks; the in-process mutex still guards.
                    if e.raw_os_error() == Some(1) || e.kind() == std::io::ErrorKind::Unsupported {
                        return Ok(LockGuard { _guard: guard, file: None });
                    }
                    return Err(e.into());
                }
            }
            if Instant::now() > deadline {
                return Err(Error::conflict(
                    "Another Kurasi window or the MCP server is writing to this project. Try again in a moment.",
                ));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// `os.replace` with 6 tries and a delay doubling from 0.1 s: OneDrive and Acrobat hold files.
pub fn replace_retry(src: &Path, dst: &Path) -> Result<()> {
    replace_retry_with(src, dst, REPLACE_TRIES, Duration::from_millis(REPLACE_DELAY_MS))
}

pub fn replace_retry_with(src: &Path, dst: &Path, tries: u32, first_delay: Duration) -> Result<()> {
    let mut delay = first_delay;
    for attempt in 0..tries {
        match fs::rename(src, dst) {
            Ok(()) => return Ok(()),
            Err(e) => {
                let busy = e.kind() == std::io::ErrorKind::PermissionDenied
                    || matches!(e.raw_os_error(), Some(5) | Some(32) | Some(33));
                if !busy {
                    return Err(e.into());
                }
            }
        }
        if attempt + 1 < tries {
            std::thread::sleep(delay);
            delay *= 2;
        }
    }
    Err(Error::FileBusy)
}

/// `.~kurasi-<name>.tmp` in the same folder as `path`.
pub fn temp_path_for(path: &Path) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let folder = path.parent().map(Path::to_path_buf).unwrap_or_default();
    folder.join(format!(".~kurasi-{}.tmp", name))
}

/// Writes to a temp file in the same folder, syncs, then replaces the target with retries.
pub fn atomic_write_bytes(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = temp_path_for(path);
    let result = (|| -> Result<()> {
        {
            let mut fh = File::create(&tmp)?;
            fh.write_all(data)?;
            fh.flush()?;
            fh.sync_all()?;
        }
        replace_retry(&tmp, path)
    })();
    let _ = fs::remove_file(&tmp);
    result
}

pub fn atomic_write_text(path: &Path, text: &str) -> Result<()> {
    atomic_write_bytes(path, text.as_bytes())
}
