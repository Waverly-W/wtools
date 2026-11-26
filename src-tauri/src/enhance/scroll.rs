//! 鼠标滚动增强（替代 Mos）：平滑滚动、独立于触控板的反向滚动
//!
//! 只处理传统鼠标滚轮（非连续事件）；触控板、妙控鼠标的连续滚动原样放行。
//! 平滑：吞掉原始滚轮事件，把距离累加到待滚动量，由动画线程按约 120Hz 逐帧补发像素滚动事件。

use super::tap::{self, EventRef, Tap};
use serde::{Deserialize, Serialize};
use std::ffi::c_void;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScrollConfig {
    pub enabled: bool,
    pub smooth: bool,
    /// 鼠标滚轮反向（不影响触控板）
    pub reverse: bool,
    /// 每次滚动的最小距离（像素）
    pub step: f64,
    /// 速度倍率：滚轮每行 10 像素乘以此倍率
    pub speed: f64,
    /// 过渡时长：越大越平滑、停得越慢
    pub duration: f64,
}

impl Default for ScrollConfig {
    fn default() -> Self {
        Self { enabled: false, smooth: true, reverse: true, step: 20.0, speed: 3.0, duration: 3.5 }
    }
}

const F_DELTA_1: u32 = 11;
const F_DELTA_2: u32 = 12;
const F_IS_CONTINUOUS: u32 = 88;
const F_FIXED_1: u32 = 93;
const F_FIXED_2: u32 = 94;
const F_POINT_1: u32 = 96;
const F_POINT_2: u32 = 97;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreateScrollWheelEvent2(
        source: *const c_void,
        units: u32,
        wheel_count: u32,
        wheel1: i32,
        wheel2: i32,
        wheel3: i32,
    ) -> EventRef;
}

static CONFIG: Mutex<Option<ScrollConfig>> = Mutex::new(None);
static TAP: Mutex<Option<Tap>> = Mutex::new(None);
static ERROR: Mutex<Option<String>> = Mutex::new(None);

#[derive(Default)]
struct Anim {
    /// 剩余待滚动距离（像素，正数向上 / 向左）
    y: f64,
    x: f64,
    /// 已滚动但不足 1 像素的部分
    carry_y: f64,
    carry_x: f64,
    flags: u64,
    started: bool,
}

static ANIM: Mutex<Anim> = Mutex::new(Anim { y: 0.0, x: 0.0, carry_y: 0.0, carry_x: 0.0, flags: 0, started: false });
static WAKE: Condvar = Condvar::new();

pub fn apply(cfg: &ScrollConfig) {
    // 只调参数时沿用已有的拦截，直接读新配置
    *CONFIG.lock().unwrap() = Some(cfg.clone());
    let mut current = TAP.lock().unwrap();
    if !cfg.enabled || !(cfg.smooth || cfg.reverse) {
        *current = None;
        *ERROR.lock().unwrap() = None;
        return;
    }
    if current.is_some() {
        return;
    }
    drop(current);
    *ERROR.lock().unwrap() = None;
    match tap::spawn("scroll", 1 << tap::EV_SCROLL, on_event) {
        Ok(t) => *TAP.lock().unwrap() = Some(t),
        Err(e) => {
            eprintln!("[scroll] {e}");
            *ERROR.lock().unwrap() = Some(e);
        }
    }
}

pub fn error() -> Option<String> {
    ERROR.lock().unwrap().clone()
}

fn on_event(_etype: u32, ev: EventRef) -> Option<EventRef> {
    unsafe {
        if tap::CGEventGetIntegerValueField(ev, F_IS_CONTINUOUS) != 0 {
            return Some(ev);
        }
        let Some(cfg) = CONFIG.lock().unwrap().clone() else { return Some(ev) };
        if !cfg.smooth {
            if cfg.reverse {
                for f in [F_DELTA_1, F_DELTA_2, F_POINT_1, F_POINT_2] {
                    tap::CGEventSetIntegerValueField(ev, f, -tap::CGEventGetIntegerValueField(ev, f));
                }
                for f in [F_FIXED_1, F_FIXED_2] {
                    tap::CGEventSetDoubleValueField(ev, f, -tap::CGEventGetDoubleValueField(ev, f));
                }
            }
            return Some(ev);
        }
        let lines = |fixed: u32, int: u32| {
            let v = tap::CGEventGetDoubleValueField(ev, fixed);
            if v != 0.0 { v } else { tap::CGEventGetIntegerValueField(ev, int) as f64 }
        };
        let sign = if cfg.reverse { -1.0 } else { 1.0 };
        let mut dy = lines(F_FIXED_1, F_DELTA_1) * sign;
        let mut dx = lines(F_FIXED_2, F_DELTA_2) * sign;
        let flags = tap::CGEventGetFlags(ev);
        // 平滑后发出的是像素滚动事件，系统不再把「⇧ + 滚轮」转成横向，这里自己转
        if flags & tap::FLAG_SHIFT != 0 && dx == 0.0 {
            (dx, dy) = (dy, 0.0);
        }
        let px = |l: f64| if l == 0.0 { 0.0 } else { l.signum() * (l.abs() * 10.0 * cfg.speed).max(cfg.step) };
        push(px(dy), px(dx), flags);
        None
    }
}

fn push(dy: f64, dx: f64, flags: u64) {
    let mut a = ANIM.lock().unwrap();
    // 反方向滚动时立即掉头，不等上一次滚完
    if dy != 0.0 && a.y * dy < 0.0 {
        a.y = 0.0;
    }
    if dx != 0.0 && a.x * dx < 0.0 {
        a.x = 0.0;
    }
    a.y += dy;
    a.x += dx;
    a.flags = flags;
    if !a.started {
        a.started = true;
        std::thread::Builder::new().name("wtools-scroll-anim".into()).spawn(animate).ok();
    }
    WAKE.notify_one();
}

/// 动画线程：空闲时阻塞等待，不占 CPU
fn animate() {
    const FRAME: Duration = Duration::from_micros(8_333);
    let mut last = Instant::now();
    let mut guard = ANIM.lock().unwrap();
    loop {
        while guard.y == 0.0 && guard.x == 0.0 {
            guard.carry_x = 0.0;
            guard.carry_y = 0.0;
            guard = WAKE.wait(guard).unwrap();
            last = Instant::now() - FRAME;
        }
        let duration = CONFIG.lock().unwrap().as_ref().map_or(3.5, |c| c.duration).clamp(1.0, 10.0);
        let now = Instant::now();
        let dt = (now - last).as_secs_f64().min(0.05);
        last = now;
        // 指数缓动：每帧走完剩余距离的一部分，tau 秒后剩约 37%
        let tau = duration * 0.025;
        let k = 1.0 - (-dt / tau).exp();
        let (sy, sx, flags) = {
            let a = &mut *guard;
            let step = |rem: &mut f64| {
                let s = if rem.abs() < 0.5 { *rem } else { *rem * k };
                *rem -= s;
                s
            };
            let sy = step(&mut a.y);
            let sx = step(&mut a.x);
            a.carry_y += sy;
            a.carry_x += sx;
            let iy = a.carry_y.trunc();
            let ix = a.carry_x.trunc();
            a.carry_y -= iy;
            a.carry_x -= ix;
            (iy as i32, ix as i32, a.flags)
        };
        drop(guard);
        if sy != 0 || sx != 0 {
            post(sy, sx, flags);
        }
        std::thread::sleep(FRAME);
        guard = ANIM.lock().unwrap();
    }
}

fn post(dy: i32, dx: i32, flags: u64) {
    unsafe {
        // kCGScrollEventUnitPixel：像素单位的连续滚动事件
        let ev = CGEventCreateScrollWheelEvent2(std::ptr::null(), 0, 2, dy, dx, 0);
        if ev.is_null() {
            return;
        }
        tap::CGEventSetIntegerValueField(ev, tap::FIELD_SOURCE_USER_DATA, tap::SYNTHETIC_MARK);
        tap::CGEventSetFlags(ev, flags);
        tap::CGEventPost(0, ev);
        tap::CFRelease(ev);
    }
}
