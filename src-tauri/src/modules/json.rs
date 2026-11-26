//! JSON 工具：格式化 / 转义 / 对比
//!
//! 计算全部在前端完成（大文本不经过 IPC），Rust 只负责打开窗口、传递要打开的内容。
//! 窗口按需创建、关闭即销毁，不占常驻内存。

use super::{CommandDef, Module};
use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

pub const WINDOW: &str = "json";

/// 打开窗口时要切到的页签和要载入的内容。
/// 窗口刚创建时前端还没开始监听事件，所以先存在这里，由前端加载完后主动来取。
#[derive(Debug, Clone, Serialize)]
pub struct Pending {
    mode: String,
    text: Option<String>,
    /// 对比页的右侧内容（从剪贴板历史选两条对比时）
    right: Option<String>,
}

static PENDING: Mutex<Option<Pending>> = Mutex::new(None);

pub fn open(app: &AppHandle, mode: &str, text: Option<String>, right: Option<String>) -> Result<(), String> {
    *PENDING.lock().unwrap() = Some(Pending { mode: mode.to_string(), text, right });
    if let Some(w) = app.get_webview_window(WINDOW) {
        let _ = w.emit_to(WINDOW, "json:open", ());
    }
    crate::window::open_tool(app, WINDOW, "json.html", "JSON 工具", (1100.0, 720.0), (720.0, 480.0))
}

pub struct JsonModule;

impl Module for JsonModule {
    fn id(&self) -> &'static str {
        "json"
    }
    fn name(&self) -> &'static str {
        "JSON"
    }

    fn commands(&self) -> Vec<CommandDef> {
        vec![
            CommandDef {
                key: "format",
                title: "JSON 格式化",
                subtitle: "格式化 / 压缩 / 转义，自动读取剪贴板",
                icon: "🧾",
                keywords: &["json", "format", "geshihua", "转义", "escape", "压缩"],
                view: None,
                hidden: false,
            },
            CommandDef {
                key: "diff",
                title: "JSON 对比",
                subtitle: "比较两段 JSON 的差异，忽略键的顺序",
                icon: "🔀",
                keywords: &["json", "diff", "compare", "比较", "duibi"],
                view: None,
                hidden: false,
            },
        ]
    }

    fn execute(&self, key: &str, app: &AppHandle) -> Result<(), String> {
        match key {
            "format" | "diff" => open(app, key, None, None),
            _ => Err(format!("未知命令 {key}")),
        }
    }
}

pub mod commands {
    use super::*;
    use crate::modules::clipboard::pasteboard::{self, Content};

    /// 前端加载完 / 收到 json:open 后调用，取走要打开的页签和内容
    #[tauri::command]
    pub fn json_take_pending() -> Option<Pending> {
        PENDING.lock().unwrap().take()
    }

    /// 从剪贴板历史等地方打开 JSON 工具并载入文本
    #[tauri::command]
    pub fn json_open(
        mode: String,
        text: Option<String>,
        right: Option<String>,
        app: AppHandle,
    ) -> Result<(), String> {
        crate::window::hide_launcher(&app, false);
        open(&app, &mode, text, right)
    }

    /// 当前剪贴板里的文本（用于打开时自动载入）
    #[tauri::command]
    pub fn json_clipboard_text() -> Option<String> {
        match pasteboard::read()? {
            Content::Text(s) => Some(s),
            _ => None,
        }
    }
}
