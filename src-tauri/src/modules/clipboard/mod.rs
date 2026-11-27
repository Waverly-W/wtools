//! 剪贴板历史：后台轮询 changeCount 记录新内容；在搜索框的「剪贴板」视图中筛选、搜索、
//! 多选粘贴、收藏打标签，并对单条记录执行字数统计 / OCR / 自定义操作。

mod actions;
pub mod commands;
pub(crate) mod pasteboard;
mod store;

use super::{CommandDef, Module};
use crate::{config::ConfigStore, window};
use pasteboard::Content;
use std::{
    hash::{Hash, Hasher},
    path::PathBuf,
    sync::{
        atomic::{AtomicIsize, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use store::{Clip, Kind, NewClip, Store};
use tauri::{
    http::{header, Request, Response, StatusCode},
    AppHandle, Emitter, Manager, UriSchemeContext, UriSchemeResponder, Wry,
};

pub const VIEW: &str = "clipboard";
pub const SCHEME: &str = "wclip";
const POLL_INTERVAL: Duration = Duration::from_millis(400);
/// 自定义数据目录下实际使用的子目录名
const SYNC_SUBDIR: &str = "wtools-clipboard";

/// 默认不记录的应用（密码管理器、钥匙串）
const IGNORED_BUNDLES: &[&str] = &[
    "com.1password.1password",
    "com.agilebits.onepassword7",
    "com.bitwarden.desktop",
    "com.apple.keychainaccess",
    "com.apple.Passwords",
    "org.keepassxc.keepassxc",
];

pub struct ClipboardService {
    store: Mutex<Store>,
    default_dir: PathBuf,
    /// 自己写入剪贴板后的 changeCount，监听时跳过，避免把来源改成 wtools
    self_change: AtomicIsize,
    /// 连续粘贴多条期间暂停记录（毫秒时间戳）
    suppress_until: AtomicU64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

impl ClipboardService {
    pub fn open(default_dir: PathBuf, custom: Option<String>) -> Result<Arc<Self>, String> {
        let (dir, synced) = match custom {
            Some(c) => (PathBuf::from(c).join(SYNC_SUBDIR), true),
            None => (default_dir.clone(), false),
        };
        let store = Store::open(&dir, synced)
            .or_else(|e| {
                // 自定义目录不可用（如网盘未挂载）时退回默认目录，保证功能可用
                eprintln!("[clipboard] 打开 {} 失败（{e}），改用默认目录", dir.display());
                Store::open(&default_dir, false)
            })
            .map_err(|e| format!("打开剪贴板数据库失败: {e}"))?;
        Ok(Arc::new(Self {
            store: Mutex::new(store),
            default_dir,
            self_change: AtomicIsize::new(-1),
            suppress_until: AtomicU64::new(0),
        }))
    }

    /// 启动后台监听线程
    pub fn start_watcher(self: &Arc<Self>, app: AppHandle) {
        let svc = self.clone();
        std::thread::Builder::new()
            .name("clipboard-watch".into())
            .spawn(move || {
                let mut last = pasteboard::change_count();
                loop {
                    std::thread::sleep(POLL_INTERVAL);
                    let cc = pasteboard::change_count();
                    if cc == last {
                        continue;
                    }
                    last = cc;
                    if cc == svc.self_change.load(Ordering::Relaxed)
                        || now_ms() < svc.suppress_until.load(Ordering::Relaxed)
                    {
                        continue;
                    }
                    let cfg = app.state::<ConfigStore>().get().clipboard;
                    if !cfg.enabled {
                        continue;
                    }
                    let (app_name, bundle) = pasteboard::frontmost_app();
                    if bundle.as_deref().is_some_and(|b| IGNORED_BUNDLES.contains(&b)) {
                        continue;
                    }
                    let Some(content) = pasteboard::read() else { continue };
                    match svc.save(&content, app_name.as_deref(), bundle.as_deref(), cfg.max_items, cfg.max_days) {
                        Ok(()) => {
                            let _ = app.emit_to(window::LAUNCHER, "clipboard:changed", ());
                        }
                        Err(e) => eprintln!("[clipboard] 保存失败: {e}"),
                    }
                }
            })
            .ok();
    }

    fn save(
        &self,
        content: &Content,
        app_name: Option<&str>,
        bundle: Option<&str>,
        max_items: u32,
        max_days: u32,
    ) -> Result<(), String> {
        let mut store = self.store.lock().unwrap();
        let (kind, body, meta, hash) = match content {
            Content::Text(s) => (Kind::Text, s.clone(), s.chars().count().to_string(), hash_of("t", s.as_bytes())),
            Content::Files(paths) => {
                let joined = paths.join("\n");
                let h = hash_of("f", joined.as_bytes());
                (Kind::Files, joined, paths.len().to_string(), h)
            }
            Content::Image { png, width, height } => {
                let h = hash_of("i", png);
                let file = store.images_dir.join(format!("{h}.png"));
                if !file.exists() {
                    std::fs::write(&file, png).map_err(|e| e.to_string())?;
                    if let Some(thumb) = pasteboard::thumbnail(png, 480.0) {
                        let _ = std::fs::write(store.images_dir.join(format!("{h}_t.png")), thumb);
                    }
                }
                (Kind::Image, h.clone(), format!("{width}×{height}"), h)
            }
        };
        store
            .upsert(&NewClip {
                kind,
                content: &body,
                hash: &hash,
                meta: &meta,
                source_app: app_name,
                source_bundle: bundle,
            })
            .map_err(|e| e.to_string())?;
        store.prune(max_items, max_days).map_err(|e| e.to_string())
    }

    fn get(&self, id: i64) -> Result<Clip, String> {
        self.store
            .lock()
            .unwrap()
            .get(id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "记录不存在".into())
    }

    fn image_path(&self, clip: &Clip) -> PathBuf {
        self.store
            .lock()
            .unwrap()
            .images_dir
            .join(format!("{}.png", clip.content))
    }

    fn content_of(&self, clip: &Clip) -> Result<Content, String> {
        Ok(match clip.kind {
            Kind::Text => Content::Text(clip.content.clone()),
            Kind::Files => Content::Files(clip.content.lines().map(str::to_string).collect()),
            Kind::Image => {
                let png = std::fs::read(self.image_path(clip)).map_err(|_| "图片文件已丢失")?;
                Content::Image { png, width: 0, height: 0 }
            }
        })
    }

    /// 把 wtools 自己产生的新文本（OCR 选中文字、操作输出等）记入历史。
    /// 写剪贴板时会记下 changeCount 让监听线程跳过（避免来源变成 wtools、避免重复），
    /// 所以这类内容需要在这里主动入库；已有相同内容时只会更新使用时间并置顶。
    pub fn record_text(&self, app: &AppHandle, text: &str, source: &str) {
        if text.trim().is_empty() {
            return;
        }
        let cfg = app.state::<ConfigStore>().get().clipboard;
        if !cfg.enabled {
            return;
        }
        let content = Content::Text(text.to_string());
        match self.save(&content, Some(source), None, cfg.max_items, cfg.max_days) {
            Ok(()) => {
                let _ = app.emit_to(window::LAUNCHER, "clipboard:changed", ());
            }
            Err(e) => eprintln!("[clipboard] 保存失败: {e}"),
        }
    }

    /// 写入系统剪贴板并记下 changeCount
    fn write(&self, content: &Content) -> Result<(), String> {
        let cc = pasteboard::write(content)?;
        self.self_change.store(cc, Ordering::Relaxed);
        Ok(())
    }

    /// 复制或粘贴多条内容（按传入顺序）。
    /// - 全是文本：用换行拼成一段；全是文件：合并为一次文件粘贴
    /// - 含图片或类型混合：粘贴时逐条依次粘贴；仅复制时无法合并图片，返回错误
    pub fn deliver(&self, app: &AppHandle, contents: Vec<Content>, paste: bool) -> Result<(), String> {
        if contents.is_empty() {
            return Err("没有选中内容".into());
        }
        let all_text = contents.iter().all(|c| matches!(c, Content::Text(_)));
        let all_files = contents.iter().all(|c| matches!(c, Content::Files(_)));
        let has_image = contents.iter().any(|c| matches!(c, Content::Image { .. }));

        let plan: Vec<Content> = if contents.len() == 1 {
            contents
        } else if all_text {
            let joined = contents
                .iter()
                .filter_map(|c| match c {
                    Content::Text(t) => Some(t.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            vec![Content::Text(joined)]
        } else if all_files {
            vec![Content::Files(
                contents
                    .into_iter()
                    .flat_map(|c| match c {
                        Content::Files(f) => f,
                        _ => Vec::new(),
                    })
                    .collect(),
            )]
        } else if paste {
            contents
        } else if !has_image {
            // 文本与文件混合：文件以路径文本形式合并
            let joined = contents
                .iter()
                .map(|c| match c {
                    Content::Text(t) => t.clone(),
                    Content::Files(f) => f.join("\n"),
                    Content::Image { .. } => String::new(),
                })
                .collect::<Vec<_>>()
                .join("\n");
            vec![Content::Text(joined)]
        } else {
            return Err("包含图片的多条内容无法一次复制，请改用粘贴（↩）".into());
        };

        if !paste {
            self.write(&plan[0])?;
            window::hide_launcher(app, true);
            return Ok(());
        }
        if !pasteboard::is_trusted(false) {
            // 弹出系统授权引导；本次只复制第一条
            self.write(&plan[0])?;
            pasteboard::is_trusted(true);
            return Err("已复制。自动粘贴需要在「系统设置 → 隐私与安全性 → 辅助功能」中允许 wtools".into());
        }
        window::hide_launcher(app, true);
        self.paste_sequence(plan, Duration::from_millis(180));
        Ok(())
    }

    /// 在后台依次写入并发送 ⌘V（等焦点回到之前的应用后开始）
    fn paste_sequence(&self, plan: Vec<Content>, initial_delay: Duration) {
        let total_ms = initial_delay.as_millis() as u64 + plan.len() as u64 * 400 + 600;
        self.suppress_until.store(now_ms() + total_ms, Ordering::Relaxed);
        let first = plan.first().cloned();
        if let Some(c) = &first {
            let _ = self.write(c);
        }
        std::thread::spawn(move || {
            std::thread::sleep(initial_delay);
            for (i, c) in plan.iter().enumerate() {
                if i > 0 {
                    let _ = pasteboard::write(c);
                    std::thread::sleep(Duration::from_millis(60));
                }
                pasteboard::post_cmd_v();
                // 给目标应用时间读取剪贴板，再写入下一条
                std::thread::sleep(Duration::from_millis(300));
            }
        });
    }

    /// 全局快捷键「粘贴第 N 条」：不打开搜索框，直接粘贴到当前应用
    pub fn paste_nth(&self, n: u32) -> Result<(), String> {
        let clip = self
            .store
            .lock()
            .unwrap()
            .nth(n)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("剪贴板历史不足 {n} 条"))?;
        let content = self.content_of(&clip)?;
        if !pasteboard::is_trusted(false) {
            self.write(&content)?;
            pasteboard::is_trusted(true);
            return Err("需要辅助功能权限才能自动粘贴".into());
        }
        let _ = self.store.lock().unwrap().touch(clip.id);
        // 等用户松开快捷键的修饰键，避免和 ⌘V 叠加
        self.paste_sequence(vec![content], Duration::from_millis(120));
        Ok(())
    }

    /// 切换数据目录；目标已有数据（如另一台电脑同步来的）则直接使用，否则复制现有数据过去
    pub fn switch_dir(&self, custom: Option<&str>) -> Result<String, String> {
        let (target, synced) = match custom {
            Some(c) => (PathBuf::from(c).join(SYNC_SUBDIR), true),
            None => (self.default_dir.clone(), false),
        };
        let mut store = self.store.lock().unwrap();
        if store.dir == target {
            return Ok("数据目录未变化".into());
        }
        let msg = if target.join("clipboard.db").exists() {
            "已切换。目标目录中已有数据（可能来自其他电脑），已直接使用；原目录数据保留未删除"
        } else {
            store.copy_to(&target)?;
            "已切换，现有记录已复制到新目录；原目录数据保留未删除"
        };
        *store = Store::open(&target, synced).map_err(|e| format!("打开新目录失败: {e}"))?;
        Ok(msg.into())
    }

    pub fn current_dir(&self) -> PathBuf {
        self.store.lock().unwrap().dir.clone()
    }
}

fn hash_of(prefix: &str, bytes: &[u8]) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    prefix.hash(&mut h);
    bytes.hash(&mut h);
    format!("{:016x}", h.finish())
}

// ---------------- 模块：搜索框入口与快捷键命令 ----------------

pub struct ClipboardModule;

const PASTE_NTH: [&str; 9] = [
    "paste-1", "paste-2", "paste-3", "paste-4", "paste-5", "paste-6", "paste-7", "paste-8", "paste-9",
];
const PASTE_NTH_TITLES: [&str; 9] = [
    "粘贴第 1 条", "粘贴第 2 条", "粘贴第 3 条", "粘贴第 4 条", "粘贴第 5 条",
    "粘贴第 6 条", "粘贴第 7 条", "粘贴第 8 条", "粘贴第 9 条",
];

impl Module for ClipboardModule {
    fn id(&self) -> &'static str {
        "clipboard"
    }
    fn name(&self) -> &'static str {
        "剪贴板"
    }
    fn commands(&self) -> Vec<CommandDef> {
        let mut v = vec![CommandDef {
            key: "open",
            title: "剪贴板历史",
            subtitle: "浏览、搜索并粘贴复制过的内容",
            icon: "📋",
            keywords: &["clipboard", "history", "粘贴板", "复制历史"],
            view: Some(VIEW),
            hidden: false,
        }];
        for (key, title) in PASTE_NTH.iter().zip(PASTE_NTH_TITLES) {
            v.push(CommandDef {
                key,
                title,
                subtitle: "",
                icon: "📋",
                keywords: &[],
                view: None,
                hidden: true,
            });
        }
        v
    }
    fn execute(&self, key: &str, app: &AppHandle) -> Result<(), String> {
        if let Some(n) = key.strip_prefix("paste-").and_then(|n| n.parse::<u32>().ok()) {
            return app.state::<Arc<ClipboardService>>().paste_nth(n);
        }
        window::show_launcher(app, VIEW);
        Ok(())
    }
}

/// `wclip://localhost/<hash>.png`：只允许访问 images 目录下的 PNG
pub fn handle_protocol(
    ctx: UriSchemeContext<'_, Wry>,
    req: Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let name = req.uri().path().trim_start_matches('/').to_string();
    let dir = ctx
        .app_handle()
        .state::<Arc<ClipboardService>>()
        .store
        .lock()
        .unwrap()
        .images_dir
        .clone();
    std::thread::spawn(move || {
        let valid = name.ends_with(".png")
            && name[..name.len() - 4]
                .chars()
                .all(|c| c.is_ascii_hexdigit() || c == '_' || c == 't');
        let res = match valid.then(|| std::fs::read(dir.join(&name)).ok()).flatten() {
            Some(bytes) => Response::builder()
                .header(header::CONTENT_TYPE, "image/png")
                .header(header::CACHE_CONTROL, "max-age=31536000, immutable")
                .body(bytes),
            None => Response::builder().status(StatusCode::NOT_FOUND).body(Vec::new()),
        };
        responder.respond(res.unwrap_or_else(|_| Response::new(Vec::new())));
    });
}
