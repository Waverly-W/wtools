//! 输入法自动切换（替代 Input Source Pro）：按应用切换输入法、窗口角落显示当前输入法
//!
//! 全部逻辑在主线程：前台应用切换（NSWorkspace 通知）、输入法变化（分布式通知）、
//! 窗口移动（AX 通知）都在主线程 RunLoop 上回调，状态放在线程局部变量里。

use super::ax::{self, CGRect};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSBox, NSBoxType, NSColor, NSFont, NSFontWeightMedium, NSImage, NSImageScaling,
    NSImageView, NSPanel, NSRunningApplication, NSTextField, NSTitlePosition, NSView, NSWindowCollectionBehavior,
    NSWindowStyleMask, NSWorkspace, NSWorkspaceApplicationKey, NSWorkspaceDidActivateApplicationNotification,
};
use objc2_foundation::{NSDictionary, NSNotification, NSNumber, NSPoint, NSRect, NSSize, NSString, NSURL};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::Once;
use tauri::AppHandle;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InputSourceConfig {
    pub enabled: bool,
    /// 没有单独规则的应用切换到此输入法；None 表示不切换
    pub default_source: Option<String>,
    /// 没有单独规则时，恢复该应用上次使用的输入法（优先于默认输入法）
    pub remember_last: bool,
    pub rules: Vec<InputRule>,
    /// 指示器："off" 不显示；"switch" 切换时短暂显示；"always" 常驻
    pub indicator: String,
    /// 指示器位置："top-left" | "top-right" | "bottom-left" | "bottom-right"
    pub corner: String,
    /// 输入法 ID → 指示器背景色（#rrggbb 或 #rrggbbaa）
    pub colors: BTreeMap<String, String>,
}

impl Default for InputSourceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            default_source: None,
            remember_last: false,
            rules: Vec::new(),
            indicator: "switch".into(),
            corner: "bottom-right".into(),
            colors: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputRule {
    pub bundle_id: String,
    pub name: String,
    /// None：跟随默认设置
    pub source: Option<String>,
}

/// 一个可选的输入法
#[derive(Debug, Clone, Serialize)]
pub struct Source {
    pub id: String,
    pub name: String,
    #[serde(skip)]
    icon_url: Option<String>,
}

// ---------- Text Input Sources ----------

type CFTypeRef = *const c_void;

#[link(name = "Carbon", kind = "framework")]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn TISCreateInputSourceList(props: CFTypeRef, include_all: bool) -> CFTypeRef;
    fn TISCopyCurrentKeyboardInputSource() -> CFTypeRef;
    fn TISSelectInputSource(src: CFTypeRef) -> i32;
    fn TISGetInputSourceProperty(src: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
    static kTISPropertyInputSourceID: CFTypeRef;
    static kTISPropertyLocalizedName: CFTypeRef;
    static kTISPropertyInputSourceIsSelectCapable: CFTypeRef;
    static kTISPropertyInputSourceCategory: CFTypeRef;
    static kTISCategoryKeyboardInputSource: CFTypeRef;
    static kTISPropertyIconImageURL: CFTypeRef;
    static kTISNotifySelectedKeyboardInputSourceChanged: CFTypeRef;
    fn CFArrayGetCount(arr: CFTypeRef) -> isize;
    fn CFArrayGetValueAtIndex(arr: CFTypeRef, idx: isize) -> CFTypeRef;
    fn CFRelease(obj: CFTypeRef);
    fn CFNotificationCenterGetDistributedCenter() -> *mut c_void;
    fn CFNotificationCenterAddObserver(
        center: *mut c_void,
        observer: *const c_void,
        callback: extern "C" fn(*mut c_void, *mut c_void, CFTypeRef, CFTypeRef, CFTypeRef),
        name: CFTypeRef,
        object: CFTypeRef,
        behavior: isize,
    );
}

unsafe fn ns<'a>(p: CFTypeRef) -> &'a NSString {
    unsafe { &*(p as *const NSString) }
}

unsafe fn source_info(src: CFTypeRef) -> Option<Source> {
    unsafe {
        let id = TISGetInputSourceProperty(src, kTISPropertyInputSourceID);
        if id.is_null() {
            return None;
        }
        let name = TISGetInputSourceProperty(src, kTISPropertyLocalizedName);
        let icon = TISGetInputSourceProperty(src, kTISPropertyIconImageURL);
        Some(Source {
            id: ns(id).to_string(),
            name: if name.is_null() { ns(id).to_string() } else { ns(name).to_string() },
            icon_url: (!icon.is_null()).then(|| (*(icon as *const NSURL)).absoluteString().map(|s| s.to_string())).flatten(),
        })
    }
}

/// 用 filter 查询输入法，对每个结果调用 f
fn query(filter: &NSDictionary<NSString, AnyObject>, mut f: impl FnMut(CFTypeRef)) {
    unsafe {
        let list = TISCreateInputSourceList(filter as *const _ as CFTypeRef, false);
        if list.is_null() {
            return;
        }
        for i in 0..CFArrayGetCount(list) {
            f(CFArrayGetValueAtIndex(list, i));
        }
        CFRelease(list);
    }
}

/// 已启用、可直接切换的键盘输入法
pub fn list_sources() -> Vec<Source> {
    let mut out = Vec::new();
    unsafe {
        let yes = NSNumber::new_bool(true);
        let filter = NSDictionary::from_slices::<NSString>(
            &[ns(kTISPropertyInputSourceIsSelectCapable), ns(kTISPropertyInputSourceCategory)],
            &[&*yes as &AnyObject, &*(kTISCategoryKeyboardInputSource as *const AnyObject)],
        );
        query(&filter, |s| out.extend(source_info(s)));
    }
    out
}

pub fn current_source() -> Option<Source> {
    unsafe {
        let s = TISCopyCurrentKeyboardInputSource();
        if s.is_null() {
            return None;
        }
        let info = source_info(s);
        CFRelease(s);
        info
    }
}

fn select_source(id: &str) {
    unsafe {
        let filter = NSDictionary::from_slices::<NSString>(
            &[ns(kTISPropertyInputSourceID)],
            &[&*NSString::from_str(id) as &AnyObject],
        );
        let mut done = false;
        query(&filter, |s| {
            if !done {
                done = true;
                TISSelectInputSource(s);
            }
        });
    }
}

// ---------- 状态（仅主线程） ----------

struct State {
    cfg: InputSourceConfig,
    /// 应用 → 上次使用的输入法
    last: HashMap<String, String>,
    front: Option<(i32, String)>,
    observer: Option<ax::Observer>,
    indicator: Option<Indicator>,
    icons: HashMap<String, Option<Retained<NSImage>>>,
    /// 每次显示递增，用于「短暂显示」的延时隐藏
    show_seq: u64,
    /// 切到 wtools 自己的窗口前正在用的输入法（仅当为 wtools 单独设置了输入法、发生了切换时记录），
    /// 回到其他应用且该应用没有可用的目标输入法时恢复它
    before_own: Option<String>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

static APP: std::sync::OnceLock<AppHandle> = std::sync::OnceLock::new();
static OBSERVERS: Once = Once::new();

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|s| s.borrow_mut().as_mut().map(f))
}

pub fn apply(app: &AppHandle, cfg: &InputSourceConfig) {
    let _ = APP.set(app.clone());
    let cfg = cfg.clone();
    let _ = app.run_on_main_thread(move || apply_main(cfg));
}

fn apply_main(cfg: InputSourceConfig) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let enabled = cfg.enabled;
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        match s.as_mut() {
            Some(st) => st.cfg = cfg,
            None => {
                *s = Some(State {
                    cfg,
                    last: HashMap::new(),
                    front: None,
                    observer: None,
                    indicator: None,
                    icons: HashMap::new(),
                    show_seq: 0,
                    before_own: None,
                })
            }
        }
    });
    if !enabled {
        with_state(|st| {
            st.observer = None;
            if let Some(i) = &st.indicator {
                i.panel.orderOut(None);
            }
        });
        return;
    }
    OBSERVERS.call_once(|| register_observers(mtm));
    // 以当前前台应用为起点
    let front = NSWorkspace::sharedWorkspace().frontmostApplication();
    if let Some(app) = front {
        on_activate(&app, false);
    }
}

fn register_observers(_mtm: MainThreadMarker) {
    unsafe {
        let center = NSWorkspace::sharedWorkspace().notificationCenter();
        let block = block2::RcBlock::new(|n: NonNull<NSNotification>| {
            let n = n.as_ref();
            let app = n
                .userInfo()
                .and_then(|info| info.objectForKey(NSWorkspaceApplicationKey))
                .and_then(|o| o.downcast::<NSRunningApplication>().ok());
            if let Some(app) = app {
                on_activate(&app, true);
            }
        });
        let token = center.addObserverForName_object_queue_usingBlock(
            Some(NSWorkspaceDidActivateApplicationNotification),
            None,
            None,
            &block,
        );
        // 与应用同生命周期
        std::mem::forget(token);
        // 4 = CFNotificationSuspensionBehaviorDeliverImmediately
        CFNotificationCenterAddObserver(
            CFNotificationCenterGetDistributedCenter(),
            std::ptr::null(),
            on_source_changed,
            kTISNotifySelectedKeyboardInputSourceChanged,
            std::ptr::null(),
            4,
        );
    }
}

/// 为该应用单独设置的输入法
fn rule_source(cfg: &InputSourceConfig, bundle: &str) -> Option<String> {
    cfg.rules.iter().find(|r| r.bundle_id == bundle).and_then(|r| r.source.clone())
}

fn on_activate(app: &NSRunningApplication, switched: bool) {
    let pid = app.processIdentifier();
    let bundle = app.bundleIdentifier().map(|s| s.to_string()).unwrap_or_default();
    let own = pid == std::process::id() as i32;
    let current = if own { current_source().map(|c| c.id) } else { None };
    let target = with_state(|st| {
        if !st.cfg.enabled {
            return None;
        }
        st.observer = None;
        if own {
            // 自己的窗口（搜索框、设置等）：只认单独设置的输入法，不套用默认输入法，也不记录「上次的输入法」
            st.front = None;
            let src = rule_source(&st.cfg, super::OWN_BUNDLE_ID);
            if src.is_some() && src != current && st.before_own.is_none() {
                st.before_own = current.clone();
            }
            return src;
        }
        let before_own = st.before_own.take();
        st.front = Some((pid, bundle.clone()));
        match rule_source(&st.cfg, &bundle) {
            Some(s) => Some(s),
            None if st.cfg.remember_last => st.last.get(&bundle).cloned().or_else(|| st.cfg.default_source.clone()),
            None => st.cfg.default_source.clone(),
        }
        // 这个应用没有目标输入法时，恢复进入 wtools 之前的输入法
        .or(before_own)
    })
    .flatten();
    if own {
        hide_indicator();
        if let Some(id) = &target {
            if current.as_ref() != Some(id) {
                select_source(id);
            }
        }
        return;
    }
    if !with_state(|st| st.cfg.enabled).unwrap_or(false) {
        return;
    }
    // 跟踪窗口变化，指示器跟着窗口走
    let observer = ax::Observer::new(
        pid,
        &["AXFocusedWindowChanged", "AXMainWindowChanged", "AXWindowMoved", "AXWindowResized", "AXWindowMiniaturized", "AXUIElementDestroyed"],
        on_window_changed,
    );
    with_state(|st| st.observer = observer);
    if let Some(id) = &target {
        if current_source().is_none_or(|c| &c.id != id) {
            select_source(id);
        }
    }
    refresh_indicator(switched || target.is_some());
}

extern "C" fn on_source_changed(_c: *mut c_void, _o: *mut c_void, _n: CFTypeRef, _obj: CFTypeRef, _info: CFTypeRef) {
    let Some(cur) = current_source() else { return };
    let enabled = with_state(|st| {
        if let Some((_, b)) = &st.front {
            st.last.insert(b.clone(), cur.id.clone());
        }
        st.cfg.enabled
    })
    .unwrap_or(false);
    if enabled {
        refresh_indicator(true);
    }
}

extern "C" fn on_window_changed(_o: ax::AXObserverRef, _el: ax::AXUIElementRef, _n: ax::CFTypeRef, _r: *mut c_void) {
    refresh_indicator(false);
}

// ---------- 指示器 ----------

struct Indicator {
    panel: Retained<NSPanel>,
    bg: Retained<NSBox>,
    icon: Retained<NSImageView>,
    label: Retained<NSTextField>,
}

const IND_H: f64 = 24.0;
const IND_PAD: f64 = 8.0;
const IND_ICON: f64 = 16.0;
const IND_MARGIN: f64 = 10.0;

impl Indicator {
    fn new(mtm: MainThreadMarker) -> Self {
        unsafe {
            let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(80.0, IND_H));
            let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
                NSPanel::alloc(mtm),
                rect,
                NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
                NSBackingStoreType::Buffered,
                false,
            );
            panel.setReleasedWhenClosed(false);
            panel.setLevel(25); // NSStatusWindowLevel
            panel.setOpaque(false);
            panel.setBackgroundColor(Some(&NSColor::clearColor()));
            panel.setHasShadow(true);
            panel.setIgnoresMouseEvents(true);
            panel.setHidesOnDeactivate(false);
            panel.setCollectionBehavior(
                NSWindowCollectionBehavior::CanJoinAllSpaces
                    | NSWindowCollectionBehavior::Stationary
                    | NSWindowCollectionBehavior::FullScreenAuxiliary
                    | NSWindowCollectionBehavior::IgnoresCycle,
            );
            let bg = NSBox::new(mtm);
            bg.setBoxType(NSBoxType::Custom);
            bg.setTitlePosition(NSTitlePosition::NoTitle);
            bg.setBorderWidth(0.0);
            // 与 DESIGN.md 对齐：圆角 --r-sm(6)，文字 --fs-sm(12) 中等字重
            bg.setCornerRadius(6.0);
            bg.setContentViewMargins(NSSize::new(0.0, 0.0));
            bg.setFrame(rect);
            let icon = NSImageView::new(mtm);
            icon.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
            icon.setContentTintColor(Some(&NSColor::whiteColor()));
            let label = NSTextField::labelWithString(&NSString::from_str(""), mtm);
            label.setFont(Some(&NSFont::systemFontOfSize_weight(12.0, NSFontWeightMedium)));
            label.setTextColor(Some(&NSColor::whiteColor()));
            if let Some(content) = bg.contentView() {
                content.addSubview(&icon);
                content.addSubview(&label);
            }
            panel.setContentView(Some(&bg as &NSView));
            Self { panel, bg, icon, label }
        }
    }

    fn update(&self, src: &Source, image: Option<&NSImage>, color: &NSColor) {
        self.bg.setFillColor(color);
        self.icon.setImage(image);
        self.icon.setHidden(image.is_none());
        self.label.setStringValue(&NSString::from_str(&src.name));
        self.label.sizeToFit();
        let lw = self.label.frame().size.width.ceil();
        let lh = self.label.frame().size.height;
        let mut x = IND_PAD;
        if image.is_some() {
            self.icon.setFrame(NSRect::new(NSPoint::new(x, (IND_H - IND_ICON) / 2.0), NSSize::new(IND_ICON, IND_ICON)));
            x += IND_ICON + 4.0;
        }
        self.label.setFrameOrigin(NSPoint::new(x, ((IND_H - lh) / 2.0).round()));
        let w = x + lw + IND_PAD;
        let mut f = self.panel.frame();
        f.size = NSSize::new(w, IND_H);
        self.panel.setFrame_display(f, false);
        self.bg.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), f.size));
    }

    /// 放到窗口指定角落（win 为左上角原点的全局坐标）
    fn place(&self, win: CGRect, corner: &str) {
        let main_h = crate::modules::screenshot::capture::main_display_height();
        let size = self.panel.frame().size;
        let left = win.origin.x + IND_MARGIN;
        let right = win.origin.x + win.size.width - size.width - IND_MARGIN;
        // AppKit 坐标原点在主屏左下角
        let bottom = main_h - (win.origin.y + win.size.height) + IND_MARGIN;
        let top = main_h - win.origin.y - size.height - IND_MARGIN;
        let (x, y) = match corner {
            "top-left" => (left, top),
            "top-right" => (right, top),
            "bottom-left" => (left, bottom),
            _ => (right, bottom),
        };
        self.panel.setFrameOrigin(NSPoint::new(x.round(), y.round()));
    }
}

fn parse_color(hex: &str) -> Option<Retained<NSColor>> {
    let h = hex.trim_start_matches('#');
    let v = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok().map(|n| n as f64 / 255.0);
    let a = if h.len() >= 8 { v(6)? } else { 1.0 };
    Some(NSColor::colorWithSRGBRed_green_blue_alpha(v(0)?, v(2)?, v(4)?, a))
}

fn hide_indicator() {
    with_state(|st| {
        if let Some(i) = &st.indicator {
            i.panel.orderOut(None);
        }
    });
}

/// 重新显示指示器。`flash` 为 true 表示刚发生切换（「短暂显示」模式下才显示）
fn refresh_indicator(flash: bool) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let Some((mode, corner, pid)) = with_state(|st| (st.cfg.indicator.clone(), st.cfg.corner.clone(), st.front.as_ref().map(|f| f.0))) else {
        return;
    };
    let visible = with_state(|st| st.indicator.as_ref().is_some_and(|i| i.panel.isVisible())).unwrap_or(false);
    if mode == "off" || (mode == "switch" && !flash && !visible) {
        return;
    }
    let (Some(pid), Some(src)) = (pid, current_source()) else {
        hide_indicator();
        return;
    };
    let frame = ax::focused_window(pid).and_then(|w| {
        // 最小化的窗口不显示
        if ax::attr_bool(&w, "AXMinimized") == Some(true) { None } else { ax::frame(&w) }
    });
    let Some(frame) = frame.filter(|f| f.size.width > 120.0 && f.size.height > 60.0) else {
        hide_indicator();
        return;
    };
    let seq = with_state(|st| {
        let image = st
            .icons
            .entry(src.id.clone())
            .or_insert_with(|| {
                let url = src.icon_url.as_deref().and_then(|u| NSURL::URLWithString(&NSString::from_str(u)))?;
                let img = NSImage::initWithContentsOfURL(NSImage::alloc(), &url)?;
                Some(img)
            })
            .clone();
        let color = st
            .cfg
            .colors
            .get(&src.id)
            .and_then(|c| parse_color(c))
            .unwrap_or_else(|| NSColor::colorWithSRGBRed_green_blue_alpha(0.0, 0.0, 0.0, 0.72));
        let ind = st.indicator.get_or_insert_with(|| Indicator::new(mtm));
        ind.update(&src, image.as_deref(), &color);
        ind.place(frame, &corner);
        ind.panel.orderFrontRegardless();
        st.show_seq += 1;
        st.show_seq
    });
    if mode == "switch" && flash {
        if let (Some(seq), Some(app)) = (seq, APP.get()) {
            let app = app.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(1500));
                let _ = app.run_on_main_thread(move || {
                    if with_state(|st| st.show_seq == seq).unwrap_or(false) {
                        hide_indicator();
                    }
                });
            });
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "读取真实输入法"]
    fn real_sources() {
        for s in super::list_sources() {
            println!("{} | {} | {:?}", s.id, s.name, s.icon_url);
        }
        println!("current: {:?}", super::current_source().map(|s| s.id));
    }
}
