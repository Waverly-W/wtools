//! 开机自启：系统「登录项」（SMAppService，macOS 13+）
//!
//! 状态以系统为准，不另存配置：用户在「系统设置 → 通用 → 登录项」里关掉后，设置页也会同步显示为关闭。

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct AutostartStatus {
    pub enabled: bool,
    /// 已注册，但需要用户在系统设置里允许
    pub needs_approval: bool,
    /// 当前运行的 wtools 所在路径（登录时启动的就是这一份）
    pub app_path: String,
}

fn app_path() -> String {
    // .../wtools.app/Contents/MacOS/wtools → .../wtools.app
    std::env::current_exe()
        .ok()
        .and_then(|p| p.ancestors().nth(3).map(|a| a.to_string_lossy().into_owned()))
        .unwrap_or_default()
}

#[cfg(target_os = "macos")]
mod imp {
    use objc2_service_management::{SMAppService, SMAppServiceStatus};

    pub fn status() -> SMAppServiceStatus {
        unsafe { SMAppService::mainAppService().status() }
    }

    pub fn set(enabled: bool) -> Result<(), String> {
        let svc = unsafe { SMAppService::mainAppService() };
        let r = unsafe {
            if enabled {
                svc.registerAndReturnError()
            } else {
                svc.unregisterAndReturnError()
            }
        };
        r.map_err(|e| e.localizedDescription().to_string())
    }

    pub fn open_settings() {
        unsafe { SMAppService::openSystemSettingsLoginItems() }
    }
}

#[tauri::command]
pub fn autostart_status() -> AutostartStatus {
    #[cfg(target_os = "macos")]
    {
        use objc2_service_management::SMAppServiceStatus as S;
        let s = imp::status();
        AutostartStatus {
            enabled: s == S::Enabled,
            needs_approval: s == S::RequiresApproval,
            app_path: app_path(),
        }
    }
    #[cfg(not(target_os = "macos"))]
    AutostartStatus { enabled: false, needs_approval: false, app_path: app_path() }
}

#[tauri::command]
pub fn autostart_set(enabled: bool) -> Result<AutostartStatus, String> {
    #[cfg(target_os = "macos")]
    {
        let current = autostart_status();
        // 已是目标状态时不重复调用（重复注销会报「未找到」）
        if enabled != (current.enabled || current.needs_approval) {
            imp::set(enabled)?;
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = enabled;
    Ok(autostart_status())
}

/// 打开「系统设置 → 通用 → 登录项」
#[tauri::command]
pub fn autostart_open_settings() {
    #[cfg(target_os = "macos")]
    imp::open_settings();
}
