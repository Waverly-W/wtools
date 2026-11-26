//! 应用搜索：扫描 .app、解析本地化名称（中文名）、拼音匹配，目录变化时自动重建索引

use super::{Item, Module};
use crate::matcher::{FuzzyMatcher, Haystack, Query};
use notify::{RecursiveMode, Watcher};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{
        mpsc::{self, RecvTimeoutError},
        Arc, RwLock,
    },
    time::{Duration, Instant},
};
use tauri::AppHandle;

#[derive(Debug, Clone)]
pub struct AppEntry {
    /// 系统显示名（随系统语言本地化，如「微信」「系统设置」）
    pub name: String,
    pub path: PathBuf,
    hay: Haystack,
}

pub struct Apps {
    index: Arc<RwLock<Arc<Vec<AppEntry>>>>,
}

/// 扫描根目录及深度（深度 2 覆盖 /Applications/Utilities 这类子文件夹）
fn roots() -> Vec<(PathBuf, usize)> {
    let mut v = vec![
        (PathBuf::from("/Applications"), 2),
        (PathBuf::from("/System/Applications"), 2),
        (PathBuf::from("/System/Library/CoreServices/Applications"), 1),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        v.push((PathBuf::from(home).join("Applications"), 2));
    }
    v
}

/// 不在常规目录里、但常用的应用
const EXTRA_APPS: &[&str] = &["/System/Library/CoreServices/Finder.app"];

impl Apps {
    pub fn new() -> Self {
        let apps = Self {
            index: Arc::new(RwLock::new(Arc::new(Vec::new()))),
        };
        let index = apps.index.clone();
        // 首次扫描与目录监听都在后台线程，不阻塞启动
        std::thread::Builder::new()
            .name("app-index".into())
            .spawn(move || {
                rebuild(&index);
                watch(&index);
            })
            .ok();
        apps
    }

    fn snapshot(&self) -> Arc<Vec<AppEntry>> {
        self.index.read().unwrap().clone()
    }
}

fn rebuild(index: &RwLock<Arc<Vec<AppEntry>>>) {
    let start = Instant::now();
    let entries = scan();
    let n = entries.len();
    *index.write().unwrap() = Arc::new(entries);
    eprintln!("[apps] 索引 {n} 个应用，耗时 {:?}", start.elapsed());
}

/// 监听应用目录，变化后防抖 1.5s 再整体重建（重建只需几十毫秒）
fn watch(index: &RwLock<Arc<Vec<AppEntry>>>) {
    let (tx, rx) = mpsc::channel();
    let mut watcher = match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if res.is_ok() {
            let _ = tx.send(());
        }
    }) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("[apps] 目录监听不可用: {e}");
            return;
        }
    };
    for (root, _) in roots() {
        if root.is_dir() {
            let _ = watcher.watch(&root, RecursiveMode::Recursive);
        }
    }
    const DEBOUNCE: Duration = Duration::from_millis(1500);
    while rx.recv().is_ok() {
        // 应用安装/更新会连续触发大量事件，等安静下来再重建
        loop {
            match rx.recv_timeout(DEBOUNCE) {
                Ok(()) => continue,
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        rebuild(index);
    }
}

fn scan() -> Vec<AppEntry> {
    let mut paths = Vec::new();
    for (root, depth) in roots() {
        collect_apps(&root, depth, &mut paths);
    }
    paths.extend(EXTRA_APPS.iter().map(PathBuf::from).filter(|p| p.exists()));

    // 同一个应用可能通过符号链接出现多次（如 /Applications/Safari.app → Cryptexes）
    let mut seen = HashSet::new();
    paths.retain(|p| seen.insert(std::fs::canonicalize(p).unwrap_or_else(|_| p.clone())));

    let mut entries: Vec<AppEntry> = paths.into_iter().filter_map(|p| entry_for(&p)).collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

fn collect_apps(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let path = e.path();
        let is_app = path.extension().is_some_and(|x| x == "app");
        if is_app {
            out.push(path);
        } else if depth > 1 && e.file_type().is_ok_and(|t| t.is_dir()) {
            collect_apps(&path, depth - 1, out);
        }
    }
}

fn entry_for(path: &Path) -> Option<AppEntry> {
    let stem = path.file_stem()?.to_string_lossy().to_string();
    let info = read_info(path);
    // 后台/辅助类应用（无界面）不出现在搜索里
    if info.background_only {
        return None;
    }
    let name = display_name(path).unwrap_or_else(|| stem.clone());

    // 关键词：文件名、Info.plist 英文名、包内中文名与系统提供的搜索词，
    // 这样不论系统语言是什么，中文 / 拼音 / 英文都能搜到
    let mut keywords: Vec<String> = vec![stem];
    keywords.extend(info.names);
    keywords.extend(localized_names(path));
    keywords.retain(|k| !k.is_empty() && *k != name);
    keywords.dedup();
    let kw: Vec<&str> = keywords.iter().map(String::as_str).collect();
    let hay = Haystack::new(&name, &kw);

    Some(AppEntry {
        name,
        path: path.to_path_buf(),
        hay,
    })
}

#[derive(Default)]
struct Info {
    names: Vec<String>,
    background_only: bool,
}

fn read_info(path: &Path) -> Info {
    let Ok(plist::Value::Dictionary(d)) = plist::Value::from_file(path.join("Contents/Info.plist"))
    else {
        return Info::default();
    };
    let s = |k: &str| d.get(k).and_then(|v| v.as_string()).map(str::to_string);
    let flag = |k: &str| {
        d.get(k).is_some_and(|v| {
            v.as_boolean().unwrap_or(false) || v.as_string().is_some_and(|s| s == "1" || s == "YES")
        })
    };
    Info {
        names: [s("CFBundleDisplayName"), s("CFBundleName")]
            .into_iter()
            .flatten()
            .collect(),
        background_only: flag("LSBackgroundOnly"),
    }
}

const ZH_LANGS: &[&str] = &["zh_CN", "zh-Hans", "zh_Hans", "zh-CN", "zh"];
const NAME_KEYS: &[&str] = &["CFBundleDisplayName", "CFBundleName", "MDItemKeywords"];

/// 读取应用包内的简体中文名称
/// - 系统应用：`InfoPlist.loctable`（所有语言放在一个二进制 plist 里）
/// - 第三方应用：`zh_CN.lproj/InfoPlist.strings`（文本 UTF-8/UTF-16、XML 或二进制 plist）
fn localized_names(path: &Path) -> Vec<String> {
    let res = path.join("Contents/Resources");
    let mut out = Vec::new();
    let mut take = |d: &plist::Dictionary| {
        for k in NAME_KEYS {
            if let Some(v) = d.get(k).and_then(|v| v.as_string()) {
                out.extend(v.split([',', '，']).map(|s| s.trim().to_string()));
            }
        }
    };

    if let Ok(plist::Value::Dictionary(table)) = plist::Value::from_file(res.join("InfoPlist.loctable"))
    {
        if let Some(d) = ZH_LANGS
            .iter()
            .find_map(|l| table.get(l).and_then(|v| v.as_dictionary()))
        {
            take(d);
        }
    }
    if let Some(d) = ZH_LANGS
        .iter()
        .find_map(|l| read_strings(&res.join(format!("{l}.lproj/InfoPlist.strings"))))
    {
        take(&d);
    }
    out.retain(|s| !s.is_empty());
    out
}

fn read_strings(file: &Path) -> Option<plist::Dictionary> {
    let bytes = std::fs::read(file).ok()?;
    // 二进制 / XML plist
    if let Ok(plist::Value::Dictionary(d)) = plist::Value::from_reader(std::io::Cursor::new(&bytes)) {
        return Some(d);
    }
    // 文本 .strings：`"key" = "value";`，补上花括号后按 OpenStep 格式解析
    let text = decode_text(&bytes)?;
    let wrapped = format!("{{{}}}", text.trim_start_matches('\u{feff}'));
    match plist::Value::from_reader(std::io::Cursor::new(wrapped.into_bytes())) {
        Ok(plist::Value::Dictionary(d)) => Some(d),
        _ => None,
    }
}

fn decode_text(bytes: &[u8]) -> Option<String> {
    let utf16 = |le: bool| {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) })
            .collect();
        String::from_utf16(&units).ok()
    };
    match bytes {
        [0xFF, 0xFE, ..] => utf16(true),
        [0xFE, 0xFF, ..] => utf16(false),
        _ => String::from_utf8(bytes.to_vec()).ok(),
    }
}

/// 系统给出的显示名，与 Finder 一致（已按系统语言本地化，并去掉 .app 后缀）
#[cfg(target_os = "macos")]
fn display_name(path: &Path) -> Option<String> {
    use objc2_foundation::{NSFileManager, NSString};
    objc2::rc::autoreleasepool(|_| {
        let p = NSString::from_str(path.to_str()?);
        let name = NSFileManager::defaultManager().displayNameAtPath(&p).to_string();
        let name = name.strip_suffix(".app").unwrap_or(&name).to_string();
        (!name.is_empty()).then_some(name)
    })
}

#[cfg(not(target_os = "macos"))]
fn display_name(_path: &Path) -> Option<String> {
    None
}

impl Module for Apps {
    fn id(&self) -> &'static str {
        "apps"
    }
    fn name(&self) -> &'static str {
        "应用"
    }

    fn search(&self, query: &Query, m: &mut FuzzyMatcher) -> Vec<Item> {
        let home = std::env::var("HOME").unwrap_or_default();
        self.snapshot()
            .iter()
            .filter_map(|app| {
                let score = m.score(query, &app.hay)?;
                let path = app.path.to_string_lossy();
                let subtitle = match path.strip_prefix(&home) {
                    Some(rest) if !home.is_empty() => format!("~{rest}"),
                    _ => path.to_string(),
                };
                Some(Item {
                    id: format!("apps:{path}"),
                    title: app.name.clone(),
                    subtitle,
                    icon: crate::icons::url_for(&app.path),
                    // 应用是最常用的结果，同分时排在命令前面
                    score: score + 1,
                })
            })
            .collect()
    }

    fn execute(&self, key: &str, _app: &AppHandle) -> Result<(), String> {
        let path = Path::new(key);
        if path.extension().is_none_or(|x| x != "app") || !path.exists() {
            return Err(format!("应用不存在: {key}"));
        }
        let mut child = std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|e| format!("打开失败: {e}"))?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 在本机真实扫描并搜索：`cargo test real_scan -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_scan() {
        let t = Instant::now();
        let entries = scan();
        println!("扫描 {} 个应用，耗时 {:?}", entries.len(), t.elapsed());
        println!(
            "系统设置的关键词: {:?}",
            localized_names(Path::new("/System/Applications/System Settings.app"))
        );
        let mut m = FuzzyMatcher::new();
        for q in ["wx", "qywx", "chrome", "xtsz", "系统设置", "xtpz", "vsc", "jsq", "finder", "hdq", "zd"] {
            let query = Query::new(q);
            let mut hits: Vec<(u32, &str)> = entries
                .iter()
                .filter_map(|a| Some((m.score(&query, &a.hay)?, a.name.as_str())))
                .collect();
            hits.sort_by(|a, b| b.0.cmp(&a.0));
            let top: Vec<_> = hits.iter().take(4).map(|h| h.1).collect();
            println!("{q:>8} → {top:?}");
        }
    }
}
