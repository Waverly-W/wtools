//! 阻止「音乐」自动打开（替代 noTunes）
//!
//! 蓝牙耳机连接、按播放键时系统会拉起「音乐」。监听应用启动通知，是「音乐」就立刻强制退出。
//! 只靠系统通知触发，不轮询；关闭功能时观察者保留，仅跳过处理。

use objc2_app_kit::{
    NSRunningApplication, NSWorkspace, NSWorkspaceApplicationKey, NSWorkspaceDidLaunchApplicationNotification,
    NSWorkspaceWillLaunchApplicationNotification,
};
use objc2_foundation::NSNotification;
use serde::{Deserialize, Serialize};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Once;
use tauri::AppHandle;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MusicBlockConfig {
    pub enabled: bool,
}

const BLOCKED: &[&str] = &["com.apple.Music", "com.apple.iTunes"];

static ENABLED: AtomicBool = AtomicBool::new(false);
static OBSERVER: Once = Once::new();

pub fn apply(app: &AppHandle, cfg: &MusicBlockConfig) {
    ENABLED.store(cfg.enabled, Ordering::Relaxed);
    if cfg.enabled {
        let _ = app.run_on_main_thread(|| OBSERVER.call_once(register));
    }
}

fn register() {
    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    let block = block2::RcBlock::new(|n: NonNull<NSNotification>| {
        if !ENABLED.load(Ordering::Relaxed) {
            return;
        }
        let app = unsafe { n.as_ref() }
            .userInfo()
            .and_then(|info| info.objectForKey(unsafe { NSWorkspaceApplicationKey }))
            .and_then(|o| o.downcast::<NSRunningApplication>().ok());
        if let Some(app) = app {
            if app.bundleIdentifier().is_some_and(|b| BLOCKED.contains(&&*b.to_string())) {
                app.forceTerminate();
            }
        }
    });
    // 「即将启动」最早拦下；有的启动方式只发「已启动」，两个都听
    for name in unsafe { [NSWorkspaceWillLaunchApplicationNotification, NSWorkspaceDidLaunchApplicationNotification] } {
        let token = unsafe { center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block) };
        // 与应用同生命周期
        std::mem::forget(token);
    }
}
