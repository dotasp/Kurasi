//! Desktop entry: native splash, Tauri app with the hidden `main` window, startup task and the
//! hand-over (CONTRACT_RUST.md section 4 as amended on 17 Oct 2026: the splash is a Win32
//! window shown before any WebView2 is created).
//!
//! Sequence:
//! 1. `Splash::show` (target: visible within 300 ms of process start).
//! 2. The Tauri app is built with only `main` (visible false). WebView2 initialises while the
//!    startup task opens the last project and starts the local server.
//! 3. `main` navigates to the server URL. The page emits `splash-status` while it loads and
//!    `kurasi-ready` after its first render.
//! 4. On `kurasi-ready`, or 30 s after start at the latest, and at least 2000 ms after the
//!    splash appeared: `main.show()`, `main.set_focus()`, splash destroyed.
//! 5. A startup error is drawn on the splash with Retry and Quit.
//!
//! Before any of that the hidden `main` window is fitted to the work area of its monitor
//! ([`fit_main_window`]), so it never opens larger than the screen.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use tauri::{Emitter, Listener, Manager};

use crate::bridge::{self, AppPaths, ServerHandle};
use crate::paths::ShellPaths;
use crate::splash::{Choice, Splash};

const MIN_SPLASH_MS: u64 = 2000;
const SAFETY_MS: u64 = 30_000;

/// Share of the work area, per dimension, that the main window may cover when it opens.
pub const FIT_SHARE: f64 = 0.92;

/// Shared, closable handle to the splash.
#[derive(Clone)]
pub struct SplashHandle(Arc<Mutex<Option<Splash>>>);

impl SplashHandle {
    fn new(s: Splash) -> SplashHandle {
        SplashHandle(Arc::new(Mutex::new(Some(s))))
    }
    pub fn status(&self, text: &str) {
        if let Ok(g) = self.0.lock() {
            if let Some(s) = g.as_ref() {
                s.status(text);
            }
        }
        log::info!("splash: {text}");
    }
    /// Blocks until a button is pressed. Call from a blocking thread.
    pub fn error(&self, text: &str) -> Choice {
        log::error!("startup error shown on the splash: {text}");
        // Take the lock only for the call; the window thread needs no lock to repaint.
        let splash = self.0.lock().ok();
        match splash.as_ref().and_then(|g| g.as_ref()) {
            Some(s) => s.error(text),
            None => Choice::Quit,
        }
    }
    pub fn close(&self) {
        let taken = self.0.lock().ok().and_then(|mut g| g.take());
        if let Some(s) = taken {
            s.close();
            log::info!("splash closed");
        }
    }
}

/// Keeps the server alive for the life of the app.
pub struct Running(pub Mutex<Option<ServerHandle>>);

/// Answers a pending startup error with Retry (same as the native button).
#[tauri::command]
fn splash_retry() {
    crate::splash::choose(Choice::Retry);
}

/// Answers a pending startup error with Quit, or closes the app when nothing is pending.
#[tauri::command]
fn splash_quit(app: tauri::AppHandle) {
    crate::splash::choose(Choice::Quit);
    app.exit(0);
}

pub fn run_app(port: Option<u16>, t0: Instant) -> Result<()> {
    let shell = ShellPaths::detect();
    crate::logging::init(&shell.log_file(), false, t0);
    log::info!(
        "Kurasi {} starting; app data root {} ({:?}); exe dir {}",
        env!("CARGO_PKG_VERSION"),
        shell.root.display(),
        shell.source,
        shell.exe_dir.display()
    );

    let splash = Splash::show(t0).map_err(|e| anyhow!("splash window: {e}"))?;
    log::info!(
        "splash visible after {:.0} ms from main(), {} from process creation",
        splash.visible_after.as_secs_f64() * 1000.0,
        crate::splash::since_process_start()
            .map(|d| format!("{:.0} ms", d.as_secs_f64() * 1000.0))
            .unwrap_or_else(|| "unknown".into())
    );
    let splash = SplashHandle::new(splash);
    splash.status("Starting");

    let paths = AppPaths::detect();
    let splash_for_setup = splash.clone();
    let splash_for_exit = splash.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Running(Mutex::new(None)))
        .invoke_handler(tauri::generate_handler![splash_retry, splash_quit])
        .setup(move |app| {
            let handle = app.handle().clone();
            let splash = splash_for_setup.clone();
            let paths = paths.clone();

            // Status texts the page sends while it loads.
            let splash_status = splash.clone();
            app.listen_any("splash-status", move |ev| {
                let text = serde_json::from_str::<serde_json::Value>(ev.payload())
                    .ok()
                    .and_then(|v| v.get("text").and_then(|t| t.as_str()).map(str::to_string))
                    .unwrap_or_else(|| ev.payload().trim_matches('"').to_string());
                splash_status.status(&text);
            });

            // Ready signal from app.js after the first render.
            let ready = Arc::new(tokio::sync::Notify::new());
            let ready_rx = ready.clone();
            app.listen_any("kurasi-ready", move |_| {
                log::info!("kurasi-ready received");
                ready.notify_one();
            });

            tauri::async_runtime::spawn(async move {
                startup_task(handle, paths, splash, ready_rx, port, t0).await;
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .map_err(|e| anyhow!("Tauri did not build: {e}"))?
        .run(move |_app, event| {
            if let tauri::RunEvent::Exit = event {
                splash_for_exit.close();
            }
        });
    Ok(())
}

async fn startup_task(
    app: tauri::AppHandle,
    paths: AppPaths,
    splash: SplashHandle,
    ready: Arc<tokio::sync::Notify>,
    port: Option<u16>,
    t0: Instant,
) {
    let Some(main) = app.get_webview_window("main") else {
        log::error!("the main window is missing from tauri.conf.json");
        app.exit(2);
        return;
    };

    // Size the hidden window first, so the page lays out at the size it is shown at.
    fit_main_window(&app, &main);

    // Open the project and start the server; on failure show Retry and Quit.
    let started = loop {
        let status = |t: &str| splash.status(t);
        match bridge::start_desktop(&paths, port, &status).await {
            Ok(s) => break s,
            Err(e) => {
                let text = format!(
                    "{e:#}\n\nRetry starts again. Quit closes Kurasi. The log is in {}.",
                    ShellPaths::detect().log_file().display()
                );
                let splash2 = splash.clone();
                let choice = tokio::task::spawn_blocking(move || splash2.error(&text))
                    .await
                    .unwrap_or(Choice::Quit);
                match choice {
                    Choice::Retry => continue,
                    Choice::Quit => {
                        app.exit(0);
                        return;
                    }
                }
            }
        }
    };
    log::info!(
        "server at {} after {:.2} s; project {}",
        started.url,
        t0.elapsed().as_secs_f64(),
        started.project_name.as_deref().unwrap_or("none")
    );
    if let Some(running) = app.try_state::<Running>() {
        if let Ok(mut g) = running.0.lock() {
            *g = Some(started.handle);
        }
    }

    splash.status(match &started.project_name {
        Some(name) => format!("Opening {name}"),
        None => "Opening Kurasi".to_string(),
    }
    .as_str());

    // Point the hidden main window at the server.
    match started.url.parse::<tauri::Url>() {
        Ok(url) => {
            if let Err(e) = main.navigate(url) {
                log::error!("main window navigation failed: {e}");
            }
        }
        Err(e) => log::error!("server URL did not parse: {e}"),
    }
    let _ = app.emit("kurasi-server", serde_json::json!({"url": started.url}));

    // Wait for the page, with the 30 s safety from process start.
    let safety_left = Duration::from_millis(SAFETY_MS).saturating_sub(t0.elapsed());
    match tokio::time::timeout(safety_left, ready.notified()).await {
        Ok(()) => log::info!("page ready after {:.2} s", t0.elapsed().as_secs_f64()),
        Err(_) => log::warn!("page did not report ready within {} s; showing the window anyway", SAFETY_MS / 1000),
    }

    // Keep the splash up for its minimum time.
    let min_left = Duration::from_millis(MIN_SPLASH_MS).saturating_sub(t0.elapsed());
    if !min_left.is_zero() {
        tokio::time::sleep(min_left).await;
    }

    if let Err(e) = main.show() {
        log::error!("main.show failed: {e}");
    }
    if let Err(e) = main.set_focus() {
        log::warn!("main.set_focus failed: {e}");
    }
    log::info!("main window shown after {:.2} s", t0.elapsed().as_secs_f64());
    splash.close();
}

/// Size and place of the main window when it opens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowFit {
    /// Client area in physical pixels.
    pub inner: (u32, u32),
    /// Top-left corner of the outer frame in physical pixels.
    pub position: (i32, i32),
    /// Minimum client area in logical pixels.
    pub min_logical: Option<(f64, f64)>,
    /// The configured size does not fit, so the window opens maximized.
    pub maximized: bool,
}

/// Fits a window whose configured client size is `want` (logical pixels) into a work area.
///
/// `work_pos` and `work_size` are the work area in physical pixels, `frame` the title bar and
/// borders in physical pixels, `min` the configured minimum in logical pixels. The configured
/// size is kept when its outer size fits within [`FIT_SHARE`] of the work area in both
/// dimensions. Otherwise each dimension that overflows shrinks to that share and the window
/// opens maximized. The outer frame is centred on the work area and the minimum never exceeds
/// the fitted client size.
pub fn fit_window(
    want: (f64, f64),
    min: Option<(f64, f64)>,
    frame: (u32, u32),
    work_pos: (i32, i32),
    work_size: (u32, u32),
    scale: f64,
) -> WindowFit {
    let limit_w = (work_size.0 as f64 * FIT_SHARE).floor();
    let limit_h = (work_size.1 as f64 * FIT_SHARE).floor();
    let want_w = (want.0 * scale).round() + frame.0 as f64;
    let want_h = (want.1 * scale).round() + frame.1 as f64;
    let maximized = want_w > limit_w || want_h > limit_h;
    let outer_w = want_w.min(limit_w);
    let outer_h = want_h.min(limit_h);
    let inner_w = (outer_w - frame.0 as f64).max(1.0);
    let inner_h = (outer_h - frame.1 as f64).max(1.0);
    let min_logical =
        min.map(|(w, h)| (w.min((inner_w / scale).floor()), h.min((inner_h / scale).floor())));
    WindowFit {
        inner: (inner_w as u32, inner_h as u32),
        position: (
            work_pos.0 + ((work_size.0 as f64 - outer_w) / 2.0).floor() as i32,
            work_pos.1 + ((work_size.1 as f64 - outer_h) / 2.0).floor() as i32,
        ),
        min_logical,
        maximized,
    }
}

/// Fits the hidden `main` window to the work area (the screen without the taskbar) of the
/// monitor it opens on, or of the primary monitor when that one is unknown. See [`fit_window`].
fn fit_main_window(app: &tauri::AppHandle, main: &tauri::WebviewWindow) {
    let monitor = match main.current_monitor() {
        Ok(Some(m)) => Some(m),
        _ => main.primary_monitor().ok().flatten(),
    };
    let Some(monitor) = monitor else {
        log::warn!("main window: no monitor found; the configured size stays");
        return;
    };
    let conf = app.config().app.windows.iter().find(|w| w.label == "main");
    let want = conf.map(|c| (c.width, c.height)).unwrap_or((1600.0, 900.0));
    let min = conf.and_then(|c| c.min_width.zip(c.min_height));
    let scale = monitor.scale_factor();
    let work = *monitor.work_area();
    // Title bar and borders of the hidden window as Windows reports them.
    let frame = match (main.outer_size(), main.inner_size()) {
        (Ok(o), Ok(i)) => (o.width.saturating_sub(i.width), o.height.saturating_sub(i.height)),
        _ => ((16.0 * scale).round() as u32, (39.0 * scale).round() as u32),
    };
    let fit = fit_window(
        want,
        min,
        frame,
        (work.position.x, work.position.y),
        (work.size.width, work.size.height),
        scale,
    );

    // Minimum first: tao re-applies the current size when the minimum changes.
    if let Some((w, h)) = fit.min_logical {
        if let Err(e) = main.set_min_size(Some(tauri::LogicalSize::new(w, h))) {
            log::warn!("main window: set_min_size failed: {e}");
        }
    }
    if let Err(e) = main.set_size(tauri::PhysicalSize::new(fit.inner.0, fit.inner.1)) {
        log::warn!("main window: set_size failed: {e}");
    }
    if let Err(e) = main.set_position(tauri::PhysicalPosition::new(fit.position.0, fit.position.1)) {
        log::warn!("main window: set_position failed: {e}");
    }
    // Last, because resizing and moving clear the maximized state. While the window is hidden
    // this only marks it; `show()` then opens it maximized over the fitted restore size.
    if fit.maximized {
        if let Err(e) = main.maximize() {
            log::warn!("main window: maximize failed: {e}");
        }
    }
    log::info!(
        "main window: monitor {} at scale {scale}; work area {:.0}x{:.0} logical ({}x{} px at {},{}); \
         frame {}x{} px; size {:.0}x{:.0} logical ({}x{} px) at {},{}; minimum {}; {}",
        monitor.name().cloned().unwrap_or_default(),
        work.size.width as f64 / scale,
        work.size.height as f64 / scale,
        work.size.width,
        work.size.height,
        work.position.x,
        work.position.y,
        frame.0,
        frame.1,
        fit.inner.0 as f64 / scale,
        fit.inner.1 as f64 / scale,
        fit.inner.0,
        fit.inner.1,
        fit.position.0,
        fit.position.1,
        fit.min_logical
            .map(|(w, h)| format!("{w:.0}x{h:.0} logical"))
            .unwrap_or_else(|| "none".to_string()),
        if fit.maximized {
            format!(
                "{:.0}x{:.0} exceeds {:.0} percent of the work area, so the window opens maximized",
                want.0,
                want.1,
                FIT_SHARE * 100.0
            )
        } else {
            format!("{:.0}x{:.0} fits", want.0, want.1)
        }
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const WANT: (f64, f64) = (1600.0, 900.0);
    const MIN: Option<(f64, f64)> = Some((960.0, 600.0));

    /// The outer frame lies inside the work area and the minimum does not exceed the size.
    fn assert_inside(fit: &WindowFit, frame: (u32, u32), work: (u32, u32), scale: f64) {
        let outer = (fit.inner.0 + frame.0, fit.inner.1 + frame.1);
        assert!(fit.position.0 >= 0 && fit.position.1 >= 0, "{fit:?}");
        assert!(fit.position.0 as u32 + outer.0 <= work.0, "{fit:?}");
        assert!(fit.position.1 as u32 + outer.1 <= work.1, "{fit:?}");
        let (mw, mh) = fit.min_logical.unwrap();
        assert!((mw * scale).round() as u32 <= fit.inner.0, "{fit:?}");
        assert!((mh * scale).round() as u32 <= fit.inner.1, "{fit:?}");
    }

    #[test]
    fn full_hd_at_100_percent_keeps_the_configured_size() {
        // 1920x1080, taskbar 48 px.
        let (frame, work) = ((16, 39), (1920, 1032));
        let fit = fit_window(WANT, MIN, frame, (0, 0), work, 1.0);
        assert!(!fit.maximized);
        assert_eq!(fit.inner, (1600, 900));
        assert_eq!(fit.position, (152, 46));
        assert_eq!(fit.min_logical, Some((960.0, 600.0)));
        assert_inside(&fit, frame, work, 1.0);
    }

    #[test]
    fn full_hd_at_150_percent_shrinks_and_maximizes() {
        // 1280x720 logical; taskbar 48 logical = 72 px.
        let (frame, work) = ((24, 58), (1920, 1008));
        let fit = fit_window(WANT, MIN, frame, (0, 0), work, 1.5);
        assert!(fit.maximized);
        assert_eq!(fit.inner, (1742, 869));
        assert_eq!(fit.position, (77, 40));
        assert_eq!(fit.min_logical, Some((960.0, 579.0)));
        assert_inside(&fit, frame, work, 1.5);
    }

    #[test]
    fn small_laptop_at_125_percent_shrinks_and_maximizes() {
        // 1366x768 at 125 percent: 1093x614 logical; taskbar 60 px.
        let (frame, work) = ((20, 49), (1366, 708));
        let fit = fit_window(WANT, MIN, frame, (0, 0), work, 1.25);
        assert!(fit.maximized);
        assert_eq!(fit.inner, (1236, 602));
        assert_eq!(fit.position, (55, 28));
        assert_eq!(fit.min_logical, Some((960.0, 481.0)));
        assert_inside(&fit, frame, work, 1.25);
    }

    #[test]
    fn second_monitor_offsets_the_position() {
        // A monitor left of the primary one, at 125 percent.
        let fit = fit_window(WANT, MIN, (20, 49), (-1920, 0), (1920, 1020), 1.25);
        assert!(fit.maximized);
        assert_eq!(fit.position, (-1920 + 77, 41));
    }
}
