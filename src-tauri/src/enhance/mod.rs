//! 系统增强：常驻后台的系统级小功能（替代 Stats / Mos / Input Source Pro / TypeRelay / AltTab / noTunes）
//!
//! 每个功能独立开关，配置保存在 config.json 的 `enhance` 字段；修改后立即生效。

mod ax;
mod import;
pub mod input_source;
pub mod music_block;
pub mod scroll;
pub mod stats;
pub mod switcher;
mod switcher_ui;
mod tap;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::config::ConfigStore;

/// wtools 自身的 bundle id（与 tauri.conf.json 的 identifier 一致）
pub const OWN_BUNDLE_ID: &str = "com.wtools.app";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EnhanceConfig {
    pub stats: stats::StatsConfig,
    pub scroll: scroll::ScrollConfig,
    pub input_source: input_source::InputSourceConfig,
    pub switcher: switcher::SwitcherConfig,
    pub music_block: music_block::MusicBlockConfig,
}

/// 启动时按配置开启各功能
pub fn init(app: &AppHandle) {
    let cfg = app.state::<ConfigStore>().get().enhance;
    stats::apply(app, &cfg.stats);
    scroll::apply(&cfg.scroll);
    input_source::apply(app, &cfg.input_source);
    switcher::apply(app, &cfg.switcher);
    music_block::apply(app, &cfg.music_block);
}

/// 只重启有变化的功能
fn apply_changes(app: &AppHandle, old: &EnhanceConfig, new: &EnhanceConfig) {
    if old.stats != new.stats {
        stats::apply(app, &new.stats);
    }
    if old.scroll != new.scroll {
        scroll::apply(&new.scroll);
    }
    if old.input_source != new.input_source {
        input_source::apply(app, &new.input_source);
    }
    if old.switcher != new.switcher {
        switcher::apply(app, &new.switcher);
    }
    if old.music_block != new.music_block {
        music_block::apply(app, &new.music_block);
    }
}

/// 与某个功能对应的原应用
#[derive(Debug, Serialize)]
pub struct Original {
    feature: &'static str,
    name: &'static str,
    bundle_id: &'static str,
    /// 正在运行（与本功能同时运行会重复处理）
    running: bool,
    /// 有它的设置可以导入
    importable: bool,
}

/// （功能, 原应用名, bundle id, 支持导入设置）
const ORIGINALS: &[(&str, &str, &str, bool)] = &[
    ("stats", "Stats", "eu.exelban.Stats", true),
    ("scroll", "Mos", "com.caldis.Mos", true),
    ("input_source", "Input Source Pro", "com.runjuu.Input-Source-Pro", true),
    // 规则存在私有数据库里，只做冲突提示
    ("input_source", "TypeRelay", "com.micky.typerelay", false),
    ("switcher", "AltTab", "com.lwouis.alt-tab-macos", true),
    ("music_block", "noTunes", "digital.twisted.noTunes", false),
];

fn originals() -> Vec<Original> {
    ORIGINALS
        .iter()
        .map(|&(feature, name, bundle_id, can_import)| Original {
            feature,
            name,
            bundle_id,
            running: is_running(bundle_id),
            importable: can_import && import::installed(bundle_id),
        })
        .collect()
}

fn is_running(bundle_id: &str) -> bool {
    use objc2_app_kit::NSRunningApplication;
    use objc2_foundation::NSString;
    !NSRunningApplication::runningApplicationsWithBundleIdentifier(&NSString::from_str(bundle_id)).is_empty()
}

pub mod commands {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Debug, Serialize)]
    pub struct EnhanceState {
        config: EnhanceConfig,
        originals: Vec<Original>,
        /// 已获得「辅助功能」权限
        accessibility: bool,
        /// 功能 → 启动失败的原因
        errors: BTreeMap<&'static str, String>,
        /// 可选的输入法
        sources: Vec<input_source::Source>,
    }

    fn state(app: &AppHandle) -> EnhanceState {
        let mut errors = BTreeMap::new();
        if let Some(e) = scroll::error() {
            errors.insert("scroll", e);
        }
        if let Some(e) = switcher::error() {
            errors.insert("switcher", e);
        }
        EnhanceState {
            config: app.state::<ConfigStore>().get().enhance,
            originals: originals(),
            accessibility: tap::accessibility_trusted(),
            errors,
            sources: input_source::list_sources(),
        }
    }

    #[tauri::command]
    pub fn enhance_get(app: AppHandle) -> EnhanceState {
        state(&app)
    }

    #[tauri::command]
    pub fn enhance_set(app: AppHandle, config: EnhanceConfig) -> Result<EnhanceState, String> {
        let store = app.state::<ConfigStore>();
        let old = store.get().enhance;
        store.update(|c| c.enhance = config.clone())?;
        apply_changes(&app, &old, &config);
        Ok(state(&app))
    }

    /// 重新启动需要权限的功能（用户刚授予「辅助功能」权限后调用）
    #[tauri::command]
    pub fn enhance_retry(app: AppHandle) -> EnhanceState {
        let cfg = app.state::<ConfigStore>().get().enhance;
        scroll::apply(&cfg.scroll);
        switcher::apply(&app, &cfg.switcher);
        input_source::apply(&app, &cfg.input_source);
        state(&app)
    }

    #[derive(Debug, Serialize)]
    pub struct ImportResult {
        state: EnhanceState,
        message: String,
    }

    /// 从原应用导入设置并立即生效
    #[tauri::command]
    pub fn enhance_import(app: AppHandle, feature: String) -> Result<ImportResult, String> {
        let store = app.state::<ConfigStore>();
        let old = store.get().enhance;
        let mut new = old.clone();
        let message = import::import(&feature, &mut new)?;
        store.update(|c| c.enhance = new.clone())?;
        apply_changes(&app, &old, &new);
        Ok(ImportResult { state: state(&app), message })
    }

    #[derive(Debug, Serialize)]
    pub struct RunningApp {
        bundle_id: String,
        name: String,
        /// wtools 自己：可以单独设置输入法，但不出现在「排除的应用」里
        own: bool,
    }

    /// 正在运行的普通应用（用于添加应用规则 / 排除的应用），wtools 自己排在第一个
    ///
    /// wtools 是菜单栏应用（Accessory），不在「普通应用」之列，需要单独加上，
    /// 好让搜索框等窗口也能单独设置输入法。
    ///
    /// 同一应用可能开了多个实例（如企业微信多开），按 bundle id 去重：
    /// 前端用 bundle id 作列表 key，重复 key 会让 Svelte 抛错、整个下拉列表渲染为空。
    #[tauri::command]
    pub fn enhance_running_apps() -> Vec<RunningApp> {
        use objc2_app_kit::{NSApplicationActivationPolicy, NSWorkspace};
        let mut seen = std::collections::HashSet::new();
        let mut apps: Vec<RunningApp> = NSWorkspace::sharedWorkspace()
            .runningApplications()
            .iter()
            .filter(|a| a.activationPolicy() == NSApplicationActivationPolicy::Regular)
            .filter_map(|a| {
                Some(RunningApp {
                    bundle_id: a.bundleIdentifier()?.to_string(),
                    name: a.localizedName().map(|s| s.to_string()).unwrap_or_default(),
                    own: false,
                })
            })
            .filter(|a| a.bundle_id != OWN_BUNDLE_ID && seen.insert(a.bundle_id.clone()))
            .collect();
        apps.sort_by(|a, b| a.name.cmp(&b.name));
        apps.insert(0, RunningApp { bundle_id: OWN_BUNDLE_ID.into(), name: "wtools".into(), own: true });
        apps
    }

    /// 打开「系统设置 → 隐私与安全性 → 辅助功能」
    #[tauri::command]
    pub fn enhance_open_accessibility() {
        let _ = std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .spawn();
    }

    /// 退出与功能冲突的原应用
    #[tauri::command]
    pub fn enhance_quit_original(bundle_id: String) -> Result<(), String> {
        if !ORIGINALS.iter().any(|(_, _, id, _)| *id == bundle_id) {
            return Err("未知应用".into());
        }
        use objc2_app_kit::NSRunningApplication;
        use objc2_foundation::NSString;
        for a in NSRunningApplication::runningApplicationsWithBundleIdentifier(&NSString::from_str(&bundle_id)) {
            a.terminate();
        }
        Ok(())
    }
}
