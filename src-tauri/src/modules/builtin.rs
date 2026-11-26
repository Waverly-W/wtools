//! wtools 自身的命令

use super::{CommandDef, Module};
use tauri::AppHandle;

pub struct Builtin;

impl Module for Builtin {
    fn id(&self) -> &'static str {
        "wtools"
    }
    fn name(&self) -> &'static str {
        "wtools"
    }

    fn commands(&self) -> Vec<CommandDef> {
        vec![
            CommandDef {
                key: "settings",
                title: "wtools 设置",
                subtitle: "快捷键与偏好设置",
                icon: "⚙️",
                keywords: &["settings", "preferences", "快捷键"],
                view: None,
                hidden: false,
            },
            CommandDef {
                key: "quit",
                title: "退出 wtools",
                subtitle: "",
                icon: "⏻",
                keywords: &["quit", "exit"],
                view: None,
                hidden: false,
            },
        ]
    }

    fn execute(&self, key: &str, app: &AppHandle) -> Result<(), String> {
        match key {
            "settings" => crate::window::open_settings(app),
            "quit" => {
                app.exit(0);
                Ok(())
            }
            _ => Err(format!("未知命令 {key}")),
        }
    }
}
