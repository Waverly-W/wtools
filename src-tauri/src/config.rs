//! 用户配置：JSON 文件，位于 ~/Library/Application Support/com.wtools.app/config.json

use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::PathBuf, sync::Mutex};
use tauri::{AppHandle, Manager};

/// 呼出/隐藏搜索框的动作 ID（不是搜索结果，但和命令共用快捷键表）
pub const LAUNCHER_TOGGLE: &str = "launcher.toggle";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// 动作 ID（`launcher.toggle` 或 `模块:命令`）→ 快捷键，如 `Alt+Space`
    pub hotkeys: BTreeMap<String, String>,
    pub clipboard: ClipboardConfig,
    pub enhance: crate::enhance::EnhanceConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ClipboardConfig {
    /// 是否记录剪贴板历史
    pub enabled: bool,
    /// 最多保留条数（收藏的不计入、不会被清理）
    pub max_items: u32,
    /// 最多保存天数，0 表示不限（收藏的不会被清理）
    pub max_days: u32,
    /// 自定义数据目录（如网盘目录，用于多台电脑同步）；None 为默认目录
    pub data_dir: Option<String>,
    /// 对单条记录的自定义操作
    pub actions: Vec<CustomAction>,
    /// 默认布局："dual" 剪贴板 + 收藏双栏；"single" 单栏
    pub layout: String,
    /// 列表中每条最多显示的行数，超出需展开查看
    pub max_lines: u32,
}

impl Default for ClipboardConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_items: 1000,
            max_days: 0,
            data_dir: None,
            actions: default_actions(),
            layout: "dual".into(),
            max_lines: 4,
        }
    }
}

/// 自定义操作：打开网址（{text}/{file} 会被替换并做 URL 编码）或运行 Shell 命令
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomAction {
    pub id: String,
    pub name: String,
    /// "url" | "shell"
    pub kind: String,
    /// url：网址模板；shell：命令，内容通过标准输入和环境变量 WTOOLS_TEXT / WTOOLS_FILE 传入
    pub template: String,
    /// 适用的内容类型："text" | "image" | "files"
    pub applies_to: Vec<String>,
    /// shell 输出的处理："none" | "show" | "copy" | "paste"
    #[serde(default = "default_output")]
    pub output: String,
}

fn default_output() -> String {
    "show".into()
}

fn default_actions() -> Vec<CustomAction> {
    let text = vec!["text".to_string()];
    vec![
        CustomAction {
            id: "bing-translate".into(),
            name: "必应翻译".into(),
            kind: "url".into(),
            template: "https://cn.bing.com/translator?text={text}&from=auto-detect&to=zh-Hans".into(),
            applies_to: text.clone(),
            output: "none".into(),
        },
        CustomAction {
            id: "baidu-search".into(),
            name: "百度搜索".into(),
            kind: "url".into(),
            template: "https://www.baidu.com/s?wd={text}".into(),
            applies_to: text.clone(),
            output: "none".into(),
        },
        CustomAction {
            id: "uppercase".into(),
            name: "转为大写".into(),
            kind: "shell".into(),
            template: "tr '[:lower:]' '[:upper:]'".into(),
            applies_to: text,
            output: "show".into(),
        },
    ]
}

impl Default for AppConfig {
    fn default() -> Self {
        let mut hotkeys = BTreeMap::new();
        hotkeys.insert(LAUNCHER_TOGGLE.to_string(), "Alt+Space".to_string());
        Self {
            hotkeys,
            clipboard: ClipboardConfig::default(),
            enhance: Default::default(),
        }
    }
}

pub struct ConfigStore {
    path: PathBuf,
    inner: Mutex<AppConfig>,
}

impl ConfigStore {
    pub fn load(app: &AppHandle) -> Self {
        let dir = app
            .path()
            .app_config_dir()
            .unwrap_or_else(|_| std::env::temp_dir().join("wtools"));
        let path = dir.join("config.json");
        let cfg = fs::read_to_string(&path)
            .ok()
            .and_then(|s| match serde_json::from_str::<AppConfig>(&s) {
                Ok(c) => Some(c),
                Err(e) => {
                    eprintln!("[config] 解析失败，使用默认配置: {e}");
                    None
                }
            })
            .unwrap_or_default();
        Self {
            path,
            inner: Mutex::new(cfg),
        }
    }

    pub fn get(&self) -> AppConfig {
        self.inner.lock().unwrap().clone()
    }

    /// 修改并落盘（先写临时文件再 rename，避免写一半崩溃导致配置损坏）
    pub fn update<F: FnOnce(&mut AppConfig)>(&self, f: F) -> Result<(), String> {
        let mut cfg = self.inner.lock().unwrap();
        f(&mut cfg);
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let data = serde_json::to_string_pretty(&*cfg).map_err(|e| e.to_string())?;
        fs::write(&tmp, data).map_err(|e| e.to_string())?;
        fs::rename(&tmp, &self.path).map_err(|e| e.to_string())
    }
}
