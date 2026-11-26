//! 窗口切换器（替代 AltTab）：按住修饰键 + Tab 弹出窗口列表，松开修饰键切换到选中的窗口
//!
//! - 键盘事件在拦截线程上处理，只改状态、不做耗时操作
//! - 窗口列表在工作线程上读取：当前桌面的可见窗口按前后顺序（即最近使用顺序），最小化 / 隐藏的排在最后
//! - 界面延迟约 100ms 才显示：快速按一下 ⌃Tab 直接切到上一个窗口，不闪界面
//! - 界面是原生面板（见 switcher_ui），不抢焦点（orderFrontRegardless），按键仍由拦截线程处理

use super::ax::{self, Cf};
use super::switcher_ui as ui;
use super::tap::{self, EventRef, Tap};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSApplicationActivationPolicy, NSRunningApplication, NSWorkspace};
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString};
use serde::{Deserialize, Serialize};
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::AppHandle;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SwitcherConfig {
    pub enabled: bool,
    /// 按住的修饰键："ctrl" | "alt" | "cmd"
    pub modifier: String,
    /// 只显示当前屏幕上的窗口
    pub current_screen_only: bool,
    /// 最小化的窗口：true 排在最后显示，false 不显示
    pub show_minimized: bool,
    /// 已隐藏应用的窗口：true 排在最后显示，false 不显示
    pub show_hidden: bool,
    /// 不显示这些应用的窗口（Bundle ID）
    pub excluded: Vec<String>,
}

impl Default for SwitcherConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            modifier: "alt".into(),
            current_screen_only: true,
            show_minimized: true,
            show_hidden: true,
            excluded: Vec::new(),
        }
    }
}

const SHOW_DELAY: Duration = Duration::from_millis(100);

/// 列表中的一个窗口
#[derive(Debug, Clone)]
pub struct Item {
    pub title: String,
    pub app: String,
    /// 应用路径（取图标用）
    pub path: String,
    /// "minimized" | "hidden" | ""
    pub state: &'static str,
    pid: i32,
    wid: u32,
}

#[derive(Default)]
struct Session {
    open: bool,
    /// 窗口列表是否已读取完
    loaded: bool,
    items: Vec<Item>,
    selected: usize,
    /// 打开后反方向（⇧Tab）开始
    backwards: bool,
    /// 列表还没读完时用户就松开了修饰键
    commit_pending: bool,
    shown: bool,
    seq: u64,
    /// 界面所在屏幕（AppKit 坐标）
    screen: Option<ax::CGRect>,
}

static SESSION: Mutex<Session> = Mutex::new(Session {
    open: false,
    loaded: false,
    items: Vec::new(),
    selected: 0,
    backwards: false,
    commit_pending: false,
    shown: false,
    seq: 0,
    screen: None,
});
static SEQ: AtomicU64 = AtomicU64::new(0);
static CONFIG: Mutex<Option<SwitcherConfig>> = Mutex::new(None);
static TAP: Mutex<Option<Tap>> = Mutex::new(None);
static ERROR: Mutex<Option<String>> = Mutex::new(None);
static APP: OnceLock<AppHandle> = OnceLock::new();

pub fn apply(app: &AppHandle, cfg: &SwitcherConfig) {
    let _ = APP.set(app.clone());
    *CONFIG.lock().unwrap() = Some(cfg.clone());
    if !cfg.enabled {
        *TAP.lock().unwrap() = None;
        *ERROR.lock().unwrap() = None;
        let _ = app.run_on_main_thread(ui::destroy);
        return;
    }
    // 只改设置时沿用已有的拦截
    if TAP.lock().unwrap().is_some() {
        return;
    }
    *ERROR.lock().unwrap() = None;
    let mask = (1 << tap::EV_KEY_DOWN) | (1 << tap::EV_KEY_UP) | (1 << tap::EV_FLAGS_CHANGED);
    match tap::spawn("switcher", mask, on_event) {
        Ok(t) => *TAP.lock().unwrap() = Some(t),
        Err(e) => {
            eprintln!("[switcher] {e}");
            *ERROR.lock().unwrap() = Some(e);
            return;
        }
    }
    // 预热：系统窗口列表接口首次调用要几十毫秒，之后不到 1 毫秒
    std::thread::spawn(|| {
        let cfg = CONFIG.lock().unwrap().clone().unwrap_or_default();
        let apps = regular_apps();
        let (_, seen, _) = list_onscreen(&cfg, &apps);
        let _ = list_offscreen(&cfg, &apps, &seen);
    });
    // 预先创建界面面板
    let _ = app.run_on_main_thread(|| {
        if let Some(mtm) = objc2::MainThreadMarker::new() {
            ui::create(mtm);
        }
    });
}

pub fn error() -> Option<String> {
    ERROR.lock().unwrap().clone()
}

fn modifier_flag() -> u64 {
    match CONFIG.lock().unwrap().as_ref().map(|c| c.modifier.as_str()) {
        Some("ctrl") => tap::FLAG_CONTROL,
        Some("cmd") => tap::FLAG_COMMAND,
        _ => tap::FLAG_ALTERNATE,
    }
}

const KEY_TAB: i64 = 48;
const KEY_RETURN: i64 = 36;
const KEY_ESC: i64 = 53;
const KEY_LEFT: i64 = 123;
const KEY_RIGHT: i64 = 124;
const KEY_DOWN: i64 = 125;
const KEY_UP: i64 = 126;
const FIELD_KEYCODE: u32 = 9;

fn on_event(etype: u32, ev: EventRef) -> Option<EventRef> {
    let flags = unsafe { tap::CGEventGetFlags(ev) };
    let modifier = modifier_flag();
    let held = flags & modifier != 0;
    let mut s = SESSION.lock().unwrap();
    match etype {
        tap::EV_FLAGS_CHANGED => {
            if s.open && !held {
                drop(s);
                commit();
            }
            Some(ev)
        }
        tap::EV_KEY_DOWN | tap::EV_KEY_UP => {
            let key = unsafe { tap::CGEventGetIntegerValueField(ev, FIELD_KEYCODE) };
            if !s.open {
                // 只认「修饰键 + Tab」（可加 ⇧），其他修饰键组合放行
                let others = (tap::FLAG_CONTROL | tap::FLAG_ALTERNATE | tap::FLAG_COMMAND) & !modifier;
                if etype == tap::EV_KEY_DOWN && key == KEY_TAB && held && flags & others == 0 {
                    let backwards = flags & tap::FLAG_SHIFT != 0;
                    drop(s);
                    open(backwards);
                    return None;
                }
                return Some(ev);
            }
            // 打开期间吞掉所有按键
            if etype == tap::EV_KEY_DOWN {
                match key {
                    KEY_TAB => {
                        let back = flags & tap::FLAG_SHIFT != 0;
                        move_selection(&mut s, if back { -1 } else { 1 });
                    }
                    KEY_LEFT | KEY_UP => move_selection(&mut s, -1),
                    KEY_RIGHT | KEY_DOWN => move_selection(&mut s, 1),
                    KEY_ESC => {
                        drop(s);
                        cancel();
                    }
                    KEY_RETURN => {
                        drop(s);
                        commit();
                    }
                    _ => {}
                }
            }
            None
        }
        _ => Some(ev),
    }
}

fn move_selection(s: &mut Session, delta: isize) {
    if !s.loaded || s.items.is_empty() {
        // 列表还没读完：先记下方向和步数（用 selected 暂存）
        s.selected = (s.selected as isize + delta).max(0) as usize;
        return;
    }
    let n = s.items.len() as isize;
    s.selected = ((s.selected as isize + delta).rem_euclid(n)) as usize;
    emit_selected(s.selected);
}

fn emit_selected(i: usize) {
    if let Some(app) = APP.get() {
        let _ = app.run_on_main_thread(move || ui::select(i));
    }
}

fn open(backwards: bool) {
    let seq = SEQ.fetch_add(1, Ordering::SeqCst) + 1;
    {
        let mut s = SESSION.lock().unwrap();
        *s = Session { open: true, backwards, seq, ..Default::default() };
    }
    let started = Instant::now();
    std::thread::spawn(move || {
        let cfg = CONFIG.lock().unwrap().clone().unwrap_or_default();
        let apps = regular_apps();
        let (items, seen, screen) = list_onscreen(&cfg, &apps);
        let pending = {
            let mut s = SESSION.lock().unwrap();
            if s.seq != seq || !s.open {
                return;
            }
            let n = items.len();
            // 默认选中第二个（上一个使用的窗口）；⇧Tab 打开则从最后一个开始；期间多按的 Tab 也算上
            let extra = s.selected as isize;
            let start: isize = if s.backwards { -1 - extra } else { 1 + extra };
            s.selected = if n <= 1 { 0 } else { start.rem_euclid(n as isize) as usize };
            s.items = items;
            s.loaded = true;
            s.screen = screen;
            s.commit_pending
        };
        if pending {
            commit();
            return;
        }
        // 界面延迟显示期间并行查询最小化 / 隐藏的窗口，查完追加到末尾
        let offscreen = std::thread::spawn(move || {
            if !(cfg.show_minimized || cfg.show_hidden) {
                return;
            }
            let more = list_offscreen(&cfg, &apps, &seen);
            if more.is_empty() {
                return;
            }
            let update = {
                let mut s = SESSION.lock().unwrap();
                if s.seq != seq || !s.open {
                    return;
                }
                let was_empty = s.items.is_empty();
                s.items.extend(more);
                if was_empty && s.backwards {
                    s.selected = s.items.len() - 1;
                }
                s.shown.then(|| (s.items.clone(), s.selected))
            };
            match update {
                Some((items, selected)) => emit_items(seq, items, selected),
                // 只有最小化窗口时：等它们加载完再显示
                None => show(seq),
            }
        });
        if let Some(rest) = SHOW_DELAY.checked_sub(started.elapsed()) {
            std::thread::sleep(rest);
        }
        show(seq);
        let _ = offscreen;
    });
}

/// 在主线程上显示 / 更新界面
fn emit_items(seq: u64, items: Vec<Item>, selected: usize) {
    let Some(app) = APP.get() else { return };
    let _ = app.run_on_main_thread(move || {
        let s = SESSION.lock().unwrap();
        if s.seq != seq || !s.shown {
            return;
        }
        let screen = s.screen;
        drop(s);
        if let Some(scr) = screen {
            ui::present(items, selected, scr);
        }
    });
}

fn show(seq: u64) {
    let (items, selected) = {
        let mut s = SESSION.lock().unwrap();
        if s.seq != seq || !s.open || s.items.is_empty() || s.shown {
            return;
        }
        s.shown = true;
        (s.items.clone(), s.selected)
    };
    emit_items(seq, items, selected);
}

fn hide() {
    if let Some(app) = APP.get() {
        let _ = app.run_on_main_thread(ui::hide);
    }
}

fn cancel() {
    let shown = {
        let mut s = SESSION.lock().unwrap();
        let shown = s.shown;
        *s = Session { seq: s.seq, ..Default::default() };
        shown
    };
    if shown {
        hide();
    }
}

fn commit() {
    let target = {
        let mut s = SESSION.lock().unwrap();
        if !s.open {
            return;
        }
        if !s.loaded {
            s.commit_pending = true;
            return;
        }
        let t = s.items.get(s.selected).map(|i| (i.pid, i.wid));
        let shown = s.shown;
        *s = Session { seq: s.seq, ..Default::default() };
        t.map(|t| (t, shown))
    };
    let Some(((pid, wid), shown)) = target else {
        cancel();
        return;
    };
    if shown {
        hide();
    }
    std::thread::spawn(move || focus(pid, wid));
}

/// 点击某一行
pub fn pick(index: usize) {
    {
        let mut s = SESSION.lock().unwrap();
        if !s.open || index >= s.items.len() {
            return;
        }
        s.selected = index;
    }
    commit();
}

// ---------- 窗口列表 ----------

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> *mut c_void;
    fn CGEventCreate(source: *const c_void) -> *mut c_void;
    fn CGEventGetLocation(event: *const c_void) -> ax::CGPoint;
    fn CFRelease(obj: *const c_void);
}

struct AppInfo {
    name: String,
    bundle: String,
    path: String,
    hidden: bool,
}

fn regular_apps() -> Vec<(i32, AppInfo)> {
    let own = std::process::id() as i32;
    NSWorkspace::sharedWorkspace()
        .runningApplications()
        .iter()
        .filter(|a| a.activationPolicy() == NSApplicationActivationPolicy::Regular && a.processIdentifier() != own)
        .map(|a| {
            let path = a.bundleURL().and_then(|u| u.path()).map(|p| p.to_string()).unwrap_or_default();
            (
                a.processIdentifier(),
                AppInfo {
                    name: a.localizedName().map(|s| s.to_string()).unwrap_or_default(),
                    bundle: a.bundleIdentifier().map(|s| s.to_string()).unwrap_or_default(),
                    path,
                    hidden: a.isHidden(),
                },
            )
        })
        .collect()
}

/// 屏幕列表（AppKit 坐标，原点在主屏左下角）
fn cg_screens() -> Vec<ax::CGRect> {
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
        fn CGDisplayBounds(display: u32) -> ax::CGRect;
    }
    let main_h = crate::modules::screenshot::capture::main_display_height();
    let mut ids = [0u32; 16];
    let mut n = 0u32;
    unsafe { CGGetActiveDisplayList(16, ids.as_mut_ptr(), &mut n) };
    ids[..n as usize]
        .iter()
        .map(|&id| {
            let b = unsafe { CGDisplayBounds(id) };
            // 左上角原点 → 左下角原点
            ax::CGRect { origin: ax::CGPoint { x: b.origin.x, y: main_h - b.origin.y - b.size.height }, size: b.size }
        })
        .collect()
}

fn contains(r: &ax::CGRect, x: f64, y: f64) -> bool {
    x >= r.origin.x && x < r.origin.x + r.size.width && y >= r.origin.y && y < r.origin.y + r.size.height
}

/// 当前桌面的可见窗口（从前到后），返回（列表, 已列出的窗口 ID, 界面所在屏幕）
fn list_onscreen(cfg: &SwitcherConfig, apps: &[(i32, AppInfo)]) -> (Vec<Item>, Vec<u32>, Option<ax::CGRect>) {
    let app_of = |pid: i32| apps.iter().find(|(p, _)| *p == pid).map(|(_, a)| a);
    let main_h = crate::modules::screenshot::capture::main_display_height();
    let screens = cg_screens();
    // 默认用鼠标所在屏幕；有可见窗口时改用最前窗口所在屏幕
    let mouse = unsafe {
        let ev = CGEventCreate(std::ptr::null());
        let p = if ev.is_null() { ax::CGPoint::default() } else { CGEventGetLocation(ev) };
        if !ev.is_null() {
            CFRelease(ev);
        }
        p
    };
    let mut screen = screens.iter().find(|s| contains(s, mouse.x, main_h - mouse.y)).copied();
    let mut items = Vec::new();
    let mut seen = Vec::new();
    objc2::rc::autoreleasepool(|_| {
        // kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements：当前桌面、从前到后
        let raw = unsafe { CGWindowListCopyWindowInfo(1 | 16, 0) };
        let Some(list) = (unsafe { Retained::from_raw(raw as *mut NSArray<AnyObject>) }) else { return };
        let keys = ["kCGWindowLayer", "kCGWindowAlpha", "kCGWindowOwnerPID", "kCGWindowBounds", "kCGWindowNumber", "kCGWindowName", "X", "Y", "Width", "Height"]
            .map(NSString::from_str);
        let [k_layer, k_alpha, k_pid, k_bounds, k_number, k_name, k_x, k_y, k_w, k_h] = &keys;
        let num = |d: &NSDictionary<AnyObject, AnyObject>, k: &NSString| {
            d.objectForKey(k.as_ref()).and_then(|v| v.downcast_ref::<NSNumber>().map(|n| n.doubleValue()))
        };
        let mut first = true;
        for item in list.iter() {
            let Some(d) = item.downcast_ref::<NSDictionary>() else { continue };
            if num(d, k_layer).unwrap_or(1.0) != 0.0 || num(d, k_alpha).unwrap_or(1.0) <= 0.01 {
                continue;
            }
            let pid = num(d, k_pid).unwrap_or(0.0) as i32;
            let Some(app) = app_of(pid) else { continue };
            let Some(b) = d.objectForKey(k_bounds.as_ref()) else { continue };
            let Some(b) = b.downcast_ref::<NSDictionary>() else { continue };
            let (x, y, w, h) = (num(b, k_x).unwrap_or(0.0), num(b, k_y).unwrap_or(0.0), num(b, k_w).unwrap_or(0.0), num(b, k_h).unwrap_or(0.0));
            // 太小的一般是工具条、提示等辅助窗口
            if w < 100.0 || h < 60.0 {
                continue;
            }
            let (cx, cy) = (x + w / 2.0, main_h - (y + h / 2.0));
            if first {
                first = false;
                if let Some(s) = screens.iter().find(|s| contains(s, cx, cy)) {
                    screen = Some(*s);
                }
            }
            if cfg.current_screen_only && screen.is_some_and(|s| !contains(&s, cx, cy)) {
                continue;
            }
            if cfg.excluded.iter().any(|e| e == &app.bundle) {
                continue;
            }
            let wid = num(d, k_number).unwrap_or(0.0) as u32;
            seen.push(wid);
            let title = d
                .objectForKey(k_name.as_ref())
                .and_then(|v| v.downcast_ref::<NSString>().map(|s| s.to_string()))
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| app.name.clone());
            items.push(Item { title, app: app.name.clone(), path: app.path.clone(), state: "", pid, wid });
        }
    });
    (items, seen, screen.or_else(|| screens.first().copied()))
}

/// 最小化的窗口、已隐藏应用的窗口：要通过辅助功能接口逐个应用查询（每个应用十几到上百毫秒），各应用并行
fn list_offscreen(cfg: &SwitcherConfig, apps: &[(i32, AppInfo)], seen: &[u32]) -> Vec<Item> {
    let per_app: Vec<Vec<Item>> = std::thread::scope(|scope| {
        let handles: Vec<_> = apps
            .iter()
            .filter(|(_, a)| !cfg.excluded.iter().any(|e| e == &a.bundle) && (cfg.show_hidden || !a.hidden))
            .map(|(pid, app)| {
                scope.spawn(move || {
                    let mut out = Vec::new();
                    let Some(el) = ax::app(*pid) else { return out };
                    for w in ax::attr_array(&el, "AXWindows") {
                        let Some(wid) = ax::window_id(&w) else { continue };
                        if seen.contains(&wid) || ax::attr_string(&w, "AXSubrole").as_deref() != Some("AXStandardWindow") {
                            continue;
                        }
                        let state = if app.hidden {
                            "hidden"
                        } else if cfg.show_minimized && ax::attr_bool(&w, "AXMinimized") == Some(true) {
                            "minimized"
                        } else {
                            continue;
                        };
                        let title = ax::attr_string(&w, "AXTitle").filter(|t| !t.is_empty()).unwrap_or_else(|| app.name.clone());
                        out.push(Item { title, app: app.name.clone(), path: app.path.clone(), state, pid: *pid, wid });
                    }
                    out
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap_or_default()).collect()
    });
    let mut all: Vec<Item> = per_app.into_iter().flatten().collect();
    // 最小化的在前，隐藏应用的在后
    all.sort_by_key(|i| i.state == "hidden");
    all
}

// ---------- 切换到窗口 ----------

#[repr(C)]
#[derive(Default)]
struct ProcessSerialNumber {
    high: u32,
    low: u32,
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn GetProcessForPID(pid: i32, psn: *mut ProcessSerialNumber) -> i32;
}

type SetFrontFn = unsafe extern "C" fn(*const ProcessSerialNumber, u32, u32) -> i32;
type PostEventFn = unsafe extern "C" fn(*const ProcessSerialNumber, *const u8) -> i32;

/// SkyLight 私有接口（AltTab 同款）：把指定窗口设为前台应用的主窗口。找不到时返回 None，退回公开接口。
fn skylight() -> Option<(SetFrontFn, PostEventFn)> {
    static FNS: OnceLock<Option<(usize, usize)>> = OnceLock::new();
    let fns = FNS.get_or_init(|| unsafe {
        let h = libc::dlopen(c"/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight".as_ptr(), libc::RTLD_LAZY);
        if h.is_null() {
            return None;
        }
        let a = libc::dlsym(h, c"_SLPSSetFrontProcessWithOptions".as_ptr());
        let b = libc::dlsym(h, c"SLPSPostEventRecordTo".as_ptr());
        (!a.is_null() && !b.is_null()).then_some((a as usize, b as usize))
    });
    fns.map(|(a, b)| unsafe { (std::mem::transmute::<usize, SetFrontFn>(a), std::mem::transmute::<usize, PostEventFn>(b)) })
}

fn ax_window(pid: i32, wid: u32) -> Option<Cf> {
    let el = ax::app(pid)?;
    ax::attr_array(&el, "AXWindows").into_iter().find(|w| ax::window_id(w) == Some(wid))
}

fn focus(pid: i32, wid: u32) {
    let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) else { return };
    if app.isHidden() {
        app.unhide();
    }
    let win = ax_window(pid, wid);
    if let Some(w) = &win {
        if ax::attr_bool(w, "AXMinimized") == Some(true) {
            ax::set_bool(w, "AXMinimized", false);
        }
    }
    let mut psn = ProcessSerialNumber::default();
    let ok = unsafe { GetProcessForPID(pid, &mut psn) } == 0;
    match (ok, skylight()) {
        (true, Some((set_front, post))) => unsafe {
            // kCPSUserGenerated
            set_front(&psn, wid, 0x200);
            // 构造「设为关键窗口」的事件记录（与 AltTab 相同）
            let mut bytes = [0u8; 0xf8];
            bytes[0x04] = 0xf8;
            bytes[0x3a] = 0x10;
            bytes[0x3c..0x40].copy_from_slice(&wid.to_ne_bytes());
            bytes[0x20..0x30].fill(0xff);
            bytes[0x08] = 0x01;
            post(&psn, bytes.as_ptr());
            bytes[0x08] = 0x02;
            post(&psn, bytes.as_ptr());
        },
        _ => {
            #[allow(deprecated)]
            app.activateWithOptions(objc2_app_kit::NSApplicationActivationOptions::ActivateIgnoringOtherApps);
        }
    }
    if let Some(w) = &win {
        ax::perform(w, "AXRaise");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "读取真实窗口"]
    fn real_list() {
        let t = std::time::Instant::now();
        let cfg = super::SwitcherConfig { current_screen_only: false, ..Default::default() };
        let apps = super::regular_apps();
        let (mut items, seen, screen) = super::list_onscreen(&cfg, &apps);
        println!("可见窗口耗时 {:?}", t.elapsed());
        items.extend(super::list_offscreen(&cfg, &apps, &seen));
        println!("{} 个窗口，耗时 {:?}，屏幕 {:?}", items.len(), t.elapsed(), screen);
        for i in items.iter().take(15) {
            println!("{:>6} {:<10} {:<20} {}", i.wid, i.state, i.app, i.title.chars().take(40).collect::<String>());
        }
    }
}
