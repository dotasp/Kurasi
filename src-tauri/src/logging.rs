//! Small logger: every record goes to `<root>\logs\kurasi.log`; records also go to stderr when
//! `to_stderr` is set (the console subcommands) so the caller sees them live.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

struct ShellLogger {
    file: Mutex<Option<File>>,
    to_stderr: bool,
    t0: Instant,
}

impl log::Log for ShellLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Debug
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let target = record.target();
        // Keep the shell's own records and warnings from libraries; drop library chatter.
        if record.level() > log::Level::Warn && !target.starts_with("kurasi") {
            return;
        }
        let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let line = format!(
            "{} +{:>8.3}s {:<5} {}: {}\n",
            iso_time(secs),
            self.t0.elapsed().as_secs_f64(),
            record.level(),
            target,
            record.args()
        );
        if self.to_stderr {
            let _ = std::io::stderr().write_all(line.as_bytes());
        }
        if let Ok(mut guard) = self.file.lock() {
            if let Some(f) = guard.as_mut() {
                let _ = f.write_all(line.as_bytes());
            }
        }
    }

    fn flush(&self) {
        if let Ok(mut guard) = self.file.lock() {
            if let Some(f) = guard.as_mut() {
                let _ = f.flush();
            }
        }
    }
}

fn iso_time(secs: u64) -> String {
    // Civil date from days since the epoch (Howard Hinnant's algorithm), UTC.
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m,
        d,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Installs the logger. Safe to call once; later calls are ignored.
pub fn init(log_file: &Path, to_stderr: bool, t0: Instant) {
    let file = std::fs::create_dir_all(log_file.parent().unwrap_or(Path::new(".")))
        .ok()
        .and_then(|_| OpenOptions::new().create(true).append(true).open(log_file).ok());
    let logger = ShellLogger {
        file: Mutex::new(file),
        to_stderr,
        t0,
    };
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(log::LevelFilter::Debug);
    }
}
