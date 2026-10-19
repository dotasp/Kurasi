//! The scan runner: scans of the open project run one at a time in a background thread and
//! their progress is kept for `GET /api/scan/status` (a port of `ScanRunner` in scanner.py).

use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use kurasi_core::model::Origin;
use kurasi_core::scanner::{Progress, ScanMode, ScanReport};
use kurasi_core::util::now_iso;
use kurasi_core::Project;
use serde::Serialize;
use serde_json::Value;

const LOG_LINES: usize = 200;

#[derive(Debug, Clone, Default, Serialize)]
pub struct Status {
    pub running: bool,
    pub pct: u8,
    pub message: String,
    pub log: Vec<String>,
    pub result: Option<ScanReport>,
    pub error: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub background: bool,
    pub full: bool,
}

#[derive(Debug, Clone)]
struct Job {
    full: bool,
    origin: Origin,
    background: bool,
}

pub struct ScanRunner {
    project: Project,
    status: Mutex<Status>,
    thread: Mutex<Option<JoinHandle<()>>>,
    pending: Mutex<Option<Job>>,
    last_full_scan: Mutex<Instant>,
}

impl ScanRunner {
    pub fn new(project: Project) -> Arc<ScanRunner> {
        Arc::new(ScanRunner {
            project,
            status: Mutex::new(Status::default()),
            thread: Mutex::new(None),
            pending: Mutex::new(None),
            last_full_scan: Mutex::new(Instant::now()),
        })
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    fn status(&self) -> std::sync::MutexGuard<'_, Status> {
        self.status.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn snapshot(&self) -> Value {
        serde_json::to_value(&*self.status()).unwrap_or(Value::Null)
    }

    pub fn running(&self) -> bool {
        self.status().running
    }

    /// Seconds since the last whole-folder scan started (the auto-scan timer reads it).
    pub fn since_last_scan(&self) -> Duration {
        self.last_full_scan.lock().unwrap_or_else(|e| e.into_inner()).elapsed()
    }

    pub fn touch_last_scan(&self) {
        *self.last_full_scan.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    }

    fn progress(&self, p: Progress) {
        let mut st = self.status();
        st.pct = p.pct;
        st.message = p.message.clone();
        if p.line {
            st.log.push(format!("{:>3}%  {}", p.pct, p.message));
            if st.log.len() > LOG_LINES {
                let drop = st.log.len() - LOG_LINES;
                st.log.drain(..drop);
            }
        }
    }

    fn thread_alive(&self) -> bool {
        let guard = self.thread.lock().unwrap_or_else(|e| e.into_inner());
        guard.as_ref().map(|t| !t.is_finished()).unwrap_or(false)
    }

    /// Starts a scan of the whole folder. A request during a running scan queues one more
    /// and returns `false`.
    pub fn start(self: &Arc<Self>, full: bool, origin: Origin, background: bool) -> bool {
        let mut thread = self.thread.lock().unwrap_or_else(|e| e.into_inner());
        if thread.as_ref().map(|t| !t.is_finished()).unwrap_or(false) {
            let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            let full = full || pending.as_ref().map(|j| j.full).unwrap_or(false);
            *pending = Some(Job { full, origin, background: false });
            return false;
        }
        if let Some(t) = thread.take() {
            let _ = t.join();
        }
        let job = Job { full, origin, background };
        self.begin(&job);
        let me = self.clone();
        *thread = Some(std::thread::Builder::new().name("kurasi-scan".into()).spawn(move || me.run(job)).expect("scan thread"));
        true
    }

    fn begin(&self, job: &Job) {
        *self.status() = Status {
            running: true,
            pct: 0,
            message: "Starting".into(),
            started_at: Some(now_iso()),
            background: job.background,
            full: job.full,
            ..Status::default()
        };
    }

    fn run(self: Arc<Self>, first: Job) {
        let mut job = Some(first);
        while let Some(j) = job.take() {
            let mode = if j.full { ScanMode::Full } else { ScanMode::Incremental };
            let me = self.clone();
            let outcome = self.project.scan_as(mode, j.origin, move |p| me.progress(p));
            self.touch_last_scan();
            {
                let mut st = self.status();
                st.running = false;
                st.pct = 100;
                st.finished_at = Some(now_iso());
                match outcome {
                    Ok(report) => st.result = Some(report),
                    Err(e) => {
                        let msg = e.message();
                        st.log.push(format!("Error: {}", msg));
                        st.error = Some(msg);
                    }
                }
            }
            let next = self.pending.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some(n) = next {
                self.begin(&n);
                job = Some(n);
            }
        }
    }

    /// Waits until the running scan (and anything queued behind it) has finished.
    pub fn wait(&self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while self.thread_alive() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        if !self.thread_alive() {
            if let Some(t) = self.thread.lock().unwrap_or_else(|e| e.into_inner()).take() {
                let _ = t.join();
            }
        }
    }
}
