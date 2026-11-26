//! 暴露给前端的命令

use crate::{
    config::{ConfigStore, LAUNCHER_TOGGLE},
    hotkey::{self, HotkeyState},
    modules::{Item, Registry},
    window,
};
use serde::Serialize;
use tauri::{AppHandle, State};

#[tauri::command]
pub fn search(query: String, registry: State<'_, Registry>) -> Vec<Item> {
    registry.search(&query)
}

/// 从搜索框执行：视图类命令（如剪贴板）在搜索框内切换；其他命令先隐藏搜索框并归还焦点，再执行
#[tauri::command]
pub fn execute(id: String, app: AppHandle, registry: State<'_, Registry>) -> Result<(), String> {
    if let Some(view) = registry.view_of(&id) {
        window::show_launcher(&app, view);
        return Ok(());
    }
    window::hide_launcher(&app, true);
    registry.execute(&id, &app)
}

/// 前端在视图间切换时同步给 Rust（快捷键切换/隐藏的判断依赖它）
#[tauri::command]
pub fn set_launcher_view(view: String) {
    window::set_current_view(&view);
}

#[tauri::command]
pub fn hide_launcher(app: AppHandle) {
    window::hide_launcher(&app, true);
}

#[derive(Serialize)]
pub struct HotkeyRow {
    id: String,
    title: String,
    module: String,
    accelerator: Option<String>,
    error: Option<String>,
}

#[tauri::command]
pub fn list_hotkeys(
    registry: State<'_, Registry>,
    store: State<'_, ConfigStore>,
    hk: State<'_, HotkeyState>,
) -> Vec<HotkeyRow> {
    let cfg = store.get();
    let errors = hk.errors();
    let row = |id: String, title: String, module: String| HotkeyRow {
        accelerator: cfg.hotkeys.get(&id).cloned(),
        error: errors.get(&id).cloned(),
        id,
        title,
        module,
    };
    let mut rows = vec![row(
        LAUNCHER_TOGGLE.into(),
        "呼出 / 隐藏搜索框".into(),
        "wtools".into(),
    )];
    rows.extend(
        registry
            .bindable_commands()
            .into_iter()
            .map(|c| row(c.id, c.title, c.module)),
    );
    rows
}

#[tauri::command]
pub fn set_hotkey(id: String, accelerator: Option<String>, app: AppHandle) -> Result<(), String> {
    hotkey::set(&app, &id, accelerator)
}

/// 录制快捷键时暂停（true）/ 恢复（false）全局快捷键
#[tauri::command]
pub fn suspend_hotkeys(suspend: bool, app: AppHandle) {
    if suspend {
        hotkey::suspend(&app);
    } else {
        hotkey::apply_all(&app);
    }
}

