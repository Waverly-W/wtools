//! 窗口管理：搜索框常驻预热（只隐藏不销毁），设置等工具窗口按需创建、关闭即销毁

use std::sync::Mutex;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

pub const LAUNCHER: &str = "launcher";
pub const SETTINGS: &str = "settings";
pub const SEARCH_VIEW: &str = "search";

/// 搜索框当前显示的视图
static CURRENT_VIEW: Mutex<String> = Mutex::new(String::new());

pub fn set_current_view(view: &str) {
    *CURRENT_VIEW.lock().unwrap() = view.to_string();
}

fn current_view() -> String {
    CURRENT_VIEW.lock().unwrap().clone()
}

/// 搜索框逻辑宽度，需与 tauri.conf.json、前端 win.ts 保持一致
const LAUNCHER_WIDTH: f64 = 1080.0;
/// 搜索框可能达到的最大逻辑高度（剪贴板视图），用于保证窗口不超出屏幕
const LAUNCHER_MAX_HEIGHT: f64 = 780.0;
/// 与屏幕边缘保持的最小距离
const SCREEN_MARGIN: f64 = 24.0;

fn launcher(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(LAUNCHER)
}

/// 启动时调用：调整 macOS 窗口行为
pub fn setup_launcher(app: &AppHandle) {
    let Some(win) = launcher(app) else {
        eprintln!("[window] 未找到 launcher 窗口");
        return;
    };
    #[cfg(target_os = "macos")]
    tune_macos_window(&win);
    let _ = win;
}

/// 让搜索框在所有桌面（包括其他应用的全屏空间）上都能出现
#[cfg(target_os = "macos")]
fn tune_macos_window(win: &WebviewWindow) {
    use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior};
    if let Ok(ptr) = win.ns_window() {
        // SAFETY: setup 在主线程执行，指针由 Tauri 保证指向有效的 NSWindow
        let ns: &NSWindow = unsafe { &*(ptr as *const NSWindow) };
        ns.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
    }
}

/// 快捷键触发：同一视图已显示则隐藏，否则显示（或切换到）该视图
pub fn toggle_launcher(app: &AppHandle, view: &str) {
    match launcher(app) {
        Some(w) if w.is_visible().unwrap_or(false) && current_view() == view => {
            hide_launcher(app, true)
        }
        Some(_) => show_launcher(app, view),
        None => {}
    }
}

pub fn show_launcher(app: &AppHandle, view: &str) {
    let Some(win) = launcher(app) else { return };
    if !win.is_visible().unwrap_or(false) {
        position_on_cursor_monitor(app, &win);
    }
    set_current_view(view);
    let _ = win.emit_to(LAUNCHER, "launcher:show", view);
    let _ = win.show();
    let _ = win.set_focus();
}

/// `restore_focus`：是否把焦点还给之前的应用（Esc / 执行命令时需要；失焦触发时焦点已经在别处，不需要）
pub fn hide_launcher(app: &AppHandle, restore_focus: bool) {
    let Some(win) = launcher(app) else { return };
    let _ = win.hide();
    let _ = win.emit_to(LAUNCHER, "launcher:hide", ());
    if restore_focus {
        yield_focus(app);
    }
}

/// 没有其他可见窗口时隐藏整个应用，系统会把焦点还给之前的应用
pub fn yield_focus(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    {
        let others_visible = app
            .webview_windows()
            .values()
            .any(|w| w.label() != LAUNCHER && w.is_visible().unwrap_or(false));
        if !others_visible {
            let _ = app.hide();
        }
    }
    let _ = app;
}

/// 显示在鼠标所在屏幕的水平居中、上方 1/4 处；屏幕较矮时上移，保证最大高度也能完整显示
fn position_on_cursor_monitor(app: &AppHandle, win: &WebviewWindow) {
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| win.current_monitor().ok().flatten());
    let Some(m) = monitor else { return };
    let scale = m.scale_factor();
    let (mp, ms) = (m.position(), m.size());
    let width = (LAUNCHER_WIDTH * scale) as i32;
    let max_height = (LAUNCHER_MAX_HEIGHT * scale) as i32;
    let margin = (SCREEN_MARGIN * scale) as i32;
    let x = mp.x + ((ms.width as i32 - width) / 2).max(0);
    let top = (ms.height as i32 / 4)
        .min(ms.height as i32 - max_height - margin)
        .max(margin);
    let y = mp.y + top;
    let _ = win.set_position(PhysicalPosition::new(x, y));
}

pub fn open_settings(app: &AppHandle) -> Result<(), String> {
    open_tool(app, SETTINGS, "settings.html", "wtools 设置", (800.0, 580.0), (640.0, 420.0))
}

/// 打开工具窗口：已存在则唤到前台，否则新建（关闭即销毁，不常驻内存）
pub fn open_tool(
    app: &AppHandle,
    label: &str,
    url: &str,
    title: &str,
    size: (f64, f64),
    min_size: (f64, f64),
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let _ = app.show();
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.unminimize();
        let _ = w.show();
        return w.set_focus().map_err(|e| e.to_string());
    }
    let w = WebviewWindowBuilder::new(app, label, WebviewUrl::App(url.into()))
        .title(title)
        .inner_size(size.0, size.1)
        .min_inner_size(min_size.0, min_size.1)
        .center()
        .build()
        .map_err(|e| e.to_string())?;
    w.set_focus().map_err(|e| e.to_string())
}
