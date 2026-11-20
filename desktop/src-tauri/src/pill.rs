//! The push-to-talk pill: a small box at the bottom of the screen while the shortcut is held,
//! with the voice's waveform, then "Transcribing" until the words are typed. It floats over
//! every app and never takes the focus, so the words still go where the cursor is.
use std::{
    sync::atomic::{AtomicU64, Ordering::SeqCst},
    time::Duration,
};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

const LABEL: &str = "pill";
/// The window: the 200 × 40 pill with room around it for its shadow.
const WIDTH: f64 = 224.0;
const HEIGHT: f64 = 64.0;
/// Above the taskbar.
const GAP: f64 = 16.0;
/// How long "Didn't catch that" stays.
pub const MESSAGE: Duration = Duration::from_millis(1600);

/// Counts the times the pill was shown, so a late hide never hides the next take's pill.
static SHOWN: AtomicU64 = AtomicU64::new(0);

/// Made at start, so it is ready the moment the shortcut is pressed. On Windows it is shown
/// once, off the screen, and only moved on and off it after: a window hidden and shown again is
/// not drawn again by its web view.
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::default())
        .title("3Yiddish push-to-talk")
        .inner_size(WIDTH, HEIGHT)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .focusable(false)
        .visible(false)
        .build()?;
    // Clicks go through it to whatever is underneath.
    let _ = window.set_ignore_cursor_events(true);
    win::prepare(&window);
    Ok(())
}

/// Show it at the bottom of the screen the mouse is on.
pub fn show(app: &AppHandle) {
    SHOWN.fetch_add(1, SeqCst);
    let Some(window) = app.get_webview_window(LABEL) else { return };
    let Ok(at) = app.cursor_position() else { return };
    let Some(screen) = app
        .monitor_from_point(at.x, at.y)
        .ok()
        .flatten()
        .or_else(|| app.primary_monitor().ok().flatten())
    else {
        return;
    };
    let area = screen.work_area();
    let scale = screen.scale_factor();
    let (w, h) = ((WIDTH * scale).round() as i32, (HEIGHT * scale).round() as i32);
    let x = area.position.x + (area.size.width as i32 - w) / 2;
    let y = area.position.y + area.size.height as i32 - h - (GAP * scale).round() as i32;
    // On the app's main thread: there the pixels are the screen's own, wherever it is scaled.
    let _ = app.run_on_main_thread(move || win::place(&window, x, y, w, h));
}

/// Hide it now, or after `after` unless another take has shown it since.
pub fn hide(app: &AppHandle, after: Duration) {
    if after.is_zero() {
        if let Some(window) = app.get_webview_window(LABEL) {
            let _ = app.run_on_main_thread(move || win::away(&window));
        }
        return;
    }
    let shown = SHOWN.load(SeqCst);
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(after);
        if SHOWN.load(SeqCst) == shown {
            hide(&app, Duration::ZERO);
        }
    });
}

#[cfg(windows)]
mod win {
    use tauri::WebviewWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowLongW, HWND_TOPMOST, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOSIZE,
        SWP_SHOWWINDOW, SetWindowLongW, SetWindowPos, ShowWindow, WS_EX_APPWINDOW,
        WS_EX_TOOLWINDOW,
    };

    /// Where it waits, out of sight.
    const AWAY: i32 = -32000;

    fn hwnd(window: &WebviewWindow) -> Option<windows_sys::Win32::Foundation::HWND> {
        window.hwnd().ok().map(|h| h.0 as _)
    }

    /// Kept out of Alt + Tab, and shown once, off the screen, without taking the focus.
    pub fn prepare(window: &WebviewWindow) {
        let Some(hwnd) = hwnd(window) else { return };
        unsafe {
            let style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
            SetWindowLongW(hwnd, GWL_EXSTYLE, ((style & !WS_EX_APPWINDOW) | WS_EX_TOOLWINDOW) as i32);
            SetWindowPos(hwnd, HWND_TOPMOST, AWAY, AWAY, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE);
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
    }

    /// On the screen at this place and size (physical pixels), above everything, focus untouched.
    pub fn place(window: &WebviewWindow, x: i32, y: i32, w: i32, h: i32) {
        let Some(hwnd) = hwnd(window) else { return };
        unsafe { SetWindowPos(hwnd, HWND_TOPMOST, x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW) };
    }

    pub fn away(window: &WebviewWindow) {
        let Some(hwnd) = hwnd(window) else { return };
        unsafe { SetWindowPos(hwnd, HWND_TOPMOST, AWAY, AWAY, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE) };
    }
}

#[cfg(not(windows))]
mod win {
    use tauri::{PhysicalPosition, PhysicalSize, WebviewWindow};

    pub fn prepare(_: &WebviewWindow) {}

    pub fn place(window: &WebviewWindow, x: i32, y: i32, w: i32, h: i32) {
        let _ = window.set_position(PhysicalPosition::new(x, y));
        let _ = window.set_size(PhysicalSize::new(w as u32, h as u32));
        let _ = window.show();
    }

    pub fn away(window: &WebviewWindow) {
        let _ = window.hide();
    }
}
