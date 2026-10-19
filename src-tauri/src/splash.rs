//! Native Win32 splash window, shown before any WebView2 is created.
//!
//! WebView2 initialisation took 11 to 26 s on the reference PC, so a splash that is itself a
//! webview appears late. This window is plain GDI: a borderless, topmost 960 x 600 popup
//! (scaled by the system DPI, shrunk with its aspect kept when it would cover more than
//! [`crate::app::FIT_SHARE`] of the work area, centred on the work area of the primary
//! monitor) painting `ui/assets/splash.jpg` and one status line on a dark band at the bottom,
//! with the version and the licence at the right end of the band. The window lives on its own
//! thread with its own message loop; the rest of the process talks to it through [`Splash`].
//!
//! Errors during startup are drawn on the splash with two native buttons, Retry and Quit.

use std::ffi::c_void;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::Instant;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    UpdateWindow,
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect,
    InvalidateRect, SelectObject, SetBkMode, SetStretchBltMode, SetTextColor, StretchDIBits,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET,
    DEFAULT_PITCH, DIB_RGB_COLORS, DT_CALCRECT, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT,
    DT_SINGLELINE,
    DT_VCENTER, DT_WORDBREAK, FF_DONTCARE, FW_BOLD, FW_NORMAL, HALFTONE, HBRUSH, HFONT, HGDIOBJ,
    OUT_DEFAULT_PRECIS, PAINTSTRUCT, SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    GetDpiForSystem, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetSystemMetrics, LoadCursorW, PostMessageW, PostQuitMessage, RegisterClassW, SendMessageW,
    ShowWindow, SystemParametersInfoW, TranslateMessage, BS_DEFPUSHBUTTON, BS_PUSHBUTTON,
    CS_HREDRAW, CS_VREDRAW, HMENU, IDC_ARROW, MSG, SM_CXSCREEN, SM_CYSCREEN, SPI_GETWORKAREA,
    SW_HIDE, SW_SHOW, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP,
    WM_CLOSE, WM_COMMAND, WM_DESTROY, WM_ERASEBKGND, WM_PAINT, WM_SETFONT, WNDCLASSW, WS_CHILD,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_TABSTOP, WS_VISIBLE,
};

const BASE_W: i32 = 960;
const BASE_H: i32 = 600;
const BAND_H: i32 = 36;
const FONT_PT: i32 = 14;
/// Right end of the status band: version and licence (LICENSE at the repository root).
const LICENSE_LINE: &str = concat!("v", env!("CARGO_PKG_VERSION"), " \u{b7} MIT License \u{b7} provided as is, without warranty");

const WM_APP_STATUS: u32 = WM_APP + 1;
const WM_APP_ERROR: u32 = WM_APP + 2;
const WM_APP_IMAGE: u32 = WM_APP + 3;

const ID_RETRY: usize = 101;
const ID_QUIT: usize = 102;

const SPLASH_JPG: &[u8] = include_bytes!("../../ui/assets/splash.jpg");

/// What the person chose on the error panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Retry,
    Quit,
}

/// Decoded splash image as a top-down BGRA buffer.
struct Bitmap {
    width: i32,
    height: i32,
    bgra: Vec<u8>,
}

struct State {
    status: String,
    error: Option<String>,
    image: Option<Arc<Bitmap>>,
    choice_tx: Option<Sender<Choice>>,
    // GDI objects owned by the window thread
    hwnd: isize,
    font: isize,
    font_bold: isize,
    btn_retry: isize,
    btn_quit: isize,
    scale: f64,
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();

fn state() -> &'static Mutex<State> {
    STATE.get_or_init(|| {
        Mutex::new(State {
            status: "Starting".to_string(),
            error: None,
            image: None,
            choice_tx: None,
            hwnd: 0,
            font: 0,
            font_bold: 0,
            btn_retry: 0,
            btn_quit: 0,
            scale: 1.0,
        })
    })
}

/// Handle to the splash window. Dropping it without `close` leaves the window open until the
/// process ends; call [`Splash::close`] when the main window is up.
pub struct Splash {
    hwnd: isize,
    thread: Option<JoinHandle<()>>,
    choice_rx: Receiver<Choice>,
    /// Time from process start to the splash becoming visible.
    pub visible_after: std::time::Duration,
}

impl Splash {
    /// Creates the window on its own thread and returns once it is visible.
    pub fn show(t0: Instant) -> anyhow::Result<Splash> {
        let (choice_tx, choice_rx) = channel::<Choice>();
        if let Ok(mut st) = state().lock() {
            st.choice_tx = Some(choice_tx);
        }
        let (ready_tx, ready_rx) = channel::<Result<isize, String>>();
        let thread = std::thread::Builder::new()
            .name("kurasi-splash".into())
            .spawn(move || window_thread(ready_tx))?;
        // Decode the JPEG beside the window creation so the window never waits for it.
        std::thread::Builder::new()
            .name("kurasi-splash-image".into())
            .spawn(decode_image)?;
        let hwnd = ready_rx
            .recv()
            .map_err(|_| anyhow::anyhow!("the splash thread ended before the window was created"))?
            .map_err(|e| anyhow::anyhow!("splash window: {e}"))?;
        let visible_after = t0.elapsed();
        Ok(Splash {
            hwnd,
            thread: Some(thread),
            choice_rx,
            visible_after,
        })
    }

    fn hwnd(&self) -> HWND {
        HWND(self.hwnd as *mut c_void)
    }

    /// Replaces the status line.
    pub fn status(&self, text: &str) {
        if let Ok(mut st) = state().lock() {
            st.status = text.to_string();
        }
        unsafe {
            let _ = PostMessageW(Some(self.hwnd()), WM_APP_STATUS, WPARAM(0), LPARAM(0));
        }
    }

    /// Shows `text` with Retry and Quit buttons and blocks until one is pressed.
    pub fn error(&self, text: &str) -> Choice {
        if let Ok(mut st) = state().lock() {
            st.error = Some(text.to_string());
        }
        unsafe {
            let _ = PostMessageW(Some(self.hwnd()), WM_APP_ERROR, WPARAM(1), LPARAM(0));
        }
        self.choice_rx.recv().unwrap_or(Choice::Quit)
    }

    /// Destroys the window and joins its thread.
    pub fn close(mut self) {
        unsafe {
            let _ = PostMessageW(Some(self.hwnd()), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Answers a pending error panel from outside the window (the `splash_retry` and `splash_quit`
/// commands). Without a pending error the choice is dropped.
pub fn choose(c: Choice) {
    let (tx, hwnd) = match state().lock() {
        Ok(mut st) => {
            if st.error.is_none() {
                return;
            }
            st.error = None;
            (st.choice_tx.clone(), st.hwnd)
        }
        Err(_) => return,
    };
    if let Some(tx) = tx {
        let _ = tx.send(c);
    }
    if hwnd != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as *mut c_void)), WM_APP_ERROR, WPARAM(0), LPARAM(0));
        }
    }
}

fn decode_image() {
    let decoded = match image::load_from_memory(SPLASH_JPG) {
        Ok(img) => img.to_rgba8(),
        Err(e) => {
            log::warn!("splash image did not decode: {e}");
            return;
        }
    };
    let (w, h) = decoded.dimensions();
    let mut bgra = decoded.into_raw();
    for px in bgra.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    let bmp = Arc::new(Bitmap {
        width: w as i32,
        height: h as i32,
        bgra,
    });
    let hwnd = if let Ok(mut st) = state().lock() {
        st.image = Some(bmp);
        st.hwnd
    } else {
        0
    };
    if hwnd != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as *mut c_void)), WM_APP_IMAGE, WPARAM(0), LPARAM(0));
        }
    }
}

/// Shrinks `size`, keeping its aspect, so it covers at most [`crate::app::FIT_SHARE`] of `work`
/// in each dimension. A size that already fits comes back unchanged.
fn fit_into(size: (i32, i32), work: (i32, i32)) -> (i32, i32) {
    if size.0 <= 0 || size.1 <= 0 || work.0 <= 0 || work.1 <= 0 {
        return size;
    }
    let share = crate::app::FIT_SHARE;
    let k = (share * work.0 as f64 / size.0 as f64)
        .min(share * work.1 as f64 / size.1 as f64)
        .min(1.0);
    ((size.0 as f64 * k).floor() as i32, (size.1 as f64 * k).floor() as i32)
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn window_thread(ready: Sender<Result<isize, String>>) {
    unsafe {
        // Harmless when the embedded manifest already set it.
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let dpi = GetDpiForSystem();
        let scale = if dpi == 0 { 1.0 } else { dpi as f64 / 96.0 };

        // Work area of the primary monitor (the screen without the taskbar), in the same
        // pixels as the window.
        let mut work = RECT::default();
        let have_work = SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut work as *mut RECT as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok();
        if !have_work || work.right <= work.left || work.bottom <= work.top {
            work = RECT {
                left: 0,
                top: 0,
                right: GetSystemMetrics(SM_CXSCREEN),
                bottom: GetSystemMetrics(SM_CYSCREEN),
            };
        }
        let work_w = work.right - work.left;
        let work_h = work.bottom - work.top;
        let (width, height) = fit_into(
            (
                (BASE_W as f64 * scale).round() as i32,
                (BASE_H as f64 * scale).round() as i32,
            ),
            (work_w, work_h),
        );

        let hinstance = match GetModuleHandleW(None) {
            Ok(h) => h,
            Err(e) => {
                let _ = ready.send(Err(e.to_string()));
                return;
            }
        };
        let hinstance = HINSTANCE::from(hinstance);
        let class_name = w!("KurasiSplash");
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: class_name,
            ..Default::default()
        };
        if RegisterClassW(&wc) == 0 {
            let _ = ready.send(Err("RegisterClassW failed".into()));
            return;
        }

        let font_h = -((FONT_PT as f64 * dpi.max(96) as f64 / 72.0).round() as i32);
        let make_font = |weight: i32| -> HFONT {
            CreateFontW(
                font_h,
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                DEFAULT_PITCH.0 as u32 | FF_DONTCARE.0 as u32,
                w!("Segoe UI"),
            )
        };
        let font = make_font(FW_NORMAL.0 as i32);
        let font_bold = make_font(FW_BOLD.0 as i32);
        if let Ok(mut st) = state().lock() {
            st.font = font.0 as isize;
            st.font_bold = font_bold.0 as isize;
            st.scale = scale;
        }

        let x = work.left + ((work_w - width) / 2).max(0);
        let y = work.top + ((work_h - height) / 2).max(0);

        let hwnd = match CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class_name,
            w!("Kurasi"),
            WS_POPUP,
            x,
            y,
            width,
            height,
            None,
            None,
            Some(hinstance),
            None,
        ) {
            Ok(h) => h,
            Err(e) => {
                let _ = ready.send(Err(e.to_string()));
                return;
            }
        };
        if let Ok(mut st) = state().lock() {
            st.hwnd = hwnd.0 as isize;
        }
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);
        let _ = ready.send(Ok(hwnd.0 as isize));
        log::info!(
            "splash window: {width}x{height} px at {x},{y}; work area {work_w}x{work_h} px at {},{}; system scale {scale}",
            work.left,
            work.top
        );

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = DeleteObject(HGDIOBJ(font.0));
        let _ = DeleteObject(HGDIOBJ(font_bold.0));
        if let Ok(mut st) = state().lock() {
            st.hwnd = 0;
            st.font = 0;
            st.font_bold = 0;
            st.btn_retry = 0;
            st.btn_quit = 0;
        }
    }
}

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF((r as u32) | ((g as u32) << 8) | ((b as u32) << 16))
}

unsafe fn fill(hdc: windows::Win32::Graphics::Gdi::HDC, rc: &RECT, colour: COLORREF) {
    let brush: HBRUSH = CreateSolidBrush(colour);
    let _ = FillRect(hdc, rc, brush);
    let _ = DeleteObject(HGDIOBJ(brush.0));
}

unsafe fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    let mut client = RECT::default();
    let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut client);
    let w = client.right - client.left;
    let h = client.bottom - client.top;

    let (status, error, image, font, font_bold, scale) = match state().lock() {
        Ok(st) => (
            st.status.clone(),
            st.error.clone(),
            st.image.clone(),
            HFONT(st.font as *mut c_void),
            HFONT(st.font_bold as *mut c_void),
            st.scale,
        ),
        Err(_) => {
            let _ = EndPaint(hwnd, &ps);
            return;
        }
    };
    let band_h = (BAND_H as f64 * scale).round() as i32;
    let pad = (14.0 * scale).round() as i32;

    // Picture (object-fit: cover) or a dark background until the image is decoded.
    let picture = RECT {
        left: 0,
        top: 0,
        right: w,
        bottom: h,
    };
    match image {
        Some(bmp) => {
            let bi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: bmp.width,
                    biHeight: -bmp.height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            // Crop the source so its aspect matches the window (cover).
            let win_ratio = w as f64 / h as f64;
            let img_ratio = bmp.width as f64 / bmp.height as f64;
            let (sw, sh) = if img_ratio > win_ratio {
                ((bmp.height as f64 * win_ratio).round() as i32, bmp.height)
            } else {
                (bmp.width, (bmp.width as f64 / win_ratio).round() as i32)
            };
            let sx = (bmp.width - sw) / 2;
            let sy = (bmp.height - sh) / 2;
            let _ = SetStretchBltMode(hdc, HALFTONE);
            let _ = StretchDIBits(
                hdc,
                0,
                0,
                w,
                h,
                sx,
                sy,
                sw,
                sh,
                Some(bmp.bgra.as_ptr() as *const c_void),
                &bi,
                DIB_RGB_COLORS,
                SRCCOPY,
            );
        }
        None => fill(hdc, &picture, rgb(16, 20, 26)),
    }

    // Status band.
    let band = RECT {
        left: 0,
        top: h - band_h,
        right: w,
        bottom: h,
    };
    fill(hdc, &band, rgb(16, 20, 26));
    let _ = SetBkMode(hdc, TRANSPARENT);
    let old = SelectObject(hdc, HGDIOBJ(font_bold.0));
    let _ = SetTextColor(hdc, rgb(242, 242, 242));
    let mut title = to_wide("Kurasi");
    let mut title_rc = RECT {
        left: pad,
        top: band.top,
        right: pad + (90.0 * scale).round() as i32,
        bottom: band.bottom,
    };
    let n = title.len() - 1;
    let _ = DrawTextW(hdc, &mut title[..n], &mut title_rc, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX);
    let _ = SelectObject(hdc, HGDIOBJ(font.0));
    // Licence line, right-aligned and dimmer; the status line ends where it starts.
    let mut lic = to_wide(LICENSE_LINE);
    let n = lic.len() - 1;
    let mut lic_rc = RECT::default();
    let _ = DrawTextW(hdc, &mut lic[..n], &mut lic_rc, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
    let lic_w = lic_rc.right - lic_rc.left;
    let mut lic_rc = RECT {
        left: w - pad - lic_w,
        top: band.top,
        right: w - pad,
        bottom: band.bottom,
    };
    let _ = SetTextColor(hdc, rgb(138, 144, 152));
    let _ = DrawTextW(hdc, &mut lic[..n], &mut lic_rc, DT_RIGHT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX);
    let _ = SetTextColor(hdc, rgb(196, 200, 205));
    let status_left = pad + (76.0 * scale).round() as i32;
    let mut status_w = to_wide(&status);
    let mut status_rc = RECT {
        left: status_left,
        top: band.top,
        right: (w - 2 * pad - lic_w).max(status_left),
        bottom: band.bottom,
    };
    let n = status_w.len() - 1;
    let _ = DrawTextW(
        hdc,
        &mut status_w[..n],
        &mut status_rc,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS,
    );

    // Error panel.
    if let Some(err) = error {
        let panel_w = (640.0 * scale).round() as i32;
        let panel_h = (220.0 * scale).round() as i32;
        let panel = RECT {
            left: (w - panel_w) / 2,
            top: (h - band_h - panel_h) / 2,
            right: (w + panel_w) / 2,
            bottom: (h - band_h + panel_h) / 2,
        };
        fill(hdc, &panel, rgb(16, 20, 26));
        let _ = SelectObject(hdc, HGDIOBJ(font_bold.0));
        let _ = SetTextColor(hdc, rgb(242, 242, 242));
        let mut head = to_wide("Kurasi could not start");
        let mut head_rc = RECT {
            left: panel.left + pad,
            top: panel.top + pad,
            right: panel.right - pad,
            bottom: panel.top + pad + (24.0 * scale).round() as i32,
        };
        let n = head.len() - 1;
        let _ = DrawTextW(hdc, &mut head[..n], &mut head_rc, DT_LEFT | DT_SINGLELINE | DT_NOPREFIX);
        let _ = SelectObject(hdc, HGDIOBJ(font.0));
        let _ = SetTextColor(hdc, rgb(230, 230, 230));
        let mut body = to_wide(&err);
        let mut body_rc = RECT {
            left: panel.left + pad,
            top: head_rc.bottom + pad / 2,
            right: panel.right - pad,
            bottom: panel.bottom - (60.0 * scale).round() as i32,
        };
        let n = body.len() - 1;
        let _ = DrawTextW(hdc, &mut body[..n], &mut body_rc, DT_LEFT | DT_WORDBREAK | DT_NOPREFIX);
    }
    let _ = SelectObject(hdc, old);
    let _ = EndPaint(hwnd, &ps);
}

unsafe fn show_buttons(hwnd: HWND, show: bool) {
    let (mut retry, mut quit, font, scale) = match state().lock() {
        Ok(st) => (st.btn_retry, st.btn_quit, st.font, st.scale),
        Err(_) => return,
    };
    if show && retry == 0 {
        let mut client = RECT::default();
        let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut client);
        let w = client.right;
        let h = client.bottom;
        let band_h = (BAND_H as f64 * scale).round() as i32;
        let panel_h = (220.0 * scale).round() as i32;
        let panel_bottom = (h - band_h + panel_h) / 2;
        let bw = (120.0 * scale).round() as i32;
        let bh = (32.0 * scale).round() as i32;
        let gap = (16.0 * scale).round() as i32;
        let y = panel_bottom - bh - (14.0 * scale).round() as i32;
        let make = |text: PCWSTR, x: i32, id: usize, default: bool| -> isize {
            let button_style: i32 = if default { BS_DEFPUSHBUTTON } else { BS_PUSHBUTTON };
            let style = WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(button_style as u32);
            match CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("BUTTON"),
                text,
                style,
                x,
                y,
                bw,
                bh,
                Some(hwnd),
                Some(HMENU(id as *mut c_void)),
                None,
                None,
            ) {
                Ok(b) => {
                    let _ = SendMessageW(b, WM_SETFONT, Some(WPARAM(font as usize)), Some(LPARAM(1)));
                    b.0 as isize
                }
                Err(_) => 0,
            }
        };
        retry = make(w!("Retry"), (w - 2 * bw - gap) / 2, ID_RETRY, true);
        quit = make(w!("Quit"), (w - 2 * bw - gap) / 2 + bw + gap, ID_QUIT, false);
        if let Ok(mut st) = state().lock() {
            st.btn_retry = retry;
            st.btn_quit = quit;
        }
    }
    for b in [retry, quit] {
        if b != 0 {
            let _ = ShowWindow(HWND(b as *mut c_void), if show { SW_SHOW } else { SW_HIDE });
        }
    }
    if show && retry != 0 {
        let _ = SetFocus(Some(HWND(retry as *mut c_void)));
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_APP_STATUS | WM_APP_IMAGE => {
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_APP_ERROR => {
            show_buttons(hwnd, wparam.0 == 1);
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xffff) as usize;
            let choice = match id {
                ID_RETRY => Some(Choice::Retry),
                ID_QUIT => Some(Choice::Quit),
                _ => None,
            };
            if let Some(c) = choice {
                if let Ok(mut st) = state().lock() {
                    st.error = None;
                    if c == Choice::Retry {
                        st.status = "Retrying".to_string();
                    }
                    if let Some(tx) = st.choice_tx.as_ref() {
                        let _ = tx.send(c);
                    }
                }
                show_buttons(hwnd, false);
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Time since the process was created, measured by the kernel (covers loader time before `main`).
pub fn since_process_start() -> Option<std::time::Duration> {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime;
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    unsafe {
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        GetProcessTimes(GetCurrentProcess(), &mut creation, &mut exit, &mut kernel, &mut user).ok()?;
        let now: FILETIME = GetSystemTimePreciseAsFileTime();
        let to_u64 = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
        let delta = to_u64(now).saturating_sub(to_u64(creation));
        Some(std::time::Duration::from_nanos(delta * 100))
    }
}

#[cfg(test)]
mod tests {
    use super::fit_into;

    #[test]
    fn splash_keeps_its_size_when_it_fits() {
        // 1920x1080 at 125 and 150 percent, taskbar 48 logical.
        assert_eq!(fit_into((1200, 750), (1920, 1020)), (1200, 750));
        assert_eq!(fit_into((1440, 900), (1920, 1008)), (1440, 900));
    }

    #[test]
    fn splash_shrinks_on_a_small_screen() {
        // 1366x768 at 125 percent, taskbar 60 px: height limits, aspect kept.
        assert_eq!(fit_into((1200, 750), (1366, 708)), (1042, 651));
    }
}
