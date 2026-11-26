//! 全局快捷键：配置中的 动作ID → 快捷键，统一注册与分发

use crate::{
    config::{ConfigStore, LAUNCHER_TOGGLE},
    modules::Registry,
    window,
};
use std::{collections::HashMap, sync::Mutex};
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

#[derive(Default)]
pub struct HotkeyState {
    /// 快捷键 id → 动作 ID
    bound: Mutex<HashMap<u32, String>>,
    /// 动作 ID → 注册失败原因（供设置页展示）
    errors: Mutex<HashMap<String, String>>,
}

impl HotkeyState {
    pub fn errors(&self) -> HashMap<String, String> {
        self.errors.lock().unwrap().clone()
    }
}

pub fn parse(accel: &str) -> Result<Shortcut, String> {
    accel
        .parse::<Shortcut>()
        .map_err(|e| format!("快捷键格式无效「{accel}」: {e}"))
}

/// 全局快捷键回调（插件在主线程调用）
pub fn on_shortcut(app: &AppHandle, shortcut: &Shortcut, event: ShortcutEvent) {
    if event.state != ShortcutState::Pressed {
        return;
    }
    let action = app
        .state::<HotkeyState>()
        .bound
        .lock()
        .unwrap()
        .get(&shortcut.id())
        .cloned();
    if let Some(action) = action {
        dispatch(app, &action);
    }
}

fn dispatch(app: &AppHandle, action: &str) {
    if action == LAUNCHER_TOGGLE {
        window::toggle_launcher(app, window::SEARCH_VIEW);
        return;
    }
    let registry = app.state::<Registry>();
    if let Some(view) = registry.view_of(action) {
        window::toggle_launcher(app, view);
        return;
    }
    if let Err(e) = registry.execute(action, app) {
        eprintln!("[hotkey] 执行 {action} 失败: {e}");
    }
}

/// 按当前配置重新注册全部快捷键，返回注册失败的动作
pub fn apply_all(app: &AppHandle) -> HashMap<String, String> {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let state = app.state::<HotkeyState>();
    let mut bound = state.bound.lock().unwrap();
    let mut errors = HashMap::new();
    bound.clear();

    for (action, accel) in app.state::<ConfigStore>().get().hotkeys {
        let result = parse(&accel).and_then(|sc| {
            gs.register(sc)
                .map(|_| sc)
                .map_err(|e| format!("注册失败（可能被其他应用占用）: {e}"))
        });
        match result {
            Ok(sc) => {
                bound.insert(sc.id(), action);
            }
            Err(e) => {
                eprintln!("[hotkey] {action} = {accel}: {e}");
                errors.insert(action, e);
            }
        }
    }
    *state.errors.lock().unwrap() = errors.clone();
    errors
}

/// 设置页录制快捷键期间暂停全部全局快捷键，否则按下已绑定的组合会被拦截
pub fn suspend(app: &AppHandle) {
    let _ = app.global_shortcut().unregister_all();
    app.state::<HotkeyState>().bound.lock().unwrap().clear();
}

/// 设置或清除某个动作的快捷键
pub fn set(app: &AppHandle, action: &str, accel: Option<String>) -> Result<(), String> {
    let registry = app.state::<Registry>();
    if action != LAUNCHER_TOGGLE && !registry.has_command(action) {
        return Err(format!("未知动作 {action}"));
    }
    if action == LAUNCHER_TOGGLE && accel.is_none() {
        return Err("呼出搜索框的快捷键不能为空".into());
    }

    let store = app.state::<ConfigStore>();
    let previous = store.get().hotkeys.get(action).cloned();

    if let Some(accel) = &accel {
        let id = parse(accel)?.id();
        let conflict = store.get().hotkeys.into_iter().find(|(other, other_accel)| {
            other != action && parse(other_accel).map(|s| s.id()).ok() == Some(id)
        });
        if let Some((other, _)) = conflict {
            return Err(format!("与「{}」的快捷键冲突", action_title(app, &other)));
        }
    }

    let write = |value: Option<String>| {
        store.update(|cfg| match value {
            Some(v) => {
                cfg.hotkeys.insert(action.to_string(), v);
            }
            None => {
                cfg.hotkeys.remove(action);
            }
        })
    };

    write(accel)?;
    let errors = apply_all(app);
    if let Some(err) = errors.get(action) {
        // 注册失败则回滚，避免把不可用的快捷键留在配置里
        let err = err.clone();
        write(previous)?;
        apply_all(app);
        return Err(err);
    }
    Ok(())
}

pub fn action_title(app: &AppHandle, action: &str) -> String {
    if action == LAUNCHER_TOGGLE {
        return "呼出搜索框".into();
    }
    app.state::<Registry>()
        .bindable_commands()
        .into_iter()
        .find(|c| c.id == action)
        .map(|c| c.title)
        .unwrap_or_else(|| action.to_string())
}
