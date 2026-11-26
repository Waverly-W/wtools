//! 辅助功能（Accessibility）接口的薄封装：读取其他应用的窗口信息、监听窗口变化
//!
//! 对其他应用的 AX 调用是跨进程的，目标应用卡住时会阻塞，因此每个应用元素都设置了较短的超时。

use objc2_foundation::NSString;
use std::ffi::c_void;

pub type AXUIElementRef = *mut c_void;
pub type AXObserverRef = *mut c_void;
pub type CFTypeRef = *const c_void;

pub type AXObserverCallback = extern "C" fn(AXObserverRef, AXUIElementRef, CFTypeRef, *mut c_void);

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CGPoint {
    pub x: f64,
    pub y: f64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CGSize {
    pub width: f64,
    pub height: f64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CGRect {
    pub origin: CGPoint,
    pub size: CGSize,
}

#[link(name = "ApplicationServices", kind = "framework")]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(el: AXUIElementRef, attr: CFTypeRef, value: *mut CFTypeRef) -> i32;
    fn AXUIElementSetAttributeValue(el: AXUIElementRef, attr: CFTypeRef, value: CFTypeRef) -> i32;
    fn AXUIElementPerformAction(el: AXUIElementRef, action: CFTypeRef) -> i32;
    fn AXUIElementSetMessagingTimeout(el: AXUIElementRef, seconds: f32) -> i32;
    fn AXValueGetValue(value: CFTypeRef, kind: u32, out: *mut c_void) -> bool;
    fn AXObserverCreate(pid: i32, callback: AXObserverCallback, out: *mut AXObserverRef) -> i32;
    fn AXObserverAddNotification(obs: AXObserverRef, el: AXUIElementRef, name: CFTypeRef, refcon: *mut c_void) -> i32;
    fn AXObserverGetRunLoopSource(obs: AXObserverRef) -> *mut c_void;
    /// 私有但广泛使用（AltTab、Hammerspoon 等）：取 AX 窗口对应的 CGWindowID
    fn _AXUIElementGetWindow(el: AXUIElementRef, wid: *mut u32) -> i32;
    fn CFRetain(obj: CFTypeRef) -> CFTypeRef;
    fn CFRelease(obj: CFTypeRef);
    fn CFGetTypeID(obj: CFTypeRef) -> usize;
    fn CFArrayGetTypeID() -> usize;
    fn CFArrayGetCount(arr: CFTypeRef) -> isize;
    fn CFArrayGetValueAtIndex(arr: CFTypeRef, idx: isize) -> CFTypeRef;
    fn CFBooleanGetValue(b: CFTypeRef) -> bool;
    fn CFBooleanGetTypeID() -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFRunLoopGetMain() -> *mut c_void;
    fn CFRunLoopAddSource(rl: *mut c_void, source: *mut c_void, mode: CFTypeRef);
    fn CFRunLoopRemoveSource(rl: *mut c_void, source: *mut c_void, mode: CFTypeRef);
    static kCFRunLoopDefaultMode: CFTypeRef;
    static kCFBooleanFalse: CFTypeRef;
    static kCFBooleanTrue: CFTypeRef;
}

/// 拥有一个 CF 对象的引用，drop 时释放
pub struct Cf(CFTypeRef);

impl Clone for Cf {
    fn clone(&self) -> Self {
        unsafe { CFRetain(self.0) };
        Cf(self.0)
    }
}

impl Drop for Cf {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) }
    }
}

// AXUIElement 等 CF 对象可以跨线程使用
unsafe impl Send for Cf {}
unsafe impl Sync for Cf {}

fn cfstr(s: &str) -> objc2::rc::Retained<NSString> {
    NSString::from_str(s)
}

fn key(s: &NSString) -> CFTypeRef {
    s as *const NSString as CFTypeRef
}

/// 应用的 AX 元素
pub fn app(pid: i32) -> Option<Cf> {
    let el = unsafe { AXUIElementCreateApplication(pid) };
    if el.is_null() {
        return None;
    }
    unsafe { AXUIElementSetMessagingTimeout(el, 0.25) };
    Some(Cf(el))
}

pub fn attr(el: &Cf, name: &str) -> Option<Cf> {
    let mut v: CFTypeRef = std::ptr::null();
    let name = cfstr(name);
    let r = unsafe { AXUIElementCopyAttributeValue(el.0 as AXUIElementRef, key(&name), &mut v) };
    (r == 0 && !v.is_null()).then(|| Cf(v))
}

pub fn attr_bool(el: &Cf, name: &str) -> Option<bool> {
    let v = attr(el, name)?;
    unsafe { (CFGetTypeID(v.0) == CFBooleanGetTypeID()).then(|| CFBooleanGetValue(v.0)) }
}

pub fn attr_string(el: &Cf, name: &str) -> Option<String> {
    let v = attr(el, name)?;
    unsafe {
        if CFGetTypeID(v.0) != CFStringGetTypeID() {
            return None;
        }
        Some((*(v.0 as *const NSString)).to_string())
    }
}

pub fn attr_array(el: &Cf, name: &str) -> Vec<Cf> {
    let Some(v) = attr(el, name) else { return Vec::new() };
    unsafe {
        if CFGetTypeID(v.0) != CFArrayGetTypeID() {
            return Vec::new();
        }
        (0..CFArrayGetCount(v.0))
            .map(|i| {
                let item = CFArrayGetValueAtIndex(v.0, i);
                CFRetain(item);
                Cf(item)
            })
            .collect()
    }
}

pub fn set_bool(el: &Cf, name: &str, value: bool) -> bool {
    let name = cfstr(name);
    unsafe {
        let v = if value { kCFBooleanTrue } else { kCFBooleanFalse };
        AXUIElementSetAttributeValue(el.0 as AXUIElementRef, key(&name), v) == 0
    }
}

pub fn perform(el: &Cf, action: &str) -> bool {
    let a = cfstr(action);
    unsafe { AXUIElementPerformAction(el.0 as AXUIElementRef, key(&a)) == 0 }
}

/// 窗口位置和大小（全局坐标，原点在主屏左上角）
pub fn frame(el: &Cf) -> Option<CGRect> {
    let mut r = CGRect::default();
    unsafe {
        let p = attr(el, "AXPosition")?;
        let s = attr(el, "AXSize")?;
        // kAXValueCGPointType = 1，kAXValueCGSizeType = 2
        if !AXValueGetValue(p.0, 1, &mut r.origin as *mut _ as *mut c_void)
            || !AXValueGetValue(s.0, 2, &mut r.size as *mut _ as *mut c_void)
        {
            return None;
        }
    }
    Some(r)
}

pub fn window_id(el: &Cf) -> Option<u32> {
    let mut wid = 0u32;
    let r = unsafe { _AXUIElementGetWindow(el.0 as AXUIElementRef, &mut wid) };
    (r == 0 && wid != 0).then_some(wid)
}

/// 前台应用当前聚焦的窗口
pub fn focused_window(pid: i32) -> Option<Cf> {
    let a = app(pid)?;
    attr(&a, "AXFocusedWindow")
}

/// 监听某个应用的 AX 通知；回调在主线程 RunLoop 上执行。drop 时移除。
pub struct Observer {
    obs: Cf,
    source: *mut c_void,
}

impl Observer {
    /// 必须在主线程调用
    pub fn new(pid: i32, names: &[&str], callback: AXObserverCallback) -> Option<Self> {
        let app_el = app(pid)?;
        let mut obs: AXObserverRef = std::ptr::null_mut();
        unsafe {
            if AXObserverCreate(pid, callback, &mut obs) != 0 || obs.is_null() {
                return None;
            }
            let obs = Cf(obs);
            for n in names {
                let s = cfstr(n);
                AXObserverAddNotification(obs.0 as AXObserverRef, app_el.0 as AXUIElementRef, key(&s), std::ptr::null_mut());
            }
            let source = AXObserverGetRunLoopSource(obs.0 as AXObserverRef);
            CFRunLoopAddSource(CFRunLoopGetMain(), source, kCFRunLoopDefaultMode);
            Some(Self { obs, source })
        }
    }
}

impl Drop for Observer {
    fn drop(&mut self) {
        unsafe { CFRunLoopRemoveSource(CFRunLoopGetMain(), self.source, kCFRunLoopDefaultMode) };
        let _ = &self.obs;
    }
}
