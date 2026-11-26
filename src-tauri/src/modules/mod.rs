//! 功能模块协议
//!
//! 每个功能（系统命令、应用搜索、剪贴板、截图、JSON……）实现 [`Module`]：
//! - `commands()`：静态命令，会被统一索引，可被搜索、也可绑定全局快捷键
//! - `search()`：动态结果（如应用列表、计算器），每次输入时调用
//! - `execute()`：执行命令
//!
//! 命令的全局 ID 为 `模块ID:命令key`，例如 `system:lock`。

pub mod apps;
pub mod builtin;
pub mod clipboard;
pub mod json;
pub mod screenshot;
pub mod system;

use crate::matcher::{FuzzyMatcher, Haystack, Query};
use crate::usage::Usage;
use serde::Serialize;
use std::sync::Mutex;
use tauri::AppHandle;

#[derive(Debug, Clone)]
pub struct CommandDef {
    pub key: &'static str,
    pub title: &'static str,
    pub subtitle: &'static str,
    pub icon: &'static str,
    /// 额外的搜索关键词（英文名、别名）
    pub keywords: &'static [&'static str],
    /// 在搜索框内打开的视图（如剪贴板）；None 表示执行后隐藏搜索框
    pub view: Option<&'static str>,
    /// 不出现在搜索结果里，只用于绑定快捷键（如「粘贴第 N 条」）
    pub hidden: bool,
}

/// 返回给前端的搜索结果
#[derive(Debug, Clone, Serialize)]
pub struct Item {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub icon: String,
    #[serde(skip)]
    pub score: u32,
}

pub trait Module: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn commands(&self) -> Vec<CommandDef> {
        Vec::new()
    }
    fn search(&self, _query: &Query, _m: &mut FuzzyMatcher) -> Vec<Item> {
        Vec::new()
    }
    fn execute(&self, key: &str, app: &AppHandle) -> Result<(), String>;
}

struct IndexedCommand {
    module_idx: usize,
    def: CommandDef,
    hay: Haystack,
}

/// 可绑定快捷键的命令（设置页使用）
#[derive(Debug, Clone, Serialize)]
pub struct BindableCommand {
    pub id: String,
    pub title: String,
    pub module: String,
}

pub struct Registry {
    modules: Vec<Box<dyn Module>>,
    commands: Vec<IndexedCommand>,
    matcher: Mutex<FuzzyMatcher>,
    usage: Usage,
}

const MAX_RESULTS: usize = 30;

impl Registry {
    pub fn new(modules: Vec<Box<dyn Module>>, usage: Usage) -> Self {
        let mut commands = Vec::new();
        for (module_idx, m) in modules.iter().enumerate() {
            for def in m.commands() {
                let hay = Haystack::new(def.title, def.keywords);
                commands.push(IndexedCommand {
                    module_idx,
                    def,
                    hay,
                });
            }
        }
        Self {
            modules,
            commands,
            matcher: Mutex::new(FuzzyMatcher::new()),
            usage,
        }
    }

    pub fn search(&self, raw: &str) -> Vec<Item> {
        let query = Query::new(raw);
        if query.is_empty() {
            return Vec::new();
        }
        let mut m = self.matcher.lock().unwrap();
        let mut items: Vec<Item> = self
            .commands
            .iter()
            .filter(|c| !c.def.hidden)
            .filter_map(|c| {
                let score = m.score(&query, &c.hay)?;
                Some(Item {
                    id: format!("{}:{}", self.modules[c.module_idx].id(), c.def.key),
                    title: c.def.title.to_string(),
                    subtitle: c.def.subtitle.to_string(),
                    icon: c.def.icon.to_string(),
                    score,
                })
            })
            .collect();
        for module in &self.modules {
            items.extend(module.search(&query, &mut m));
        }
        drop(m);
        for item in &mut items {
            item.score += self.usage.boost(&item.id);
        }
        items.sort_by(|a, b| b.score.cmp(&a.score));
        items.truncate(MAX_RESULTS);
        items
    }

    pub fn execute(&self, id: &str, app: &AppHandle) -> Result<(), String> {
        let (module_id, key) = id.split_once(':').ok_or("无效的命令 ID")?;
        let module = self
            .modules
            .iter()
            .find(|m| m.id() == module_id)
            .ok_or_else(|| format!("未找到模块 {module_id}"))?;
        module.execute(key, app)?;
        self.usage.record(id);
        Ok(())
    }

    pub fn bindable_commands(&self) -> Vec<BindableCommand> {
        self.commands
            .iter()
            .map(|c| {
                let m = &self.modules[c.module_idx];
                BindableCommand {
                    id: format!("{}:{}", m.id(), c.def.key),
                    title: c.def.title.to_string(),
                    module: m.name().to_string(),
                }
            })
            .collect()
    }

    /// 命令对应的搜索框视图（如果有）
    pub fn view_of(&self, id: &str) -> Option<&'static str> {
        self.commands
            .iter()
            .find(|c| format!("{}:{}", self.modules[c.module_idx].id(), c.def.key) == id)
            .and_then(|c| c.def.view)
    }

    pub fn has_command(&self, id: &str) -> bool {
        self.commands
            .iter()
            .any(|c| format!("{}:{}", self.modules[c.module_idx].id(), c.def.key) == id)
    }
}
