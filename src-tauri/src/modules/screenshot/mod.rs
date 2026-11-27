//! 截图：框选区域 → 标注 → 复制 / 保存；或框选后直接识别文字
//!
//! 流程：
//! 1. 快捷键触发 → 后台线程截取鼠标所在显示器、读取窗口位置（窗口吸附用）
//! 2. 主线程把遮罩窗口铺满该显示器，通知前端；前端取走原始像素画到画布上，再请求显示窗口
//! 3. 用户框选、标注；前端合成 PNG 交回 Rust 写入剪贴板或保存
//!
//! 遮罩窗口第一次使用时创建，之后只隐藏不销毁（再次截图无需重新加载页面）；
//! 截图像素在结束后立即释放。

pub mod capture;

use crate::window;
use capture::{Capture, Display, WinRect};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use super::{CommandDef, Module};

pub const WINDOW: &str = "shot";

/// 交给前端的截图信息
#[derive(Debug, Clone, Serialize)]
pub struct StartInfo {
    seq: u64,
    /// "capture" 截图标注；"ocr" 框选后直接识别文字
    mode: String,
    /// 像素尺寸
    width: usize,
    height: usize,
    /// 逻辑尺寸（点）
    lw: f64,
    lh: f64,
    scale: f64,
    windows: Vec<WinRect>,
}

struct Session {
    seq: u64,
    capture: Capture,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);
static PENDING: Mutex<Option<StartInfo>> = Mutex::new(None);
/// 截图进行中（防止重复触发）
static BUSY: AtomicBool = AtomicBool::new(false);
static SEQ: AtomicU64 = AtomicU64::new(0);

const PRIVACY_URL: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";

pub fn start(app: &AppHandle, mode: &str) -> Result<(), String> {
    if !capture::has_permission() {
        capture::request_permission();
        let _ = std::process::Command::new("open").arg(PRIVACY_URL).spawn();
        return Err("截图需要「屏幕录制」权限：请在 系统设置 → 隐私与安全性 → 屏幕与系统录音 中允许 wtools，然后重启 wtools".into());
    }
    if BUSY.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let launcher_visible = app
        .get_webview_window(window::LAUNCHER)
        .is_some_and(|w| w.is_visible().unwrap_or(false));
    if launcher_visible {
        window::hide_launcher(app, false);
    }
    let app = app.clone();
    let mode = mode.to_string();
    std::thread::spawn(move || {
        // 等搜索框从屏幕上消失，避免被截进去
        if launcher_visible {
            std::thread::sleep(Duration::from_millis(120));
        }
        match grab(&mode) {
            Ok(display) => {
                let handle = app.clone();
                let _ = app.run_on_main_thread(move || {
                    if let Err(e) = present(&handle, &display) {
                        eprintln!("[screenshot] {e}");
                        reset();
                    }
                });
            }
            Err(e) => {
                eprintln!("[screenshot] {e}");
                reset();
            }
        }
    });
    Ok(())
}

/// 截屏并记录本次会话
fn grab(mode: &str) -> Result<Display, String> {
    let display = capture::display_under_cursor();
    let windows = capture::windows_on(&display);
    let cap = Capture::display(&display)?;
    let b = display.bounds.size;
    let seq = SEQ.fetch_add(1, Ordering::SeqCst) + 1;
    let info = StartInfo {
        seq,
        mode: mode.to_string(),
        width: cap.width,
        height: cap.height,
        lw: b.width,
        lh: b.height,
        scale: cap.width as f64 / b.width,
        windows,
    };
    *SESSION.lock().unwrap() = Some(Session { seq, capture: cap });
    *PENDING.lock().unwrap() = Some(info);
    Ok(display)
}

/// 主线程：准备遮罩窗口并通知前端加载截图（前端加载完后调用 shot_show 才显示）
fn present(app: &AppHandle, display: &Display) -> Result<(), String> {
    let win = match app.get_webview_window(WINDOW) {
        Some(w) => w,
        None => create_window(app)?,
    };
    #[cfg(target_os = "macos")]
    cover_display(&win, display);
    win.emit_to(WINDOW, "shot:start", ()).map_err(|e| e.to_string())
}

fn create_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    let win = WebviewWindowBuilder::new(app, WINDOW, WebviewUrl::App("shot.html".into()))
        .title("wtools 截图")
        .decorations(false)
        .resizable(false)
        .visible(false)
        .skip_taskbar(true)
        .shadow(false)
        .always_on_top(true)
        .accept_first_mouse(true)
        .build()
        .map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    if let Ok(ptr) = win.ns_window() {
        use objc2_app_kit::{NSWindow, NSWindowAnimationBehavior, NSWindowCollectionBehavior};
        // SAFETY: 在主线程调用，指针由 Tauri 保证有效
        let ns: &NSWindow = unsafe { &*(ptr as *const NSWindow) };
        // 盖住菜单栏和 Dock；在全屏应用的空间里也能出现
        ns.setLevel(1000);
        ns.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        ns.setAnimationBehavior(NSWindowAnimationBehavior::None);
        ns.setHasShadow(false);
        ns.setMovable(false);
    }
    Ok(win)
}

/// 窗口铺满指定显示器（AppKit 坐标原点在主屏左下角）
#[cfg(target_os = "macos")]
fn cover_display(win: &WebviewWindow, d: &Display) {
    use objc2_app_kit::NSWindow;
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    let Ok(ptr) = win.ns_window() else { return };
    // SAFETY: 在主线程调用，指针由 Tauri 保证有效
    let ns: &NSWindow = unsafe { &*(ptr as *const NSWindow) };
    let b = d.bounds;
    let y = capture::main_display_height() - b.origin.y - b.size.height;
    ns.setFrame_display(
        NSRect::new(NSPoint::new(b.origin.x, y), NSSize::new(b.size.width, b.size.height)),
        false,
    );
}

fn reset() {
    *SESSION.lock().unwrap() = None;
    *PENDING.lock().unwrap() = None;
    BUSY.store(false, Ordering::SeqCst);
}

/// 结束截图：隐藏遮罩、释放截图。`restore_focus` 为 true 时把焦点还给之前的应用。
fn finish(app: &AppHandle, restore_focus: bool) {
    if let Some(w) = app.get_webview_window(WINDOW) {
        let _ = w.hide();
    }
    reset();
    if restore_focus {
        window::yield_focus(app);
    }
}

fn save_dir() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    // 跟随系统截图的保存位置
    let custom = std::process::Command::new("defaults")
        .args(["read", "com.apple.screencapture", "location"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().replacen('~', &home, 1))
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_dir());
    custom.unwrap_or_else(|| std::path::Path::new(&home).join("Desktop"))
}

fn timestamp() -> String {
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        format!(
            "{}-{:02}-{:02} {:02}.{:02}.{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        )
    }
}

pub struct ScreenshotModule;

impl Module for ScreenshotModule {
    fn id(&self) -> &'static str {
        "screenshot"
    }
    fn name(&self) -> &'static str {
        "截图"
    }

    fn commands(&self) -> Vec<CommandDef> {
        vec![
            CommandDef {
                key: "capture",
                title: "截图",
                subtitle: "框选区域或窗口，标注后复制或保存",
                icon: "✂️",
                keywords: &["screenshot", "jietu", "截屏", "capture", "snip"],
                view: None,
                hidden: false,
            },
            CommandDef {
                key: "ocr",
                title: "截图识别文字",
                subtitle: "框选区域，识别其中的文字（OCR）",
                icon: "🔤",
                keywords: &["ocr", "shibie", "文字识别", "jietu", "截图"],
                view: None,
                hidden: false,
            },
        ]
    }

    fn execute(&self, key: &str, app: &AppHandle) -> Result<(), String> {
        match key {
            "capture" | "ocr" => start(app, key),
            _ => Err(format!("未知命令 {key}")),
        }
    }
}

pub mod commands {
    use super::*;
    use crate::modules::clipboard::{pasteboard, ClipboardService};
    use std::sync::Arc;
    use tauri::ipc::{InvokeBody, Request, Response};

    fn body(request: &Request<'_>) -> Result<Vec<u8>, String> {
        match request.body() {
            InvokeBody::Raw(bytes) if !bytes.is_empty() => Ok(bytes.clone()),
            _ => Err("没有收到图片数据".into()),
        }
    }

    /// 前端加载完 / 收到 shot:start 后调用，取走本次截图信息
    #[tauri::command]
    pub fn shot_take_pending() -> Option<StartInfo> {
        PENDING.lock().unwrap().take()
    }

    /// 截图的 RGBA 像素（原始字节，前端直接放进 ImageData，省去编解码）
    #[tauri::command]
    pub async fn shot_image(seq: u64) -> Result<Response, String> {
        tauri::async_runtime::spawn_blocking(move || {
            let guard = SESSION.lock().unwrap();
            let s = guard.as_ref().filter(|s| s.seq == seq).ok_or("截图已失效")?;
            s.capture.rgba().map(Response::new)
        })
        .await
        .map_err(|e| e.to_string())?
    }

    /// 截图已画好，显示遮罩窗口
    #[tauri::command]
    pub fn shot_show(app: AppHandle) -> Result<(), String> {
        let w = app.get_webview_window(WINDOW).ok_or("截图窗口不存在")?;
        w.show().map_err(|e| e.to_string())?;
        w.set_focus().map_err(|e| e.to_string())
    }

    #[tauri::command]
    pub fn shot_cancel(app: AppHandle) {
        finish(&app, true);
    }

    /// 复制合成好的 PNG（请求体为图片字节）。不标记为「自己写入」，会照常进入剪贴板历史。
    #[tauri::command]
    pub fn shot_copy(request: Request<'_>, app: AppHandle) -> Result<(), String> {
        let png = body(&request)?;
        finish(&app, true);
        pasteboard::write(&pasteboard::Content::Image { png, width: 0, height: 0 }).map(|_| ())
    }

    /// 保存合成好的 PNG：先关闭遮罩，再弹出保存对话框。返回保存路径，取消时返回 None。
    #[tauri::command]
    pub async fn shot_save(request: Request<'_>, app: AppHandle) -> Result<Option<String>, String> {
        let png = body(&request)?;
        finish(&app, false);
        tauri::async_runtime::spawn_blocking(move || {
            let dir = save_dir();
            let name = format!("截图 {}.png", timestamp());
            let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
            let script = format!(
                r#"POSIX path of (choose file name with prompt "保存截图" default name "{}" default location (POSIX file "{}"))"#,
                esc(&name),
                esc(&dir.to_string_lossy())
            );
            let out = std::process::Command::new("osascript")
                .args(["-e", &script])
                .output()
                .map_err(|e| e.to_string())?;
            if !out.status.success() {
                return Ok(None); // 用户取消
            }
            let mut path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !path.to_lowercase().ends_with(".png") {
                path.push_str(".png");
            }
            std::fs::write(&path, png).map_err(|e| format!("保存失败: {e}"))?;
            Ok(Some(path))
        })
        .await
        .map_err(|e| e.to_string())?
    }

    /// 识别选区内的文字（像素坐标）
    #[tauri::command]
    pub async fn shot_ocr(seq: u64, x: f64, y: f64, w: f64, h: f64) -> Result<crate::ocr::OcrResult, String> {
        tauri::async_runtime::spawn_blocking(move || {
            let png = {
                let guard = SESSION.lock().unwrap();
                let s = guard.as_ref().filter(|s| s.seq == seq).ok_or("截图已失效")?;
                s.capture.crop_png(x, y, w, h)?
            };
            let path = std::env::temp_dir().join(format!("wtools-shot-ocr-{}-{seq}.png", std::process::id()));
            std::fs::write(&path, png).map_err(|e| e.to_string())?;
            let result = crate::ocr::recognize_file(&path);
            let _ = std::fs::remove_file(&path);
            result
        })
        .await
        .map_err(|e| e.to_string())?
    }

    /// 复制或粘贴文字（识别结果、颜色值），同时结束截图；文字会记入剪贴板历史
    #[tauri::command]
    pub fn shot_text(
        text: String,
        paste: bool,
        app: AppHandle,
        svc: tauri::State<'_, Arc<ClipboardService>>,
    ) -> Result<(), String> {
        finish(&app, false);
        svc.record_text(&app, &text, "截图");
        svc.deliver(&app, vec![pasteboard::Content::Text(text)], paste)
    }
}
