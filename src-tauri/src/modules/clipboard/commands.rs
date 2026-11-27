//! 剪贴板相关的前端命令
//!
//! 注意：Tauri 的同步命令在主线程执行，耗时操作（OCR、自定义操作、选择目录）都写成 async 并放到阻塞线程池。

use super::{
    actions::{self, Input, Outcome, TestResult},
    pasteboard::Content,
    store::{Clip, Filter, Kind},
    ClipboardService,
};
use crate::{
    config::{ConfigStore, CustomAction},
    ocr::{self, OcrResult},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, Manager, State};

type Svc<'a> = State<'a, Arc<ClipboardService>>;

const TITLE_CHARS: usize = 200;
const DETAIL_CHARS: usize = 100_000;
/// 列表预览最多携带的行数与字符数（列表按设置的行数截断显示，展开时再取全文）
const PREVIEW_LINES: usize = 40;
const PREVIEW_CHARS: usize = 4_000;

#[derive(Serialize)]
pub struct ClipItem {
    id: i64,
    kind: Kind,
    /// 列表展示的单行标题
    title: String,
    meta: String,
    source_app: Option<String>,
    favorite: bool,
    tags: Vec<String>,
    used_at: i64,
    has_ocr: bool,
    /// 图片缩略图地址
    thumb: Option<String>,
    /// 图片原图地址（展开时显示）
    image: Option<String>,
    /// 列表中显示的多行预览：文本前若干行 / 文件路径
    preview: String,
    /// 预览之外还有更多内容（展开时需请求全文）
    more: bool,
}

#[derive(Serialize)]
pub struct ClipDetail {
    id: i64,
    kind: Kind,
    text: Option<String>,
    truncated: bool,
    image: Option<String>,
    files: Vec<String>,
}

fn image_url(name: &str) -> String {
    format!("{}://localhost/{name}.png", super::SCHEME)
}

fn to_item(c: Clip) -> ClipItem {
    let title = match c.kind {
        Kind::Text => c
            .content
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(TITLE_CHARS)
            .collect(),
        Kind::Image => format!("图片 {}", c.meta),
        Kind::Files => {
            let names: Vec<String> = c
                .content
                .lines()
                .map(|p| {
                    std::path::Path::new(p)
                        .file_name()
                        .map_or_else(|| p.to_string(), |n| n.to_string_lossy().to_string())
                })
                .collect();
            match names.len() {
                1 => names[0].clone(),
                n => format!("{} 等 {n} 个文件", names[0]),
            }
        }
    };
    let (preview, more) = match c.kind {
        Kind::Text => {
            let mut out = String::new();
            let mut more = false;
            for (i, line) in c.content.lines().enumerate() {
                if i >= PREVIEW_LINES || out.chars().count() + line.chars().count() > PREVIEW_CHARS {
                    more = true;
                    break;
                }
                if i > 0 {
                    out.push('\n');
                }
                out.push_str(line);
            }
            if out.is_empty() && !c.content.is_empty() {
                // 单行超长文本
                out = c.content.chars().take(PREVIEW_CHARS).collect();
                more = c.content.chars().count() > PREVIEW_CHARS;
            }
            (out, more)
        }
        Kind::Files => (c.content.clone(), false),
        Kind::Image => (String::new(), false),
    };
    ClipItem {
        thumb: (c.kind == Kind::Image).then(|| image_url(&format!("{}_t", c.content))),
        image: (c.kind == Kind::Image).then(|| image_url(&c.content)),
        preview,
        more,
        id: c.id,
        kind: c.kind,
        title,
        meta: c.meta,
        source_app: c.source_app,
        favorite: c.favorite,
        tags: c.tags,
        used_at: c.used_at,
        has_ocr: c.has_ocr,
    }
}

/// `filter`：all / text / image / files / favorite；`tag`：只看带该标签的收藏
#[tauri::command]
pub fn clipboard_list(
    query: String,
    filter: Option<String>,
    tag: Option<String>,
    limit: Option<u32>,
    svc: Svc<'_>,
) -> Result<Vec<ClipItem>, String> {
    let filter = Filter::parse(filter.as_deref().unwrap_or("all"));
    let clips = svc
        .store
        .lock()
        .unwrap()
        .list(&query, filter, tag.as_deref(), limit.unwrap_or(300).min(2000))
        .map_err(|e| e.to_string())?;
    Ok(clips.into_iter().map(to_item).collect())
}

#[tauri::command]
pub fn clipboard_tags(svc: Svc<'_>) -> Result<Vec<String>, String> {
    svc.store.lock().unwrap().tags().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn clipboard_detail(id: i64, svc: Svc<'_>) -> Result<ClipDetail, String> {
    let clip = svc.get(id)?;
    let mut d = ClipDetail {
        id,
        kind: clip.kind,
        text: None,
        truncated: false,
        image: None,
        files: Vec::new(),
    };
    match clip.kind {
        Kind::Text => {
            d.truncated = clip.content.chars().count() > DETAIL_CHARS;
            d.text = Some(clip.content.chars().take(DETAIL_CHARS).collect());
        }
        Kind::Image => d.image = Some(image_url(&clip.content)),
        Kind::Files => d.files = clip.content.lines().map(str::to_string).collect(),
    }
    Ok(d)
}

/// 粘贴（`paste`=true）或仅复制多条记录，按 `ids` 的顺序组合
#[tauri::command]
pub fn clipboard_paste(ids: Vec<i64>, paste: bool, app: AppHandle, svc: Svc<'_>) -> Result<(), String> {
    let mut contents = Vec::with_capacity(ids.len());
    for id in &ids {
        contents.push(svc.content_of(&svc.get(*id)?)?);
    }
    {
        let store = svc.store.lock().unwrap();
        for id in ids.iter().rev() {
            let _ = store.touch(*id);
        }
    }
    svc.deliver(&app, contents, paste)
}

/// 粘贴或复制任意文本（OCR 选中的文字、操作的输出等），同时记入剪贴板历史
#[tauri::command]
pub fn clipboard_paste_text(text: String, paste: bool, app: AppHandle, svc: Svc<'_>) -> Result<(), String> {
    svc.record_text(&app, &text, "wtools");
    svc.deliver(&app, vec![Content::Text(text)], paste)
}

#[tauri::command]
pub fn clipboard_delete(ids: Vec<i64>, svc: Svc<'_>) -> Result<(), String> {
    let mut store = svc.store.lock().unwrap();
    for id in ids {
        store.delete(id).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn clipboard_favorite(ids: Vec<i64>, favorite: bool, svc: Svc<'_>) -> Result<(), String> {
    let store = svc.store.lock().unwrap();
    for id in ids {
        store.set_favorite(id, favorite).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn clipboard_set_tags(id: i64, tags: Vec<String>, svc: Svc<'_>) -> Result<(), String> {
    svc.store
        .lock()
        .unwrap()
        .set_tags(id, &tags)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn clipboard_clear(svc: Svc<'_>) -> Result<(), String> {
    svc.store.lock().unwrap().clear().map_err(|e| e.to_string())
}

/// 识别图片中的文字；结果缓存到数据库，之后可直接显示，且图片可按文字被搜索到
#[tauri::command]
pub async fn clipboard_ocr(id: i64, svc: Svc<'_>) -> Result<OcrResult, String> {
    let svc = svc.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let clip = svc.get(id)?;
        if clip.kind != Kind::Image {
            return Err("只有图片可以识别文字".into());
        }
        if let Ok(Some(json)) = svc.store.lock().unwrap().ocr(id) {
            if let Ok(cached) = serde_json::from_str::<OcrResult>(&json) {
                return Ok(cached);
            }
        }
        let path = svc.image_path(&clip);
        if !path.exists() {
            return Err("图片文件已丢失".into());
        }
        let result = ocr::recognize_file(&path)?;
        if let Ok(json) = serde_json::to_string(&result) {
            let _ = svc.store.lock().unwrap().set_ocr(id, &json, &result.text());
        }
        Ok(result)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 文件记录：在访达中显示
#[tauri::command]
pub fn clipboard_reveal(id: i64, svc: Svc<'_>) -> Result<(), String> {
    let clip = svc.get(id)?;
    let path = match clip.kind {
        Kind::Files => clip.content.lines().next().unwrap_or_default().to_string(),
        Kind::Image => svc.image_path(&clip).to_string_lossy().into_owned(),
        Kind::Text => return Err("文本没有对应的文件".into()),
    };
    std::process::Command::new("open")
        .args(["-R", &path])
        .spawn()
        .map(|mut c| {
            std::thread::spawn(move || c.wait());
        })
        .map_err(|e| e.to_string())
}

#[derive(Serialize)]
pub struct ActionResult {
    /// 需要展示给用户的输出（output = show）
    output: Option<String>,
    message: Option<String>,
}

#[tauri::command]
pub async fn clipboard_run_action(
    action_id: String,
    id: i64,
    app: AppHandle,
    svc: Svc<'_>,
) -> Result<ActionResult, String> {
    let svc = svc.inner().clone();
    let action = app
        .state::<ConfigStore>()
        .get()
        .clipboard
        .actions
        .into_iter()
        .find(|a| a.id == action_id)
        .ok_or("操作不存在")?;
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let clip = svc.get(id)?;
        let input = match clip.kind {
            Kind::Text => Input { kind: "text", text: clip.content.clone(), file: None },
            Kind::Image => {
                let text = svc
                    .store
                    .lock()
                    .unwrap()
                    .ocr(id)
                    .ok()
                    .flatten()
                    .and_then(|j| serde_json::from_str::<OcrResult>(&j).ok())
                    .map(|r| r.text())
                    .unwrap_or_default();
                Input { kind: "image", text, file: Some(svc.image_path(&clip).to_string_lossy().into_owned()) }
            }
            Kind::Files => Input {
                kind: "files",
                text: clip.content.clone(),
                file: clip.content.lines().next().map(str::to_string),
            },
        };
        match actions::run(&action, &input)? {
            Outcome::Done => {
                crate::window::hide_launcher(&app2, false);
                Ok(ActionResult { output: None, message: None })
            }
            Outcome::Output(out) => match action.output.as_str() {
                "copy" => {
                    svc.deliver(&app2, vec![Content::Text(out)], false)?;
                    Ok(ActionResult { output: None, message: None })
                }
                "paste" => {
                    svc.deliver(&app2, vec![Content::Text(out)], true)?;
                    Ok(ActionResult { output: None, message: None })
                }
                "none" => Ok(ActionResult { output: None, message: Some("已执行".into()) }),
                _ => Ok(ActionResult { output: Some(out), message: None }),
            },
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn clipboard_test_action(action: CustomAction) -> Result<TestResult, String> {
    tauri::async_runtime::spawn_blocking(move || actions::test(&action))
        .await
        .map_err(|e| e.to_string())
}

// ---------------- 设置 ----------------

#[derive(Serialize, Deserialize)]
pub struct ClipboardSettings {
    enabled: bool,
    max_items: u32,
    max_days: u32,
    actions: Vec<CustomAction>,
    #[serde(default = "default_layout")]
    layout: String,
    #[serde(default = "default_max_lines")]
    max_lines: u32,
    /// 只读：当前实际使用的数据目录
    #[serde(default)]
    data_dir: String,
    /// 只读：是否为默认目录
    #[serde(default)]
    is_default_dir: bool,
}

fn default_layout() -> String {
    "dual".into()
}

fn default_max_lines() -> u32 {
    4
}

#[tauri::command]
pub fn clipboard_get_settings(store: State<'_, ConfigStore>, svc: Svc<'_>) -> ClipboardSettings {
    let c = store.get().clipboard;
    ClipboardSettings {
        enabled: c.enabled,
        max_items: c.max_items,
        max_days: c.max_days,
        actions: c.actions,
        layout: c.layout,
        max_lines: c.max_lines,
        data_dir: svc.current_dir().to_string_lossy().into_owned(),
        is_default_dir: c.data_dir.is_none(),
    }
}

#[tauri::command]
pub fn clipboard_set_settings(settings: ClipboardSettings, store: State<'_, ConfigStore>) -> Result<(), String> {
    for a in &settings.actions {
        if a.name.trim().is_empty() {
            return Err("操作名称不能为空".into());
        }
    }
    store.update(|c| {
        c.clipboard.enabled = settings.enabled;
        c.clipboard.max_items = settings.max_items.clamp(50, 100_000);
        c.clipboard.max_days = settings.max_days;
        c.clipboard.actions = settings.actions;
        c.clipboard.layout = if settings.layout == "single" { "single".into() } else { "dual".into() };
        c.clipboard.max_lines = settings.max_lines.clamp(1, 20);
    })
}

/// 弹出系统选择文件夹对话框；取消返回 None
#[tauri::command]
pub async fn clipboard_choose_dir() -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let out = std::process::Command::new("osascript")
            .args([
                "-e",
                r#"POSIX path of (choose folder with prompt "选择剪贴板数据目录（可选网盘目录，在多台电脑间同步）")"#,
            ])
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Ok(None); // 用户取消
        }
        let path = String::from_utf8_lossy(&out.stdout).trim().trim_end_matches('/').to_string();
        Ok((!path.is_empty()).then_some(path))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 切换数据目录；`dir` 为 None 时恢复默认目录
#[tauri::command]
pub fn clipboard_set_dir(dir: Option<String>, store: State<'_, ConfigStore>, svc: Svc<'_>) -> Result<String, String> {
    let msg = svc.switch_dir(dir.as_deref())?;
    store.update(|c| c.clipboard.data_dir = dir)?;
    Ok(msg)
}
