//! 常用系统命令（macOS）

use super::{CommandDef, Module};
use std::process::Command;
use tauri::AppHandle;

pub struct System;

impl Module for System {
    fn id(&self) -> &'static str {
        "system"
    }
    fn name(&self) -> &'static str {
        "系统命令"
    }

    fn commands(&self) -> Vec<CommandDef> {
        vec![
            cmd("lock", "锁屏", "立即锁定屏幕", "🔒", &["lock", "lock screen"]),
            cmd("sleep", "睡眠", "让电脑进入睡眠", "🌙", &["sleep"]),
            cmd("display-sleep", "关闭显示器", "让显示器进入睡眠", "🖥️", &["display sleep", "screen off"]),
            cmd("empty-trash", "清空废纸篓", "", "🗑️", &["empty trash", "回收站"]),
            cmd("dark-mode", "切换深色模式", "在浅色与深色外观间切换", "🌓", &["dark mode", "appearance", "暗黑"]),
            cmd("restart", "重启", "重新启动电脑", "🔄", &["restart", "reboot"]),
            cmd("shutdown", "关机", "", "⏼", &["shutdown", "power off"]),
            cmd("logout", "注销", "退出当前用户登录", "🚪", &["logout", "log out"]),
        ]
    }

    fn execute(&self, key: &str, _app: &AppHandle) -> Result<(), String> {
        match key {
            "lock" => lock_screen(),
            "sleep" => spawn("pmset", &["sleepnow"]),
            "display-sleep" => spawn("pmset", &["displaysleepnow"]),
            "empty-trash" => osascript(r#"tell application "Finder" to empty trash"#),
            "dark-mode" => osascript(
                r#"tell application "System Events" to tell appearance preferences to set dark mode to not dark mode"#,
            ),
            "restart" => osascript(r#"tell application "System Events" to restart"#),
            "shutdown" => osascript(r#"tell application "System Events" to shut down"#),
            "logout" => osascript(r#"tell application "System Events" to log out"#),
            _ => Err(format!("未知命令 {key}")),
        }
    }
}

const fn cmd(
    key: &'static str,
    title: &'static str,
    subtitle: &'static str,
    icon: &'static str,
    keywords: &'static [&'static str],
) -> CommandDef {
    CommandDef {
        key,
        title,
        subtitle,
        icon,
        keywords,
        view: None,
        hidden: false,
    }
}

/// 后台执行，不阻塞界面；在独立线程里 wait，避免僵尸进程
fn spawn(program: &str, args: &[&str]) -> Result<(), String> {
    let mut child = Command::new(program)
        .args(args)
        .spawn()
        .map_err(|e| format!("执行 {program} 失败: {e}"))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

fn osascript(script: &str) -> Result<(), String> {
    spawn("osascript", &["-e", script])
}

/// 锁屏：调用系统 login.framework 的 SACLockScreenImmediate（不需要辅助功能权限），
/// 失败时退回到「关闭显示器」（系统默认在显示器睡眠后立即要求密码）。
fn lock_screen() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    unsafe {
        let path = c"/System/Library/PrivateFrameworks/login.framework/Versions/Current/login";
        let handle = libc::dlopen(path.as_ptr(), libc::RTLD_LAZY);
        if !handle.is_null() {
            let sym = libc::dlsym(handle, c"SACLockScreenImmediate".as_ptr());
            if !sym.is_null() {
                let f: extern "C" fn() -> i32 = std::mem::transmute(sym);
                f();
                return Ok(());
            }
        }
    }
    spawn("pmset", &["displaysleepnow"])
}
