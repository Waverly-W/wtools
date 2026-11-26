//! 从原应用导入设置：Stats、Mos、Input Source Pro、AltTab

use super::EnhanceConfig;
use plist::{Dictionary, Value};

/// 读取某个应用的偏好设置
fn defaults(domain: &str) -> Option<Dictionary> {
    let out = std::process::Command::new("defaults").args(["export", domain, "-"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Value::from_reader_xml(&out.stdout[..]).ok()?.into_dictionary()
}

pub fn installed(domain: &str) -> bool {
    defaults(domain).is_some_and(|d| !d.is_empty())
}

fn int(d: &Dictionary, k: &str) -> Option<i64> {
    match d.get(k)? {
        Value::Integer(i) => i.as_signed(),
        Value::Real(r) => Some(*r as i64),
        Value::Boolean(b) => Some(*b as i64),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn real(d: &Dictionary, k: &str) -> Option<f64> {
    match d.get(k)? {
        Value::Integer(i) => i.as_signed().map(|v| v as f64),
        Value::Real(r) => Some(*r),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn string(d: &Dictionary, k: &str) -> Option<String> {
    d.get(k)?.as_string().map(str::to_string)
}

/// Input Source Pro 用 JSON 编码后存成 Data
fn json_data(d: &Dictionary, k: &str) -> Option<serde_json::Value> {
    serde_json::from_slice(d.get(k)?.as_data()?).ok()
}

/// 把导入结果合并进 cfg，返回导入了什么（给用户看）
pub fn import(feature: &str, cfg: &mut EnhanceConfig) -> Result<String, String> {
    match feature {
        "stats" => {
            let d = defaults("eu.exelban.Stats").ok_or("没有找到 Stats 的设置")?;
            let on = |k: &str| int(&d, k) == Some(1);
            let c = &mut cfg.stats;
            c.enabled = true;
            c.cpu = on("CPU_state");
            c.memory = on("RAM_state");
            c.disk = on("Disk_state");
            Ok(format!("已导入：显示 {}", [("CPU", c.cpu), ("内存", c.memory), ("磁盘", c.disk)].iter().filter(|x| x.1).map(|x| x.0).collect::<Vec<_>>().join("、")))
        }
        "scroll" => {
            let d = defaults("com.caldis.Mos").ok_or("没有找到 Mos 的设置")?;
            let c = &mut cfg.scroll;
            c.enabled = true;
            c.smooth = int(&d, "smooth").map_or(c.smooth, |v| v != 0);
            c.reverse = int(&d, "reverse").map_or(c.reverse, |v| v != 0);
            c.step = real(&d, "step").unwrap_or(c.step).clamp(0.0, 100.0);
            c.speed = real(&d, "speed").unwrap_or(c.speed).clamp(0.5, 10.0);
            c.duration = real(&d, "duration").unwrap_or(c.duration).clamp(1.0, 10.0);
            Ok(format!(
                "已导入：平滑{}、反向{}、步长 {}、速度 {}、过渡 {}",
                if c.smooth { "开" } else { "关" },
                if c.reverse { "开" } else { "关" },
                c.step,
                c.speed,
                c.duration
            ))
        }
        "input_source" => {
            let d = defaults("com.runjuu.Input-Source-Pro").ok_or("没有找到 Input Source Pro 的设置")?;
            let c = &mut cfg.input_source;
            c.enabled = true;
            c.default_source = string(&d, "systemWideDefaultKeyboardId").or(c.default_source.take());
            c.remember_last = int(&d, "isRestorePreviouslyUsedInputSource") == Some(1);
            c.indicator = if int(&d, "isEnableAlwaysOnIndicator") == Some(1) { "always".into() } else { "switch".into() };
            if let Some(p) = json_data(&d, "indicatorPosition") {
                if p["base"] == "windowCorner" {
                    c.corner = "bottom-right".into();
                }
            }
            let db = std::env::var("HOME").unwrap_or_default() + "/Library/Application Support/Input Source Pro/Main.sqlite";
            let mut rules = 0;
            if let Ok(conn) = rusqlite::Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) {
                // 只导入指定了输入法的规则；没指定的等同于跟随默认
                if let Ok(mut st) = conn.prepare(
                    "select ZBUNDLEID, ZBUNDLENAME, ZINPUTSOURCEID from ZAPPRULE where ZBUNDLEID is not null and ifnull(ZREMOVED,0)=0 and ifnull(ZINPUTSOURCEID,'')<>''",
                ) {
                    let found = st
                        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, String>(2)?)))
                        .map(|it| it.flatten().collect::<Vec<_>>())
                        .unwrap_or_default();
                    for (bundle, name, source) in found {
                        c.rules.retain(|r| r.bundle_id != bundle);
                        c.rules.push(super::input_source::InputRule { name: name.unwrap_or_else(|| bundle.clone()), bundle_id: bundle, source: Some(source) });
                        rules += 1;
                    }
                }
                if let Ok(mut st) = conn.prepare("select ZID, ZBGCOLORHEX from ZKEYBOARDCONFIG where ZID not like '%::%' and ZBGCOLORHEX is not null") {
                    let colors = st
                        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                        .map(|it| it.flatten().collect::<Vec<_>>())
                        .unwrap_or_default();
                    for (id, hex) in colors {
                        c.colors.insert(id, format!("#{hex}"));
                    }
                }
            }
            Ok(format!("已导入：默认输入法、{rules} 条应用规则、指示器设置与颜色"))
        }
        "switcher" => {
            let d = defaults("com.lwouis.alt-tab-macos").ok_or("没有找到 AltTab 的设置")?;
            let c = &mut cfg.switcher;
            c.enabled = true;
            let hold = d.get("holdShortcut").and_then(|v| v.as_dictionary()).and_then(|h| string(h, "string")).or_else(|| string(&d, "holdShortcut"));
            if let Some(h) = hold {
                c.modifier = if h.contains('⌃') {
                    "ctrl"
                } else if h.contains('⌘') {
                    "cmd"
                } else {
                    "alt"
                }
                .into();
            }
            // 0 全部屏幕 / 1 显示切换器的屏幕
            c.current_screen_only = int(&d, "screensToShow").map_or(c.current_screen_only, |v| v == 1);
            // 0 显示 / 1 隐藏 / 2 显示在最后
            c.show_minimized = int(&d, "showMinimizedWindows").map_or(c.show_minimized, |v| v != 1);
            c.show_hidden = int(&d, "showHiddenWindows").map_or(c.show_hidden, |v| v != 1);
            if let Some(list) = string(&d, "exceptions").and_then(|s| serde_json::from_str::<Vec<serde_json::Value>>(&s).ok()) {
                for e in list {
                    // hide = "1"：始终隐藏该应用的窗口
                    if e["hide"] == "1" {
                        if let Some(b) = e["bundleIdentifier"].as_str() {
                            if !c.excluded.iter().any(|x| x == b) {
                                c.excluded.push(b.to_string());
                            }
                        }
                    }
                }
            }
            let key = match c.modifier.as_str() {
                "ctrl" => "⌃",
                "cmd" => "⌘",
                _ => "⌥",
            };
            Ok(format!("已导入：快捷键 {key}Tab、{} 个排除的应用", c.excluded.len()))
        }
        _ => Err("未知功能".into()),
    }
}
