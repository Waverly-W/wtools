//! 系统级输入事件拦截（CGEventTap），每个拦截在自己的线程和 RunLoop 上运行
//!
//! 回调必须尽快返回：回调耗时过长时系统会停用拦截（这里会自动重新启用），期间全系统的对应输入都会卡顿。

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::Arc;
use std::sync::mpsc;

pub type EventRef = *mut c_void;

/// 处理函数：返回 None 表示吞掉事件，Some(event) 表示放行（可以是修改后的同一事件）
pub type Handler = fn(etype: u32, event: EventRef) -> Option<EventRef>;

pub const EV_KEY_DOWN: u32 = 10;
pub const EV_KEY_UP: u32 = 11;
pub const EV_FLAGS_CHANGED: u32 = 12;
pub const EV_SCROLL: u32 = 22;
const EV_TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
const EV_TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFF_FFFF;

/// 自己合成的事件打上这个标记（kCGEventSourceUserData），拦截时直接放行
pub const SYNTHETIC_MARK: i64 = 0x7774_6F6F; // "wtoo"
pub const FIELD_SOURCE_USER_DATA: u32 = 42;

#[link(name = "CoreGraphics", kind = "framework")]
#[link(name = "CoreFoundation", kind = "framework")]
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        mask: u64,
        callback: extern "C" fn(*mut c_void, u32, EventRef, *mut c_void) -> EventRef,
        user_info: *mut c_void,
    ) -> *mut c_void;
    fn CGEventTapEnable(tap: *mut c_void, enable: bool);
    pub fn CGEventGetIntegerValueField(event: EventRef, field: u32) -> i64;
    pub fn CGEventSetIntegerValueField(event: EventRef, field: u32, value: i64);
    pub fn CGEventGetDoubleValueField(event: EventRef, field: u32) -> f64;
    pub fn CGEventSetDoubleValueField(event: EventRef, field: u32, value: f64);
    pub fn CGEventGetFlags(event: EventRef) -> u64;
    pub fn CGEventSetFlags(event: EventRef, flags: u64);
    pub fn CGEventPost(tap: u32, event: EventRef);
    fn CFMachPortCreateRunLoopSource(alloc: *const c_void, port: *mut c_void, order: isize) -> *mut c_void;
    fn CFMachPortInvalidate(port: *mut c_void);
    fn CFRunLoopGetCurrent() -> *mut c_void;
    fn CFRunLoopAddSource(rl: *mut c_void, source: *mut c_void, mode: *const c_void);
    fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, return_after_source: bool) -> i32;
    static kCFRunLoopDefaultMode: *const c_void;
    fn CFRunLoopStop(rl: *mut c_void);
    pub fn CFRelease(obj: *const c_void);
    static kCFRunLoopCommonModes: *const c_void;
    fn AXIsProcessTrusted() -> bool;
}

pub const FLAG_SHIFT: u64 = 1 << 17;
pub const FLAG_CONTROL: u64 = 1 << 18;
pub const FLAG_ALTERNATE: u64 = 1 << 19;
pub const FLAG_COMMAND: u64 = 1 << 20;

/// 是否已获得「辅助功能」权限（拦截 / 修改输入事件需要）
pub fn accessibility_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

pub fn is_synthetic(event: EventRef) -> bool {
    unsafe { CGEventGetIntegerValueField(event, FIELD_SOURCE_USER_DATA) == SYNTHETIC_MARK }
}

struct Ctx {
    port: AtomicPtr<c_void>,
    handler: Handler,
}

extern "C" fn trampoline(_proxy: *mut c_void, etype: u32, event: EventRef, user: *mut c_void) -> EventRef {
    // SAFETY: user 指向 Tap 存活期间不释放的 Ctx
    let ctx = unsafe { &*(user as *const Ctx) };
    if etype == EV_TAP_DISABLED_BY_TIMEOUT || etype == EV_TAP_DISABLED_BY_USER_INPUT {
        let port = ctx.port.load(Ordering::Acquire);
        if !port.is_null() {
            unsafe { CGEventTapEnable(port, true) };
        }
        return event;
    }
    if event.is_null() || is_synthetic(event) {
        return event;
    }
    // 拦截线程是常驻线程，没有自动排空的 autorelease 池，逐个事件包一层
    objc2::rc::autoreleasepool(|_| (ctx.handler)(etype, event)).unwrap_or(std::ptr::null_mut())
}

#[derive(Clone, Copy)]
struct SendPtr(*mut c_void);
unsafe impl Send for SendPtr {}

/// 运行中的拦截，drop 时停止
pub struct Tap {
    run_loop: SendPtr,
    port: SendPtr,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Tap {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        unsafe { CGEventTapEnable(self.port.0, false) };
        if let Some(t) = self.thread.take() {
            // RunLoop 可能还没开始运行（此时 Stop 无效），反复发送直到线程退出
            while !t.is_finished() {
                unsafe { CFRunLoopStop(self.run_loop.0) };
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let _ = t.join();
        }
    }
}

/// 在新线程上创建拦截。`mask` 为事件类型位掩码（1 << 类型）
pub fn spawn(name: &str, mask: u64, handler: Handler) -> Result<Tap, String> {
    if !accessibility_trusted() {
        return Err("需要「辅助功能」权限".into());
    }
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let (tx, rx) = mpsc::channel::<Result<(SendPtr, SendPtr), String>>();
    let thread = std::thread::Builder::new()
        .name(format!("wtools-tap-{name}"))
        .spawn(move || unsafe {
            let ctx = Box::into_raw(Box::new(Ctx { port: AtomicPtr::new(std::ptr::null_mut()), handler }));
            // kCGSessionEventTap、kCGHeadInsertEventTap、kCGEventTapOptionDefault
            let port = CGEventTapCreate(1, 0, 0, mask, trampoline, ctx as *mut c_void);
            if port.is_null() {
                drop(Box::from_raw(ctx));
                let _ = tx.send(Err("无法拦截输入事件，请确认已在「辅助功能」中允许 wtools".into()));
                return;
            }
            (*ctx).port.store(port, Ordering::Release);
            let source = CFMachPortCreateRunLoopSource(std::ptr::null(), port, 0);
            let rl = CFRunLoopGetCurrent();
            CFRunLoopAddSource(rl, source, kCFRunLoopCommonModes);
            CGEventTapEnable(port, true);
            let _ = tx.send(Ok((SendPtr(rl), SendPtr(port))));
            // 空闲时一直阻塞，不定时唤醒；停止时由 Drop 发 CFRunLoopStop
            while !stop2.load(Ordering::SeqCst) {
                CFRunLoopRunInMode(kCFRunLoopDefaultMode, 1.0e10, false);
            }
            CFMachPortInvalidate(port);
            CFRelease(source);
            CFRelease(port);
            drop(Box::from_raw(ctx));
        })
        .map_err(|e| e.to_string())?;
    match rx.recv() {
        Ok(Ok((rl, port))) => Ok(Tap { run_loop: rl, port, stop, thread: Some(thread) }),
        Ok(Err(e)) => {
            let _ = thread.join();
            Err(e)
        }
        Err(_) => Err("拦截线程意外退出".into()),
    }
}
